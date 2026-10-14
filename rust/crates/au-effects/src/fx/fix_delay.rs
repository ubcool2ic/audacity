use crate::core::*;
use au_core::AudioBuffer;

const DELAY_TYPES: &[&str] = &["Regular", "BouncingBall", "ReverseBouncingBall"];
const PITCH_TYPES: &[&str] = &["PitchTempo", "LQPitchShift", "HQPitchShift"];
const CONSTRAIN_OPTIONS: &[&str] = &["Yes", "No"];

/// Audacity's Delay effect.
#[derive(Clone, Debug)]
pub struct Delay {
    /// Delay type choice index.
    pub delay_type: f64,
    /// Gain change per echo, in decibels.
    pub dgain: f64,
    /// Delay time, in seconds.
    pub delay: f64,
    /// Pitch change algorithm choice index.
    pub pitch_type: f64,
    /// Pitch change per echo, in semitones.
    pub shift: f64,
    /// Number of echoes.
    pub number: f64,
    /// Whether to constrain the result to the selection duration.
    pub constrain: f64,
}

impl Default for Delay {
    fn default() -> Self {
        Self {
            delay_type: 0.0,
            dgain: -6.0,
            delay: 0.3,
            pitch_type: 0.0,
            shift: 0.0,
            number: 5.0,
            constrain: 0.0,
        }
    }
}

impl Delay {
    fn echo_offset_seconds(&self, echo_num: usize, number: usize) -> f64 {
        let mut time_shift = self.delay;
        if self.delay_type >= 1.0 {
            time_shift = self.delay / number as f64;
        }

        match self.delay_type.round() as usize {
            1 => {
                let mut dly = 0.0;
                for count in 1..=echo_num {
                    dly += time_shift * (number + 1 - count) as f64;
                }
                dly
            }
            2 => {
                let mut dly = 0.0;
                for count in 1..=echo_num {
                    dly += time_shift * count as f64;
                }
                dly
            }
            _ => time_shift * echo_num as f64,
        }
    }

    fn echo_gain(&self, echo_num: usize) -> f32 {
        let db = echo_num as f64 * self.dgain;
        // Nyquist's db-to-linear computes exp(ln(10)/20 * db).
        (0.11512925464970229_f64 * db).exp() as f32
    }

    fn shifted_length(&self, input_len: usize, ratio: f64, pitch_type: usize) -> usize {
        if input_len == 0 {
            return 0;
        }
        if pitch_type == 0 {
            ((input_len as f64 / ratio).round() as usize).max(1)
        } else {
            input_len
        }
    }

    fn change_speed(input: &[f32], ratio: f64, output_len: usize) -> Vec<f32> {
        if ratio == 1.0 {
            return input.to_vec();
        }
        if input.is_empty() || output_len == 0 {
            return Vec::new();
        }

        let mut output = Vec::with_capacity(output_len);
        for i in 0..output_len {
            let position = i as f64 * ratio;
            let index = position.floor() as usize;
            let fraction = (position - index as f64) as f32;
            if index + 1 < input.len() {
                output.push(input[index] + (input[index + 1] - input[index]) * fraction);
            } else if index < input.len() {
                output.push(input[index]);
            } else {
                output.push(0.0);
            }
        }
        output
    }

    // A simple overlap-add pitch shifter used for Nyquist's non-speed pitch modes.
    fn overlap_add_pitch(input: &[f32], ratio: f64) -> Vec<f32> {
        if input.is_empty() || ratio == 1.0 {
            return input.to_vec();
        }

        const GRAIN: usize = 2048;
        const HOP: usize = 512;
        let mut output = vec![0.0_f32; input.len()];
        let mut weights = vec![0.0_f32; input.len()];

        let mut start = 0usize;
        while start < input.len() {
            for j in 0..GRAIN {
                let out_pos = start + j;
                if out_pos >= input.len() {
                    break;
                }

                let centered = j as f64 - (GRAIN / 2) as f64;
                let source_pos = start as f64 + (GRAIN / 2) as f64 + centered * ratio;
                let source_pos = source_pos.clamp(0.0, input.len().saturating_sub(1) as f64);
                let source_index = source_pos.floor() as usize;
                let fraction = (source_pos - source_index as f64) as f32;
                let sample = if source_index + 1 < input.len() {
                    input[source_index]
                        + (input[source_index + 1] - input[source_index]) * fraction
                } else {
                    input[source_index]
                };

                let phase = j as f32 / (GRAIN - 1) as f32;
                let window = 0.5 - 0.5 * (std::f32::consts::TAU * phase).cos();
                output[out_pos] += sample * window;
                weights[out_pos] += window;
            }
            start = start.saturating_add(HOP);
        }

        for (sample, weight) in output.iter_mut().zip(weights) {
            if weight > 1.0e-12 {
                *sample /= weight;
            }
        }
        output
    }

