// SPDX-License-Identifier: GPL-3.0-or-later
//! Audacity-style two-pass noise reduction.

use crate::core::*;
use crate::db_to_linear;
use au_core::AudioBuffer;
use rustfft::{num_complex::Complex, FftPlanner};
use std::f32::consts::PI;

/// Stored spectral statistics from a noise-only selection.
#[derive(Clone, Debug, PartialEq)]
pub struct NoiseProfile {
    /// Mean power for each real-FFT frequency bin.
    pub means: Vec<f32>,
    /// Number of window spectra contributing to the means.
    pub count: u64,
    /// Sample rate used to gather the profile.
    pub sample_rate: u32,
    /// FFT window size used to gather the profile.
    pub window_size: usize,
}

/// Audacity's two-pass Noise Reduction effect.
#[derive(Clone, Debug)]
pub struct NoiseReduction {
    /// Noise reduction amount in decibels.
    pub noise_reduction: f64,
    /// Classification sensitivity.
    pub sensitivity: f64,
    /// Number of neighboring frequency bins used for smoothing.
    pub frequency_smoothing: f64,
    /// Output choice: reduce noise or output the residue.
    pub noise_reduction_choice: f64,
    /// Analysis/synthesis window choice.
    pub window_types: f64,
    /// FFT window size in samples.
    pub window_size: f64,
    /// Number of analysis steps per window.
    pub steps_per_window: f64,
    /// Spectral discrimination method.
    pub method: f64,
    /// Legacy sensitivity setting.
    pub old_sensitivity: f64,
    /// Profile gathered by [`NoiseReduction::get_profile`].
    pub profile: Option<NoiseProfile>,
}

const CHOICES: &[&str] = &["Reduce noise", "Residue"];
const WINDOW_TYPES: &[&str] = &[
    "Rectangular, Hann",
    "Hann, Rectangular",
    "Hann, Hann",
    "Blackman, Hann",
    "Hamming, Rectangular",
    "Hamming, Hann",
];
const METHODS: &[&str] = &["Median", "Second greatest"];

impl Default for NoiseReduction {
    fn default() -> Self {
        Self {
            noise_reduction: 6.0,
            sensitivity: 6.0,
            frequency_smoothing: 6.0,
            noise_reduction_choice: 0.0,
            window_types: 2.0,
            window_size: 2048.0,
            steps_per_window: 4.0,
            method: 1.0,
            old_sensitivity: 6.0,
            profile: None,
        }
    }
}

impl NoiseReduction {
    /// Gathers and stores noise statistics from `buf`.
    ///
    /// As in Audacity, each complete analysis window contributes its power
    /// spectrum to the profile. Calling this method replaces any previous
    /// profile.
    pub fn get_profile(&mut self, buf: &AudioBuffer) -> Result<(), EffectError> {
        let window_size = self.actual_window_size();
        let step = self.actual_step_size(window_size);
        let spectrum_size = window_size / 2 + 1;

        if buf.sample_rate == 0 || buf.channels.is_empty() {
            return Err(EffectError::Invalid(
                "Selected noise profile is too short.".to_string(),
            ));
        }

        let mut sums = vec![0.0f32; spectrum_size];
        let mut count = 0u64;

        for channel in &buf.channels {
            if channel.len() < window_size {
                continue;
            }
            let mut start = 0usize;
            while start + window_size <= channel.len() {
                let spectrum = analyze_frame(
                    &channel[start..start + window_size],
                    window_size,
                    window_function(self.window_types as usize).0,
                );
                for (sum, value) in sums.iter_mut().zip(spectrum) {
                    *sum += value;
                }
                count += 1;
                start += step;
            }
        }

        if count == 0 {
            self.profile = None;
            return Err(EffectError::Invalid(
                "Selected noise profile is too short.".to_string(),
            ));
        }

        let divisor = count as f32;
        let means = sums.into_iter().map(|sum| sum / divisor).collect();
        self.profile = Some(NoiseProfile {
            means,
            count,
            sample_rate: buf.sample_rate,
            window_size,
        });
        Ok(())
    }

