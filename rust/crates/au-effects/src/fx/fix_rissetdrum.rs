// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

use crate::core::*;
use au_core::AudioBuffer;

/// Nyquist's Risset Drum generator.
#[derive(Clone, Debug)]
pub struct RissetDrum {
    /// Fundamental frequency in hertz.
    pub freq: f64,
    /// Decay duration in seconds.
    pub decay: f64,
    /// Center frequency of the noise band in hertz.
    pub cf: f64,
    /// Width of the noise band in hertz.
    pub bw: f64,
    /// Noise amount, as supplied by the Nyquist plug-in control.
    pub noise: f64,
    /// Output amplitude.
    pub gain: f64,
}

impl Default for RissetDrum {
    fn default() -> Self {
        Self {
            freq: 100.0,
            decay: 2.0,
            cf: 500.0,
            bw: 400.0,
            noise: 25.0,
            gain: 0.8,
        }
    }
}

impl BuiltinEffect for RissetDrum {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "RissetDrum"
    }) }

    fn name(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Risset Drum"
    }) }

    fn kind(&self) -> EffectKind {
        EffectKind::Generate
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("FREQ", "Frequency (Hz)", 50.0, 2000.0, 100.0),
            ParamDef::float("DECAY", "Decay (seconds)", 0.1, 60.0, 2.0),
            ParamDef::float(
                "CF",
                "Center frequency of noise (Hz)",
                100.0,
                5000.0,
                500.0,
            ),
            ParamDef::float("BW", "Width of noise band (Hz)", 10.0, 1000.0, 400.0),
            ParamDef::float(
                "NOISE",
                "Amount of noise in mix (percent)",
                0.0,
                100.0,
                25.0,
            ),
            ParamDef::float("GAIN", "Amplitude (0 - 1)", 0.0, 1.0, 0.8),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "FREQ" => Some(self.freq),
            "DECAY" => Some(self.decay),
            "CF" => Some(self.cf),
            "BW" => Some(self.bw),
            "NOISE" => Some(self.noise),
            "GAIN" => Some(self.gain),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "FREQ" => self.freq = value,
            "DECAY" => self.decay = value,
            "CF" => self.cf = value,
            "BW" => self.bw = value,
            "NOISE" => self.noise = value,
            "GAIN" => self.gain = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let sr = buf.sample_rate.max(1);
        let channel_count = buf.channels.len().max(1);
        let decay = self.decay.max(0.0);
        let frame_count = rounded_frames(decay, sr);

        // Nyquist evaluates the plug-in independently for each selected channel.
        // Each channel therefore starts from the C library's initial rand state.
        let channels = (0..channel_count)
            .map(|_| {
                let mut random = GlibcRand::new();
                generate_channel(
                    sr,
                    frame_count,
                    self.freq,
                    decay,
                    self.cf,
                    self.bw,
                    self.noise,
                    self.gain,
                    &mut random,
                )
            })
            .collect();

        *buf = AudioBuffer::from_channels(sr, channels);
        Ok(())
    }
}

const TABLE_LEN: usize = 2048;
const SINE_TABLE_SHIFT: u32 = 20; // sine.h: SINE_TABLE_SHIFT 20, MASK 0x7FFFFFFF
const SINE_PHASE_MASK: u64 = ((TABLE_LEN as u64) << SINE_TABLE_SHIFT) - 1;

/// Builds Nyquist's fixed sine lookup table, including its guard point.
fn sine_table() -> Vec<f32> {
    let mut table = Vec::with_capacity(TABLE_LEN + 1);
    for i in 0..=TABLE_LEN {
        table.push(((std::f64::consts::TAU * i as f64) / TABLE_LEN as f64).sin() as f32);
    }
    table
}

/// Rounds a positive duration to Nyquist's sample count.
fn rounded_frames(seconds: f64, sample_rate: u32) -> usize {
    let frames = (seconds * sample_rate as f64 + 0.5).floor();
    if !frames.is_finite() || frames <= 0.0 {
        0
    } else {
        frames.min(usize::MAX as f64) as usize
    }
}

