// SPDX-License-Identifier: GPL-3.0-or-later
//! Nyquist Rhythm Track generator (Audacity 3.7.3).

use crate::core::*;
use au_core::AudioBuffer;

use crate::fx::nyq_tables::{
    SMALL_FILTER_IMP, SMALL_FILTER_IMPD, SMALL_FILTER_NMULT, SMALL_FILTER_NWING,
    SMALL_FILTER_SCALE,
};

const CLICK_TYPES: &[&str] = &[
    "Metronome",
    "Ping (short)",
    "Ping (long)",
    "Cowbell",
    "ResonantNoise",
    "NoiseClick",
    "Drip (short)",
    "Drip (long)",
];

#[allow(dead_code)]
const NYQUIST_BLOCK: usize = 1024;

/// Audacity's Nyquist Rhythm Track generator.
#[derive(Clone, Debug)]
pub struct RhythmTrack {
    /// Tempo in beats per minute.
    pub tempo: f64,
    /// Beats per bar.
    pub timesig: i32,
    /// Swing amount, from -1 to 1.
    pub swing: f64,
    /// Number of bars; zero selects duration-based generation.
    pub bars: i32,
    /// Requested duration when `bars` is zero, in seconds.
    pub click_track_dur: f64,
    /// Silence before the first beat, in seconds.
    pub offset: f64,
    /// Index into the beat-sound choices.
    pub click_type: usize,
    /// MIDI pitch of the strong beat.
    pub high: i32,
    /// MIDI pitch of weak beats.
    pub low: i32,
}

impl Default for RhythmTrack {
    fn default() -> Self {
        Self {
            tempo: 120.0,
            timesig: 4,
            swing: 0.0,
            bars: 16,
            click_track_dur: 0.0,
            offset: 0.0,
            click_type: 0,
            high: 84,
            low: 80,
        }
    }
}

fn samples(seconds: f64, rate: f64) -> usize {
    if !seconds.is_finite() || seconds <= 0.0 {
        0
    } else {
        (seconds * rate + 0.5).floor().min(usize::MAX as f64) as usize
    }
}

fn hz(pitch: i32) -> f64 {
    440.0 * 2.0_f64.powf((pitch as f64 - 69.0) / 12.0)
}

// Nyquist's snd-pwl computes each segment in double precision, but stores
// each individual control sample as sample_type (float).
fn pwl(rate: f64, points: &[(f64, f64)], end: f64) -> Vec<f32> {
    let end_n = samples(end, rate);
    let mut output = vec![0.0; end_n];
    let mut at = 0usize;
    let mut level = 0.0_f64;
    for &(time, target) in points {
        let next = samples(time, rate).min(end_n);
        if next > at {
            let increment = (target - level) / (next - at) as f64;
            let mut value = level;
            for x in &mut output[at..next] {
                *x = value as f32;
                value += increment;
            }
            level += increment * (next - at) as f64;
            at = next;
        } else {
            level = target;
        }
    }
    if end_n > at {
        let increment = -level / (end_n - at) as f64;
        let mut value = level;
        for x in &mut output[at..] {
            *x = value as f32;
            value += increment;
        }
    }
    output
}

// snd-mult's interpolating variant for a control-rate operand.
fn control_at(control: &[f32], frame: usize, sound_rate: f64, control_rate: f64) -> f32 {
    if control.is_empty() {
        return 0.0;
    }
    if sound_rate == control_rate {
        return control.get(frame).copied().unwrap_or(0.0);
    }
    let position = frame as f64 * control_rate / sound_rate;
    let index = position.floor() as usize;
    let a = control.get(index).copied().unwrap_or(0.0);
    let b = control.get(index + 1).copied().unwrap_or(0.0);
    (a as f64 + (position - index as f64) * (b as f64 - a as f64)) as f32
}