    fn actual_window_size(&self) -> usize {
        let requested = self.window_size.round().clamp(256.0, 8192.0) as usize;
        requested
            .next_power_of_two()
            .min(8192)
            .max(256)
    }

    fn actual_steps_per_window(&self) -> usize {
        let requested = self.steps_per_window.round().clamp(2.0, 16.0) as usize;
        [2usize, 4, 8, 16]
            .into_iter()
            .min_by_key(|candidate| candidate.abs_diff(requested))
            .unwrap_or(4)
    }

    fn actual_step_size(&self, window_size: usize) -> usize {
        (window_size / self.actual_steps_per_window()).max(1)
    }

    fn process_buffer(&self, buf: &mut AudioBuffer) -> Result<(), EffectError> {
        let profile = self.profile.as_ref().ok_or_else(|| {
            EffectError::Invalid("A noise profile must be obtained before processing.".to_string())
        })?;
        let window_size = self.actual_window_size();

        if profile.window_size != window_size {
            return Err(EffectError::Invalid(
                "You must specify the same window size for steps 1 and 2.".to_string(),
            ));
        }
        if profile.sample_rate != buf.sample_rate {
            return Err(EffectError::Invalid(
                "The sample rate of the noise profile must match that of the sound to be processed."
                    .to_string(),
            ));
        }
        if buf.channels.is_empty() || buf.len() == 0 {
            return Ok(());
        }

        let step_size = self.actual_step_size(window_size);
        let (analysis_type, synthesis_type) = window_function(self.window_types as usize);
        let analysis_window = make_window(window_size, analysis_type);
        let synthesis_window = make_window(window_size, synthesis_type);
        let reduction_factor = db_to_linear(-(self.noise_reduction as f32));
        let attack_blocks =
            1.0 + (0.02 * buf.sample_rate as f64 / step_size as f64) as f32;
        let release_blocks =
            1.0 + (0.10 * buf.sample_rate as f64 / step_size as f64) as f32;
        let attack_factor =
            db_to_linear(-(self.noise_reduction as f32) / attack_blocks);
        let release_factor =
            db_to_linear(-(self.noise_reduction as f32) / release_blocks);
        let sensitivity = (self.sensitivity * std::f64::consts::LN_10) as f32;
        let smoothing_bins = self.frequency_smoothing.round().clamp(0.0, 12.0) as usize;
        let residue = self.noise_reduction_choice.round() as usize == 1;
        let median_method = self.method.round() as usize == 0;

        for channel in &mut buf.channels {
            process_channel(
                channel,
                buf.sample_rate,
                window_size,
                step_size,
                &analysis_window,
                &synthesis_window,
                &profile.means,
                reduction_factor,
                attack_factor,
                release_factor,
                sensitivity,
                smoothing_bins,
                median_method,
                residue,
            );
        }
        Ok(())
    }
}

impl BuiltinEffect for NoiseReduction {
    fn learn_profile(&mut self, noise: &AudioBuffer) -> Result<(), EffectError> { self.get_profile(noise) }

