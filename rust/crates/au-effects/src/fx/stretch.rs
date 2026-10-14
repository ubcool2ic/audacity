//! Time and pitch changing effects.
//!
//! The WSOLA and sliding-stretch implementations are self-contained
//! approximations of Audacity's SoundTouch/SBSMS processing. Paulstretch uses
//! a deterministic per-channel LCG for randomized FFT phases rather than the
//! process-global `rand()` state used by Audacity.

use crate::core::*;
use au_core::AudioBuffer;
use rustfft::{num_complex::Complex, FftPlanner};
use std::f64::consts::PI;

/// Change playback speed and pitch together by resampling.
#[derive(Clone, Debug)]
pub struct ChangeSpeed {
    /// Percentage change in speed; zero leaves the audio unchanged.
    pub percentage: f64,
}

impl Default for ChangeSpeed {
    fn default() -> Self {
        Self { percentage: 0.0 }
    }
}

impl BuiltinEffect for ChangeSpeed {
    fn id(&self) -> &'static str {
        "ChangeSpeed"
    }

    fn name(&self) -> &'static str {
        "Change Speed and Pitch"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![ParamDef::float(
            "Percentage",
            "Percentage",
            -99.0,
            4900.0,
            0.0,
        )]
    }

    fn get(&self, key: &str) -> Option<f64> {
        (key == "Percentage").then_some(self.percentage)
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        if key != "Percentage" {
            return false;
        }
        self.percentage = value.clamp(-99.0, 4900.0);
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let ratio = 100.0 / (100.0 + self.percentage);
        if ratio == 1.0 || buf.len() == 0 {
            return Ok(());
        }
        let output_len = ((buf.len() as f64 * ratio).round() as usize).max(0);
        for channel in &mut buf.channels {
            *channel = resample_to_length(channel, output_len, ratio);
        }
        Ok(())
    }
}

/// Change tempo without changing pitch using WSOLA-style overlap-add.
#[derive(Clone, Debug)]
pub struct ChangeTempo {
    /// Tempo change in percent.
    pub percentage: f64,
    /// Retained for automation compatibility; SBSMS is not used by this port.
    pub use_sbsms: bool,
}

impl Default for ChangeTempo {
    fn default() -> Self {
        Self {
            percentage: 0.0,
            use_sbsms: false,
        }
    }
}

impl BuiltinEffect for ChangeTempo {
    fn id(&self) -> &'static str {
        "ChangeTempo"
    }

    fn name(&self) -> &'static str {
        "Change Tempo"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Percentage", "Percentage", -99.0, 4900.0, 0.0),
            ParamDef::boolean("UseSBSMS", "Use SBSMS", false),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Percentage" => Some(self.percentage),
            "UseSBSMS" => Some(if self.use_sbsms { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Percentage" => {
                self.percentage = value.clamp(-99.0, 4900.0);
                true
            }
            "UseSBSMS" => {
                self.use_sbsms = value >= 0.5;
                true
            }
            _ => false,
        }
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if buf.len() == 0 || self.percentage == 0.0 {
            return Ok(());
        }
        let tempo = 1.0 + self.percentage / 100.0;
        stretch_channels(&mut buf.channels, buf.sample_rate, tempo);
        Ok(())
    }
}

/// Change pitch while approximately preserving duration.
#[derive(Clone, Debug)]
pub struct ChangePitch {
    /// Pitch change in percent.
    pub percentage: f64,
    /// Retained for automation compatibility; SBSMS is not used by this port.
    pub use_sbsms: bool,
}

impl Default for ChangePitch {
    fn default() -> Self {
        Self {
            percentage: 0.0,
            use_sbsms: false,
        }
    }
}