fn apply_control(sound: &mut [f32], control: &[f32], sr: f64, cr: f64) {
    for (i, sample) in sound.iter_mut().enumerate() {
        *sample *= control_at(control, i, sr, cr);
    }
}

#[derive(Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Biquad {
    fn lowpass(hz: f64, q: f64, rate: f64) -> Self {
        let w = 2.0 * std::f64::consts::PI * (hz / rate);
        let cw = w.cos();
        let alpha = w.sin() * (0.5 / q).sinh();
        let a0r = 1.0 / (1.0 + alpha);
        let b1 = 1.0 - cw;
        Self {
            b0: a0r * (0.5 * b1),
            b1: a0r * b1,
            b2: a0r * (0.5 * b1),
            a1: a0r * (2.0 * cw),
            a2: a0r * (alpha - 1.0),
        }
    }

    fn highpass(hz: f64, q: f64, rate: f64) -> Self {
        let w = 2.0 * std::f64::consts::PI * (hz / rate);
        let cw = w.cos();
        let alpha = w.sin() * (0.5 / q).sinh();
        let a0r = 1.0 / (1.0 + alpha);
        let b1 = 1.0 + cw;
        Self {
            b0: a0r * (-0.5 * b1),
            b1: a0r * b1,
            b2: a0r * (-0.5 * b1),
            a1: a0r * (2.0 * cw),
            a2: a0r * (alpha - 1.0),
        }
    }

    fn process(self, input: &mut [f32]) {
        // biquadfilt.c uses a double-precision direct-form II state and
        // rounds the result to sample_type after every sample.
        let mut z1 = 0.0_f64;
        let mut z2 = 0.0_f64;
        for x in input {
            let z0 = *x as f64 + self.a1 * z1 + self.a2 * z2;
            *x = (z0 * self.b0 + z1 * self.b1 + z2 * self.b2) as f32;
            z2 = z1;
            z1 = z0;
        }
    }
}

fn peak(input: &[f32], count: usize) -> f64 {
    input
        .iter()
        .take(count)
        .fold(0.0_f64, |m, &v| m.max((v as f64).abs()))
}

// The default, seed-1 glibc TYPE_3 additive-feedback generator. Nyquist's
// white.c calls C rand(), not a Nyquist-specific random-number generator.
struct GlibcRand {
    state: [u32; 344],
    index: usize,
}

impl GlibcRand {
    fn new() -> Self {
        let mut state = [0u32; 344];
        state[0] = 1;
        for i in 1..31 {
            state[i] = ((16807_i64 * state[i - 1] as i64) % 2147483647) as u32;
        }
        for i in 31..34 {
            state[i] = state[i - 31];
        }
        for i in 34..344 {
            state[i] = state[i - 31].wrapping_add(state[i - 3]);
        }
        Self { state, index: 344 }
    }

    fn next(&mut self) -> u32 {
        let i = self.index % 344;
        let value = self.state[(self.index - 31) % 344]
            .wrapping_add(self.state[(self.index - 3) % 344]);
        self.state[i] = value;
        self.index += 1;
        (value >> 1) & 0x7fff_ffff
    }

    fn white(&mut self) -> f32 {
        (self.next() as f64 * (2.0 / 2147483647.0) - 1.0) as f32
    }
}

// STK's Delay starts with a 4096-element buffer even when JCRev requests a
// smaller maximum delay. setMaximumDelay therefore does not shrink it.
struct Delay {
    input: Vec<f64>,
    write: usize,
    read: usize,
    last: f64,
}

impl Delay {
    fn new(length: usize) -> Self {
        let capacity = 4096usize.max(length + 1);
        Self {
            input: vec![0.0; capacity],
            write: 0,
            read: (capacity - length) % capacity,
            last: 0.0,
        }
    }

    fn tick(&mut self, sample: f64) -> f64 {
        self.input[self.write] = sample;
        self.write += 1;
        if self.write == self.input.len() {
            self.write = 0;
        }
        self.last = self.input[self.read];
        self.read += 1;
        if self.read == self.input.len() {
            self.read = 0;
        }
        self.last
    }
}