    fn id(&self) -> &'static str {
        "NoiseReduction"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::int("NoiseReduction", "Noise Reduction (dB)", 0.0, 48.0, 6.0),
            ParamDef::float("Sensitivity", "Sensitivity", 0.01, 24.0, 6.0),
            ParamDef::int(
                "FrequencySmoothing",
                "Frequency Smoothing",
                0.0,
                12.0,
                6.0,
            ),
            ParamDef::choice(
                "NoiseReductionChoice",
                "Noise Reduction Choice",
                CHOICES,
                0,
            ),
            ParamDef::choice("WindowTypes", "Window Types", WINDOW_TYPES, 2),
            ParamDef::int("WindowSize", "Window Size", 256.0, 8192.0, 2048.0),
            ParamDef::int("StepsPerWindow", "Steps per Window", 2.0, 16.0, 4.0),
            ParamDef::choice("Method", "Method", METHODS, 1),
            ParamDef::float("OldSensitivity", "Old Sensitivity", 0.01, 24.0, 6.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "NoiseReduction" => Some(self.noise_reduction),
            "Sensitivity" => Some(self.sensitivity),
            "FrequencySmoothing" => Some(self.frequency_smoothing),
            "NoiseReductionChoice" => Some(self.noise_reduction_choice),
            "WindowTypes" => Some(self.window_types),
            "WindowSize" => Some(self.window_size),
            "StepsPerWindow" => Some(self.steps_per_window),
            "Method" => Some(self.method),
            "OldSensitivity" => Some(self.old_sensitivity),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let (slot, def) = match key {
            "NoiseReduction" => (&mut self.noise_reduction, ParamDef::int("", "", 0.0, 48.0, 6.0)),
            "Sensitivity" => (&mut self.sensitivity, ParamDef::float("", "", 0.01, 24.0, 6.0)),
            "FrequencySmoothing" => {
                (&mut self.frequency_smoothing, ParamDef::int("", "", 0.0, 12.0, 6.0))
            }
            "NoiseReductionChoice" => (
                &mut self.noise_reduction_choice,
                ParamDef::choice("", "", CHOICES, 0),
            ),
            "WindowTypes" => (&mut self.window_types, ParamDef::choice("", "", WINDOW_TYPES, 2)),
            "WindowSize" => (&mut self.window_size, ParamDef::int("", "", 256.0, 8192.0, 2048.0)),
            "StepsPerWindow" => {
                (&mut self.steps_per_window, ParamDef::int("", "", 2.0, 16.0, 4.0))
            }
            "Method" => (&mut self.method, ParamDef::choice("", "", METHODS, 1)),
            "OldSensitivity" => {
                (&mut self.old_sensitivity, ParamDef::float("", "", 0.01, 24.0, 6.0))
            }
            _ => return false,
        };
        *slot = def.clamp(value);
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        self.process_buffer(buf)
    }
}

#[derive(Clone, Copy)]
enum WindowKind {
    Rectangular,
    Hann,
    Blackman,
    Hamming,
}

fn window_function(choice: usize) -> (WindowKind, WindowKind) {
    match choice {
        0 => (WindowKind::Rectangular, WindowKind::Hann),
        1 => (WindowKind::Hann, WindowKind::Rectangular),
        3 => (WindowKind::Blackman, WindowKind::Hann),
        4 => (WindowKind::Hamming, WindowKind::Rectangular),
        5 => (WindowKind::Hamming, WindowKind::Hann),
        _ => (WindowKind::Hann, WindowKind::Hann),
    }
}

fn make_window(size: usize, kind: WindowKind) -> Vec<f32> {
    if size <= 1 {
        return vec![1.0; size];
    }
    (0..size)
        .map(|index| {
            let phase = 2.0 * PI * index as f32 / (size - 1) as f32;
            match kind {
                WindowKind::Rectangular => 1.0,
                WindowKind::Hann => 0.5 - 0.5 * phase.cos(),
                WindowKind::Blackman => {
                    0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos()
                }
                WindowKind::Hamming => 0.54 - 0.46 * phase.cos(),
            }
        })
        .collect()
}

fn analyze_frame(samples: &[f32], size: usize, window: WindowKind) -> Vec<f32> {
    let window = make_window(size, window);
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(size);
    let mut data: Vec<Complex<f32>> = samples
        .iter()
        .zip(window)
        .map(|(&sample, weight)| Complex::new(sample * weight, 0.0))
        .collect();
    fft.process(&mut data);

    let mut power = Vec::with_capacity(size / 2 + 1);
    for bin in 0..=size / 2 {
        power.push(data[bin].norm_sqr());
    }
    power
}