    fn process_channel(&self, input: &[f32], sample_rate: u32, output_len: usize) -> Vec<f32> {
        let number = self.number.round().clamp(1.0, 30.0) as usize;
        let pitch_type = self.pitch_type.round().clamp(0.0, 2.0) as usize;
        let rate = sample_rate as f64;
        let mut echoes = vec![0.0_f32; output_len];

        for echo_num in 1..=number {
            let ratio = 2.0_f64.powf(echo_num as f64 * self.shift / 12.0);
            let modified = if pitch_type == 0 {
                let len = self.shifted_length(input.len(), ratio, pitch_type);
                Self::change_speed(input, ratio, len)
            } else if self.shift == 0.0 {
                input.to_vec()
            } else {
                // Both Nyquist pitch-shift modes retain the sound duration.
                Self::overlap_add_pitch(input, ratio)
            };

            let gain = self.echo_gain(echo_num);
            let offset = (self.echo_offset_seconds(echo_num, number) * rate + 0.5) as usize;
            for (i, &sample) in modified.iter().enumerate() {
                let out_index = offset.saturating_add(i);
                if out_index < output_len {
                    echoes[out_index] += sample * gain;
                }
            }
        }

        for (i, &sample) in input.iter().enumerate() {
            if i < output_len {
                echoes[i] += sample;
            }
        }
        echoes
    }
}

impl BuiltinEffect for Delay {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Delay"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::choice("DELAY-TYPE", "Delay type", DELAY_TYPES, 0),
            ParamDef::float("DGAIN", "Delay level per echo (dB)", -30.0, 1.0, -6.0),
            ParamDef::float("DELAY", "Delay time (seconds)", 0.0, 5.0, 0.3),
            ParamDef::choice("PITCH-TYPE", "Pitch change effect", PITCH_TYPES, 0),
            ParamDef::float(
                "SHIFT",
                "Pitch change per echo (semitones)",
                -2.0,
                2.0,
                0.0,
            ),
            ParamDef::int("NUMBER", "Number of echoes", 1.0, 30.0, 5.0),
            ParamDef::choice("CONSTRAIN", "Allow duration to change", CONSTRAIN_OPTIONS, 0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "DELAY-TYPE" => Some(self.delay_type),
            "DGAIN" => Some(self.dgain),
            "DELAY" => Some(self.delay),
            "PITCH-TYPE" => Some(self.pitch_type),
            "SHIFT" => Some(self.shift),
            "NUMBER" => Some(self.number),
            "CONSTRAIN" => Some(self.constrain),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "DELAY-TYPE" => self.delay_type = value.clamp(0.0, 2.0).round(),
            "DGAIN" => self.dgain = value.clamp(-30.0, 1.0),
            "DELAY" => self.delay = value.clamp(0.0, 5.0),
            "PITCH-TYPE" => self.pitch_type = value.clamp(0.0, 2.0).round(),
            "SHIFT" => self.shift = value.clamp(-2.0, 2.0),
            "NUMBER" => self.number = value.clamp(1.0, 30.0).round(),
            "CONSTRAIN" => self.constrain = value.clamp(0.0, 1.0).round(),
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let input_len = buf.len();
        if input_len == 0 || buf.channels.is_empty() {
            return Ok(());
        }

        let sample_rate = buf.sample_rate;
        let number = self.number.round().clamp(1.0, 30.0) as usize;
        let pitch_type = self.pitch_type.round().clamp(0.0, 2.0) as usize;
        let rate = sample_rate as f64;

        let mut output_len = input_len;
        if self.constrain.round() != 0.0 {
            output_len = input_len;
        } else {
            for echo_num in 1..=number {
                let ratio = 2.0_f64.powf(echo_num as f64 * self.shift / 12.0);
                let transformed_len =
                    self.shifted_length(input_len, ratio, pitch_type);
                let offset =
                    (self.echo_offset_seconds(echo_num, number) * rate + 0.5) as usize;
                output_len = output_len.max(offset.saturating_add(transformed_len));
            }
        }

        let channels = buf
            .channels
            .iter()
            .map(|channel| self.process_channel(channel, sample_rate, output_len))
            .collect();
        buf.channels = channels;
        Ok(())
    }
}

/// Returns the effects provided by this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(Delay::default())]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_buffer(frames: usize) -> AudioBuffer {
        let channel: Vec<f32> = (0..frames)
            .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / 44_100.0).sin())
            .collect();
        AudioBuffer::from_channels(44_100, vec![channel.clone(), channel])
    }

    #[test]
    fn default_runs_on_stereo() {
        let mut buffer = sine_buffer(512);
        let mut context = EffectContext::default();
        Delay::default()
            .process(&mut buffer, &mut context)
            .unwrap();
        assert_eq!(buffer.channels.len(), 2);
        assert!(buffer.len() >= 512); // delay.ny may extend the selection (matches Audacity)
    }

    #[test]
    fn empty_buffer_does_not_panic() {
        let mut buffer = AudioBuffer::from_channels(44_100, vec![Vec::new(), Vec::new()]);
        let mut context = EffectContext::default();
        Delay::default()
            .process(&mut buffer, &mut context)
            .unwrap();
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn processing_is_repeatable() {
        let mut first = sine_buffer(512);
        let mut second = sine_buffer(512);
        let mut context = EffectContext::default();
        let mut a = Delay::default();
        let mut b = Delay::default();
        a.process(&mut first, &mut context).unwrap();
        b.process(&mut second, &mut context).unwrap();
        assert_eq!(first.channels, second.channels);
    }

    #[test]
    fn parameters_clamp() {
        let mut effect = Delay::default();
        assert!(set_param(&mut effect, "NUMBER", 100.0));
        assert_eq!(effect.number, 30.0);
        assert!(set_param(&mut effect, "DELAY-TYPE", -1.0));
        assert_eq!(effect.delay_type, 0.0);
        assert!(!set_param(&mut effect, "unknown", 1.0));
    }
}