// JCRev.cpp and Effect.cpp use StkFloat = double. The return value of
// Effect::lastOut is the average of the two decorrelation-delay outputs.
struct JcRev {
    allpass: [Delay; 3],
    comb: [Delay; 4],
    left: Delay,
    right: Delay,
    coefficient: [f64; 4],
}

impl JcRev {
    fn new(rate: u32, t60: f64) -> Self {
        let mut lengths = [1777usize, 1847, 1993, 2137, 389, 127, 43, 211, 179];
        let scaler = rate as f64 / 44100.0;
        if scaler != 1.0 {
            for length in &mut lengths {
                let mut delay = (scaler * *length as f64).floor() as usize;
                if delay % 2 == 0 {
                    delay += 1;
                }
                while !prime(delay) {
                    delay += 2;
                }
                *length = delay;
            }
        }
        let coefficient =
            std::array::from_fn(|i| 10.0_f64.powf(-3.0 * lengths[i] as f64 / (t60 * rate as f64)));
        Self {
            allpass: std::array::from_fn(|i| Delay::new(lengths[i + 4])),
            comb: std::array::from_fn(|i| Delay::new(lengths[i])),
            left: Delay::new(lengths[7]),
            right: Delay::new(lengths[8]),
            coefficient,
        }
    }

    fn tick(&mut self, input: f64) -> f64 {
        let mut value = input;
        for delay in &mut self.allpass {
            let previous = delay.last;
            let mut fed = 0.7 * previous;
            fed += value;
            delay.tick(fed);
            value = -(0.7 * fed) + previous;
        }
        let mut temp = [0.0; 4];
        for i in 0..4 {
            temp[i] = value + self.coefficient[i] * self.comb[i].last;
        }
        for (delay, &sample) in self.comb.iter_mut().zip(&temp) {
            delay.tick(sample);
        }
        let filtout = temp[0] + temp[1] + temp[2] + temp[3];
        let mut left = 0.1 * self.left.tick(filtout);
        let mut right = 0.1 * self.right.tick(filtout);
        let dry = (1.0 - 0.1) * input;
        left += dry;
        right += dry;
        (left + right) * 0.5
    }
}

fn prime(number: usize) -> bool {
    if number == 2 {
        return true;
    }
    if number < 2 || number % 2 == 0 {
        return false;
    }
    let mut i = 3;
    while i <= number / i {
        if number % i == 0 {
            return false;
        }
        i += 2;
    }
    true
}

// ffilterkit.c: at unity rate resamp.c selects SrcUp, including the
// right-wing omitted coefficient and its special case at phase zero.
fn filter_up(input: &[f32], center: isize, phase: f64, direction: isize) -> f64 {
    let exact = phase * 256.0;
    let index = exact as usize;
    let fraction = exact - index as f64;
    let mut coefficient = index;
    let mut sample_index = center;
    let end = SMALL_FILTER_NWING - usize::from(direction == 1);
    if direction == 1 && phase == 0.0 {
        coefficient += 256;
    }
    let mut result = 0.0_f64;
    while coefficient < end {
        let mut t = SMALL_FILTER_IMP[coefficient] as f64;
        t += SMALL_FILTER_IMPD[coefficient] as f64 * fraction;
        let x = if sample_index >= 0 {
            input.get(sample_index as usize).copied().unwrap_or(0.0)
        } else {
            0.0
        };
        result += t * x as f64;
        coefficient += 256;
        sample_index += direction;
    }
    result
}

