// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

//! Audacity's Find Clipping analyzer.

use crate::core::*;
use au_core::AudioBuffer;

/// The Audacity audio threshold used to detect clipping.
const MAX_AUDIO: f32 = 1.0;

const DUTY_CYCLE_START: &str = "Duty Cycle Start";
const DUTY_CYCLE_END: &str = "Duty Cycle End";

/// Finds runs of clipped samples and appends labels describing them.
pub struct FindClipping {
    /// Minimum number of consecutive clipped samples required to start a run.
    pub duty_cycle_start: i32,
    /// Minimum number of consecutive non-clipped samples required to end a run.
    pub duty_cycle_end: i32,
}

impl Default for FindClipping {
    fn default() -> Self {
        Self {
            duty_cycle_start: 3,
            duty_cycle_end: 3,
        }
    }
}

impl BuiltinEffect for FindClipping {
    fn id(&self) -> &'static str {
        "FindClipping"
    }

    fn name(&self) -> &'static str {
        "Find Clipping"
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Analyze
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::int(
                DUTY_CYCLE_START,
                DUTY_CYCLE_START,
                1.0,
                i32::MAX as f64,
                3.0,
            ),
            ParamDef::int(
                DUTY_CYCLE_END,
                DUTY_CYCLE_END,
                1.0,
                i32::MAX as f64,
                3.0,
            ),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            DUTY_CYCLE_START => Some(self.duty_cycle_start as f64),
            DUTY_CYCLE_END => Some(self.duty_cycle_end as f64),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let definition = match key {
            DUTY_CYCLE_START => self.params().remove(0),
            DUTY_CYCLE_END => self.params().remove(1),
            _ => return false,
        };
        let value = definition.clamp(value) as i32;
        match key {
            DUTY_CYCLE_START => self.duty_cycle_start = value,
            DUTY_CYCLE_END => self.duty_cycle_end = value,
            _ => unreachable!(),
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let start_threshold = self.duty_cycle_start as u64;
        let stop_threshold = self.duty_cycle_end as u64;
        let sample_rate = buf.sample_rate;

        for channel in &buf.channels {
            let mut startrun = 0_u64;
            let mut stoprun = 0_u64;
            let mut samps = 0_u64;
            let mut start_time = -1.0_f64;

            for (s, &sample) in channel.iter().enumerate() {
                let clipped = sample.abs() >= MAX_AUDIO;

                if clipped {
                    if startrun == 0 {
                        start_time = if sample_rate == 0 {
                            0.0
                        } else {
                            s as f64 / sample_rate as f64
                        };
                        samps = 0;
                    } else {
                        stoprun = 0;
                    }
                    startrun += 1;
                    samps += 1;
                } else if startrun >= start_threshold {
                    stoprun += 1;
                    samps += 1;

                    if stoprun >= stop_threshold {
                        let end_sample = s as u64 - stop_threshold;
                        let end_time = if sample_rate == 0 {
                            0.0
                        } else {
                            end_sample as f64 / sample_rate as f64
                        };

                        ctx.labels.push(OutLabel {
                            t0: start_time,
                            t1: end_time,
                            text: format!("{} of {}", startrun, samps - stop_threshold),
                        });

                        startrun = 0;
                        stoprun = 0;
                        samps = 0;
                    }
                } else {
                    startrun = 0;
                }
            }
        }

        Ok(())
    }
}

/// Returns the Find Clipping analyzer.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(FindClipping::default())]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_stereo_sine_does_not_panic_or_label() {
        let samples: Vec<f32> = (0..256)
            .map(|i| (i as f32 * 0.07).sin() * 0.5)
            .collect();
        let mut buffer =
            AudioBuffer::from_channels(48_000, vec![samples.clone(), samples]);
        let mut context = EffectContext::default();

        FindClipping::default()
            .process(&mut buffer, &mut context)
            .unwrap();

        assert!(context.labels.is_empty());
    }

    #[test]
    fn empty_buffer_does_not_panic() {
        let mut buffer = AudioBuffer::from_channels(48_000, vec![Vec::new(), Vec::new()]);
        let mut context = EffectContext::default();

        FindClipping::default()
            .process(&mut buffer, &mut context)
            .unwrap();

        assert!(context.labels.is_empty());
    }

    #[test]
    fn labels_match_audacity_counts_and_are_repeatable() {
        let mut samples = vec![0.0_f32; 16];
        samples[3..8].fill(1.0);

        let run = || {
            let mut buffer =
                AudioBuffer::from_channels(48_000, vec![samples.clone(), samples.clone()]);
            let mut context = EffectContext::default();
            FindClipping::default()
                .process(&mut buffer, &mut context)
                .unwrap();
            context.labels
        };

        let first = run();
        let second = run();
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].text, "5 of 5");
        assert_eq!(first[0].t0, 3.0 / 48_000.0);
        assert_eq!(first[0].t1, 7.0 / 48_000.0);
    }

    #[test]
    fn parameters_clamp_to_their_ranges() {
        let mut effect = FindClipping::default();

        assert!(set_param(&mut effect, DUTY_CYCLE_START, 0.0));
        assert_eq!(effect.duty_cycle_start, 1);

        assert!(set_param(
            &mut effect,
            DUTY_CYCLE_END,
            (i32::MAX as f64) + 1.0
        ));
        assert_eq!(effect.duty_cycle_end, i32::MAX);
        assert!(!set_param(&mut effect, "Unknown", 1.0));
    }
}
