#![allow(dead_code)]
// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.
//! Silence, vocal-reduction, and vocoder effects.
//!
//! Vocal Reduction and Isolation is an approximate port: it uses direct
//! mid/side extraction and a simple frequency-domain vocal-band filter rather
//! than reproducing every detail of Audacity's historical Nyquist plug-in.

use crate::core::*;
use au_core::AudioBuffer;

/// Truncate or compress detected silent passages.
#[derive(Clone, Debug)]
pub struct TruncateSilence {
    /// Silence threshold in dB.
    pub threshold: f64,
    /// Action: zero truncates; one compresses.
    pub action: usize,
    /// Minimum detected silence duration in seconds.
    pub minimum: f64,
    /// Maximum retained silence duration when truncating, in seconds.
    pub truncate: f64,
    /// Percentage of excess silence retained when compressing.
    pub compress: f64,
    /// Detect silence independently per channel group.
    pub independent: bool,
}

impl Default for TruncateSilence {
    fn default() -> Self {
        Self {
            threshold: -20.0,
            action: 0,
            minimum: 0.5,
            truncate: 0.5,
            compress: 50.0,
            independent: false,
        }
    }
}

impl BuiltinEffect for TruncateSilence {
    fn id(&self) -> &'static str {
        "TruncateSilence"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Threshold", "Threshold", -80.0, -20.0, -20.0),
            ParamDef::choice(
                "Action",
                "Action",
                &["Truncate Detected Silence", "Compress Excess Silence"],
                0,
            ),
            ParamDef::float("Minimum", "Minimum", 0.001, 10000.0, 0.5),
            ParamDef::float("Truncate", "Truncate", 0.0, 10000.0, 0.5),
            ParamDef::float("Compress", "Compress", 0.0, 99.9, 50.0),
            ParamDef::boolean("Independent", "Independent", false),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Threshold" => Some(self.threshold),
            "Action" => Some(self.action as f64),
            "Minimum" => Some(self.minimum),
            "Truncate" => Some(self.truncate),
            "Compress" => Some(self.compress),
            "Independent" => Some(if self.independent { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Threshold" => self.threshold = value,
            "Action" => self.action = value.round() as usize,
            "Minimum" => self.minimum = value,
            "Truncate" => self.truncate = value,
            "Compress" => self.compress = value,
            "Independent" => self.independent = value != 0.0,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let sr = buf.sample_rate as f64;
        if sr == 0.0 || buf.channels.is_empty() {
            return Ok(());
        }
        let n = buf.len();
        let threshold = 10.0_f64.powf(self.threshold / 20.0) as f32;
        let min_frames = (self.minimum.max(0.001) * sr) as usize;
        let channels = buf.channels.len();

        // In the ordinary (non-independent) mode all channels must be below
        // threshold for a frame to count as silence.
        let mut silent = vec![false; n];
        for (i, is_silent) in silent.iter_mut().enumerate() {
            *is_silent = if self.independent {
                buf.channels.iter().all(|c| c[i].abs() < threshold)
            } else {
                buf.channels.iter().all(|c| c[i].abs() < threshold)
            };
        }

        let mut regions = Vec::new();
        let mut i = 0;
        while i < n {
            if !silent[i] {
                i += 1;
                continue;
            }
            let start = i;
            while i < n && silent[i] {
                i += 1;
            }
            let end = i;
            if end - start >= min_frames {
                let len = end - start;
                let retained = if self.action == 0 {
                    (self.truncate * sr) as usize
                } else {
                    (self.minimum * sr
                        + (len as f64 - self.minimum * sr).max(0.0) * self.compress / 100.0)
                        as usize
                }
                .min(len);
                let cut = len - retained;
                if cut > 0 {
                    // Audacity centers the removed interval in each detected
                    // silence region.
                    let cut_start = start + (len - cut) / 2;
                    regions.push((cut_start, cut_start + cut));
                }
            }
        }

        // Apply cuts from the end so earlier sample indices remain valid.
        for (start, end) in regions.into_iter().rev() {
            let cut_len = end - start;
            let blend = 100usize.min(cut_len);
            for channel in &mut buf.channels {
                let left = start.saturating_sub(blend / 2);
                let right = (end.saturating_sub(blend / 2)).min(channel.len());
                let count = blend.min(channel.len().saturating_sub(left))
                    .min(channel.len().saturating_sub(right));
                let mut cross = Vec::with_capacity(count);
                for j in 0..count {
                    let a = channel[left + j] as f64;
                    let b = channel[right + j] as f64;
                    cross.push((((count - j) as f64 * a + j as f64 * b) / count.max(1) as f64) as f32);
                }
                channel.drain(start..end);
                for (j, sample) in cross.into_iter().enumerate() {
                    let at = left + j;
                    if at < channel.len() {
                        channel[at] = sample;
                    }
                }
            }
        }
        debug_assert!(buf.channels.iter().all(|c| c.len() <= n));
        let _ = channels;
        Ok(())
    }
}

/// Remove or isolate center-panned material.
///
/// This implementation approximates Audacity's historical vocalrediso.ny
/// effect. Center extraction uses the mid signal; vocal isolation additionally
/// applies a simple FFT-domain bandpass in the configured frequency range.
#[derive(Clone, Debug)]
pub struct VocalReduction {
    /// Operation selector: remove/isolate vocals, center, or analyze.
    pub action: usize,
    /// Effect strength, from 0 to 100 percent.
    pub strength: f64,
    /// Lower bound of the isolated frequency band in Hz.
    pub low_cut: f64,
    /// Upper bound of the isolated frequency band in Hz.
    pub high_cut: f64,
}

impl Default for VocalReduction {
    fn default() -> Self {
        Self {
            action: 0,
            strength: 100.0,
            low_cut: 200.0,
            high_cut: 2000.0,
        }
    }
}

fn strength() -> f64 {
    100.0
}

impl BuiltinEffect for VocalReduction {
    fn id(&self) -> &'static str {
        "VocalReduction"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::choice(
                "Action",
                "Action",
                &[
                    "Remove Vocals",
                    "Isolate Vocals",
                    "Isolate Vocals and Invert",
                    "Remove Center",
                    "Isolate Center",
                    "Isolate Center and Invert",
                    "Remove Center Classic",
                    "Analyze",
                ],
                0,
            ),
            ParamDef::float("Strength", "Strength", 0.0, 100.0, 100.0),
            ParamDef::float("LowCut", "Low-cut frequency", 0.0, 2000.0, 200.0),
            ParamDef::float("HighCut", "High-cut frequency", 0.0, 20000.0, 2000.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Action" => Some(self.action as f64),
            "Strength" => Some(self.strength),
            "LowCut" => Some(self.low_cut),
            "HighCut" => Some(self.high_cut),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Action" => self.action = value.round() as usize,
            "Strength" => self.strength = value,
            "LowCut" => self.low_cut = value,
            "HighCut" => self.high_cut = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if buf.channels.len() < 2 {
            return Ok(());
        }
        let n = buf.len();
        let left = buf.channels[0].clone();
        let right = buf.channels[1].clone();
        let amount = (self.strength / 100.0) as f32;
        let classic = self.action == 6;
        let center: Vec<f32> = (0..n)
            .map(|i| if classic { left[i] - right[i] } else { (left[i] + right[i]) * 0.5 })
            .collect();

        let isolate_vocals = matches!(self.action, 1 | 2);
        let isolate_center = matches!(self.action, 4 | 5 | 6);
        if isolate_vocals || isolate_center {
            let mut extracted = center;
            if isolate_vocals {
                extracted = approximate_bandpass(
                    &extracted,
                    buf.sample_rate,
                    self.low_cut,
                    self.high_cut,
                );
            }
            for i in 0..n {
                let v = extracted[i] * amount;
                buf.channels[0][i] = v;
                buf.channels[1][i] = if matches!(self.action, 2 | 5) { -v } else { v };
            }
        } else if matches!(self.action, 0 | 3) {
            for i in 0..n {
                let mid = (left[i] + right[i]) * 0.5;
                buf.channels[0][i] = left[i] - mid * amount;
                buf.channels[1][i] = right[i] - mid * amount;
            }
        }
        Ok(())
    }
}

fn approximate_bandpass(input: &[f32], sample_rate: u32, low: f64, high: f64) -> Vec<f32> {
    use rustfft::{num_complex::Complex, FftPlanner};
    if input.is_empty() || sample_rate == 0 {
        return input.to_vec();
    }
    let size = input.len().next_power_of_two().max(2);
    let mut data = vec![Complex::new(0.0f32, 0.0); size];
    for (dst, src) in data.iter_mut().zip(input) {
        dst.re = *src;
    }
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(size);
    let inverse = planner.plan_fft_inverse(size);
    fft.process(&mut data);
    let nyquist = sample_rate as f64 / 2.0;
    let lo = low.max(0.0).min(nyquist);
    let hi = high.max(lo).min(nyquist);
    for (k, bin) in data.iter_mut().enumerate() {
        let freq = k.min(size - k) as f64 * sample_rate as f64 / size as f64;
        if freq < lo || freq > hi {
            *bin = Complex::new(0.0, 0.0);
        }
    }
    inverse.process(&mut data);
    data.into_iter().take(input.len()).map(|x| x.re / size as f32).collect()
}

/// Nyquist-style vocoder using a control track when supplied, or a generated
/// carrier otherwise.
#[derive(Clone, Debug)]
pub struct Vocoder {
    /// Vocoder distance control.
    pub dst: f64,
    /// Output choice: both channels or right only.
    pub mst: usize,
    /// Number of vocoder bands.
    pub bands: usize,
    /// Carrier-track level, percent.
    pub track_vl: f64,
    /// White-noise carrier level, percent.
    pub noise_vl: f64,
    /// Radar-needle carrier level, percent.
    pub radar_vl: f64,
    /// Radar-needle repetition frequency, Hz.
    pub radar_f: f64,
}

impl Default for Vocoder {
    fn default() -> Self {
        Self {
            dst: 20.0,
            mst: 0,
            bands: 40,
            track_vl: 100.0,
            noise_vl: 0.0,
            radar_vl: 0.0,
            radar_f: 30.0,
        }
    }
}

impl BuiltinEffect for Vocoder {
    fn id(&self) -> &'static str {
        "Vocoder"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("DST", "Distance", 1.0, 120.0, 20.0),
            ParamDef::choice("MST", "Output choice", &["Both Channels", "Right Only"], 0),
            ParamDef::int("BANDS", "Number of vocoder bands", 10.0, 240.0, 40.0),
            ParamDef::float("TRACK-VL", "Amplitude of carrier wave", 0.0, 100.0, 100.0),
            ParamDef::float("NOISE-VL", "Amplitude of white noise", 0.0, 100.0, 0.0),
            ParamDef::float("RADAR-VL", "Amplitude of Radar Needles", 0.0, 100.0, 0.0),
            ParamDef::float("RADAR-F", "Frequency of Radar Needles", 1.0, 100.0, 30.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "DST" => Some(self.dst),
            "MST" => Some(self.mst as f64),
            "BANDS" => Some(self.bands as f64),
            "TRACK-VL" => Some(self.track_vl),
            "NOISE-VL" => Some(self.noise_vl),
            "RADAR-VL" => Some(self.radar_vl),
            "RADAR-F" => Some(self.radar_f),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "DST" => self.dst = value,
            "MST" => self.mst = value.round() as usize,
            "BANDS" => self.bands = value.round() as usize,
            "TRACK-VL" => self.track_vl = value,
            "NOISE-VL" => self.noise_vl = value,
            "RADAR-VL" => self.radar_vl = value,
            "RADAR-F" => self.radar_f = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let n = buf.len();
        if n == 0 || buf.channels.is_empty() {
            return Ok(());
        }
        let sr = buf.sample_rate.max(1) as f64;
        let modulator = buf.channels[0].clone();
        let carrier_track = ctx.aux.and_then(|aux| aux.channels.first());
        let mut carrier = vec![0.0f32; n];
        if let Some(track) = carrier_track {
            for i in 0..n {
                carrier[i] = track.get(i).copied().unwrap_or(0.0);
            }
        } else {
            // Deterministic noise carrier makes repeated processing identical.
            let mut state = 0x1234_5678u32;
            for (i, sample) in carrier.iter_mut().enumerate() {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = ((state >> 8) as f32 / 16_777_216.0) * 2.0 - 1.0;
                let radar_period = (sr / self.radar_f.max(1.0)) as usize;
                let radar = if self.radar_vl > 0.0 && radar_period > 0 && i % radar_period == 0 {
                    1.0
                } else {
                    0.0
                };
                *sample = noise * (self.noise_vl / 100.0) as f32
                    + radar * (self.radar_vl / 100.0) as f32;
            }
        }

        let bands = self.bands.max(1);
        let octaves = (sr / 2.205 / 20.0).max(1.0).log2();
        let interval = octaves * 12.0 / bands as f64;
        let mut output = vec![0.0f32; n];
        for band in 0..bands {
            let semitone = (band as f64 + 0.5) * interval;
            let freq = (20.0 * 2.0_f64.powf(semitone / 12.0)).min(sr * 0.49);
            if freq <= 0.0 {
                continue;
            }
            let q = (std::f64::consts::SQRT_2 / (octaves / bands as f64).max(0.001))
                .max(0.1);
            let modulation = bandpass_biquad(&modulator, sr, freq, q);
            let carrier_band = bandpass_biquad(&carrier, sr, freq, q);
            let alpha = (-2.0 * std::f64::consts::PI * (freq / self.dst.max(1.0)) / sr)
                .exp() as f32;
            let mut envelope = 0.0f32;
            for i in 0..n {
                envelope = alpha * envelope + (1.0 - alpha) * modulation[i].abs();
                output[i] += envelope * carrier_band[i];
            }
        }
        let peak = output.iter().fold(0.0f32, |a, x| a.max(x.abs()));
        if peak > 0.0 {
            for x in &mut output {
                *x /= peak;
            }
        }
        if buf.channels.len() == 1 {
            buf.channels[0] = output;
        } else if self.mst == 1 {
            buf.channels[1] = output;
        } else {
            buf.channels[0] = output.clone();
            buf.channels[1] = output;
        }
        Ok(())
    }
}

fn bandpass_biquad(input: &[f32], sr: f64, frequency: f64, q: f64) -> Vec<f32> {
    let w0 = 2.0 * std::f64::consts::PI * frequency.min(sr * 0.49) / sr;
    let alpha = w0.sin() / (2.0 * q.max(0.01));
    let a0 = 1.0 + alpha;
    let b0 = alpha / a0;
    let b2 = -alpha / a0;
    let a1 = -2.0 * w0.cos() / a0;
    let a2 = (1.0 - alpha) / a0;
    let mut x1 = 0.0f64;
    let mut x2 = 0.0f64;
    let mut y1 = 0.0f64;
    let mut y2 = 0.0f64;
    input
        .iter()
        .map(|&x| {
            let x = x as f64;
            let y = b0 * x + b2 * x2 - a1 * y1 - a2 * y2;
            x2 = x1;
            x1 = x;
            y2 = y1;
            y1 = y;
            y as f32
        })
        .collect()
}

/// Returns one default instance of each effect in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![
        Box::new(TruncateSilence::default()),
        Box::new(VocalReduction::default()),
        Box::new(Vocoder::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine() -> AudioBuffer {
        let rate = 8000;
        let channel: Vec<f32> = (0..512)
            .map(|i| (i as f32 * 0.07).sin() * 0.3)
            .collect();
        AudioBuffer::from_channels(rate, vec![channel.clone(), channel])
    }

    fn check_effect(mut effect: Box<dyn BuiltinEffect>) {
        let mut a = sine();
        let mut b = sine();
        let mut ctx = EffectContext::default();
        effect.process(&mut a, &mut ctx).unwrap();
        effect.process(&mut b, &mut EffectContext::default()).unwrap();
        assert_eq!(a.channels, b.channels);

        let mut empty = AudioBuffer::from_channels(8000, vec![vec![], vec![]]);
        effect.process(&mut empty, &mut EffectContext::default()).unwrap();
    }

    #[test]
    fn defaults_run_and_repeat() {
        for effect in all() {
            check_effect(effect);
        }
    }

    #[test]
    fn parameter_clamping() {
        let mut trunc = TruncateSilence::default();
        assert!(set_param(&mut trunc, "Threshold", -200.0));
        assert_eq!(trunc.threshold, -80.0);
        let mut vocoder = Vocoder::default();
        assert!(set_param(&mut vocoder, "BANDS", 1000.0));
        assert_eq!(vocoder.bands, 240);
    }

    #[test]
    fn truncate_silence_shortens_long_silence() {
        let mut effect = TruncateSilence {
            minimum: 0.01,
            truncate: 0.005,
            ..Default::default()
        };
        let mut buffer = AudioBuffer::from_channels(1000, vec![vec![0.0; 1000], vec![0.0; 1000]]);
        effect.process(&mut buffer, &mut EffectContext::default()).unwrap();
        assert!(buffer.len() < 1000);
    }

    #[test]
    fn vocal_center_is_isolated() {
        let mut effect = VocalReduction {
            action: 4,
            ..Default::default()
        };
        let mut buffer = AudioBuffer::from_channels(8000, vec![vec![0.5; 32], vec![0.5; 32]]);
        effect.process(&mut buffer, &mut EffectContext::default()).unwrap();
        assert!(buffer.channels[0].iter().all(|x| *x == 0.5));
    }

    #[test]
    fn vocoder_produces_finite_output() {
        let mut effect = Vocoder::default();
        let mut buffer = sine();
        effect.process(&mut buffer, &mut EffectContext::default()).unwrap();
        assert!(buffer.channels.iter().flatten().all(|x| x.is_finite()));
    }
}