fn filter_ud(input: &[f32], center: isize, phase: f64, direction: isize, dh: f64) -> f64 {
    let mut position = phase * dh;
    let end = SMALL_FILTER_NWING - usize::from(direction == 1);
    if direction == 1 && phase == 0.0 {
        position += dh;
    }
    let mut sample_index = center;
    let mut result = 0.0;
    while (position as usize) < end {
        let index = position as usize;
        let mut t = SMALL_FILTER_IMP[index] as f64;
        t += SMALL_FILTER_IMPD[index] as f64 * (position - index as f64);
        let x = if sample_index >= 0 {
            input.get(sample_index as usize).copied().unwrap_or(0.0)
        } else {
            0.0
        };
        result += t * x as f64;
        position += dh;
        sample_index += direction;
    }
    result
}

fn resample(input: &[f32], output_rate: u32) -> Vec<f32> {
    let factor = output_rate as f64 / 44100.0;
    let length = samples(input.len() as f64 / 44100.0, output_rate as f64);
    let xoff = (((SMALL_FILTER_NMULT as f64 + 1.0) / 2.0)
        * (1.0_f64).max(1.0 / factor)
        + 10.0) as usize;
    let mut padded = vec![0.0; xoff];
    padded.extend_from_slice(input);
    padded.resize(padded.len() + xoff + 4, 0.0);

    let mut scale = SMALL_FILTER_SCALE as f64 / 32768.0;
    scale /= 16384.0;
    scale /= 1.0011;
    if factor < 1.0 {
        scale *= factor;
    }

    let mut out = Vec::with_capacity(length);
    let mut time = xoff as f64;
    let dt = 1.0 / factor;
    for _ in 0..length {
        let integer = time as usize;
        let fraction = time - integer as f64;
        let value = if factor >= 1.0 {
            filter_up(&padded, integer as isize, fraction, -1)
                + filter_up(&padded, integer as isize + 1, 1.0 - fraction, 1)
        } else {
            let dh = (factor * 256.0).min(256.0);
            filter_ud(&padded, integer as isize, fraction, -1, dh)
                + filter_ud(
                    &padded,
                    integer as isize + 1,
                    1.0 - fraction,
                    1,
                    dh,
                )
        };
        out.push((value * scale) as f32);
        time += dt;
    }
    out
}

fn metronome_tick(pitch: i32, amplitude: f64, output_rate: u32) -> Vec<f32> {
    let rate = 44100.0;
    let mut sound = vec![0.0_f32; samples(0.2, rate)];
    let mut x = 1_i32;
    for sample in sound.iter_mut().take(300) {
        x = (479 * x) % 997;
        *sample = (x as f64 / 500.0 - 1.0) as f32;
    }

    let frequency = hz(pitch);
    Biquad::lowpass(2.0 * frequency, 6.0, rate).process(&mut sound);
    for q in [0.57622191, 0.66045510, 0.94276399, 2.57900101] {
        Biquad::highpass(frequency, q, rate).process(&mut sound);
    }

    // pwev applies exp to a piecewise-linear logarithmic envelope. The
    // control rate in sound-srate-abs 44100 remains the track rate / 20.
    let control_rate = output_rate as f64 / 20.0;
    let first_time = 300.0 / 44100.0;
    let end = samples(1.0, control_rate);
    let breakpoint = samples(first_time, control_rate);
    let mut logarithm = vec![0.0_f32; end];
    for (i, sample) in logarithm.iter_mut().enumerate() {
        let log = if i < breakpoint && breakpoint != 0 {
            10.0_f64.ln()
                + (2.0_f64.ln() - 10.0_f64.ln()) * i as f64 / breakpoint as f64
        } else if end > breakpoint {
            2.0_f64.ln()
                + (-40.0 - 2.0_f64.ln()) * (i - breakpoint) as f64
                    / (end - breakpoint) as f64
        } else {
            0.0
        };
        *sample = (log as f32).exp();
    }
    apply_control(&mut sound, &logarithm, rate, control_rate);

    let maximum = peak(&sound, 300);
    let gain = if maximum > 0.0 {
        amplitude / maximum
    } else {
        0.0
    };
    let mut rev = JcRev::new(44100, 0.01);
    let mut result = Vec::with_capacity(sound.len());
    // stkrev.c moves an input sound's scale to its output: STK sees the
    // unscaled signal, and the gain is applied after its float output.
    for sample in sound {
        result.push((rev.tick(sample as f64) as f32 as f64 * gain) as f32);
    }

    // pwlv starts at 1.11 and retains it through 20 ms, then falls
    // linearly to zero at 50 ms.
    let gate = pwl(control_rate, &[(0.0, 1.11), (0.02, 1.11)], 0.05);
    apply_control(&mut result, &gate, rate, control_rate);
    resample(&result, output_rate)
}

