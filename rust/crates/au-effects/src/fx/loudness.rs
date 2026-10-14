// SPDX-License-Identifier: GPL-3.0-or-later
//! Loudness normalization and loudness measurement effects.

use crate::core::*;
use au_core::AudioBuffer;

const NORMALIZE_TARGETS: &[&str] = &["perceived loudness (LUFS)", "RMS"];

/// Normalizes audio to a perceived loudness or RMS target.
pub struct LoudnessNormalization {
    /// Normalize stereo channels independently.
    pub stereo_independent: bool,
    /// Target integrated loudness, in LUFS.
    pub lufs_level: f64,
    /// Target RMS level, in dB.
    pub rms_level: f64,
    /// Treat mono material as dual mono when normalizing loudness.
    pub dual_mono: bool,
    /// Normalization target: 0 for LUFS, 1 for RMS.
    pub normalize_to: usize,
}

impl Default for LoudnessNormalization {
    fn default() -> Self {
        Self {
            stereo_independent: false,
            lufs_level: -23.0,
            rms_level: -20.0,
            dual_mono: true,
            normalize_to: 0,
        }
    }
}

impl LoudnessNormalization {
    fn measure_channels_energy(channels: &[&[f32]], sample_rate: u32) -> f64 {
        if channels.is_empty() {
            return 0.0;
        }

        let mut ebur128 = EbuR128::new(sample_rate as f64, channels.len());
        let frames = channels.iter().map(|channel| channel.len()).min().unwrap_or(0);
        for frame in 0..frames {
            for (channel_index, channel) in channels.iter().enumerate() {
                ebur128.process_sample(channel[frame], channel_index);
            }
            ebur128.next_sample();
        }
        ebur128.integrated_energy()
    }

    fn channel_rms(channel: &[f32]) -> f64 {
        if channel.is_empty() {
            return 0.0;
        }
        let sum = channel
            .iter()
            .map(|&sample| {
                let sample = sample as f64;
                sample * sample
            })
            .sum::<f64>();
        (sum / channel.len() as f64).sqrt()
    }

    fn process_loudness_gain(
        &self,
        channels: &[&[f32]],
        sample_rate: u32,
        is_stereo_independent: bool,
    ) -> Result<f64, EffectError> {
        let energy = Self::measure_channels_energy(channels, sample_rate);
        if energy == 0.0 {
            return Err(EffectError::Invalid(
                "Cannot normalize silent audio to a loudness target".to_string(),
            ));
        }

        let target_energy = 10.0_f64.powf(self.lufs_level / 10.0);
        let mut ratio = target_energy / energy;

        // Audacity treats each independently processed stereo channel as
        // dual-mono. For linked processing, only mono material uses this
        // adjustment, and only when the option is enabled.
        if channels.len() == 1
            && (self.dual_mono || is_stereo_independent)
        {
            ratio /= 2.0;
        }

        Ok(ratio.sqrt())
    }

    fn process_rms_gain(&self, channels: &[&[f32]]) -> Result<f64, EffectError> {
        let rms = if channels.len() > 1 {
            let power_sum = channels
                .iter()
                .map(|channel| {
                    let value = Self::channel_rms(channel);
                    value * value
                })
                .sum::<f64>();
            (power_sum / channels.len() as f64).sqrt()
        } else {
            Self::channel_rms(channels[0])
        };

        if rms == 0.0 {
            return Err(EffectError::Invalid(
                "Cannot normalize silent audio to an RMS target".to_string(),
            ));
        }

        Ok(10.0_f64.powf(self.rms_level / 20.0) / rms)
    }

    fn gain_for_channels(
        &self,
        channels: &[&[f32]],
        sample_rate: u32,
        is_stereo_independent: bool,
    ) -> Result<f64, EffectError> {
        if self.normalize_to == 0 {
            self.process_loudness_gain(channels, sample_rate, is_stereo_independent)
        } else {
            self.process_rms_gain(channels)
        }
    }
}