/// Converts hertz to Nyquist pitch steps and back, as done by `hz-to-step` and
/// `calculate-hz`.
fn nyquist_hz(hz: f64) -> f64 {
    const LOG_SCALE: f64 = 17.312340490667565;
    const LOG_OFFSET: f64 = 2.1011784386926213;
    const EXP_SCALE: f64 = 0.0577622650466621;

    let step = LOG_SCALE * (hz.ln() - LOG_OFFSET);
    (step * EXP_SCALE + LOG_OFFSET).exp()
}

/// Generates the fixed-table sine used by Nyquist's `sine` primitive.
fn generate_sine(sample_rate: u32, frames: usize, hz: f64, table: &[f32]) -> Vec<f32> {
    let increment = ((nyquist_hz(hz) * TABLE_LEN as f64)
        * (1u64 << SINE_TABLE_SHIFT) as f64
        / sample_rate as f64
        + 0.5)
        .floor() as u64;

    let mut phase = 0u64;
    let mut output = Vec::with_capacity(frames);
    for _ in 0..frames {
        output.push(table[(phase >> SINE_TABLE_SHIFT) as usize]);
        phase = phase.wrapping_add(increment) & SINE_PHASE_MASK;
    }
    output
}

/// Builds the harmonic table used by the Risset Drum oscillator.
fn build_risset_table(sine: &[f32]) -> Vec<f32> {
    let harmonics = [(10usize, 1.0f32), (16, 1.5), (22, 2.0), (23, 1.5)];
    let mut table = Vec::with_capacity(TABLE_LEN + 1);

    for i in 0..TABLE_LEN {
        let mut sum = 0.0f32;
        for (harmonic, scale) in harmonics {
            sum += sine[(harmonic * i) % TABLE_LEN] * scale;
        }
        table.push(sum * 0.17f32);
    }
    table.push(table[0]);
    table
}

/// Generates Nyquist's interpolating table oscillator.
fn generate_table_osc(sample_rate: u32, frames: usize, hz: f64, table: &[f32]) -> Vec<f32> {
    let increment = nyquist_hz(hz) * TABLE_LEN as f64 / sample_rate as f64;
    let mut phase = 0.0f64;
    let mut output = Vec::with_capacity(frames);

    for _ in 0..frames {
        let index = phase as usize;
        let x1 = table[index] as f64;
        let fraction = phase - index as f64;
        output.push((x1 + fraction * (table[index + 1] as f64 - x1)) as f32);

        phase += increment;
        while phase >= TABLE_LEN as f64 {
            phase -= TABLE_LEN as f64;
        }
    }
    output
}

/// Reproduces `exp-dec`'s control-rate logarithmic ramp and its Nyquist
/// control-to-audio-rate linear interpolation.
fn percussion_envelope(sample_rate: u32, frames: usize, duration: f64) -> Vec<f32> {
    if frames == 0 {
        return Vec::new();
    }

    let half_life = 2.0f64.powf(duration.ln() / 2.0f64.ln() - 3.0);
    let target = 0.5f64.powf(duration / half_life);
    let target_log = target.ln();

    let control_rate = sample_rate as f64 / 20.0;
    let endpoint = (duration * control_rate + 0.5).floor();
    let endpoint = if endpoint.is_finite() && endpoint >= 0.0 {
        endpoint.min(usize::MAX as f64) as usize
    } else {
        0
    };

    // PWL's endpoint is at `endpoint`; its samples run from zero up to it.
    let control_count = endpoint.saturating_add(1).max(2);
    let segments = control_count - 1;
    let increment = target_log / segments as f64;

    let mut control = Vec::with_capacity(control_count);
    let mut level = 0.0f64;
    for index in 0..control_count {
        let log_sample = if index == endpoint {
            target_log as f32
        } else {
            let sample = level as f32;
            level += increment;
            sample
        };
        control.push((log_sample as f64).exp() as f32);
    }

    let mut output = Vec::with_capacity(frames);
    let mut control_index = 0usize;
    let mut phase = 0.0f64;
    let phase_increment = 1.0 / 20.0;

    while output.len() < frames {
        let x1 = control[control_index];
        let x2 = control.get(control_index + 1).copied().unwrap_or(x1);
        let mut value = (x1 as f64 * (1.0 - phase) + x2 as f64 * phase) as f32;
        let delta = ((x2 - x1) as f64 * phase_increment) as f32;

        let remaining = ((1.0 - phase) * 20.0) as usize;
        let count = remaining.max(1).min(frames - output.len());
        for _ in 0..count {
            output.push(value);
            value += delta;
        }

        phase += count as f64 * phase_increment;
        if phase >= 1.0 {
            let steps = phase.floor() as usize;
            control_index = control_index.saturating_add(steps);
            phase -= steps as f64;
        }
    }

    output
}