fn sine_table() -> Vec<f32> {
    (0..=2048)
        .map(|i| (2.0 * std::f64::consts::PI * i as f64 / 2048.0).sin() as f32)
        .collect()
}

fn triangle_table() -> Vec<f32> {
    (0..=2048)
        .map(|i| {
            let phase = i as f64 / 2048.0;
            (1.0 - 4.0 * (phase - 0.5).abs()) as f32
        })
        .collect()
}

fn oscillator(table: &[f32], frequency: f64, rate: f64, length: usize) -> Vec<f32> {
    let increment = 2048.0 * frequency / rate;
    let mut phase = 0.0_f64;
    let mut output = Vec::with_capacity(length);
    for _ in 0..length {
        let index = phase as usize;
        let x1 = table[index] as f64;
        output.push(
            (x1 + (phase - index as f64) * (table[index + 1] as f64 - x1)) as f32,
        );
        phase += increment;
        while phase >= 2048.0 {
            phase -= 2048.0;
        }
    }
    output
}

fn ping(pitch: i32, amplitude: f64, duration: f64, rate: u32) -> Vec<f32> {
    let sr = rate as f64;
    let length = samples(duration, sr);
    let table = sine_table();
    let mut output = oscillator(&table, hz(pitch), sr, length);
    // control-srate-abs *sound-srate*: the PWL itself is at audio rate.
    let envelope = pwl(
        sr,
        &[(0.005 * duration, amplitude), (0.995 * duration, amplitude)],
        duration,
    );
    for (sample, env) in output.iter_mut().zip(envelope) {
        *sample *= env;
    }
    output
}

fn cowbell(pitch: i32, rate: u32) -> Vec<f32> {
    let sr = rate as f64;
    let frequency = hz(pitch);
    let table = triangle_table();
    let length = samples(0.8, sr);
    let a = oscillator(&table, frequency, sr, length);
    let b = oscillator(&table, frequency * 3.46, sr, length);
    let c = oscillator(&table, frequency * 7.3, sr, length);
    let d = oscillator(&table, frequency * 1.52, sr, length);
    let cr = sr / 20.0;
    let env1 = pwl(cr, &[(0.0, 0.3), (0.8, 0.0005)], 0.8);
    let env2 = pwl(cr, &[(0.0, 0.7), (0.2, 0.01)], 0.2);
    let mut result = Vec::with_capacity(length);
    for i in 0..length {
        let e1 = (control_at(&env1, i, sr, cr) as f64).exp() as f32;
        let e2 = (control_at(&env2, i, sr, cr) as f64).exp() as f32;
        let first = (a[i] * b[i]) * e1;
        let second = if i < samples(0.2, sr) {
            (c[i] * d[i]) * e2
        } else {
            0.0
        };
        result.push((first + second) * 0.8);
    }
    result
}