impl BuiltinEffect for LoudnessNormalization {
    fn id(&self) -> &'static str {
        "LoudnessNormalization"
    }

    fn name(&self) -> &'static str {
        "Loudness Normalization"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::boolean("StereoIndependent", "Normalize stereo channels independently", false),
            ParamDef::float("LUFSLevel", "Loudness target (LUFS)", -36.0, 0.0, -23.0),
            ParamDef::float("RMSLevel", "RMS target (dB)", -145.0, 0.0, -20.0),
            ParamDef::boolean("DualMono", "Treat mono as dual-mono", true),
            ParamDef::choice("NormalizeTo", "Normalize to", NORMALIZE_TARGETS, 0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "StereoIndependent" => Some(if self.stereo_independent { 1.0 } else { 0.0 }),
            "LUFSLevel" => Some(self.lufs_level),
            "RMSLevel" => Some(self.rms_level),
            "DualMono" => Some(if self.dual_mono { 1.0 } else { 0.0 }),
            "NormalizeTo" => Some(self.normalize_to as f64),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let parameter = match key {
            "StereoIndependent" => ParamDef::boolean(
                "StereoIndependent",
                "Normalize stereo channels independently",
                false,
            ),
            "LUFSLevel" => ParamDef::float("LUFSLevel", "Loudness target (LUFS)", -36.0, 0.0, -23.0),
            "RMSLevel" => ParamDef::float("RMSLevel", "RMS target (dB)", -145.0, 0.0, -20.0),
            "DualMono" => ParamDef::boolean("DualMono", "Treat mono as dual-mono", true),
            "NormalizeTo" => ParamDef::choice("NormalizeTo", "Normalize to", NORMALIZE_TARGETS, 0),
            _ => return false,
        };
        let value = parameter.clamp(value);
        match key {
            "StereoIndependent" => self.stereo_independent = value != 0.0,
            "LUFSLevel" => self.lufs_level = value,
            "RMSLevel" => self.rms_level = value,
            "DualMono" => self.dual_mono = value != 0.0,
            "NormalizeTo" => self.normalize_to = value as usize,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if buf.channels.is_empty() || buf.len() == 0 {
            return Ok(());
        }

        let channel_count = buf.channels.len();
        let frames = buf.len();

        if self.stereo_independent && channel_count > 1 {
            let mut gains = Vec::with_capacity(channel_count);
            for channel in &buf.channels {
                gains.push(self.gain_for_channels(
                    &[channel.as_slice()],
                    buf.sample_rate,
                    true,
                )?);
            }
            for (channel, gain) in buf.channels.iter_mut().zip(gains) {
                for sample in channel.iter_mut().take(frames) {
                    *sample = (*sample as f64 * gain) as f32;
                }
            }
        } else {
            let refs: Vec<&[f32]> = buf.channels.iter().map(Vec::as_slice).collect();
            let gain = self.gain_for_channels(&refs, buf.sample_rate, false)?;
            for channel in &mut buf.channels {
                for sample in channel.iter_mut().take(frames) {
                    *sample = (*sample as f64 * gain) as f32;
                }
            }
        }

        Ok(())
    }
}

/// Measures integrated loudness using EBU R128 K-weighting and gating.
///
/// The returned value is in LUFS. Silence returns negative infinity.
pub fn integrated_loudness(buf: &AudioBuffer) -> f64 {
    if buf.channels.is_empty() || buf.len() == 0 {
        return f64::NEG_INFINITY;
    }

    let channels: Vec<&[f32]> = buf.channels.iter().map(Vec::as_slice).collect();
    let energy = LoudnessNormalization::measure_channels_energy(&channels, buf.sample_rate);
    if energy <= 0.0 {
        f64::NEG_INFINITY
    } else {
        10.0 * energy.log10()
    }
}

/// Analyzer that reports integrated LUFS, RMS level, and peak level.
pub struct LoudnessMeasure;

impl Default for LoudnessMeasure {
    fn default() -> Self {
        Self
    }
}

impl BuiltinEffect for LoudnessMeasure {
    fn id(&self) -> &'static str {
        "MeasureLoudness"
    }

    fn name(&self) -> &'static str {
        "Loudness"
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Analyze
    }

    fn params(&self) -> Vec<ParamDef> {
        Vec::new()
    }

    fn get(&self, _key: &str) -> Option<f64> {
        None
    }

    fn set(&mut self, _key: &str, _value: f64) -> bool {
        false
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let loudness = integrated_loudness(buf);
        let mut square_sum = 0.0_f64;
        let mut count = 0_u64;
        let mut peak = 0.0_f64;

        for channel in &buf.channels {
            for &sample in channel {
                let sample = sample as f64;
                square_sum += sample * sample;
                peak = peak.max(sample.abs());
                count += 1;
            }
        }

        let rms_db = if count == 0 || square_sum == 0.0 {
            f64::NEG_INFINITY
        } else {
            20.0 * (square_sum / count as f64).sqrt().log10()
        };
        let peak_db = if peak == 0.0 {
            f64::NEG_INFINITY
        } else {
            20.0 * peak.log10()
        };

        ctx.report = format!(
            "Integrated loudness: {} LUFS\nRMS level: {} dB\nPeak level: {} dBFS",
            format_db(loudness),
            format_db(rms_db),
            format_db(peak_db)
        );
        Ok(())
    }
}