/// Applies one Nyquist low-pass biquad section with zero initial state.
fn lowpass2(input: &[f32], sample_rate: u32, cutoff: f64, q: f64) -> Vec<f32> {
    let cutoff = cutoff.min(sample_rate as f64 * 0.5);
    let w = 2.0 * std::f64::consts::PI * (cutoff / sample_rate as f64);
    let cw = w.cos();
    let sw = w.sin();
    let alpha = sw * (0.5 / q).sinh();
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cw;
    let a2 = 1.0 - alpha;
    let b1 = 1.0 - cw;
    let b0 = 0.5 * b1;
    let b2 = b0;

    let a0r = 1.0 / a0;
    let b0 = b0 * a0r;
    let b1 = b1 * a0r;
    let b2 = b2 * a0r;
    let a1 = a1 * a0r;
    let a2 = a2 * a0r;

    let mut z1 = 0.0f64;
    let mut z2 = 0.0f64;
    let mut output = Vec::with_capacity(input.len());

    for &sample in input {
        // The denominator coefficients returned by lowpass2 are the
        // conventional denominator terms; biquad feedback subtracts them.
        let z0 = sample as f64 - a1 * z1 - a2 * z2;
        output.push((z0 * b0 + z1 * b1 + z2 * b2) as f32);
        z2 = z1;
        z1 = z0;
    }
    output
}

/// Generates the sound for one output channel.
fn generate_channel(
    sample_rate: u32,
    frames: usize,
    frequency: f64,
    decay: f64,
    center_frequency: f64,
    bandwidth: f64,
    noise_amount: f64,
    gain: f64,
    random: &mut GlibcRand,
) -> Vec<f32> {
    if frames == 0 {
        return Vec::new();
    }

    let sine = sine_table();
    let drum_table = build_risset_table(&sine);
    let half_decay = decay * 0.50;
    let half_frames = rounded_frames(half_decay, sample_rate).min(frames);
    let low_note = frequency * 0.10;

    let mut drum = generate_table_osc(sample_rate, half_frames, low_note, &drum_table);
    let mut noise: Vec<f32> = (0..half_frames).map(|_| random.next_sample()).collect();

    noise = lowpass2(&noise, sample_rate, bandwidth, 0.58338080);
    noise = lowpass2(&noise, sample_rate, bandwidth, 0.75932572);
    noise = lowpass2(&noise, sample_rate, bandwidth, 1.95302407);

    let noise_sine = generate_sine(sample_rate, half_frames, center_frequency, &sine);
    let tone_gain = (1.0 - noise_amount) as f32;
    let noise_scale = noise_amount as f32;

    let mut first_mix = Vec::with_capacity(half_frames);
    for i in 0..half_frames {
        drum[i] *= tone_gain;
        let noise_band = (noise_sine[i] * noise[i]) * noise_scale;
        first_mix.push(drum[i] + noise_band);
    }

    let half_envelope = percussion_envelope(sample_rate, half_frames, half_decay);
    let mut output = vec![0.0f32; frames];
    for i in 0..half_frames {
        output[i] = first_mix[i] * half_envelope[i];
    }

    let full_envelope = percussion_envelope(sample_rate, frames, decay);
    let full_sine = generate_sine(sample_rate, frames, frequency, &sine);
    for i in 0..frames {
        let second = full_sine[i] * full_envelope[i];
        output[i] += second * tone_gain;
    }

    let peak = output
        .iter()
        .fold(0.0f64, |maximum, &sample| maximum.max((sample as f64).abs()));
    let scale = if peak == 0.0 {
        0.0f32
    } else {
        (gain / peak) as f32
    };
    for sample in &mut output {
        *sample *= scale;
    }

    output
}