#[allow(clippy::too_many_arguments)]
fn process_channel(
    samples: &mut [f32],
    sample_rate: u32,
    size: usize,
    step: usize,
    analysis_window: &[f32],
    synthesis_window: &[f32],
    profile_means: &[f32],
    attenuation: f32,
    attack_factor: f32,
    release_factor: f32,
    sensitivity: f32,
    smoothing_bins: usize,
    median_method: bool,
    residue: bool,
) {
    if samples.is_empty() {
        return;
    }

    let spectrum_size = size / 2 + 1;
    let first_start = -(size as isize / 2);
    let last_start = samples.len() as isize - 1;
    let frame_count = ((last_start - first_start).max(0) as usize / step) + 1;

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(size);
    let inverse = planner.plan_fft_inverse(size);
    let mut spectra = Vec::with_capacity(frame_count);
    let mut fft_data = vec![Complex::new(0.0f32, 0.0f32); size];

    for frame in 0..frame_count {
        let start = first_start + (frame * step) as isize;
        for index in 0..size {
            let sample_index = start + index as isize;
            let sample = if sample_index >= 0 && (sample_index as usize) < samples.len() {
                samples[sample_index as usize]
            } else {
                0.0
            };
            fft_data[index] = Complex::new(sample * analysis_window[index], 0.0);
        }
        fft.process(&mut fft_data);
        spectra.push(fft_data.clone());
    }

    let centers = spectra.len();
    let mut gains = vec![vec![attenuation; spectrum_size]; centers];
    let center_offset = 2usize;
    let examine = 5usize;

    for center in 0..centers {
        let begin = center.saturating_sub(center_offset);
        let end = (center + (examine - center_offset)).min(centers);
        let count = end.saturating_sub(begin);
        if count == 0 {
            continue;
        }

        for bin in 0..spectrum_size {
            let mut values: Vec<f32> = (begin..end)
                .map(|index| spectra[index][bin].norm_sqr())
                .collect();
            let threshold = sensitivity * profile_means.get(bin).copied().unwrap_or(0.0);

            let is_noise = if median_method && values.len() > 3 {
                values.sort_by(|a, b| a.total_cmp(b));
                let third_greatest = values[values.len() - 3];
                third_greatest <= threshold
            } else {
                let mut greatest = 0.0f32;
                let mut second = 0.0f32;
                for value in values {
                    if value >= greatest {
                        second = greatest;
                        greatest = value;
                    } else if value >= second {
                        second = value;
                    }
                }
                second <= threshold
            };

            gains[center][bin] = if is_noise { attenuation } else { 1.0 };
        }
    }

    // Attack propagates a detected signal toward earlier frames; release
    // propagates it toward later frames.
    for frame in (0..centers.saturating_sub(1)).rev() {
        for bin in 0..spectrum_size {
            gains[frame][bin] =
                gains[frame][bin].max((gains[frame + 1][bin] * attack_factor).max(attenuation));
        }
    }
    for frame in 1..centers {
        for bin in 0..spectrum_size {
            gains[frame][bin] =
                gains[frame][bin].max((gains[frame - 1][bin] * release_factor).max(attenuation));
        }
    }

    if smoothing_bins > 0 {
        for frame_gains in &mut gains {
            let logs: Vec<f32> = frame_gains
                .iter()
                .map(|gain| gain.max(f32::MIN_POSITIVE).ln())
                .collect();
            for bin in 0..spectrum_size {
                let begin = bin.saturating_sub(smoothing_bins);
                let end = (bin + smoothing_bins + 1).min(spectrum_size);
                let mean = logs[begin..end].iter().sum::<f32>() / (end - begin) as f32;
                frame_gains[bin] = mean.exp();
            }
        }
    }

    let mut output = vec![0.0f32; samples.len()];
    let mut normalization = vec![0.0f32; samples.len()];
    let inverse_scale = 1.0 / size as f32;

    for frame in 0..centers {
        let start = first_start + (frame * step) as isize;
        for bin in 0..spectrum_size {
            let gain = if residue {
                gains[frame][bin] - 1.0
            } else {
                gains[frame][bin]
            };
            if bin == 0 || bin == size / 2 {
                spectra[frame][bin] *= gain;
            } else {
                spectra[frame][bin] *= gain;
                spectra[frame][size - bin] *= gain;
            }
        }

        inverse.process(&mut spectra[frame]);
        for index in 0..size {
            let sample_index = start + index as isize;
            if sample_index >= 0 && (sample_index as usize) < samples.len() {
                let out_index = sample_index as usize;
                output[out_index] += spectra[frame][index].re
                    * inverse_scale
                    * synthesis_window[index];
                normalization[out_index] +=
                    analysis_window[index] * synthesis_window[index];
            }
        }
    }

    for (index, sample) in samples.iter_mut().enumerate() {
        if normalization[index].abs() > 1.0e-12 {
            *sample = output[index] / normalization[index];
        }
    }

    let _ = sample_rate;
}

