// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

use crate::core::*;
use au_core::AudioBuffer;

/// Nyquist's Pluck generator.
pub struct Pluck {
    /// MIDI pitch.
    pub pitch: i32,
    /// Fade-out choice: 0 is Abrupt, 1 is Gradual.
    pub fade: usize,
    /// Generated duration in seconds.
    pub duration: f64,
}

impl Default for Pluck {
    fn default() -> Self {
        Self {
            pitch: 60,
            fade: 0,
            duration: 1.0,
        }
    }
}

impl BuiltinEffect for Pluck {
    fn id(&self) -> &'static str {
        "Pluck"
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Generate
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::int("PITCH", "Pluck MIDI pitch", 1.0, 120.0, 60.0),
            ParamDef::choice(
                "FADE",
                "Fade-out type",
                &["Abrupt", "Gradual"],
                0,
            ),
            ParamDef::float("DUR", "Duration (60s max)", 0.0, 60.0, 1.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "PITCH" => Some(self.pitch as f64),
            "FADE" => Some(self.fade as f64),
            "DUR" => Some(self.duration),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "PITCH" => {
                self.pitch = ParamDef::int("PITCH", "", 1.0, 120.0, 60.0)
                    .clamp(value) as i32;
                true
            }
            "FADE" => {
                self.fade = ParamDef::choice(
                    "FADE",
                    "",
                    &["Abrupt", "Gradual"],
                    0,
                )
                .clamp(value) as usize;
                true
            }
            "DUR" => {
                self.duration =
                    ParamDef::float("DUR", "", 0.0, 60.0, 1.0).clamp(value);
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
        let sample_rate = buf.sample_rate;
        if sample_rate == 0 {
            return Err(EffectError::Invalid(
                "Pluck requires a non-zero sample rate".into(),
            ));
        }

        let pitch = ParamDef::int("PITCH", "", 1.0, 120.0, 60.0)
            .clamp(self.pitch as f64) as i32;
        let fade = ParamDef::choice("FADE", "", &["Abrupt", "Gradual"], 0)
            .clamp(self.fade as f64) as usize;
        let duration = ParamDef::float("DUR", "", 0.0, 60.0, 1.0)
            .clamp(self.duration);

        let channel_count = buf.channels.len().max(1);
        if duration == 0.0 {
            buf.channels = vec![Vec::new(); channel_count];
            return Ok(());
        }

        let generated = generate_pluck(sample_rate, pitch, fade, duration)?;
        buf.channels = vec![generated; channel_count];
        Ok(())
    }
}

fn generate_pluck(
    sample_rate: u32,
    pitch: i32,
    fade: usize,
    duration: f64,
) -> Result<Vec<f32>, EffectError> {
    let sr = sample_rate as f64;
    let hz = (0.057_762_265_046_662_1 * pitch as f64
        + 2.101_178_438_692_621_3)
        .exp();
    let final_amp = if fade == 1 { 0.001 } else { 0.000_001 };

    let (stretch, cons, loss, len) = pluck_parameters(hz, sr, final_amp, duration)?;

    let mut shiftreg = vec![0.0_f32; len + 4];
    initialize_pluck(&mut shiftreg, len, cons);

    let x2 = -cons * (stretch - 1.0);
    let x3 = cons * stretch - stretch + 1.0;

    let output_len = (duration * sr + 0.5).floor() as usize;
    let mut output = Vec::with_capacity(output_len);

    let mut i1 = len + 1;
    let mut i2 = len;
    let mut i3 = len - 1;
    let mut i4 = len - 2;
    let endptr = len + 2;

    for _ in 0..output_len {
        let sum = (shiftreg[i1] as f64 * x2
            + shiftreg[i2] as f64 * x3
            + shiftreg[i3] as f64 * stretch
            - shiftreg[i4] as f64 * cons) as f32;

        i1 += 1;
        i2 += 1;
        i3 += 1;
        i4 += 1;
        if i1 == endptr {
            i1 = 0;
        }
        if i2 == endptr {
            i2 = 0;
        }
        if i3 == endptr {
            i3 = 0;
        }
        if i4 == endptr {
            i4 = 0;
        }

        shiftreg[i4] = (sum as f64 * loss) as f32;
        output.push(sum);
    }

    let peak = output
        .iter()
        .fold(0.0_f32, |current, sample| current.max(sample.abs()));
    if peak > 0.0 {
        let scale = 0.8_f64 / peak as f64;
        for sample in &mut output {
            *sample = (*sample as f64 * scale) as f32;
        }
    }

    Ok(output)
}

fn pluck_parameters(
    hz: f64,
    sr: f64,
    final_amp: f64,
    duration: f64,
) -> Result<(f64, f64, f64, usize), EffectError> {
    const MAX_LENGTH: f64 = 20_000.0;
    let t = std::f64::consts::PI * (hz / sr);
    let y = t.cos().abs();
    let lf = -final_amp.ln();
    let tdecay = -lf / (hz * y.ln());

    if hz <= sr / MAX_LENGTH {
        return Err(EffectError::Invalid("pluck hz is too low".into()));
    }
    if hz >= sr / 3.0 {
        return Err(EffectError::Invalid("pluck hz is too high".into()));
    }

    let st = hz * duration;
    let (rho, stretch) = if duration < tdecay {
        ((-lf / st).exp() / y, 0.5)
    } else {
        let stretch = 0.5
            + (0.25
                - (1.0 - (2.0 * lf * (hz - sr) / (st * sr)).exp())
                    / (2.0 - 2.0 * (2.0 * t).cos()))
            .sqrt();
        (1.0, stretch)
    };

    let len = ((sr / hz) - stretch - 0.001) as usize;
    let x = (sr / hz) - len as f64 - stretch;
    let cons = (1.0 - x) / (1.0 + x);

    if len <= 1 {
        return Err(EffectError::Invalid(
            "internal error: pluck delay line length too short".into(),
        ));
    }

    Ok((stretch, cons, rho, len))
}

fn initialize_pluck(shiftreg: &mut [f32], len: usize, cons: f64) {
    let _ = cons;
    let mut random_state = 1_u32;
    let mut sum = 0.0_f32;

    // The C routine receives shiftreg + len - 1 as `array`. It writes
    // array[len] first, then fills indices len - 1 down to zero.
    shiftreg[len] = 0.0;
    for index in (0..len).rev() {
        random_state = random_state
            .wrapping_mul(1_103_515_245)
            .wrapping_add(12_345);
        let value = (((random_state >> 16) & 0x7fff) & 2) as i32 - 1;
        shiftreg[index] = value as f32;
        sum += shiftreg[index];
    }

    let average = sum / len as f32;
    for sample in shiftreg.iter_mut().take(len + 2) {
        *sample -= average;
    }
    shiftreg[len] = 0.0;
    shiftreg[len + 1] = 0.0;
}

/// Returns the built-in effects in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(Pluck::default())]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_buffer() -> AudioBuffer {
        let sample_rate = 44_100;
        let channel: Vec<f32> = (0..256)
            .map(|i| (i as f32 * 0.07).sin() * 0.2)
            .collect();
        AudioBuffer::from_channels(sample_rate, vec![channel.clone(), channel])
    }

    #[test]
    fn default_runs_on_stereo_audio() {
        let mut effect = Pluck::default();
        let mut buffer = test_buffer();
        let mut context = EffectContext::default();
        effect.process(&mut buffer, &mut context).unwrap();
        assert_eq!(buffer.channels.len(), 2);
        assert_eq!(buffer.len(), 44_100);
    }

    #[test]
    fn empty_buffer_does_not_panic() {
        let mut effect = Pluck::default();
        let mut buffer = AudioBuffer::from_channels(44_100, vec![]);
        let mut context = EffectContext::default();
        effect.process(&mut buffer, &mut context).unwrap();
        assert_eq!(buffer.len(), 44_100);
        assert_eq!(buffer.channels.len(), 1);
    }

    #[test]
    fn output_is_repeatable() {
        let mut first = test_buffer();
        let mut second = test_buffer();
        let mut context = EffectContext::default();

        Pluck::default()
            .process(&mut first, &mut context)
            .unwrap();
        Pluck::default()
            .process(&mut second, &mut context)
            .unwrap();

        assert_eq!(first.channels, second.channels);
    }

    #[test]
    fn parameters_clamp() {
        let mut effect = Pluck::default();
        assert!(set_param(&mut effect, "PITCH", 1000.0));
        assert_eq!(effect.pitch, 120);
        assert!(set_param(&mut effect, "FADE", -5.0));
        assert_eq!(effect.fade, 0);
        assert!(set_param(&mut effect, "DUR", 100.0));
        assert_eq!(effect.duration, 60.0);
        assert!(!set_param(&mut effect, "unknown", 1.0));
    }
}
