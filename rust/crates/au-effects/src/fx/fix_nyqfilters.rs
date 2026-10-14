//! Nyquist-compatible ports of Audacity's high-pass, low-pass, and notch filters.

use crate::core::*;
use au_core::AudioBuffer;

const ROLLOFF_CHOICES: &[&str] = &["dB6", "dB12", "dB24", "dB36", "dB48"];
const PI: f64 = std::f64::consts::PI;

#[derive(Clone, Copy)]
enum FilterKind {
    HighPass,
    LowPass,
}

#[derive(Clone, Copy)]
enum Stage {
    /// Nyquist's `tone` one-pole state.
    Tone { c1: f64, c2: f64, prev: f64 },
    /// Nyquist's `atone` one-pole state.
    Atone { cc: f64, prev: f64 },
    /// Nyquist's `snd-biquad` state and coefficients.
    Biquad {
        b0: f64,
        b1: f64,
        b2: f64,
        a1: f64,
        a2: f64,
        z1: f64,
        z2: f64,
    },
}

impl Stage {
    fn process(&mut self, sample: f32) -> f32 {
        match self {
            Stage::Tone { c1, c2, prev } => {
                let current = sample as f64;
                *prev = *c1 * current + *c2 * *prev;
                *prev as f32
            }
            Stage::Atone { cc, prev } => {
                let current = sample as f64;
                *prev = *cc * (*prev + current);
                let output = *prev as f32;
                *prev -= current;
                output
            }
            Stage::Biquad {
                b0,
                b1,
                b2,
                a1,
                a2,
                z1,
                z2,
            } => {
                let z0 = sample as f64 + *a1 * *z1 + *a2 * *z2;
                let output = (z0 * *b0 + *z1 * *b1 + *z2 * *b2) as f32;
                *z2 = *z1;
                *z1 = z0;
                output
            }
        }
    }
}

fn validate_frequency(frequency: f64, sample_rate: u32) -> Result<(), EffectError> {
    if frequency < 0.1 {
        return Err(EffectError::Invalid(
            "Frequency must be at least 0.1 Hz.".into(),
        ));
    }
    if sample_rate == 0 || frequency >= sample_rate as f64 / 2.0 {
        return Err(EffectError::Invalid(
            "Frequency must be less than half the track sample rate.".into(),
        ));
    }
    Ok(())
}

