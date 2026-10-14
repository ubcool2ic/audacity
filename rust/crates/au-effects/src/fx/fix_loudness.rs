// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.
//! Audacity-compatible loudness normalization.

use crate::core::*;
use au_core::AudioBuffer;

const NORMALIZE_TARGETS: &[&str] = &["Perceived loudness", "RMS"];

/// Normalizes audio to a perceived loudness or RMS target.
pub struct FixLoudness {
    /// Whether stereo channels are analyzed and normalized independently.
    pub stereo_independent: bool,
    /// Target loudness in LUFS.
    pub lufs_level: f64,
    /// Target RMS level in dB.
    pub rms_level: f64,
    /// Treat mono audio as dual mono when normalizing loudness.
    pub dual_mono: bool,
    /// Normalization target: 0 is perceived loudness, 1 is RMS.
    pub normalize_to: usize,
}

impl Default for FixLoudness {
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

impl BuiltinEffect for FixLoudness {
    fn id(&self) -> &'static str {
        "LoudnessNormalization"
    }

    fn name(&self) -> &'static str {
        "Loudness Normalization"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::boolean("StereoIndependent", "Stereo Independent", false),
            ParamDef::float("LUFSLevel", "LUFS Level", -145.0, 0.0, -23.0),
            ParamDef::float("RMSLevel", "RMS Level", -145.0, 0.0, -20.0),
            ParamDef::boolean("DualMono", "Dual Mono", true),
            ParamDef::choice("NormalizeTo", "Normalize To", NORMALIZE_TARGETS, 0),
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
        let Some(def) = self.params().into_iter().find(|param| param.key == key) else {
            return false;
        };
        let value = def.clamp(value);
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
        let stereo = channel_count > 1;
        let loudness_mode = self.normalize_to == 0;

        if self.stereo_independent {
            for channel in &mut buf.channels {
                let extent = if loudness_mode {
                    ebu_r128_loudness(&[channel.as_slice()], buf.sample_rate)
                } else {
                    rms_level(channel)
                };
                let multiplier = self.multiplier(extent, loudness_mode, true, stereo)?;
                apply_gain(channel, multiplier);
            }
        } else {
            let extent = if loudness_mode {
                let channels: Vec<&[f32]> =
                    buf.channels.iter().map(|channel| channel.as_slice()).collect();
                ebu_r128_loudness(&channels, buf.sample_rate)
            } else if stereo {
                let left = rms_level(&buf.channels[0]);
                let right = rms_level(&buf.channels[1]);
                // Audacity evaluates the products and sum in float before
                // dividing by the double literal and taking the square root.
                let square_sum = left * left + right * right;
                ((square_sum as f64 / 2.0).sqrt()) as f32
            } else {
                rms_level(&buf.channels[0])
            };

            let multiplier = self.multiplier(extent, loudness_mode, false, stereo)?;
            for channel in &mut buf.channels {
                apply_gain(channel, multiplier);
            }
        }

        Ok(())
    }
}

impl FixLoudness {
    fn multiplier(
        &self,
        extent: f32,
        loudness_mode: bool,
        independent: bool,
        stereo_track: bool,
    ) -> Result<f32, EffectError> {
        if extent == 0.0 {
            return Err(EffectError::Invalid(
                "Cannot normalize silent audio".to_string(),
            ));
        }

        let ratio = if loudness_mode {
            let target_db = (self.lufs_level * 2.0).clamp(-145.0, 0.0);
            10.0_f64.powf(target_db / 20.0) as f32
        } else {
            let target_db = self.rms_level.clamp(-145.0, 0.0);
            10.0_f64.powf(target_db / 20.0) as f32
        };

        let mut multiplier = ratio / extent;
        if loudness_mode && independent && (self.dual_mono || stereo_track) {
            multiplier /= 2.0;
        }
        if loudness_mode {
            multiplier = multiplier.sqrt();
        }
        Ok(multiplier)
    }
}

fn apply_gain(channel: &mut [f32], multiplier: f32) {
    for sample in channel {
        *sample *= multiplier;
    }
}

fn rms_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    let mut sum = 0.0_f64;
    for &sample in samples {
        // The input sample is float; its product is float before being added
        // to the double-precision accumulator.
        let square = sample * sample;
        sum += square as f64;
    }
    (sum / samples.len() as f64).sqrt() as f32
}

