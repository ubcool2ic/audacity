// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

use crate::core::*;
use au_core::AudioBuffer;

const TABLE_LEN: usize = 2048;
const PI2: f64 = std::f64::consts::PI * 2.0;
const MAX_SAMPLE_BLOCK_LEN: usize = 1024;

/// Audacity's Studio Fade Out effect.
#[derive(Clone, Debug, Default)]
pub struct StudioFadeOut;

impl StudioFadeOut {
    fn rounded_sample_count(seconds: f64, sample_rate: f64) -> usize {
        (seconds * sample_rate + 0.5).floor().max(0.0) as usize
    }

    /// Reproduces Nyquist's table-based oscillator and raised-cosine construction.
    fn raised_cosine(duration: f64, sample_rate: u32, output_len: usize) -> Vec<f32> {
        if output_len == 0 || duration <= 0.0 || sample_rate == 0 {
            return vec![0.0; output_len];
        }

        // hz-to-step followed by calculate-hz, as performed by Nyquist's OSC.
        let requested_hz = 1.0 / (duration * 2.0);
        let pitch = 17.312_340_490_667_565 * (requested_hz.ln() - 2.101_178_438_692_621_3);
        let hz = 8.175_798_915_643_707 * 2.0_f64.powf(pitch / 12.0);

        let mut table = vec![0.0_f32; TABLE_LEN + 1];
        for (i, value) in table.iter_mut().take(TABLE_LEN).enumerate() {
            let angle = i as f64 * PI2 / TABLE_LEN as f64;
            *value = angle.sin() as f32;
        }
        table[TABLE_LEN] = table[0];

        // Nyquist's phase is specified in degrees; 90 degrees is a quarter table.
        let mut phase = 90.0 / 360.0 * TABLE_LEN as f64;
        let phase_increment = hz * TABLE_LEN as f64 / sample_rate as f64;
        let mut result = Vec::with_capacity(output_len);

        for _ in 0..output_len {
            let table_index = phase as usize;
            let x1 = table[table_index] as f64;
            let value = x1
                + (phase - table_index as f64)
                    * (table[table_index + 1] as f64 - x1);
            let oscillator = value as f32;

            // Nyquist's SUM and MULT operate on sample_type values.
            let raised = (1.0_f32 + oscillator) * 0.5_f32;
            result.push(raised);

            phase += phase_increment;
            while phase >= TABLE_LEN as f64 {
                phase -= TABLE_LEN as f64;
            }
        }
        result
    }

    /// Builds PWLV's control-rate cutoff samples, including the endpoint needed
    /// by the audio-rate linear interpolator.
    fn cutoff_control(
        duration: f64,
        sample_rate: u32,
        needed: usize,
    ) -> Vec<f32> {
        let control_rate = sample_rate as f64 / 20.0;
        let endpoint = Self::rounded_sample_count(duration, control_rate);
        let nyquist_hz = sample_rate as f64 / 2.0;
        let increment = if endpoint == 0 {
            0.0
        } else {
            (100.0 - nyquist_hz) / endpoint as f64
        };

        let mut result = Vec::with_capacity(needed);
        let mut block_start = 0usize;
        let mut level_at_block_start = nyquist_hz;

        while result.len() < needed {
            let block_len = MAX_SAMPLE_BLOCK_LEN.min(needed - result.len());
            for offset in 0..block_len {
                let sample_index = block_start + offset;
                let level = if sample_index < endpoint {
                    // PWL emits a double level and casts each sample to sample_type.
                    let mut value = level_at_block_start;
                    for _ in 0..offset {
                        value += increment;
                    }
                    value as f32
                } else {
                    100.0_f32
                };
                result.push(level);
            }

            // pwl.c updates the suspension level in one operation at a block edge.
            level_at_block_start += increment * block_len as f64;
            block_start += block_len;
        }

        result
    }