fn noise_click(
    pitch: i32,
    amplitude: f64,
    duration: f64,
    q: f64,
    rate: u32,
    random: &mut GlibcRand,
) -> Vec<f32> {
    let sr = rate as f64;
    // normalize evaluates peak over the entire un-stretched, one-second
    // filtered noise sound. Its second use reads the cached samples.
    let mut noise: Vec<f32> = (0..rate).map(|_| random.white()).collect();
    Biquad::lowpass(hz(pitch), q, sr).process(&mut noise);
    let maximum = peak(&noise, noise.len());
    let gain = if maximum > 0.0 { 1.0 / maximum } else { 0.0 };
    let length = samples(duration, sr);
    let env = pwl(
        sr,
        &[(0.005 * duration, amplitude), (0.995 * duration, amplitude)],
        duration,
    );
    let mut output = Vec::with_capacity(length);
    for i in 0..length {
        let position = i as f64 / duration;
        let source = samples(position, sr);
        output.push(
            (noise.get(source).copied().unwrap_or(0.0) as f64
                * gain
                * env.get(i).copied().unwrap_or(0.0) as f64) as f32,
        );
    }
    output
}

fn drip(pitch: i32, amplitude: f64, stretch: f64, rate: u32) -> Vec<f32> {
    let sr = rate as f64;
    let base_length = samples(0.25, sr);
    let maxhz = sr / 2.1;
    let frequency = hz(pitch);
    let table = sine_table();
    let oscillators: Vec<Vec<f32>> = [2.40483, 5.52008, 8.653, 11.8]
        .iter()
        .map(|&multiple| oscillator(&table, maxhz.min(multiple * frequency), sr, base_length))
        .collect();

    let mut sound = Vec::with_capacity(base_length);
    for i in 0..base_length {
        let sum = oscillators[0][i] * 0.5
            + oscillators[1][i] * 0.25
            + oscillators[2][i] * 0.125
            + oscillators[3][i] * 0.0625;
        let decay = 0.5_f64.powf((i as f64 / sr) / 0.015);
        sound.push((sum as f64 * decay) as f32);
    }
    // tone.c, the one-pole low-pass used by Nyquist's lp.
    let b = 2.0 - (440.0 * 2.0 * std::f64::consts::PI / sr).cos();
    let c2 = b - (b * b - 1.0).sqrt();
    let c1 = 1.0 - c2;
    let mut previous = 0.0_f64;
    for sample in &mut sound {
        previous = c1 * *sample as f64 + c2 * previous;
        *sample = previous as f32;
    }
    let maximum = peak(&sound, sound.len());
    let gain = if maximum > 0.0 { 1.0 / maximum } else { 0.0 };

    let length = samples(0.25 * stretch, sr);
    let env = pwl(
        sr,
        &[(0.005 * stretch, amplitude), (0.995 * stretch, amplitude)],
        stretch,
    );
    let mut result = Vec::with_capacity(length);
    for i in 0..length {
        let source = samples(i as f64 / stretch / sr, sr);
        result.push(
            (sound.get(source).copied().unwrap_or(0.0) as f64
                * gain
                * env.get(i).copied().unwrap_or(0.0) as f64) as f32,
        );
    }
    result
}

fn add_at(output: &mut [f32], start: usize, click: &[f32]) {
    if start >= output.len() {
        return;
    }
    for (out, &value) in output[start..].iter_mut().zip(click) {
        *out += value;
    }
}