impl BuiltinEffect for ChangePitch {
    fn id(&self) -> &'static str {
        "ChangePitch"
    }

    fn name(&self) -> &'static str {
        "Change Pitch"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Percentage", "Percentage", -99.0, 4900.0, 0.0),
            ParamDef::boolean("UseSBSMS", "Use SBSMS", false),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Percentage" => Some(self.percentage),
            "UseSBSMS" => Some(if self.use_sbsms { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Percentage" => {
                self.percentage = value.clamp(-99.0, 4900.0);
                true
            }
            "UseSBSMS" => {
                self.use_sbsms = value >= 0.5;
                true
            }
            _ => false,
        }
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let pitch_ratio = 1.0 + self.percentage / 100.0;
        let original_len = buf.len();
        if original_len == 0 || pitch_ratio == 1.0 {
            return Ok(());
        }

        let mut stretched = buf.channels.clone();
        stretch_channels(&mut stretched, buf.sample_rate, pitch_ratio);
        for (dst, src) in buf.channels.iter_mut().zip(stretched.iter()) {
            *dst = resample_to_length(src, original_len, 1.0 / pitch_ratio);
        }
        Ok(())
    }
}

/// Extreme time-stretch and time-smear effect.
#[derive(Clone, Debug)]
pub struct Paulstretch {
    /// Stretch factor.
    pub stretch_factor: f64,
    /// Analysis time resolution in seconds.
    pub time_resolution: f64,
}

impl Default for Paulstretch {
    fn default() -> Self {
        Self {
            stretch_factor: 10.0,
            time_resolution: 0.25,
        }
    }
}

impl BuiltinEffect for Paulstretch {
    fn id(&self) -> &'static str {
        "Paulstretch"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Stretch Factor", "Stretch Factor", 1.0, 100.0, 10.0),
            ParamDef::float("Time Resolution", "Time Resolution", 0.01, 10.0, 0.25),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Stretch Factor" => Some(self.stretch_factor),
            "Time Resolution" => Some(self.time_resolution),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Stretch Factor" => {
                self.stretch_factor = value.clamp(1.0, 100.0);
                true
            }
            "Time Resolution" => {
                self.time_resolution = value.clamp(0.01, 10.0);
                true
            }
            _ => false,
        }
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let n = buf.len();
        if n == 0 || buf.sample_rate == 0 {
            return Ok(());
        }

        let fft_size = paulstretch_fft_size(buf.sample_rate, self.time_resolution);
        // Audacity reports selections shorter than two analysis windows as
        // invalid. Returning them unchanged makes short selections safe.
        if n < fft_size.saturating_mul(2).saturating_add(1) {
            return Ok(());
        }

        let half = fft_size;
        let pool_size = half.saturating_mul(2);
        let adjusted_factor =
            1.0 + (self.stretch_factor - 1.0) * n as f64 / (n - pool_size) as f64;
        let factor = adjusted_factor.max(1.0);
        for (channel_index, channel) in buf.channels.iter_mut().enumerate() {
            *channel = paulstretch_channel(
                channel,
                buf.sample_rate,
                half,
                factor,
                channel_index as u32,
            );
        }
        Ok(())
    }
}

/// Continuously vary tempo and pitch over the selection.
///
/// This port approximates SBSMS by processing short, overlapping-independent
/// segments whose parameters are interpolated across the source.
#[derive(Clone, Debug)]
pub struct SlidingStretch {
    /// Tempo change at the beginning of the selection, in percent.
    pub rate_percent_change_start: f64,
    /// Tempo change at the end of the selection, in percent.
    pub rate_percent_change_end: f64,
    /// Pitch change at the beginning of the selection, in percent.
    pub pitch_percent_change_start: f64,
    /// Pitch change at the end of the selection, in percent.
    pub pitch_percent_change_end: f64,
}

impl Default for SlidingStretch {
    fn default() -> Self {
        Self {
            rate_percent_change_start: 0.0,
            rate_percent_change_end: 0.0,
            pitch_percent_change_start: 0.0,
            pitch_percent_change_end: 0.0,
        }
    }
}