/// Apply `nyq:biquad-m`'s feedback sign conversion, followed by
/// `nyq:biquad`'s normalization. Nyquist's `snd-biquad` stores both the
/// resulting coefficients and its delay elements as doubles.
fn make_biquad(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Stage {
    let a0r = 1.0 / a0;
    Stage::Biquad {
        b0: a0r * b0,
        b1: a0r * b1,
        b2: a0r * b2,
        a1: a0r * -a1,
        a2: a0r * -a2,
        z1: 0.0,
        z2: 0.0,
    }
}

fn make_lowpass2(frequency: f64, sample_rate: u32, q: f64) -> Stage {
    let w = 2.0 * PI * (frequency / sample_rate as f64);
    let cw = w.cos();
    let sw = w.sin();
    let alpha = sw * (0.5 / q).sinh();
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cw;
    let a2 = 1.0 - alpha;
    let b1 = 1.0 - cw;
    let b0 = 0.5 * b1;
    let b2 = b0;

    make_biquad(b0, b1, b2, a0, a1, a2)
}

fn make_highpass2(frequency: f64, sample_rate: u32, q: f64) -> Stage {
    let w = 2.0 * PI * (frequency / sample_rate as f64);
    let cw = w.cos();
    let sw = w.sin();
    let alpha = sw * (0.5 / q).sinh();
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cw;
    let a2 = 1.0 - alpha;

    // Nyquist's `b1 = (-1.0 - cw)` and `b0 = (-0.5 * b1)`.
    let b1 = -1.0 - cw;
    let b0 = -0.5 * b1;
    let b2 = b0;

    make_biquad(b0, b1, b2, a0, a1, a2)
}

fn make_notch2(frequency: f64, sample_rate: u32, q: f64) -> Stage {
    let w = 2.0 * PI * (frequency / sample_rate as f64);
    let cw = w.cos();
    let sw = w.sin();

    // `nyq:notch2` uses the same sinh-based alpha as Nyquist's other
    // second-order filters, not the conventional RBJ sin(w)/(2*Q).
    let alpha = sw * (0.5 / q).sinh();
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cw;
    let a2 = 1.0 - alpha;

    // `nyq:notch2` calls `nyq:biquad-m`, which negates a1 and a2 before
    // `nyq:biquad` divides every coefficient by a0.
    make_biquad(1.0, a1, 1.0, a0, a1, a2)
}

fn make_one_pole(kind: FilterKind, frequency: f64, sample_rate: u32) -> Stage {
    let b = 2.0 - (frequency * (2.0 * PI) / sample_rate as f64).cos();
    let c2 = b - ((b * b) - 1.0).sqrt();

    match kind {
        FilterKind::LowPass => Stage::Tone {
            c1: 1.0 - c2,
            c2,
            prev: 0.0,
        },
        FilterKind::HighPass => Stage::Atone { cc: c2, prev: 0.0 },
    }
}

fn make_stages(
    kind: FilterKind,
    frequency: f64,
    rolloff: f64,
    sample_rate: u32,
) -> Result<Vec<Stage>, EffectError> {
    validate_frequency(frequency, sample_rate)?;

    let choice = rolloff.round().clamp(0.0, 4.0) as usize;
    let stages = match (kind, choice) {
        (_, 0) => vec![make_one_pole(kind, frequency, sample_rate)],
        (FilterKind::LowPass, 1) => vec![make_lowpass2(frequency, sample_rate, 0.7071)],
        (FilterKind::HighPass, 1) => vec![make_highpass2(frequency, sample_rate, 0.7071)],
        (FilterKind::LowPass, 2) => vec![
            make_lowpass2(frequency, sample_rate, 0.60492333),
            make_lowpass2(frequency, sample_rate, 1.33722126),
        ],
        (FilterKind::HighPass, 2) => vec![
            make_highpass2(frequency, sample_rate, 0.60492333),
            make_highpass2(frequency, sample_rate, 1.33722126),
        ],
        (FilterKind::LowPass, 3) => vec![
            make_lowpass2(frequency, sample_rate, 0.58338080),
            make_lowpass2(frequency, sample_rate, 0.75932572),
            make_lowpass2(frequency, sample_rate, 1.95302407),
        ],
        (FilterKind::HighPass, 3) => vec![
            make_highpass2(frequency, sample_rate, 0.58338080),
            make_highpass2(frequency, sample_rate, 0.75932572),
            make_highpass2(frequency, sample_rate, 1.95302407),
        ],
        (FilterKind::LowPass, 4) => vec![
            make_lowpass2(frequency, sample_rate, 0.57622191),
            make_lowpass2(frequency, sample_rate, 0.66045510),
            make_lowpass2(frequency, sample_rate, 0.94276399),
            make_lowpass2(frequency, sample_rate, 2.57900101),
        ],
        (FilterKind::HighPass, 4) => vec![
            make_highpass2(frequency, sample_rate, 0.57622191),
            make_highpass2(frequency, sample_rate, 0.66045510),
            make_highpass2(frequency, sample_rate, 0.94276399),
            make_highpass2(frequency, sample_rate, 2.57900101),
        ],
        _ => unreachable!(),
    };
    Ok(stages)
}

fn process_with_stages(
    buf: &mut AudioBuffer,
    make: impl Fn(u32) -> Result<Vec<Stage>, EffectError>,
) -> Result<(), EffectError> {
    for channel in &mut buf.channels {
        let mut stages = make(buf.sample_rate)?;
        for sample in channel {
            let mut value = *sample;
            for stage in &mut stages {
                value = stage.process(value);
            }
            *sample = value;
        }
    }
    Ok(())
}

/// Audacity's Nyquist high-pass filter.
pub struct HighPassFilter {
    /// Cutoff frequency in hertz.
    pub frequency: f64,
    /// Roll-off choice index: 0 = 6 dB, 1 = 12 dB, and so on.
    pub rolloff: f64,
    rt_sample_rate: u32,
    rt_states: Vec<Vec<Stage>>,
}

impl Default for HighPassFilter {
    fn default() -> Self {
        Self {
            frequency: 1000.0,
            rolloff: 0.0,
            rt_sample_rate: 0,
            rt_states: Vec::new(),
        }
    }
}

impl BuiltinEffect for HighPassFilter {
    fn id(&self) -> &'static str {
        "High-passFilter"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("FREQUENCY", "Frequency (Hz)", 0.0, f64::MAX, 1000.0),
            ParamDef::choice(
                "ROLLOFF",
                "Roll-off (dB per octave)",
                ROLLOFF_CHOICES,
                0,
            ),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "FREQUENCY" => Some(self.frequency),
            "ROLLOFF" => Some(self.rolloff),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "FREQUENCY" => self.frequency = value,
            "ROLLOFF" => self.rolloff = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        process_with_stages(buf, |sr| {
            make_stages(FilterKind::HighPass, self.frequency, self.rolloff, sr)
        })
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        self.rt_sample_rate = sample_rate;
        self.rt_states = (0..channels)
            .map(|_| {
                make_stages(
                    FilterKind::HighPass,
                    self.frequency,
                    self.rolloff,
                    sample_rate,
                )
                .unwrap_or_default()
            })
            .collect();
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        if self.rt_states.len() != block.len() {
            self.rt_reset(self.rt_sample_rate, block.len());
        }
        for (channel, stages) in block.iter_mut().zip(&mut self.rt_states) {
            for sample in channel {
                let mut value = *sample;
                for stage in stages.iter_mut() {
                    value = stage.process(value);
                }
                *sample = value;
            }
        }
    }
}