impl BuiltinEffect for RhythmTrack {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "RhythmTrack"
    }) }

    fn name(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Rhythm Track"
    }) }

    fn kind(&self) -> EffectKind {
        EffectKind::Generate
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("TEMPO", "Tempo (bpm)", 30.0, 300.0, 120.0),
            ParamDef::int("TIMESIG", "Beats per bar", 1.0, 20.0, 4.0),
            ParamDef::float("SWING", "Swing amount", -1.0, 1.0, 0.0),
            ParamDef::int("BARS", "Number of bars", 0.0, 1000.0, 16.0),
            ParamDef::float(
                "CLICK-TRACK-DUR",
                "Rhythm track duration",
                0.0,
                f64::MAX,
                0.0,
            ),
            ParamDef::float("OFFSET", "Start time offset", 0.0, f64::MAX, 0.0),
            ParamDef::choice("CLICK-TYPE", "Beat sound", CLICK_TYPES, 0),
            ParamDef::int("HIGH", "MIDI pitch of strong beat", 18.0, 116.0, 84.0),
            ParamDef::int("LOW", "MIDI pitch of weak beat", 18.0, 116.0, 80.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        Some(match key {
            "TEMPO" => self.tempo,
            "TIMESIG" => self.timesig as f64,
            "SWING" => self.swing,
            "BARS" => self.bars as f64,
            "CLICK-TRACK-DUR" => self.click_track_dur,
            "OFFSET" => self.offset,
            "CLICK-TYPE" => self.click_type as f64,
            "HIGH" => self.high as f64,
            "LOW" => self.low as f64,
            _ => return None,
        })
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "TEMPO" => self.tempo = value,
            "TIMESIG" => self.timesig = value as i32,
            "SWING" => self.swing = value,
            "BARS" => self.bars = value as i32,
            "CLICK-TRACK-DUR" => self.click_track_dur = value,
            "OFFSET" => self.offset = value,
            "CLICK-TYPE" => self.click_type = value as usize,
            "HIGH" => self.high = value as i32,
            "LOW" => self.low = value as i32,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if buf.sample_rate == 0 {
            return Err(EffectError::Invalid("sample rate must be positive".into()));
        }
        let sr = buf.sample_rate as f64;
        let tempo = self.tempo.clamp(30.0, 300.0);
        let timesig = self.timesig.clamp(1, 20) as usize;
        let beatlen = 60.0 / tempo;
        let measure_time = timesig as f64 * beatlen;
        let bars = if self.bars == 0 {
            (self.click_track_dur.max(0.0) / measure_time).ceil() as usize
        } else {
            self.bars.clamp(0, 1000) as usize
        };
        if bars == 0 {
            return Err(EffectError::Invalid(
                "Set either 'Number of bars' or 'Rhythm track duration' to greater than zero."
                    .into(),
            ));
        }

        let offset = samples(self.offset.max(0.0), sr);
        let total = samples(bars as f64 * measure_time, sr);
        let output_len = offset.saturating_add(total);
        // Avoid attempting allocations for pathological, unbounded time
        // parameters while retaining the plug-in's normal output length.
        if output_len > 200_000_000 {
            return Err(EffectError::Invalid("rhythm track is too long".into()));
        }

        let mut random = GlibcRand::new();
        let make_click = |accent: bool, random: &mut GlibcRand| {
            let pitch = if accent { self.high } else { self.low };
            let amplitude = if accent { 0.75 } else { 0.5 };
            match self.click_type {
                0 => metronome_tick(pitch, amplitude, buf.sample_rate),
                1 => ping(pitch, amplitude, 0.01, buf.sample_rate),
                2 => ping(pitch, amplitude, 0.08, buf.sample_rate),
                3 => cowbell(pitch, buf.sample_rate),
                4 => noise_click(pitch, amplitude, 0.05, 20.0, buf.sample_rate, random),
                5 => noise_click(pitch, amplitude, 0.005, 2.0, buf.sample_rate, random),
                6 => drip(pitch, amplitude, 0.007, buf.sample_rate),
                _ => drip(pitch, amplitude, 0.1, buf.sample_rate),
            }
        };

        // makemeasure evaluates its accented click once and each simrep
        // unaccented click once. seqrep cues that completed measure.
        let strong = make_click(true, &mut random);
        let weak: Vec<Vec<f32>> = (0..timesig.saturating_sub(1))
            .map(|_| make_click(false, &mut random))
            .collect();
        let last_start = if timesig > 1 {
            let count = timesig - 2;
            let adjustment = if (count + 1) % 2 == 1 {
                self.swing / 3.0
            } else {
                0.0
            };
            samples((count as f64 + 1.0 + adjustment) * beatlen, sr)
        } else {
            0
        };
        let tail = weak
            .last()
            .map_or(strong.len(), |click| last_start.saturating_add(click.len()));
        let measure_frames = samples(measure_time, sr).max(tail);
        let end_len = output_len.max(
            offset.saturating_add(
                samples((bars - 1) as f64 * measure_time, sr).saturating_add(measure_frames),
            ),
        );
        if end_len > 200_000_000 {
            return Err(EffectError::Invalid("rhythm track is too long".into()));
        }

        let mut mono = vec![0.0_f32; end_len];
        let mut previous_end = 0usize;
        for bar in 0..bars {
            // samplecount/get-measure: bars have integer cumulative sample
            // boundaries rather than independently rounded bar lengths.
            let end = samples((bar + 1) as f64 * measure_time, sr);
            let start = previous_end;
            previous_end = end;
            let base = offset + start;
            add_at(&mut mono, base, &strong);
            for (count, click) in weak.iter().enumerate() {
                let adjustment = if (count + 1) % 2 == 1 {
                    self.swing / 3.0
                } else {
                    0.0
                };
                let position =
                    samples(beatlen * (count as f64 + 1.0 + adjustment), sr);
                add_at(&mut mono, base.saturating_add(position), click);
            }
        }
        // The Nyquist plug-in is invoked separately for each selected
        // channel; it returns the same synthesized track for each.
        let channels = buf.channels.len().max(1);
        *buf = AudioBuffer::from_channels(
            buf.sample_rate,
            (0..channels).map(|_| mono.clone()).collect(),
        );
        Ok(())
    }
}