#[derive(Clone, Copy)]
struct BiquadCoefficients {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

struct Biquad {
    coefficients: BiquadCoefficients,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    fn new(coefficients: BiquadCoefficients) -> Self {
        Self {
            coefficients,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process_one(&mut self, x: f64) -> f64 {
        let c = self.coefficients;
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

fn weighting_filter(rate: f64) -> [BiquadCoefficients; 2] {
    let db = 3.999843853973347_f64;
    let mut f0 = 1681.974450955533_f64;
    let mut q = 0.7071752369554196_f64;
    let mut k = (std::f64::consts::PI * f0 / rate).tan();

    let vh = 10.0_f64.powf(db / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;

    let high_shelf = BiquadCoefficients {
        b0: (vh + vb * k / q + k * k) / a0,
        b1: 2.0 * (k * k - vh) / a0,
        b2: (vh - vb * k / q + k * k) / a0,
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
    };

    f0 = 38.13547087602444;
    q = 0.5003270373238773;
    k = (std::f64::consts::PI * f0 / rate).tan();
    let hpf_a0 = 1.0 + k / q + k * k;

    let high_pass = BiquadCoefficients {
        b0: 1.0,
        b1: -2.0,
        b2: 1.0,
        a1: 2.0 * (k * k - 1.0) / hpf_a0,
        a2: (1.0 - k / q + k * k) / hpf_a0,
    };

    [high_shelf, high_pass]
}

struct EbuR128 {
    channel_filters: Vec<[Biquad; 2]>,
    histogram: Vec<i64>,
    ring: Vec<f64>,
    sample_count: usize,
    ring_pos: usize,
    ring_size: usize,
    block_size: usize,
    block_overlap: usize,
}

impl EbuR128 {
    const HIST_BIN_COUNT: usize = 65_536;
    const GAMMA_A: f64 = (-70.0 + 0.691) / 10.0;

    fn new(rate: f64, channels: usize) -> Self {
        let safe_rate = if rate > 0.0 { rate } else { 1.0 };
        let block_size = (0.4 * safe_rate).ceil().max(1.0) as usize;
        let block_overlap = (0.1 * safe_rate).ceil().max(1.0) as usize;
        let coefficients = weighting_filter(safe_rate);

        let channel_filters = (0..channels)
            .map(|_| {
                [
                    Biquad::new(coefficients[0]),
                    Biquad::new(coefficients[1]),
                ]
            })
            .collect();

        Self {
            channel_filters,
            histogram: vec![0; Self::HIST_BIN_COUNT],
            ring: vec![0.0; block_size],
            sample_count: 0,
            ring_pos: 0,
            ring_size: 0,
            block_size,
            block_overlap,
        }
    }

    fn process_sample_from_channel(&mut self, input: f32, channel: usize) {
        let value = self.channel_filters[channel][0].process_one(input as f64);
        let value = self.channel_filters[channel][1].process_one(value);
        let power = value * value;
        if channel == 0 {
            self.ring[self.ring_pos] = power;
        } else {
            self.ring[self.ring_pos] += power;
        }
    }

    fn next_sample(&mut self) {
        self.ring_pos += 1;
        self.ring_size += 1;

        if self.ring_pos % self.block_overlap == 0 && self.ring_size >= self.block_size {
            self.add_block_to_histogram(self.block_size);
        }

        if self.ring_pos == self.block_size {
            self.ring_pos = 0;
        }
        self.sample_count += 1;
    }

    fn histogram_sums(&self, start_idx: usize) -> (f64, i64) {
        let mut sum_v = 0.0;
        let mut sum_c = 0_i64;

        for i in start_idx..Self::HIST_BIN_COUNT {
            let value = -Self::GAMMA_A / Self::HIST_BIN_COUNT as f64 * (i + 1) as f64
                + Self::GAMMA_A;
            sum_v += 10.0_f64.powf(value) * self.histogram[i] as f64;
            sum_c += self.histogram[i];
        }
        (sum_v, sum_c)
    }

    fn add_block_to_histogram(&mut self, valid_len: usize) {
        self.ring_size = self.block_size;
        if valid_len == 0 {
            return;
        }

        let mut block_value = 0.0;
        for i in 0..valid_len {
            block_value += self.ring[i];
        }
        block_value = (block_value / valid_len as f64).log10();

        let idx_value = ((block_value - Self::GAMMA_A) * Self::HIST_BIN_COUNT as f64
            / -Self::GAMMA_A
            - 1.0)
            .round();

        if idx_value.is_finite()
            && idx_value >= 0.0
            && idx_value < Self::HIST_BIN_COUNT as f64
        {
            self.histogram[idx_value as usize] += 1;
        }
    }

    fn integrative_loudness(&mut self) -> f64 {
        let (mut sum_v, mut sum_c) = self.histogram_sums(0);

        if sum_c == 0 {
            self.add_block_to_histogram(self.ring_size);
            (sum_v, sum_c) = self.histogram_sums(0);
        }

        if sum_c == 0 {
            return 0.0;
        }

        let gamma_r = (sum_v / sum_c as f64).log10() - 1.0;
        let idx_r_value = ((gamma_r - Self::GAMMA_A) * Self::HIST_BIN_COUNT as f64
            / -Self::GAMMA_A
            - 1.0)
            .round();

        if !idx_r_value.is_finite()
            || idx_r_value < 0.0
            || idx_r_value >= Self::HIST_BIN_COUNT as f64
        {
            return 0.0;
        }

        let (gated_sum, gated_count) = self.histogram_sums(idx_r_value as usize + 1);
        if gated_count == 0 {
            return 0.0;
        }

        0.8529037031 * gated_sum / gated_count as f64
    }
}

fn ebu_r128_loudness(channels: &[&[f32]], sample_rate: u32) -> f32 {
    if channels.is_empty() {
        return 0.0;
    }
    let frame_count = channels.iter().map(|channel| channel.len()).min().unwrap_or(0);
    if frame_count == 0 {
        return 0.0;
    }

    let mut processor = EbuR128::new(sample_rate as f64, channels.len());
    for frame in 0..frame_count {
        for (channel_index, channel) in channels.iter().enumerate() {
            processor.process_sample_from_channel(channel[frame], channel_index);
        }
        processor.next_sample();
    }
    processor.integrative_loudness() as f32
}

/// Returns the effects defined in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(FixLoudness::default())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::set_param;

    fn stereo_sine() -> AudioBuffer {
        let sample_rate = 48_000;
        let channels = (0..2)
            .map(|channel| {
                (0..4096)
                    .map(|i| {
                        let phase = 2.0 * std::f64::consts::PI * 1000.0 * i as f64
                            / sample_rate as f64;
                        (phase.sin() * if channel == 0 { 0.2 } else { 0.15 }) as f32
                    })
                    .collect()
            })
            .collect();
        AudioBuffer::from_channels(sample_rate, channels)
    }

    #[test]
    fn default_processes_short_stereo_sine() {
        let mut effect = FixLoudness::default();
        let mut buffer = stereo_sine();
        let mut context = EffectContext::default();
        let _ = effect.process(&mut buffer, &mut context);
        assert_eq!(buffer.channels.len(), 2);
        assert_eq!(buffer.len(), 4096);
    }

    #[test]
    fn empty_buffer_does_not_panic() {
        let mut effect = FixLoudness::default();
        let mut buffer = AudioBuffer::from_channels(48_000, vec![Vec::new(), Vec::new()]);
        let mut context = EffectContext::default();
        assert!(effect.process(&mut buffer, &mut context).is_ok());
    }

    #[test]
    fn processing_is_repeatable() {
        let mut first = stereo_sine();
        let mut second = stereo_sine();
        let mut effect_a = FixLoudness::default();
        let mut effect_b = FixLoudness::default();
        let mut context_a = EffectContext::default();
        let mut context_b = EffectContext::default();

        let _ = effect_a.process(&mut first, &mut context_a);
        let _ = effect_b.process(&mut second, &mut context_b);

        assert_eq!(first.channels, second.channels);
    }

    #[test]
    fn parameters_clamp() {
        let mut effect = FixLoudness::default();
        assert!(set_param(&mut effect, "LUFSLevel", 100.0));
        assert_eq!(effect.lufs_level, 0.0);
        assert!(set_param(&mut effect, "NormalizeTo", 99.0));
        assert_eq!(effect.normalize_to, 1);
        assert!(!set_param(&mut effect, "Unknown", 1.0));
    }
}