/// Audacity's Nyquist low-pass filter.
pub struct LowPassFilter {
    /// Cutoff frequency in hertz.
    pub frequency: f64,
    /// Roll-off choice index: 0 = 6 dB, 1 = 12 dB, and so on.
    pub rolloff: f64,
    rt_sample_rate: u32,
    rt_states: Vec<Vec<Stage>>,
}

impl Default for LowPassFilter {
    fn default() -> Self {
        Self {
            frequency: 1000.0,
            rolloff: 0.0,
            rt_sample_rate: 0,
            rt_states: Vec::new(),
        }
    }
}

impl BuiltinEffect for LowPassFilter {
    fn id(&self) -> &'static str {
        "Low-passFilter"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("FREQUENCY", "Frequency (Hz)", 0.0, f64::MAX, 1000.0),
            ParamDef::choice(
                "ROLLOFF",
                "Roll-off (dB per octave)",
                ROLLOFF_CHOICES,
                0,
            ),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "FREQUENCY" => Some(self.frequency),
            "ROLLOFF" => Some(self.rolloff),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "FREQUENCY" => self.frequency = value,
            "ROLLOFF" => self.rolloff = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        process_with_stages(buf, |sr| {
            make_stages(FilterKind::LowPass, self.frequency, self.rolloff, sr)
        })
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        self.rt_sample_rate = sample_rate;
        self.rt_states = (0..channels)
            .map(|_| {
                make_stages(
                    FilterKind::LowPass,
                    self.frequency,
                    self.rolloff,
                    sample_rate,
                )
                .unwrap_or_default()
            })
            .collect();
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        if self.rt_states.len() != block.len() {
            self.rt_reset(self.rt_sample_rate, block.len());
        }
        for (channel, stages) in block.iter_mut().zip(&mut self.rt_states) {
            for sample in channel {
                let mut value = *sample;
                for stage in stages.iter_mut() {
                    value = stage.process(value);
                }
                *sample = value;
            }
        }
    }
}

/// Audacity's Nyquist second-order notch filter.
pub struct NotchFilter {
    /// Notch center frequency in hertz.
    pub frequency: f64,
    /// Filter Q; larger values produce a narrower notch.
    pub q: f64,
    rt_sample_rate: u32,
    rt_states: Vec<Vec<Stage>>,
}

impl Default for NotchFilter {
    fn default() -> Self {
        Self {
            frequency: 60.0,
            q: 1.0,
            rt_sample_rate: 0,
            rt_states: Vec::new(),
        }
    }
}

impl BuiltinEffect for NotchFilter {
    fn id(&self) -> &'static str {
        "NotchFilter"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("FREQUENCY", "Frequency (Hz)", 0.0, f64::MAX, 60.0),
            ParamDef::float("Q", "Q (higher value reduces width)", 0.1, 1000.0, 1.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "FREQUENCY" => Some(self.frequency),
            "Q" => Some(self.q),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "FREQUENCY" => self.frequency = value,
            "Q" => self.q = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if !(self.q > 0.0) {
            return Err(EffectError::Invalid("Q must be positive.".into()));
        }
        process_with_stages(buf, |sr| {
            validate_frequency(self.frequency, sr)?;
            Ok(vec![make_notch2(self.frequency, sr, self.q)])
        })
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        self.rt_sample_rate = sample_rate;
        self.rt_states = (0..channels)
            .map(|_| {
                if self.q > 0.0 && validate_frequency(self.frequency, sample_rate).is_ok() {
                    vec![make_notch2(self.frequency, sample_rate, self.q)]
                } else {
                    Vec::new()
                }
            })
            .collect();
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        if self.rt_states.len() != block.len() {
            self.rt_reset(self.rt_sample_rate, block.len());
        }
        for (channel, stages) in block.iter_mut().zip(&mut self.rt_states) {
            for sample in channel {
                let mut value = *sample;
                for stage in stages.iter_mut() {
                    value = stage.process(value);
                }
                *sample = value;
            }
        }
    }
}