/// Returns this module's built-in effects.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(RhythmTrack::default())]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn short_stereo() -> AudioBuffer {
        let channel: Vec<f32> = (0..256)
            .map(|i| (2.0 * std::f32::consts::PI * i as f32 / 32.0).sin())
            .collect();
        AudioBuffer::from_channels(44100, vec![channel.clone(), channel])
    }

    #[test]
    fn default_stereo_run() {
        let mut effect = RhythmTrack::default();
        effect.bars = 1;
        let mut audio = short_stereo();
        effect
            .process(&mut audio, &mut EffectContext::default())
            .unwrap();
        assert_eq!(audio.channels.len(), 2);
        assert_eq!(audio.len(), 88200);
        assert_eq!(audio.channels[0], audio.channels[1]);
    }

    #[test]
    fn empty_input() {
        let mut effect = RhythmTrack::default();
        effect.bars = 1;
        let mut audio = AudioBuffer::from_channels(44100, vec![vec![]]);
        effect
            .process(&mut audio, &mut EffectContext::default())
            .unwrap();
        assert_eq!(audio.len(), 88200);
    }

    #[test]
    fn repeatable() {
        let mut effect = RhythmTrack::default();
        effect.bars = 1;
        effect.click_type = 5;
        let mut a = short_stereo();
        let mut b = short_stereo();
        effect.process(&mut a, &mut EffectContext::default()).unwrap();
        effect.process(&mut b, &mut EffectContext::default()).unwrap();
        assert_eq!(a.channels, b.channels);
    }

    #[test]
    fn parameters_are_clamped() {
        let mut effect = RhythmTrack::default();
        assert!(set_param(&mut effect, "TEMPO", 1000.0));
        assert_eq!(effect.tempo, 300.0);
        assert!(set_param(&mut effect, "CLICK-TYPE", 100.0));
        assert_eq!(effect.click_type, 7);
        assert!(set_param(&mut effect, "BARS", -3.0));
        assert_eq!(effect.bars, 0);
    }

    #[test]
    fn glibc_default_seed() {
        let mut random = GlibcRand::new();
        assert_eq!(random.next(), 1804289383);
        assert_eq!(random.next(), 846930886);
    }

    #[test]
    fn nyquist_block_size_is_positive() {
        assert!(NYQUIST_BLOCK > 0);
    }
}