impl BuiltinEffect for SlidingStretch {
    fn id(&self) -> &'static str {
        "SlidingStretch"
    }

    fn name(&self) -> &'static str {
        "Sliding stretch"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float(
                "RatePercentChangeStart",
                "Tempo change start",
                -99.0,
                4900.0,
                0.0,
            ),
            ParamDef::float(
                "RatePercentChangeEnd",
                "Tempo change end",
                -99.0,
                4900.0,
                0.0,
            ),
            ParamDef::float(
                "PitchPercentChangeStart",
                "Pitch change start",
                -99.0,
                4900.0,
                0.0,
            ),
            ParamDef::float(
                "PitchPercentChangeEnd",
                "Pitch change end",
                -99.0,
                4900.0,
                0.0,
            ),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "RatePercentChangeStart" => Some(self.rate_percent_change_start),
            "RatePercentChangeEnd" => Some(self.rate_percent_change_end),
            "PitchPercentChangeStart" => Some(self.pitch_percent_change_start),
            "PitchPercentChangeEnd" => Some(self.pitch_percent_change_end),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let value = value.clamp(-99.0, 4900.0);
        match key {
            "RatePercentChangeStart" => self.rate_percent_change_start = value,
            "RatePercentChangeEnd" => self.rate_percent_change_end = value,
            "PitchPercentChangeStart" => self.pitch_percent_change_start = value,
            "PitchPercentChangeEnd" => self.pitch_percent_change_end = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let source_len = buf.len();
        if source_len == 0 {
            return Ok(());
        }
        if self.rate_percent_change_start == 0.0
            && self.rate_percent_change_end == 0.0
            && self.pitch_percent_change_start == 0.0
            && self.pitch_percent_change_end == 0.0
        {
            return Ok(());
        }

        let segment_frames = ((buf.sample_rate as usize).max(1) / 2).max(256);
        let mut output = vec![Vec::new(); buf.channels.len()];
        let mut start = 0usize;
        while start < source_len {
            let end = (start + segment_frames).min(source_len);
            let progress = (start as f64 + (end - start) as f64 * 0.5) / source_len as f64;
            let tempo_percent = lerp(
                self.rate_percent_change_start,
                self.rate_percent_change_end,
                progress,
            );
            let pitch_percent = lerp(
                self.pitch_percent_change_start,
                self.pitch_percent_change_end,
                progress,
            );
            let tempo_ratio = 1.0 + tempo_percent / 100.0;
            let pitch_ratio = 1.0 + pitch_percent / 100.0;

            let mut piece: Vec<Vec<f32>> = buf
                .channels
                .iter()
                .map(|channel| channel[start..end].to_vec())
                .collect();
            stretch_channels(&mut piece, buf.sample_rate, tempo_ratio);
            if pitch_ratio != 1.0 {
                let pitch_len = piece.first().map_or(0, Vec::len);
                for channel in &mut piece {
                    *channel = resample_to_length(channel, pitch_len, 1.0 / pitch_ratio);
                }
            }
            for (dst, src) in output.iter_mut().zip(piece.iter()) {
                dst.extend_from_slice(src);
            }
            start = end;
        }
        buf.channels = output;
        Ok(())
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn stretch_channels(channels: &mut [Vec<f32>], sample_rate: u32, tempo_ratio: f64) {
    if channels.is_empty() || channels[0].is_empty() || tempo_ratio == 1.0 {
        return;
    }
    let input_len = channels[0].len();
    let target_len = ((input_len as f64 / tempo_ratio).round() as usize).max(1);
    let sequence = ((sample_rate as usize * 103) / 1000).max(32);
    let overlap = ((sample_rate as usize * 8) / 1000)
        .max(8)
        .min(sequence.saturating_sub(1));
    let seek = ((sample_rate as usize * 28) / 1000).max(1);

    let mut output = vec![Vec::<f32>::new(); channels.len()];
    for (out, input) in output.iter_mut().zip(channels.iter()) {
        *out = wsola_channel(input, target_len, tempo_ratio, sequence, overlap, seek);
    }
    channels.clone_from_slice(&output);
}

fn wsola_channel(
    input: &[f32],
    target_len: usize,
    tempo_ratio: f64,
    sequence: usize,
    overlap: usize,
    seek: usize,
) -> Vec<f32> {
    if input.is_empty() || target_len == 0 {
        return Vec::new();
    }
    if input.len() < sequence || overlap == 0 || tempo_ratio <= 0.0 {
        return linear_resample(input, target_len);
    }
    if (tempo_ratio - 1.0).abs() < 1.0e-12 {
        return input.to_vec();
    }

    let step = sequence - overlap;
    let mut output = vec![0.0f32; sequence];
    for i in 0..sequence {
        output[i] = input.get(i).copied().unwrap_or(0.0);
    }

    let max_start = input.len().saturating_sub(sequence);
    let mut previous_start = 0usize;
    let mut guard = 0usize;
    while output.len() < target_len && guard < target_len.saturating_add(16) {
        guard += 1;
        let expected = previous_start as f64 + step as f64 * tempo_ratio;
        let nominal = expected.round() as isize;
        let low = (nominal - seek as isize).max(0) as usize;
        let high = (nominal + seek as isize).max(0) as usize;
        let low = low.min(max_start);
        let high = high.min(max_start).max(low);
        let stride = (seek / 64).max(1);
        let mut best_start = low;
        let mut best_score = f64::NEG_INFINITY;

        let mut candidate = low;
        while candidate <= high {
            let score = correlation(&output, input, candidate, overlap);
            if score > best_score {
                best_score = score;
                best_start = candidate;
            }
            match candidate.checked_add(stride) {
                Some(next) if next > candidate => candidate = next,
                _ => break,
            }
        }
        let refine_low = best_start.saturating_sub(stride);
        let refine_high = best_start.saturating_add(stride).min(high);
        for candidate in refine_low..=refine_high {
            let score = correlation(&output, input, candidate, overlap);
            if score > best_score {
                best_score = score;
                best_start = candidate;
            }
        }

        let tail_start = output.len().saturating_sub(overlap);
        for i in 0..overlap {
            let source = input.get(best_start + i).copied().unwrap_or(0.0);
            let fraction = (i + 1) as f32 / (overlap + 1) as f32;
            let out_index = tail_start + i;
            if out_index < output.len() {
                output[out_index] = output[out_index] * (1.0 - fraction) + source * fraction;
            }
        }
        for i in overlap..sequence {
            output.push(input.get(best_start + i).copied().unwrap_or(0.0));
        }
        previous_start = best_start;
    }

    output.truncate(target_len);
    if output.len() < target_len {
        output.resize(target_len, 0.0);
    }
    output
}

fn correlation(output: &[f32], input: &[f32], candidate: usize, overlap: usize) -> f64 {
    let tail_start = output.len().saturating_sub(overlap);
    let mut score = 0.0f64;
    for i in 0..overlap {
        let a = output.get(tail_start + i).copied().unwrap_or(0.0) as f64;
        let b = input.get(candidate + i).copied().unwrap_or(0.0) as f64;
        score += a * b;
    }
    score
}

fn linear_resample(input: &[f32], output_len: usize) -> Vec<f32> {
    if output_len == 0 || input.is_empty() {
        return Vec::new();
    }
    if input.len() == 1 {
        return vec![input[0]; output_len];
    }
    let scale = input.len() as f64 / output_len as f64;
    (0..output_len)
        .map(|i| {
            let position = (i as f64 * scale).min((input.len() - 1) as f64);
            let left = position.floor() as usize;
            let right = (left + 1).min(input.len() - 1);
            let fraction = (position - left as f64) as f32;
            input[left] * (1.0 - fraction) + input[right] * fraction
        })
        .collect()
}

fn resample_to_length(input: &[f32], output_len: usize, output_per_input: f64) -> Vec<f32> {
    if output_len == 0 || input.is_empty() {
        return Vec::new();
    }
    if output_per_input == 1.0 && output_len == input.len() {
        return input.to_vec();
    }

    let cutoff = 0.95 * output_per_input.min(1.0);
    let radius = 64isize;
    let beta = 8.6;
    let denominator = bessel_i0(beta);
    let mut output = Vec::with_capacity(output_len);

    for out_index in 0..output_len {
        let position = out_index as f64 / output_per_input;
        let center = position.floor() as isize;
        let mut sum = 0.0f64;
        let mut weight_sum = 0.0f64;
        for tap in (center - radius + 1)..=(center + radius) {
            let distance = position - tap as f64;
            let normalized = distance / radius as f64;
            if normalized.abs() > 1.0 {
                continue;
            }
            let window = bessel_i0(beta * (1.0 - normalized * normalized).sqrt()) / denominator;
            let x = distance * cutoff;
            let sinc = if x.abs() < 1.0e-12 {
                1.0
            } else {
                (PI * x).sin() / (PI * x)
            };
            let weight = cutoff * sinc * window;
            let sample = if tap < 0 || tap >= input.len() as isize {
                0.0
            } else {
                input[tap as usize] as f64
            };
            sum += sample * weight;
            weight_sum += weight;
        }
        output.push(if weight_sum.abs() > 1.0e-15 {
            (sum / weight_sum) as f32
        } else {
            0.0
        });
    }
    output
}

fn bessel_i0(x: f64) -> f64 {
    let y = x * x / 4.0;
    let mut sum = 1.0;
    let mut term = 1.0;
    for k in 1..40 {
        term *= y / (k as f64 * k as f64);
        sum += term;
        if term < sum * 1.0e-16 {
            break;
        }
    }
    sum
}

fn paulstretch_fft_size(sample_rate: u32, time_resolution: f64) -> usize {
    let value = sample_rate as f64 * time_resolution / 2.0;
    if !value.is_finite() || value <= 0.0 {
        return 128;
    }
    let rounded_power = 2.0f64.powf((value.log2() + 0.5).floor());
    if !rounded_power.is_finite() || rounded_power > (1usize << 20) as f64 {
        return 1usize << 20;
    }
    (rounded_power as usize).max(128)
}

fn paulstretch_channel(
    input: &[f32],
    _sample_rate: u32,
    half: usize,
    amount: f64,
    channel_index: u32,
) -> Vec<f32> {
    let pool_size = half * 2;
    let output_size = half.max(8);
    let mut input_pool = vec![0.0f32; pool_size];
    let mut old_output = vec![0.0f32; output_size * 2];
    let mut remainder = 0.0f64;
    let mut position = 0usize;
    let mut first_time = true;
    let fade_len = 100usize.min(pool_size / 2 - 1);
    let mut output = Vec::new();
    let mut rng = 0xA341_316Cu32 ^ channel_index.wrapping_mul(0x9E37_79B9);

    let mut nget = pool_size;
    while position < input.len() {
        let count = nget.min(pool_size);
        if count < pool_size {
            input_pool.copy_within(count.., 0);
        }
        let available = input.len().saturating_sub(position).min(count);
        let destination = pool_size - count;
        input_pool[destination..destination + count].fill(0.0);
        if available > 0 {
            input_pool[destination..destination + available]
                .copy_from_slice(&input[position..position + available]);
        }

        let mut generated = paulstretch_frame(
            &input_pool,
            output_size,
            &mut old_output,
            &mut rng,
            amount as f32,
        );
        if first_time {
            // Audacity runs an additional zero-input frame before using the
            // first output block.
            generated = paulstretch_frame(
                &input_pool,
                output_size,
                &mut old_output,
                &mut rng,
                amount as f32,
            );
        }

        position = position.saturating_add(count);
        if first_time {
            first_time = false;
            for i in 0..fade_len.min(generated.len()).min(input.len()) {
                let fraction = i as f32 / fade_len.max(1) as f32;
                generated[i] =
                    generated[i] * fraction + input[i] * (1.0 - fraction);
            }
        }
        if position >= input.len() {
            for i in 0..fade_len.min(generated.len()) {
                let index = output_size / 2 - 1 - i;
                if index < generated.len() {
                    let fraction = i as f32 / fade_len.max(1) as f32;
                    let source_index = input.len().saturating_sub(1 + i);
                    generated[index] = generated[index] * fraction
                        + input[source_index] * (1.0 - fraction);
                }
            }
        }
        output.extend_from_slice(&generated);

        let exact = output_size as f64 / amount.max(1.0);
        let integral = exact.floor() as usize;
        remainder += exact - exact.floor();
        nget = integral;
        if remainder >= 1.0 {
            nget += remainder.floor() as usize;
            remainder -= remainder.floor();
        }
        nget = nget.min(pool_size);
        if nget == 0 {
            nget = 1;
        }
    }
    output
}

fn paulstretch_frame(
    input_pool: &[f32],
    output_size: usize,
    old_output: &mut [f32],
    rng: &mut u32,
    amount: f32,
) -> Vec<f32> {
    let n = input_pool.len();
    let mut planner = FftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(n);
    let inverse = planner.plan_fft_inverse(n);
    let mut spectrum: Vec<Complex<f32>> = input_pool
        .iter()
        .enumerate()
        .map(|(i, &sample)| {
            let window = if n <= 1 {
                1.0
            } else {
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos()
            };
            Complex::new(sample * window, 0.0)
        })
        .collect();
    forward.process(&mut spectrum);

    let mut magnitudes = vec![0.0f32; n / 2 + 1];
    for i in 0..=n / 2 {
        magnitudes[i] = spectrum[i].norm();
    }
    spectrum.fill(Complex::new(0.0, 0.0));
    for i in 1..n / 2 {
        *rng = rng.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let random = (*rng >> 16) & 0x7fff;
        let phase = random as f32 * (std::f32::consts::PI / 16384.0);
        let value = Complex::from_polar(magnitudes[i], phase);
        spectrum[i] = value;
        spectrum[n - i] = value.conj();
    }

    inverse.process(&mut spectrum);
    let normalization = 1.0 / n as f32;
    let tmp = std::f32::consts::PI / output_size as f32;
    let hinv_sqrt2 = 0.853_553_4f32;
    let amplitude = if amount < 1.0 {
        amount * 0.707
    } else {
        (output_size as f32 / n as f32) * 4.0
    };

    let mut output = vec![0.0f32; output_size];
    for i in 0..output_size {
        let a = 0.5 + 0.5 * (i as f32 * tmp).cos();
        let current = spectrum.get(i + output_size).map_or(0.0, |v| v.re * normalization);
        let old = old_output.get(i).copied().unwrap_or(0.0);
        let blended = current * (1.0 - a) + old * a;
        let synthesis = hinv_sqrt2 - (1.0 - hinv_sqrt2) * (i as f32 * 2.0 * tmp).cos();
        output[i] = blended * synthesis * amplitude;
    }

    for i in 0..old_output.len() {
        old_output[i] = spectrum.get(i).map_or(0.0, |v| v.re * normalization);
    }
    output
}

/// Returns one default instance of each effect in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![
        Box::new(ChangeSpeed::default()),
        Box::new(ChangeTempo::default()),
        Box::new(ChangePitch::default()),
        Box::new(Paulstretch::default()),
        Box::new(SlidingStretch::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{set_param, EffectContext};

    fn sine_buffer(frames: usize) -> AudioBuffer {
        let left: Vec<f32> = (0..frames)
            .map(|i| (2.0 * PI * 440.0 * i as f64 / 44_100.0).sin() as f32)
            .collect();
        let right: Vec<f32> = (0..frames)
            .map(|i| (2.0 * PI * 660.0 * i as f64 / 44_100.0).sin() as f32)
            .collect();
        AudioBuffer::from_channels(44_100, vec![left, right])
    }

    fn run(effect: &mut dyn BuiltinEffect, buf: &mut AudioBuffer) {
        effect.process(buf, &mut EffectContext::default()).unwrap();
    }

    #[test]
    fn defaults_run_short_stereo_and_empty_buffers() {
        for mut effect in all() {
            let mut short = sine_buffer(512);
            run(effect.as_mut(), &mut short);
            assert_eq!(short.channels.len(), 2);

            let mut empty = AudioBuffer::from_channels(44_100, vec![vec![], vec![]]);
            run(effect.as_mut(), &mut empty);
            assert_eq!(empty.len(), 0);
        }
    }

    #[test]
    fn repeated_processing_is_deterministic() {
        for mut effect in all() {
            let mut a = sine_buffer(512);
            let mut b = sine_buffer(512);
            run(effect.as_mut(), &mut a);
            run(effect.as_mut(), &mut b);
            assert_eq!(a.channels, b.channels);
        }
    }

    #[test]
    fn parameter_clamping_works() {
        let mut speed = ChangeSpeed::default();
        assert!(set_param(&mut speed, "Percentage", -10_000.0));
        assert_eq!(speed.percentage, -99.0);
        assert!(set_param(&mut speed, "Percentage", 10_000.0));
        assert_eq!(speed.percentage, 4900.0);

        let mut paul = Paulstretch::default();
        assert!(set_param(&mut paul, "Stretch Factor", 1000.0));
        assert_eq!(paul.stretch_factor, 100.0);
        assert!(set_param(&mut paul, "Time Resolution", 0.0));
        assert_eq!(paul.time_resolution, 0.01);

        let mut tempo = ChangeTempo::default();
        assert!(set_param(&mut tempo, "UseSBSMS", 1.0));
        assert!(tempo.use_sbsms);

        let mut sliding = SlidingStretch::default();
        assert!(set_param(
            &mut sliding,
            "PitchPercentChangeEnd",
            10_000.0
        ));
        assert_eq!(sliding.pitch_percent_change_end, 4900.0);
    }

    #[test]
    fn speed_and_tempo_change_duration() {
        let mut speed = ChangeSpeed {
            percentage: 100.0,
        };
        let mut audio = sine_buffer(1000);
        run(&mut speed, &mut audio);
        assert_eq!(audio.len(), 500);

        let mut tempo = ChangeTempo {
            percentage: 100.0,
            use_sbsms: false,
        };
        let mut audio = sine_buffer(1000);
        run(&mut tempo, &mut audio);
        assert_eq!(audio.len(), 500);
    }

    #[test]
    fn pitch_preserves_duration_and_sliding_changes_duration() {
        let mut pitch = ChangePitch {
            percentage: 50.0,
            use_sbsms: false,
        };
        let mut audio = sine_buffer(1000);
        run(&mut pitch, &mut audio);
        assert_eq!(audio.len(), 1000);

        let mut sliding = SlidingStretch {
            rate_percent_change_start: 50.0,
            rate_percent_change_end: 50.0,
            ..SlidingStretch::default()
        };
        let mut audio = sine_buffer(44_100);
        run(&mut sliding, &mut audio);
        assert!(audio.len() < 44_100);
    }

    #[test]
    fn paulstretch_short_selection_is_safe_and_unchanged() {
        let mut paul = Paulstretch::default();
        let mut audio = sine_buffer(512);
        let original = audio.channels.clone();
        run(&mut paul, &mut audio);
        assert_eq!(audio.channels, original);
    }
}