/// A private implementation of glibc's TYPE_3 `rand()` sequence, seeded with 1.
struct GlibcRand {
    state: [u32; 344],
    index: usize,
}

impl GlibcRand {
    fn new() -> Self {
        let mut state = [0u32; 344];
        state[0] = 1;
        for i in 1..31 {
            let previous = state[i - 1] as i64;
            let hi = previous / 127_773;
            let lo = previous % 127_773;
            let mut value = 16_807 * lo - 2_836 * hi;
            if value < 0 {
                value += 2_147_483_647;
            }
            state[i] = value as u32;
        }
        state[31] = state[0];
        state[32] = state[1];
        state[33] = state[2];

        for i in 34..344 {
            state[i] = state[i - 31].wrapping_add(state[i - 3]);
        }
        Self { state, index: 344 }
    }

    fn next_u31(&mut self) -> u32 {
        let index = self.index;
        let value =
            self.state[(index - 31) % 344].wrapping_add(self.state[(index - 3) % 344]);
        self.state[index % 344] = value;
        self.index += 1;
        value >> 1
    }

    fn next_sample(&mut self) -> f32 {
        (self.next_u31() as f64 * (2.0 / 2_147_483_647.0) - 1.0) as f32
    }
}

/// Returns the available built-in effect instances in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(RissetDrum::default())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::set_param;

    #[test]
    fn default_runs_on_short_stereo_audio() {
        let sample_rate = 44_100;
        let mut buffer = AudioBuffer::from_channels(
            sample_rate,
            (0..2)
                .map(|channel| {
                    (0..128)
                        .map(|i| {
                            (i as f32 * 0.1).sin()
                                * if channel == 0 { 0.5 } else { 0.25 }
                        })
                        .collect()
                })
                .collect(),
        );

        let mut effect = RissetDrum::default();
        effect.decay = 0.1;
        effect
            .process(&mut buffer, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buffer.channels.len(), 2);
        assert_eq!(buffer.len(), rounded_frames(0.1, sample_rate));
    }

    #[test]
    fn empty_buffer_does_not_panic() {
        let mut buffer = AudioBuffer::from_channels(44_100, Vec::new());
        let mut effect = RissetDrum::default();
        effect.decay = 0.1;
        effect
            .process(&mut buffer, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buffer.channels.len(), 1);
        assert_eq!(buffer.len(), rounded_frames(0.1, 44_100));
    }

    #[test]
    fn generation_is_repeatable() {
        let mut first = AudioBuffer::from_channels(8_000, vec![vec![0.0; 1]]);
        let mut second = AudioBuffer::from_channels(8_000, vec![vec![0.0; 1]]);
        let mut a = RissetDrum::default();
        let mut b = RissetDrum::default();
        a.decay = 0.1;
        b.decay = 0.1;

        a.process(&mut first, &mut EffectContext::default()).unwrap();
        b.process(&mut second, &mut EffectContext::default()).unwrap();
        assert_eq!(first.channels, second.channels);
    }

    #[test]
    fn set_param_clamps_values() {
        let mut effect = RissetDrum::default();
        assert!(set_param(&mut effect, "FREQ", 10_000.0));
        assert_eq!(effect.freq, 2_000.0);
        assert!(set_param(&mut effect, "NOISE", -1.0));
        assert_eq!(effect.noise, 0.0);
        assert!(!set_param(&mut effect, "UNKNOWN", 1.0));
    }
}