fn format_db(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.2}")
    } else if value.is_sign_negative() {
        "-inf".to_string()
    } else {
        "inf".to_string()
    }
}

const HIST_BIN_COUNT: usize = 65_536;
const GAMMA_A: f64 = (-70.0 + 0.691) / 10.0;

#[derive(Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    fn new(b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, input: f64) -> f64 {
        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

fn weighting_filters(sample_rate: f64) -> [Biquad; 2] {
    let fs = sample_rate.max(1.0);

    // High-shelf pre-filter.
    let db = 3.999_843_853_973_347_f64;
    let f0 = 1_681.974_450_955_533_f64;
    let q = 0.707_175_236_955_419_6_f64;
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let vh = 10.0_f64.powf(db / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad::new(
        (vh + vb * k / q + k * k) / a0,
        2.0 * (k * k - vh) / a0,
        (vh - vb * k / q + k * k) / a0,
        2.0 * (k * k - 1.0) / a0,
        (1.0 - k / q + k * k) / a0,
    );

    // High-pass weighting filter.
    let f0 = 38.135_470_876_024_44_f64;
    let q = 0.500_327_037_323_877_3_f64;
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let high_pass = Biquad::new(
        1.0 / a0,
        -2.0 / a0,
        1.0 / a0,
        2.0 * (k * k - 1.0) / a0,
        (1.0 - k / q + k * k) / a0,
    );

    [shelf, high_pass]
}

struct EbuR128 {
    hist: Vec<u64>,
    block_ring: Vec<f64>,
    ring_pos: usize,
    ring_size: usize,
    block_size: usize,
    block_overlap: usize,
    filters: Vec<[Biquad; 2]>,
}

impl EbuR128 {
    fn new(sample_rate: f64, channels: usize) -> Self {
        let rate = sample_rate.max(1.0);
        let block_size = (0.4 * rate).ceil().max(1.0) as usize;
        let block_overlap = (0.1 * rate).ceil().max(1.0) as usize;
        Self {
            hist: vec![0; HIST_BIN_COUNT],
            block_ring: vec![0.0; block_size],
            ring_pos: 0,
            ring_size: 0,
            block_size,
            block_overlap,
            filters: (0..channels).map(|_| weighting_filters(rate)).collect(),
        }
    }

    fn process_sample(&mut self, input: f32, channel: usize) {
        if channel >= self.filters.len() {
            return;
        }
        let value = self.filters[channel][0].process(input as f64);
        let value = self.filters[channel][1].process(value);
        let power = value * value;
        if channel == 0 {
            self.block_ring[self.ring_pos] = power;
        } else {
            self.block_ring[self.ring_pos] += power;
        }
    }

    fn next_sample(&mut self) {
        self.ring_pos += 1;
        self.ring_size = self.ring_size.saturating_add(1);

        if self.ring_pos % self.block_overlap == 0 && self.ring_size >= self.block_size {
            self.add_block(self.block_size);
        }

        if self.ring_pos == self.block_size {
            self.ring_pos = 0;
        }
    }

    fn add_block(&mut self, valid_len: usize) {
        self.ring_size = self.block_size;
        let len = valid_len.min(self.block_size);
        if len == 0 {
            return;
        }

        let block_power = self.block_ring[..len].iter().sum::<f64>() / len as f64;
        let block_value = block_power.log10();
        let index_value = ((block_value - GAMMA_A) * HIST_BIN_COUNT as f64 / -GAMMA_A - 1.0)
            .round();

        if index_value.is_finite() && index_value >= 0.0 && index_value < HIST_BIN_COUNT as f64 {
            self.hist[index_value as usize] += 1;
        }
    }

    fn histogram_sums(&self, start_idx: usize) -> (f64, u64) {
        let mut sum_value = 0.0;
        let mut sum_count = 0_u64;
        for i in start_idx.min(HIST_BIN_COUNT)..HIST_BIN_COUNT {
            let value = -GAMMA_A / HIST_BIN_COUNT as f64 * (i + 1) as f64 + GAMMA_A;
            let count = self.hist[i];
            sum_value += 10.0_f64.powf(value) * count as f64;
            sum_count += count;
        }
        (sum_value, sum_count)
    }

    fn integrated_energy(&mut self) -> f64 {
        let (_sum_value, sum_count) = self.histogram_sums(0);

        // Like Audacity's EBUR128 implementation, include a short selection's
        // partial block only when there are no accepted full blocks.
        if sum_count == 0 {
            self.add_block(self.ring_size.min(self.block_size));
        }

        let (sum_value, sum_count) = self.histogram_sums(0);
        if sum_count == 0 {
            return 0.0;
        }

        let gamma_r = (sum_value / sum_count as f64).log10() - 1.0;
        let index_r = ((gamma_r - GAMMA_A) * HIST_BIN_COUNT as f64 / -GAMMA_A - 1.0).round();

        let start_idx = if index_r.is_finite() && index_r >= 0.0 {
            (index_r as usize).saturating_add(1)
        } else {
            // A negative unsigned threshold in the C++ implementation makes
            // the gated histogram empty.
            HIST_BIN_COUNT
        };
        let (gated_sum, gated_count) = self.histogram_sums(start_idx);
        if gated_count == 0 {
            return 0.0;
        }

        0.852_903_703_1 * gated_sum / gated_count as f64
    }
}

/// Returns the default loudness normalization and measurement effects.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![
        Box::new(LoudnessNormalization::default()),
        Box::new(LoudnessMeasure),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{get_params, set_param};

    fn sine_buffer(frames: usize) -> AudioBuffer {
        let sample_rate = 48_000;
        let left: Vec<f32> = (0..frames)
            .map(|i| (2.0 * std::f64::consts::PI * 440.0 * i as f64 / sample_rate as f64).sin() as f32 * 0.1)
            .collect();
        let right: Vec<f32> = (0..frames)
            .map(|i| (2.0 * std::f64::consts::PI * 440.0 * i as f64 / sample_rate as f64).sin() as f32 * 0.08)
            .collect();
        AudioBuffer::from_channels(sample_rate, vec![left, right])
    }

    fn same_samples(a: &AudioBuffer, b: &AudioBuffer) -> bool {
        a.channels == b.channels
    }

    #[test]
    fn default_parameter_run_and_sanity() {
        let mut effect = LoudnessNormalization::default();
        let mut audio = sine_buffer(8_000);
        let mut context = EffectContext::default();
        effect.process(&mut audio, &mut context).unwrap();
        assert!(audio.channels.iter().flatten().all(|sample| sample.is_finite()));
        assert!(integrated_loudness(&audio).is_finite());
    }

    #[test]
    fn normalization_empty_and_repeatable() {
        let mut empty = AudioBuffer::from_channels(48_000, vec![vec![], vec![]]);
        LoudnessNormalization::default()
            .process(&mut empty, &mut EffectContext::default())
            .unwrap();

        let input_a = sine_buffer(5_000);
        let input_b = sine_buffer(5_000);
        let mut output_a = input_a;
        let mut output_b = input_b;
        LoudnessNormalization::default()
            .process(&mut output_a, &mut EffectContext::default())
            .unwrap();
        LoudnessNormalization::default()
            .process(&mut output_b, &mut EffectContext::default())
            .unwrap();
        assert!(same_samples(&output_a, &output_b));
    }

    #[test]
    fn normalization_parameter_clamping() {
        let mut effect = LoudnessNormalization::default();
        assert!(set_param(&mut effect, "LUFSLevel", 100.0));
        assert_eq!(effect.lufs_level, 0.0);
        assert!(set_param(&mut effect, "RMSLevel", -500.0));
        assert_eq!(effect.rms_level, -145.0);
        assert!(set_param(&mut effect, "NormalizeTo", 100.0));
        assert_eq!(effect.normalize_to, 1);
        assert_eq!(get_params(&effect).len(), 5);
    }

    #[test]
    fn loudness_measure_reports_results_and_is_repeatable() {
        let input = sine_buffer(12_000);
        assert!(integrated_loudness(&input) > -70.0);
        let mut first = LoudnessMeasure;
        let mut second = LoudnessMeasure;
        let mut audio_a = sine_buffer(12_000);
        let mut audio_b = sine_buffer(12_000);
        let mut context_a = EffectContext::default();
        let mut context_b = EffectContext::default();
        first.process(&mut audio_a, &mut context_a).unwrap();
        second.process(&mut audio_b, &mut context_b).unwrap();
        assert_eq!(context_a.report, context_b.report);
        assert!(context_a.report.contains("LUFS"));
        assert!(context_a.report.contains("dBFS"));
    }

    #[test]
    fn loudness_measure_empty_and_params() {
        let mut effect = LoudnessMeasure;
        let mut empty = AudioBuffer::from_channels(48_000, vec![vec![], vec![]]);
        let mut context = EffectContext::default();
        effect.process(&mut empty, &mut context).unwrap();
        assert!(context.report.contains("-inf"));
        assert!(effect.params().is_empty());
        assert!(!set_param(&mut effect, "Unknown", 0.0));
    }
}