/// Returns this module's built-in effects.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(NoiseReduction::default())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{set_param, EffectContext};

    fn sine(frames: usize) -> AudioBuffer {
        let sample_rate = 8000;
        let channels = (0..2)
            .map(|channel| {
                (0..frames)
                    .map(|index| {
                        let phase = 2.0 * std::f32::consts::PI * 440.0 * index as f32
                            / sample_rate as f32;
                        phase.sin() * if channel == 0 { 0.4 } else { 0.3 }
                    })
                    .collect()
            })
            .collect();
        AudioBuffer::from_channels(sample_rate, channels)
    }

    fn energy(buf: &AudioBuffer) -> f64 {
        buf.channels
            .iter()
            .flatten()
            .map(|sample| (*sample as f64) * (*sample as f64))
            .sum()
    }

    #[test]
    fn profile_and_default_processing_run() {
        let mut effect = NoiseReduction::default();
        let mut buf = sine(4096);
        effect.get_profile(&buf).expect("profile should be long enough");
        effect
            .process(&mut buf, &mut EffectContext::default())
            .expect("processing should succeed");
        assert_eq!(buf.len(), 4096);
    }

    #[test]
    fn empty_input_does_not_panic() {
        let mut effect = NoiseReduction::default();
        let mut buf = AudioBuffer::from_channels(8000, vec![Vec::new(), Vec::new()]);
        assert!(effect.get_profile(&buf).is_err());
        assert!(effect.process(&mut buf, &mut EffectContext::default()).is_err());
    }

    #[test]
    fn repeated_processing_is_identical() {
        let source = sine(4096);
        let mut first = NoiseReduction::default();
        let mut second = NoiseReduction::default();
        first.get_profile(&source).unwrap();
        second.get_profile(&source).unwrap();

        let mut a = source.clone();
        let mut b = source;
        first.process(&mut a, &mut EffectContext::default()).unwrap();
        second.process(&mut b, &mut EffectContext::default()).unwrap();
        assert_eq!(a.channels, b.channels);
    }

    #[test]
    fn parameters_clamp_to_boundaries() {
        let mut effect = NoiseReduction::default();
        assert!(set_param(&mut effect, "NoiseReduction", 1000.0));
        assert_eq!(effect.noise_reduction, 48.0);
        assert!(set_param(&mut effect, "Sensitivity", -10.0));
        assert_eq!(effect.sensitivity, 0.01);
        assert!(set_param(&mut effect, "FrequencySmoothing", -1.0));
        assert_eq!(effect.frequency_smoothing, 0.0);
        assert!(!set_param(&mut effect, "not-a-parameter", 1.0));
    }

    #[test]
    fn residue_of_profiled_noise_has_finite_output() {
        let source = sine(4096);
        let mut effect = NoiseReduction::default();
        effect.get_profile(&source).unwrap();
        effect.noise_reduction_choice = 1.0;

        let mut residue = source;
        effect
            .process(&mut residue, &mut EffectContext::default())
            .unwrap();
        assert!(residue.channels.iter().flatten().all(|sample| sample.is_finite()));
        assert!(energy(&residue) < 1.0e6);
    }

    #[test]
    fn missing_profile_is_reported() {
        let mut effect = NoiseReduction::default();
        let mut buf = sine(4096);
        assert!(effect
            .process(&mut buf, &mut EffectContext::default())
            .is_err());
    }
}