    /// Applies Nyquist's variable-cutoff one-pole low-pass filter.
    fn low_pass_variable(
        input: &[f32],
        sample_rate: u32,
        duration: f64,
    ) -> Vec<f32> {
        let frames = input.len();
        if frames == 0 || sample_rate == 0 {
            return vec![0.0; frames];
        }

        let control_rate = sample_rate as f64 / 20.0;
        let needed_control_samples = frames / 20 + 3;
        let control = Self::cutoff_control(duration, sample_rate, needed_control_samples);

        let mut phase = 0.0_f64;
        let phase_increment = control_rate / sample_rate as f64;
        let mut control_index = 0usize;
        let mut x1 = control[0];
        let mut x2 = control[1];
        let mut previous_output = 0.0_f64;
        let mut output = Vec::with_capacity(frames);

        for &sample in input {
            if phase >= 1.0 {
                x1 = x2;
                control_index += 1;
                x2 = control[(control_index + 1).min(control.len() - 1)];
                phase -= 1.0;
            }

            // snd-tonev receives the control sound interpolated at audio rate.
            let cutoff = (x1 as f64 * (1.0 - phase) + x2 as f64 * phase) as f32;
            let b = 2.0 - (cutoff as f64 * PI2 / sample_rate as f64).cos();
            let c2 = b - (b * b - 1.0).sqrt();
            let c1 = (1.0 - c2) * 1.0_f64;
            previous_output = c1 * sample as f64 + c2 * previous_output;
            output.push(previous_output as f32);

            phase += phase_increment;
        }

        output
    }

    fn process_channel(input: &[f32], sample_rate: u32) -> Result<Vec<f32>, EffectError> {
        let frames = input.len();
        if frames == 0 {
            return Ok(Vec::new());
        }
        if frames < 3 {
            return Err(EffectError::Invalid(
                "Selection too short.\nIt must be more than 2 samples.".into(),
            ));
        }
        if sample_rate == 0 {
            return Ok(input.to_vec());
        }

        let duration = frames as f64 / sample_rate as f64;
        let fade = Self::raised_cosine(duration, sample_rate, frames);

        if duration < 0.2 {
            return Ok(input
                .iter()
                .zip(fade)
                .map(|(&sample, envelope)| sample * envelope)
                .collect());
        }

        let crossfade_duration = (duration / 2.0).min(0.5);
        let fade_out_filter = Self::raised_cosine(
            crossfade_duration,
            sample_rate,
            Self::rounded_sample_count(crossfade_duration, sample_rate as f64),
        );
        let filtered = Self::low_pass_variable(input, sample_rate, duration);

        let mut output = Vec::with_capacity(frames);
        for i in 0..frames {
            let f_out = fade_out_filter.get(i).copied().unwrap_or(0.0);
            let f_in = 1.0_f32 - f_out;
            let full_band = f_out * input[i];
            let low_pass = f_in * filtered[i];
            let filter_mix = full_band + low_pass;
            output.push(filter_mix * fade[i]);
        }
        Ok(output)
    }
}

impl BuiltinEffect for StudioFadeOut {
    fn id(&self) -> &'static str {
        "StudioFadeOut"
    }

    fn name(&self) -> &'static str {
        "Studio Fade Out"
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
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if buf.channels.is_empty() || buf.len() == 0 {
            return Ok(());
        }

        for channel in &mut buf.channels {
            *channel = Self::process_channel(channel, buf.sample_rate)?;
        }
        Ok(())
    }
}

/// Returns this module's built-in effects.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(StudioFadeOut)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_buffer(frames: usize) -> AudioBuffer {
        let sample_rate = 48_000;
        let channel = (0..frames)
            .map(|i| {
                (2.0 * std::f64::consts::PI * 440.0 * i as f64 / sample_rate as f64)
                    .sin() as f32
            })
            .collect::<Vec<_>>();
        AudioBuffer::from_channels(sample_rate, vec![channel.clone(), channel])
    }

    #[test]
    fn default_processes_stereo_without_panicking() {
        let mut buffer = sine_buffer(4096);
        let mut effect = StudioFadeOut;
        effect
            .process(&mut buffer, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buffer.channels.len(), 2);
        assert_eq!(buffer.channels[0].len(), 4096);
    }

    #[test]
    fn empty_buffer_is_safe() {
        let mut buffer = AudioBuffer::from_channels(48_000, Vec::new());
        StudioFadeOut
            .process(&mut buffer, &mut EffectContext::default())
            .unwrap();
        assert!(buffer.channels.is_empty());
    }

    #[test]
    fn processing_is_repeatable() {
        let mut first = sine_buffer(4096);
        let mut second = sine_buffer(4096);
        let mut effect = StudioFadeOut;
        effect
            .process(&mut first, &mut EffectContext::default())
            .unwrap();
        effect
            .process(&mut second, &mut EffectContext::default())
            .unwrap();
        assert_eq!(first.channels, second.channels);
    }

    #[test]
    fn no_parameters_and_unknown_set_are_safe() {
        let mut effect = StudioFadeOut;
        assert!(effect.params().is_empty());
        assert!(!set_param(&mut effect, "Unknown", 10.0));
    }
}