/// Returns the Nyquist filter effects in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![
        Box::new(HighPassFilter::default()),
        Box::new(LowPassFilter::default()),
        Box::new(NotchFilter::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::set_param;

    fn sine_buffer() -> AudioBuffer {
        let sample_rate = 48_000;
        let channels = (0..2)
            .map(|channel| {
                (0..256)
                    .map(|n| {
                        let phase = 2.0 * PI * 440.0 * n as f64 / sample_rate as f64;
                        (phase.sin() * if channel == 0 { 0.5 } else { 0.25 }) as f32
                    })
                    .collect()
            })
            .collect();
        AudioBuffer::from_channels(sample_rate, channels)
    }

    #[test]
    fn defaults_process_short_stereo_audio() {
        let mut ctx = EffectContext::default();
        for mut effect in all() {
            let mut buffer = sine_buffer();
            effect.process(&mut buffer, &mut ctx).unwrap();
            assert_eq!(buffer.channels.len(), 2);
            assert_eq!(buffer.len(), 256);
            assert!(buffer.channels.iter().flatten().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn empty_buffers_do_not_panic() {
        let mut ctx = EffectContext::default();
        for mut effect in all() {
            let mut buffer = AudioBuffer::from_channels(48_000, vec![vec![], vec![]]);
            effect.process(&mut buffer, &mut ctx).unwrap();
            assert_eq!(buffer.len(), 0);
        }
    }

    #[test]
    fn processing_is_repeatable() {
        for mut first in all() {
            let mut second: Box<dyn BuiltinEffect> = match first.id() {
                "High-passFilter" => Box::new(HighPassFilter::default()),
                "Low-passFilter" => Box::new(LowPassFilter::default()),
                _ => Box::new(NotchFilter::default()),
            };
            let mut a = sine_buffer();
            let mut b = sine_buffer();
            first
                .process(&mut a, &mut EffectContext::default())
                .unwrap();
            second
                .process(&mut b, &mut EffectContext::default())
                .unwrap();
            assert_eq!(a.channels, b.channels);
        }
    }

    #[test]
    fn parameters_clamp() {
        let mut highpass = HighPassFilter::default();
        assert!(set_param(&mut highpass, "ROLLOFF", 99.0));
        assert_eq!(highpass.rolloff, 4.0);
        assert!(set_param(&mut highpass, "ROLLOFF", -3.0));
        assert_eq!(highpass.rolloff, 0.0);

        let mut notch = NotchFilter::default();
        assert!(set_param(&mut notch, "Q", 5000.0));
        assert_eq!(notch.q, 1000.0);
        assert!(set_param(&mut notch, "Q", 0.0));
        assert_eq!(notch.q, 0.1);
    }

    #[test]
    fn notch_uses_nyquist_sinh_alpha_and_biquad_m_signs() {
        let frequency = 220.0;
        let sample_rate = 48_000;
        let q = 1.0;
        let w = 2.0 * PI * (frequency / sample_rate as f64);
        let alpha = w.sin() * (0.5_f64 / q).sinh();
        let a0r = 1.0 / (1.0 + alpha);
        let a1 = -2.0 * w.cos();

        match make_notch2(frequency, sample_rate, q) {
            Stage::Biquad {
                b0,
                b1,
                b2,
                a1: feedback1,
                a2: feedback2,
                z1,
                z2,
            } => {
                assert_eq!(b0, a0r * 1.0);
                assert_eq!(b1, a0r * a1);
                assert_eq!(b2, a0r * 1.0);
                assert_eq!(feedback1, a0r * -a1);
                assert_eq!(feedback2, a0r * -(1.0 - alpha));
                assert_eq!(z1, 0.0);
                assert_eq!(z2, 0.0);
            }
            _ => panic!("notch2 must use snd-biquad"),
        }
    }
}
