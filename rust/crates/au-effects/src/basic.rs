// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

use au_core::AudioBuffer;

/// An audio effect that modifies a buffer in place.
pub trait Effect {
    fn name(&self) -> &str;
    fn process(&mut self, buf: &mut AudioBuffer);
}

/// Applies a fixed gain in decibels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Amplify {
    pub gain_db: f32,
}

impl Amplify {
    pub fn new(db: f32) -> Self {
        Self { gain_db: db }
    }
}

impl Effect for Amplify {
    fn name(&self) -> &str {
        "Amplify"
    }

    fn process(&mut self, buf: &mut AudioBuffer) {
        let gain = db_to_linear(self.gain_db);
        for channel in &mut buf.channels {
            for sample in channel {
                *sample *= gain;
            }
        }
    }
}

/// Scales a buffer so its peak reaches the requested level in decibels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Normalize {
    pub peak_db: f32,
    pub remove_dc: bool,
}

impl Normalize {
    pub fn new(peak_db: f32, remove_dc: bool) -> Self {
        Self {
            peak_db,
            remove_dc,
        }
    }
}

impl Effect for Normalize {
    fn name(&self) -> &str {
        "Normalize"
    }

    fn process(&mut self, buf: &mut AudioBuffer) {
        if self.remove_dc {
            for channel in &mut buf.channels {
                if !channel.is_empty() {
                    let mean =
                        channel.iter().map(|&sample| sample as f64).sum::<f64>()
                            / channel.len() as f64;
                    for sample in channel {
                        *sample -= mean as f32;
                    }
                }
            }
        }

        let peak = buf.peak();
        if peak > 0.0 {
            let scale = db_to_linear(self.peak_db) / peak;
            for channel in &mut buf.channels {
                for sample in channel {
                    *sample *= scale;
                }
            }
        }
    }
}

/// Applies a linear fade from silence to full level over the buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FadeIn;

impl Effect for FadeIn {
    fn name(&self) -> &str {
        "Fade In"
    }

    fn process(&mut self, buf: &mut AudioBuffer) {
        let frames = buf.len();
        if frames == 0 {
            return;
        }

        for channel in &mut buf.channels {
            for (frame, sample) in channel.iter_mut().take(frames).enumerate() {
                let gain = if frames == 1 {
                    1.0
                } else {
                    frame as f32 / (frames - 1) as f32
                };
                *sample *= gain;
            }
        }
    }
}

/// Applies a linear fade from full level to silence over the buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FadeOut;

impl Effect for FadeOut {
    fn name(&self) -> &str {
        "Fade Out"
    }

    fn process(&mut self, buf: &mut AudioBuffer) {
        let frames = buf.len();
        if frames == 0 {
            return;
        }

        for channel in &mut buf.channels {
            for (frame, sample) in channel.iter_mut().take(frames).enumerate() {
                let gain = if frames == 1 {
                    1.0
                } else {
                    1.0 - frame as f32 / (frames - 1) as f32
                };
                *sample *= gain;
            }
        }
    }
}

/// Reverses the samples in each channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reverse;

impl Effect for Reverse {
    fn name(&self) -> &str {
        "Reverse"
    }

    fn process(&mut self, buf: &mut AudioBuffer) {
        for channel in &mut buf.channels {
            channel.reverse();
        }
    }
}

/// Returns the linear amplitude corresponding to a decibel value.
pub fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Returns the decibel value corresponding to a linear amplitude.
pub fn linear_to_db(linear: f32) -> f32 {
    if linear == 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * linear.abs().log10()
    }
}

/// Creates the built-in effects with their default settings.
pub fn builtin_effects() -> Vec<Box<dyn Effect>> {
    vec![
        Box::new(Amplify::new(0.0)),
        Box::new(Normalize::new(0.0, false)),
        Box::new(FadeIn),
        Box::new(FadeOut),
        Box::new(Reverse),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(samples: Vec<f32>) -> AudioBuffer {
        AudioBuffer::from_channels(44_100, vec![samples])
    }

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-5,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn amplify_scales_samples() {
        let mut buf = buffer(vec![0.5, -0.5]);
        Amplify::new(6.0206).process(&mut buf);
        close(buf.channels[0][0], 1.0);
        close(buf.channels[0][1], -1.0);
    }

    #[test]
    fn normalize_reaches_target_peak() {
        let mut buf = buffer(vec![0.25, -0.5]);
        Normalize::new(-6.0206, false).process(&mut buf);
        close(buf.peak(), 0.5);
        close(buf.channels[0][0], 0.25);
    }

    #[test]
    fn normalize_can_remove_dc() {
        let mut buf = buffer(vec![2.0, 4.0]);
        Normalize::new(0.0, true).process(&mut buf);
        close(buf.channels[0][0], -1.0);
        close(buf.channels[0][1], 1.0);
    }

    #[test]
    fn normalize_leaves_silence_unchanged() {
        let mut buf = buffer(vec![0.0, 0.0]);
        Normalize::new(0.0, false).process(&mut buf);
        assert_eq!(buf.channels[0], vec![0.0, 0.0]);
    }

    #[test]
    fn fade_in_is_linear() {
        let mut buf = buffer(vec![1.0; 3]);
        FadeIn.process(&mut buf);
        assert_eq!(buf.channels[0], vec![0.0, 0.5, 1.0]);
    }

    #[test]
    fn fade_out_is_linear() {
        let mut buf = buffer(vec![1.0; 3]);
        FadeOut.process(&mut buf);
        assert_eq!(buf.channels[0], vec![1.0, 0.5, 0.0]);
    }

    #[test]
    fn reverse_reverses_each_channel() {
        let mut buf = AudioBuffer::from_channels(
            44_100,
            vec![vec![1.0, 2.0], vec![3.0, 4.0]],
        );
        Reverse.process(&mut buf);
        assert_eq!(buf.channels, vec![vec![2.0, 1.0], vec![4.0, 3.0]]);
    }

    #[test]
    fn decibel_conversions_round_trip() {
        close(db_to_linear(0.0), 1.0);
        close(linear_to_db(1.0), 0.0);
        close(db_to_linear(linear_to_db(0.25)), 0.25);
    }

    #[test]
    fn builtins_are_available() {
        assert_eq!(builtin_effects().len(), 5);
    }
}
