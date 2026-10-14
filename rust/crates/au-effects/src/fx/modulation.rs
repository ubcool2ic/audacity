#![allow(dead_code)]
// SPDX-License-Identifier: GPL-3.0-or-later
//! Modulation and delay effects ported from Audacity.

use crate::core::*;
use au_core::AudioBuffer;
use rustfft::{num_complex::Complex, FftPlanner};
use std::collections::VecDeque;
use std::f64::consts::PI;

/// Echo effect.
pub struct Echo {
    /// Delay time in seconds.
    pub delay: f64,
    /// Decay amount per echo.
    pub decay: f64,
    sample_rate: u32,
    histories: Vec<Vec<f32>>,
    positions: Vec<usize>,
}

impl Default for Echo {
    fn default() -> Self {
        Self {
            delay: 1.0,
            decay: 0.5,
            sample_rate: 0,
            histories: Vec::new(),
            positions: Vec::new(),
        }
    }
}

impl BuiltinEffect for Echo {
    fn id(&self) -> &'static str {
        "Echo"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Delay", "Delay", 0.001, f32::MAX as f64, 1.0),
            ParamDef::float("Decay", "Decay", 0.0, f32::MAX as f64, 0.5),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Delay" => Some(self.delay),
            "Decay" => Some(self.decay),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Delay" => self.delay = value,
            "Decay" => self.decay = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        process_via_realtime(self, buf);
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        self.sample_rate = sample_rate;
        let requested = ((sample_rate as f64 * self.delay).floor() as usize).max(1);
        let history_len = requested.min(10_000_000);
        self.histories = (0..channels).map(|_| vec![0.0; history_len]).collect();
        self.positions = vec![0; channels];
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        let decay = self.decay;
        for (channel, (history, position)) in block
            .iter_mut()
            .zip(self.histories.iter_mut().zip(self.positions.iter_mut()))
        {
            if history.is_empty() {
                continue;
            }
            for sample in channel {
                if *position == history.len() {
                    *position = 0;
                }
                let out = *sample as f64 + history[*position] as f64 * decay;
                let out = out as f32;
                history[*position] = out;
                *sample = out;
                *position += 1;
            }
        }
    }
}

/// Phaser effect.
pub struct Phaser {
    /// Number of phase-shifting stages.
    pub stages: i32,
    /// Dry/wet amount, from 0 to 255.
    pub dry_wet: i32,
    /// LFO frequency in hertz.
    pub freq: f64,
    /// LFO starting phase in degrees.
    pub phase: f64,
    /// LFO depth, from 0 to 255.
    pub depth: i32,
    /// Feedback amount, from -100 to 100.
    pub feedback: i32,
    /// Output gain in decibels.
    pub gain_db: f64,
    sample_rate: u32,
    states: Vec<PhaserState>,
}

#[derive(Clone)]
struct PhaserState {
    old: [f64; 24],
    gain: f64,
    fbout: f64,
    skipcount: u64,
}

impl Default for Phaser {
    fn default() -> Self {
        Self {
            stages: 2,
            dry_wet: 128,
            freq: 0.4,
            phase: 0.0,
            depth: 100,
            feedback: 0,
            gain_db: -6.0,
            sample_rate: 0,
            states: Vec::new(),
        }
    }
}

impl BuiltinEffect for Phaser {
    fn id(&self) -> &'static str {
        "Phaser"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::int("Stages", "Stages", 2.0, 24.0, 2.0),
            ParamDef::int("DryWet", "Dry/Wet", 0.0, 255.0, 128.0),
            ParamDef::float("Freq", "Frequency", 0.001, 4.0, 0.4),
            ParamDef::float("Phase", "Phase", 0.0, 360.0, 0.0),
            ParamDef::int("Depth", "Depth", 0.0, 255.0, 100.0),
            ParamDef::int("Feedback", "Feedback", -100.0, 100.0, 0.0),
            ParamDef::float("Gain", "Gain", -30.0, 30.0, -6.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Stages" => Some(self.stages as f64),
            "DryWet" => Some(self.dry_wet as f64),
            "Freq" => Some(self.freq),
            "Phase" => Some(self.phase),
            "Depth" => Some(self.depth as f64),
            "Feedback" => Some(self.feedback as f64),
            "Gain" => Some(self.gain_db),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Stages" => self.stages = value.round() as i32,
            "DryWet" => self.dry_wet = value.round() as i32,
            "Freq" => self.freq = value,
            "Phase" => self.phase = value,
            "Depth" => self.depth = value.round() as i32,
            "Feedback" => self.feedback = value.round() as i32,
            "Gain" => self.gain_db = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        process_via_realtime(self, buf);
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        self.sample_rate = sample_rate;
        self.states = (0..channels)
            .map(|_| PhaserState {
                old: [0.0; 24],
                gain: 0.0,
                fbout: 0.0,
                skipcount: 0,
            })
            .collect();
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        let stages = (self.stages & !1).clamp(2, 24) as usize;
        let dry_wet = self.dry_wet;
        let depth = self.depth;
        let feedback = self.feedback;
        let freq = self.freq;
        let phase = self.phase;
        let gain_db = self.gain_db;
        let sample_rate = self.sample_rate.max(1) as f32;
        let lfoskip = freq * 2.0 * PI / sample_rate as f64;
        let phase_radians = phase * PI / 180.0;
        let outgain = 10.0_f64.powf(gain_db / 20.0);

        for (channel, state) in block.iter_mut().zip(self.states.iter_mut()) {
            for sample in channel {
                let input = *sample as f64;
                let mut value = input + state.fbout * feedback as f64 / 101.0;

                state.skipcount = state.skipcount.wrapping_add(1);
                if (state.skipcount - 1) % 20 == 0 {
                    state.gain =
                        (1.0 + ((state.skipcount as f64) * lfoskip + phase_radians).cos()) / 2.0;
                    state.gain = ((state.gain * 4.0).exp_m1()) / 4.0_f64.exp_m1();
                    state.gain = 1.0 - state.gain / 255.0 * depth as f64;
                }

                for j in 0..stages {
                    let old = state.old[j];
                    state.old[j] = state.gain * old + value;
                    value = old - state.gain * state.old[j];
                }
                state.fbout = value;

                *sample = (outgain
                    * (value * dry_wet as f64 + input * (255 - dry_wet) as f64)
                    / 255.0) as f32;
            }
        }
    }
}

/// Wahwah effect.
pub struct Wahwah {
    /// LFO frequency in hertz.
    pub freq: f64,
    /// LFO starting phase in degrees.
    pub phase: f64,
    /// Depth, from 0 to 100.
    pub depth: i32,
    /// Resonance.
    pub resonance: f64,
    /// Frequency offset, from 0 to 100.
    pub offset: i32,
    /// Output gain in decibels.
    pub gain: f64,
    sample_rate: u32,
    states: Vec<WahwahState>,
}

#[derive(Clone)]
struct WahwahState {
    skipcount: u64,
    xn1: f64,
    xn2: f64,
    yn1: f64,
    yn2: f64,
    b0: f64,
    b1: f64,
    b2: f64,
    a0: f64,
    a1: f64,
    a2: f64,
}

impl Default for Wahwah {
    fn default() -> Self {
        Self {
            freq: 1.5,
            phase: 0.0,
            depth: 70,
            resonance: 2.5,
            offset: 30,
            gain: -6.0,
            sample_rate: 0,
            states: Vec::new(),
        }
    }
}

impl BuiltinEffect for Wahwah {
    fn id(&self) -> &'static str {
        "Wahwah"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Freq", "Frequency", 0.1, 4.0, 1.5),
            ParamDef::float("Phase", "Phase", 0.0, 360.0, 0.0),
            ParamDef::int("Depth", "Depth", 0.0, 100.0, 70.0),
            ParamDef::float("Resonance", "Resonance", 0.1, 10.0, 2.5),
            ParamDef::int("Offset", "Offset", 0.0, 100.0, 30.0),
            ParamDef::float("Gain", "Gain", -30.0, 30.0, -6.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Freq" => Some(self.freq),
            "Phase" => Some(self.phase),
            "Depth" => Some(self.depth as f64),
            "Resonance" => Some(self.resonance),
            "Offset" => Some(self.offset as f64),
            "Gain" => Some(self.gain),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Freq" => self.freq = value,
            "Phase" => self.phase = value,
            "Depth" => self.depth = value.round() as i32,
            "Resonance" => self.resonance = value,
            "Offset" => self.offset = value.round() as i32,
            "Gain" => self.gain = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        process_via_realtime(self, buf);
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        self.sample_rate = sample_rate;
        self.states = (0..channels)
            .map(|_| WahwahState {
                skipcount: 0,
                xn1: 0.0,
                xn2: 0.0,
                yn1: 0.0,
                yn2: 0.0,
                b0: 0.0,
                b1: 0.0,
                b2: 0.0,
                a0: 0.0,
                a1: 0.0,
                a2: 0.0,
            })
            .collect();
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        let freq = self.freq;
        let phase = self.phase * PI / 180.0;
        let depth = self.depth as f64 / 100.0;
        let offset = self.offset as f64 / 100.0;
        let resonance = self.resonance;
        let outgain = 10.0_f64.powf(self.gain / 20.0);
        let sample_rate = self.sample_rate.max(1) as f64;
        let lfoskip = freq * 2.0 * PI / sample_rate;

        for (channel, state) in block.iter_mut().zip(self.states.iter_mut()) {
            for sample in channel {
                let input = *sample as f64;

                state.skipcount = state.skipcount.wrapping_add(1);
                if (state.skipcount - 1) % 30 == 0 {
                    let mut frequency =
                        (1.0 + (state.skipcount as f64 * lfoskip + phase).cos()) / 2.0;
                    frequency = frequency * depth * (1.0 - offset) + offset;
                    frequency = ((frequency - 1.0) * 6.0).exp();

                    let omega = PI * frequency;
                    let sn = omega.sin();
                    let cs = omega.cos();
                    let alpha = sn / (2.0 * resonance);
                    state.b0 = (1.0 - cs) / 2.0;
                    state.b1 = 1.0 - cs;
                    state.b2 = (1.0 - cs) / 2.0;
                    state.a0 = 1.0 + alpha;
                    state.a1 = -2.0 * cs;
                    state.a2 = 1.0 - alpha;
                }

                let output = (state.b0 * input + state.b1 * state.xn1 + state.b2 * state.xn2
                    - state.a1 * state.yn1
                    - state.a2 * state.yn2)
                    / state.a0;
                state.xn2 = state.xn1;
                state.xn1 = input;
                state.yn2 = state.yn1;
                state.yn1 = output;

                *sample = (output * outgain) as f32;
            }
        }
    }
}

const DISTORTION_TYPES: &[&str] = &[
    "Hard Clipping",
    "Soft Clipping",
    "Soft Overdrive",
    "Medium Overdrive",
    "Hard Overdrive",
    "Cubic Curve (odd harmonics)",
    "Even Harmonics",
    "Expand and Compress",
    "Leveller",
    "Rectifier Distortion",
    "Hard Limiter 1413",
];

const DISTORTION_STEPS: usize = 1024;
const DISTORTION_TABLE_SIZE: usize = DISTORTION_STEPS * 2 + 1;

#[derive(Clone, Copy)]
struct DistortionSettings {
    kind: usize,
    dc_block: bool,
    threshold_db: f64,
    noise_floor: f64,
    param1: f64,
    param2: f64,
    repeats: usize,
}

struct DistortionState {
    sample_rate: f32,
    skipcount: u64,
    kind: usize,
    dc_block: bool,
    threshold_db: f64,
    noise_floor: f64,
    param1: f64,
    param2: f64,
    repeats: usize,
    table: [f64; DISTORTION_TABLE_SIZE],
    queue: VecDeque<f32>,
    queue_total: f64,
    makeup_gain: f64,
}

impl DistortionState {
    fn new(sample_rate: f32, settings: DistortionSettings) -> Self {
        let mut state = Self {
            sample_rate,
            skipcount: 0,
            kind: settings.kind,
            dc_block: settings.dc_block,
            threshold_db: settings.threshold_db,
            noise_floor: settings.noise_floor,
            param1: settings.param1,
            param2: settings.param2,
            repeats: settings.repeats,
            table: [0.0; DISTORTION_TABLE_SIZE],
            queue: VecDeque::new(),
            queue_total: 0.0,
            makeup_gain: 1.0,
        };
        state.make_table(settings);
        state
    }

    fn make_table(&mut self, settings: DistortionSettings) {
        match settings.kind {
            0 => self.hard_clip(settings),
            1 => self.soft_clip(settings),
            2 => self.half_sin_table(settings),
            3 => self.exponential_table(settings),
            4 => self.logarithmic_table(settings),
            5 => self.cubic_table(settings),
            6 => self.even_harmonic_table(settings),
            7 => self.sine_table(settings),
            8 => self.leveller(settings),
            9 => self.rectifier(settings),
            10 => self.hard_limiter(settings),
            _ => {}
        }
    }

    fn hard_clip(&mut self, settings: DistortionSettings) {
        let threshold = 10.0_f64.powf(settings.threshold_db / 20.0);
        let low_thresh = 1.0 - threshold;
        let high_thresh = 1.0 + threshold;

        for n in 0..DISTORTION_TABLE_SIZE {
            self.table[n] = if (n as f64) < DISTORTION_STEPS as f64 * low_thresh {
                -threshold
            } else if (n as f64) > DISTORTION_STEPS as f64 * high_thresh {
                threshold
            } else {
                n as f64 / DISTORTION_STEPS as f64 - 1.0
            };
            self.makeup_gain = 1.0 / threshold;
        }
    }

    fn soft_clip(&mut self, settings: DistortionSettings) {
        let threshold_linear = 10.0_f64.powf(settings.threshold_db / 20.0);
        let threshold = 1.0 + threshold_linear;
        let amount = 2.0_f64.powf(7.0 * settings.param1 / 100.0);
        let peak = Self::log_curve(threshold_linear, 1.0, amount);
        self.makeup_gain = 1.0 / peak as f64;
        self.table[DISTORTION_STEPS] = 0.0;

        for n in DISTORTION_STEPS..DISTORTION_TABLE_SIZE {
            self.table[n] = if (n as f64) < DISTORTION_STEPS as f64 * threshold {
                n as f64 / DISTORTION_STEPS as f64 - 1.0
            } else {
                Self::log_curve(
                    threshold_linear,
                    (n as f64 / DISTORTION_STEPS as f64 - 1.0) as f32,
                    amount,
                ) as f64
            };
        }
        self.copy_half_table();
    }

    fn log_curve(threshold: f64, value: f32, ratio: f64) -> f32 {
        (threshold + ((ratio * (threshold - value as f64)).exp() - 1.0) / -ratio) as f32
    }

    fn exponential_table(&mut self, settings: DistortionSettings) {
        let amount = (10.0_f64.powf(-settings.param1 / 20.0)).min(0.999);
        for n in DISTORTION_STEPS..DISTORTION_TABLE_SIZE {
            let lin_val = n as f64 / DISTORTION_STEPS as f64;
            let scale = -1.0 / (1.0 - amount);
            let curve = ((lin_val - 1.0) * amount.ln()).exp();
            self.table[n] = scale * (curve - 1.0);
        }
        self.copy_half_table();
    }

    fn logarithmic_table(&mut self, settings: DistortionSettings) {
        let amount = settings.param1;
        let step_size = 1.0 / DISTORTION_STEPS as f64;
        let mut lin_val = 0.0;
        for n in DISTORTION_STEPS..DISTORTION_TABLE_SIZE {
            self.table[n] = if amount == 0.0 {
                lin_val
            } else {
                (1.0 + amount * lin_val).ln() / (1.0 + amount).ln()
            };
            lin_val += step_size;
        }
        self.copy_half_table();
    }

    fn half_sin_table(&mut self, settings: DistortionSettings) {
        let iter = (settings.param1 / 20.0).floor() as usize;
        let fractional = settings.param1 / 20.0 - iter as f64;
        let step_size = 1.0 / DISTORTION_STEPS as f64;
        let mut lin_val = 0.0;

        for n in DISTORTION_STEPS..DISTORTION_TABLE_SIZE {
            self.table[n] = lin_val;
            for _ in 0..iter {
                self.table[n] = (self.table[n] * PI / 2.0).sin();
            }
            self.table[n] +=
                ((self.table[n] * PI / 2.0).sin() - self.table[n]) * fractional;
            lin_val += step_size;
        }
        self.copy_half_table();
    }

    fn cubic(settings: DistortionSettings, x: f64) -> f64 {
        if settings.param1 == 0.0 {
            x
        } else {
            x - x.powi(3) / 3.0
        }
    }

    fn cubic_table(&mut self, settings: DistortionSettings) {
        let amount = settings.param1 * 3.0_f64.sqrt() / 100.0;
        let gain = if amount != 0.0 {
            1.0 / Self::cubic(settings, amount.min(1.0))
        } else {
            1.0
        };
        let step_size = amount / DISTORTION_STEPS as f64;
        let mut x = -amount;

        if amount == 0.0 {
            for i in 0..DISTORTION_TABLE_SIZE {
                self.table[i] = i as f64 / DISTORTION_STEPS as f64 - 1.0;
            }
        } else {
            for i in 0..DISTORTION_TABLE_SIZE {
                self.table[i] = gain * Self::cubic(settings, x);
                for _ in 0..settings.repeats {
                    self.table[i] = gain * Self::cubic(settings, self.table[i] * amount);
                }
                x += step_size;
            }
        }
    }

    fn even_harmonic_table(&mut self, settings: DistortionSettings) {
        let amount = settings.param1 / -100.0;
        let c = settings.param2.max(0.001) / 10.0;
        let step = 1.0 / DISTORTION_STEPS as f64;
        let mut xval = -1.0;
        for entry in &mut self.table {
            *entry = ((1.0 + amount) * xval)
                - (xval * (amount / c.tanh()) * (c * xval).tanh());
            xval += step;
        }
    }

    fn sine_table(&mut self, settings: DistortionSettings) {
        let iter = (settings.param1 / 20.0).floor() as usize;
        let fractional = settings.param1 / 20.0 - iter as f64;
        let step_size = 1.0 / DISTORTION_STEPS as f64;
        let mut lin_val = 0.0;

        for n in DISTORTION_STEPS..DISTORTION_TABLE_SIZE {
            self.table[n] = lin_val;
            for _ in 0..iter {
                self.table[n] = (1.0 + (self.table[n] * PI - PI / 2.0).sin()) / 2.0;
            }
            self.table[n] +=
                ((1.0 + (self.table[n] * PI - PI / 2.0).sin()) / 2.0 - self.table[n])
                    * fractional;
            lin_val += step_size;
        }
        self.copy_half_table();
    }

    fn leveller(&mut self, settings: DistortionSettings) {
        let noise_floor = 10.0_f64.powf(settings.noise_floor / 20.0);
        let gain_factors = [0.80, 1.00, 1.20, 1.20, 1.00, 0.80];
        let mut gain_limits = [0.0001, 0.0, 0.1, 0.3, 0.5, 1.0];
        gain_limits[1] = noise_floor;
        let mut add_on = [0.0; 6];

        for i in 0..5 {
            add_on[i + 1] =
                add_on[i] + gain_limits[i] * (gain_factors[i] - gain_factors[i + 1]);
        }

        let fractional_pass = settings.param1 / 100.0;
        for n in DISTORTION_STEPS..DISTORTION_TABLE_SIZE {
            self.table[n] = (n - DISTORTION_STEPS) as f64 / DISTORTION_STEPS as f64;
            for _ in 0..settings.repeats {
                let mut index = 5;
                for i in (0..=5).rev() {
                    if self.table[n] < gain_limits[i] {
                        index = i;
                    } else {
                        break;
                    }
                }
                self.table[n] =
                    self.table[n] * gain_factors[index] + add_on[index];
            }
            if fractional_pass > 0.001 {
                let mut index = 5;
                for i in (0..=5).rev() {
                    if self.table[n] < gain_limits[i] {
                        index = i;
                    } else {
                        break;
                    }
                }
                self.table[n] += fractional_pass
                    * (self.table[n] * (gain_factors[index] - 1.0) + add_on[index]);
            }
        }
        self.copy_half_table();
    }

    fn rectifier(&mut self, settings: DistortionSettings) {
        let amount = settings.param1 / 50.0 - 1.0;
        let step_size = 1.0 / DISTORTION_STEPS as f64;

        for n in 0..=DISTORTION_STEPS {
            self.table[DISTORTION_STEPS + n] = n as f64 * step_size;
        }
        for n in 1..=DISTORTION_STEPS {
            self.table[DISTORTION_STEPS - n] = n as f64 * step_size * amount;
        }
    }

    fn hard_limiter(&mut self, settings: DistortionSettings) {
        self.hard_clip(settings);
    }

    fn copy_half_table(&mut self) {
        for n in 0..DISTORTION_STEPS {
            self.table[n] = -self.table[DISTORTION_TABLE_SIZE - 1 - n];
        }
    }

    fn waveshaper(&self, mut sample: f32, settings: DistortionSettings) -> f32 {
        if settings.kind == 0 {
            let amount = settings.param1 / 100.0;
            sample = (sample as f64 * (1.0 + amount)) as f32;
        }

        let mut index = (sample * DISTORTION_STEPS as f32).floor() as isize
            + DISTORTION_STEPS as isize;
        index = index.clamp(0, (2 * DISTORTION_STEPS - 1) as isize);
        let mut x_offset =
            ((1.0_f32 + sample) * DISTORTION_STEPS as f32) as f64 - index as f64;
        x_offset = x_offset.clamp(0.0, 1.0);

        (self.table[index as usize]
            + (self.table[index as usize + 1] - self.table[index as usize]) * x_offset)
            as f32
    }

    fn dc_filter(&mut self, sample: f32) -> f32 {
        let queue_length = (self.sample_rate as f64 / 20.0).floor() as usize;
        self.queue_total += sample as f64;
        self.queue.push_back(sample);
        if self.queue.len() > queue_length {
            if let Some(front) = self.queue.pop_front() {
                self.queue_total -= front as f64;
            }
        }
        if self.queue.is_empty() {
            sample
        } else {
            (sample as f64 - self.queue_total / self.queue.len() as f64) as f32
        }
    }

    fn process_sample(&mut self, sample: f32, settings: DistortionSettings) -> f32 {
        let update = settings.kind != self.kind
            || settings.noise_floor != self.noise_floor
            || settings.threshold_db != self.threshold_db
            || settings.param1 != self.param1
            || settings.param2 != self.param2
            || settings.repeats != self.repeats;

        if update {
            let current = self.skipcount;
            self.skipcount = self.skipcount.wrapping_add(1);
            if current % 1000 == 0 {
                self.make_table(settings);
            }
        }

        let p1 = settings.param1 / 100.0;
        let p2 = settings.param2 / 100.0;
        let shaped = self.waveshaper(sample, settings);

        let mut output = match settings.kind {
            0 | 1 => {
                (shaped as f64 * ((1.0 - p2) + self.makeup_gain * p2)) as f32
            }
            2..=5 | 7 => (shaped as f64 * p2) as f32,
            6 | 8 | 9 => shaped,
            10 => ((shaped as f64 * (p1 - p2)) + sample as f64 * p2) as f32,
            _ => shaped,
        };

        if settings.dc_block {
            output = self.dc_filter(output);
        }
        output
    }
}

/// Distortion effect.
pub struct Distortion {
    /// Distortion table type.
    pub kind: usize,
    /// Whether to remove DC offset.
    pub dc_block: bool,
    /// Clipping threshold in decibels.
    pub threshold_db: f64,
    /// Noise floor in decibels.
    pub noise_floor: f64,
    /// First distortion parameter.
    pub param1: f64,
    /// Second distortion parameter.
    pub param2: f64,
    /// Number of repeated passes.
    pub repeats: i32,
    sample_rate: u32,
    states: Vec<DistortionState>,
}

impl Default for Distortion {
    fn default() -> Self {
        Self {
            kind: 0,
            dc_block: false,
            threshold_db: -6.0,
            noise_floor: -70.0,
            param1: 50.0,
            param2: 50.0,
            repeats: 1,
            sample_rate: 0,
            states: Vec::new(),
        }
    }
}

impl BuiltinEffect for Distortion {
    fn id(&self) -> &'static str {
        "Distortion"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::choice("Type", "Type", DISTORTION_TYPES, 0),
            ParamDef::boolean("DC Block", "DC Block", false),
            ParamDef::float("Threshold dB", "Threshold (dB)", -100.0, 0.0, -6.0),
            ParamDef::float("Noise Floor", "Noise Floor", -80.0, -20.0, -70.0),
            ParamDef::float("Parameter 1", "Parameter 1", 0.0, 100.0, 50.0),
            ParamDef::float("Parameter 2", "Parameter 2", 0.0, 100.0, 50.0),
            ParamDef::int("Repeats", "Repeats", 0.0, 5.0, 1.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Type" => Some(self.kind as f64),
            "DC Block" => Some(if self.dc_block { 1.0 } else { 0.0 }),
            "Threshold dB" => Some(self.threshold_db),
            "Noise Floor" => Some(self.noise_floor),
            "Parameter 1" => Some(self.param1),
            "Parameter 2" => Some(self.param2),
            "Repeats" => Some(self.repeats as f64),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Type" => self.kind = value.round() as usize,
            "DC Block" => self.dc_block = value != 0.0,
            "Threshold dB" => self.threshold_db = value,
            "Noise Floor" => self.noise_floor = value,
            "Parameter 1" => self.param1 = value,
            "Parameter 2" => self.param2 = value,
            "Repeats" => self.repeats = value.round() as i32,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        process_via_realtime(self, buf);
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        self.sample_rate = sample_rate;
        let settings = DistortionSettings {
            kind: self.kind,
            dc_block: self.dc_block,
            threshold_db: self.threshold_db,
            noise_floor: self.noise_floor,
            param1: self.param1,
            param2: self.param2,
            repeats: self.repeats.max(0) as usize,
        };
        self.states = (0..channels)
            .map(|_| DistortionState::new(sample_rate as f32, settings))
            .collect();
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        let settings = DistortionSettings {
            kind: self.kind,
            dc_block: self.dc_block,
            threshold_db: self.threshold_db,
            noise_floor: self.noise_floor,
            param1: self.param1,
            param2: self.param2,
            repeats: self.repeats.max(0) as usize,
        };
        for (channel, state) in block.iter_mut().zip(self.states.iter_mut()) {
            for sample in channel {
                *sample = state.process_sample(*sample, settings);
            }
        }
    }
}

/// Tremolo effect.
pub struct Tremolo {
    /// Waveform selection: sine, triangle, sawtooth, inverse sawtooth, or square.
    pub wave: usize,
    /// Starting phase in degrees.
    pub phase: i32,
    /// Wet level in percent.
    pub wet: i32,
    /// LFO frequency in hertz.
    pub lfo: f64,
    sample_rate: u32,
    frame: u64,
}

impl Default for Tremolo {
    fn default() -> Self {
        Self {
            wave: 0,
            phase: 0,
            wet: 40,
            lfo: 4.0,
            sample_rate: 0,
            frame: 0,
        }
    }
}

impl BuiltinEffect for Tremolo {
    fn id(&self) -> &'static str {
        "Tremolo"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::choice(
                "WAVE",
                "Waveform type",
                &[
                    "Sine",
                    "Triangle",
                    "Sawtooth",
                    "Inverse Sawtooth",
                    "Square",
                ],
                0,
            ),
            ParamDef::int("PHASE", "Starting phase (degrees)", -180.0, 180.0, 0.0),
            ParamDef::int("WET", "Wet level (percent)", 1.0, 100.0, 40.0),
            ParamDef::float("LFO", "Frequency (Hz)", 0.001, 1000.0, 4.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "WAVE" => Some(self.wave as f64),
            "PHASE" => Some(self.phase as f64),
            "WET" => Some(self.wet as f64),
            "LFO" => Some(self.lfo),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "WAVE" => self.wave = value.round() as usize,
            "PHASE" => self.phase = value.round() as i32,
            "WET" => self.wet = value.round() as i32,
            "LFO" => self.lfo = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        process_via_realtime(self, buf);
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, _channels: usize) {
        self.sample_rate = sample_rate;
        self.frame = 0;
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        let sample_rate = self.sample_rate.max(1) as f64;
        let start = self.frame;
        self.frame += block.first().map_or(0, |c| c.len()) as u64;
        let wave = self.wave;
        let phase = self.phase as f64 / 360.0;
        let level = self.wet as f64 / 200.0;
        let lfo = self.lfo;

        for channel in block.iter_mut() {
            for (i, sample) in channel.iter_mut().enumerate() {
                let p = (phase + (start + i as u64) as f64 * lfo / sample_rate).rem_euclid(1.0);
                let modulation = match wave {
                    // tremolo.ny shifts *sine-table* by -90° so every waveform starts at its minimum.
                    0 => (2.0 * PI * (p - 0.25)).sin(),
                    1 => {
                        if p < 0.5 {
                            -1.0 + 4.0 * p
                        } else {
                            3.0 - 4.0 * p
                        }
                    }
                    2 => {
                        if p < 0.995 {
                            -1.0 + 2.0 * p / 0.995
                        } else {
                            1.0 - 2.0 * (p - 0.995) / 0.005
                        }
                    }
                    3 => {
                        if p < 0.005 {
                            -1.0 + 2.0 * p / 0.005
                        } else {
                            1.0 - 2.0 * (p - 0.005) / 0.995
                        }
                    }
                    _ => {
                        if p < 0.005 {
                            -1.0 + 2.0 * p / 0.005
                        } else if p < 0.5 {
                            1.0
                        } else if p < 0.505 {
                            1.0 - 2.0 * (p - 0.5) / 0.005
                        } else {
                            -1.0
                        }
                    }
                };
                *sample = (*sample as f64 * (1.0 - level + level * modulation)) as f32;
            }
        }
    }
}

const DELAY_TYPES: &[&str] = &["Regular", "Bouncing Ball", "Reverse Bouncing Ball"];
const PITCH_TYPES: &[&str] = &[
    "Pitch/Tempo",
    "Low-quality Pitch Shift",
    "High-quality Pitch Shift",
];
const CONSTRAIN_TYPES: &[&str] = &["Yes", "No"];

/// Delay effect.
pub struct Delay {
    /// Delay type selection.
    pub delay_type: usize,
    /// Gain per echo in decibels.
    pub dgain: f64,
    /// Delay time in seconds.
    pub delay: f64,
    /// Pitch change algorithm selection.
    pub pitch_type: usize,
    /// Pitch change per echo in semitones.
    pub shift: f64,
    /// Number of echoes.
    pub number: i32,
    /// Whether to constrain output to the input duration.
    pub constrain: usize,
}

impl Default for Delay {
    fn default() -> Self {
        Self {
            delay_type: 0,
            dgain: -6.0,
            delay: 0.3,
            pitch_type: 0,
            shift: 0.0,
            number: 5,
            constrain: 0,
        }
    }
}

impl BuiltinEffect for Delay {
    fn id(&self) -> &'static str {
        "Delay"
    }

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
            ParamDef::choice("CONSTRAIN", "Allow duration to change", CONSTRAIN_TYPES, 0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "DELAY-TYPE" => Some(self.delay_type as f64),
            "DGAIN" => Some(self.dgain),
            "DELAY" => Some(self.delay),
            "PITCH-TYPE" => Some(self.pitch_type as f64),
            "SHIFT" => Some(self.shift),
            "NUMBER" => Some(self.number as f64),
            "CONSTRAIN" => Some(self.constrain as f64),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "DELAY-TYPE" => self.delay_type = value.round() as usize,
            "DGAIN" => self.dgain = value,
            "DELAY" => self.delay = value,
            "PITCH-TYPE" => self.pitch_type = value.round() as usize,
            "SHIFT" => self.shift = value,
            "NUMBER" => self.number = value.round() as i32,
            "CONSTRAIN" => self.constrain = value.round() as usize,
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
        if buf.channels.is_empty() || input_len == 0 {
            return Ok(());
        }

        let sample_rate = buf.sample_rate.max(1) as f64;
        let count = self.number.clamp(1, 30) as usize;
        let delay_seconds = self.delay.max(0.0);
        let time_shift = if self.delay_type >= 1 {
            delay_seconds / count as f64
        } else {
            delay_seconds
        };

        let mut output_channels = Vec::with_capacity(buf.channels.len());
        for input in &buf.channels {
            let source_len = input_len.min(input.len());
            let source = &input[..source_len];
            let mut echoes = Vec::with_capacity(count);
            let mut output_len = source_len;
            let mut dly = 0.0f64; // delay.ny accumulates the per-echo shift

            for echo_num in 1..=count {
                let ratio = 2.0_f64.powf(echo_num as f64 * self.shift / 12.0);
                let modified = if self.shift == 0.0 {
                    source.to_vec()
                } else if self.pitch_type == 0 {
                    resample_speed(source, ratio)
                } else {
                    pitch_shift_preserving_duration(
                        source,
                        ratio,
                        if self.pitch_type == 1 { 512 } else { 2048 },
                    )
                };

                dly += match self.delay_type {
                    1 => time_shift * (count + 1 - echo_num) as f64,
                    2 => time_shift * echo_num as f64,
                    _ => time_shift,
                };

                let offset = (dly * sample_rate)
                    .round()
                    .max(0.0) as usize;
                let end = offset.saturating_add(modified.len());
                output_len = output_len.max(end);
                echoes.push((offset, modified, echo_num));
            }

            // CONSTRAIN: 0 = "Yes" (allow duration to change), 1 = "No" (keep length).
            if self.constrain == 1 {
                output_len = source_len;
            } else {
                output_len = output_len.min(100_000_000);
            }
            let mut output = vec![0.0f32; output_len];
            output[..source_len.min(output_len)].copy_from_slice(&source[..source_len.min(output_len)]);

            for (offset, echo, echo_num) in echoes {
                let gain = 10.0_f64.powf(echo_num as f64 * self.dgain / 20.0) as f32;
                for (i, sample) in echo.iter().enumerate() {
                    let index = offset.saturating_add(i);
                    if index < output.len() {
                        output[index] += *sample * gain;
                    }
                }
            }
            output_channels.push(output);
        }

        buf.channels = output_channels;
        Ok(())
    }
}

fn resample_speed(input: &[f32], ratio: f64) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    if ratio == 1.0 {
        return input.to_vec();
    }

    let length = ((input.len() as f64 / ratio).ceil() as usize).min(100_000_000);
    let mut output = Vec::with_capacity(length);
    for i in 0..length {
        let position = (i as f64 * ratio).min((input.len() - 1) as f64);
        let left = position.floor() as usize;
        let right = (left + 1).min(input.len() - 1);
        let fraction = position - left as f64;
        output.push(
            (input[left] as f64 * (1.0 - fraction) + input[right] as f64 * fraction) as f32,
        );
    }
    output
}

fn pitch_shift_preserving_duration(input: &[f32], ratio: f64, fft_size: usize) -> Vec<f32> {
    if input.is_empty() || ratio == 1.0 {
        return input.to_vec();
    }
    let shortened = resample_speed(input, ratio);
    phase_vocoder_stretch(&shortened, input.len(), ratio, fft_size)
}

fn phase_vocoder_stretch(
    input: &[f32],
    target_len: usize,
    stretch: f64,
    fft_size: usize,
) -> Vec<f32> {
    if input.is_empty() || target_len == 0 {
        return Vec::new();
    }

    let nfft = fft_size.max(16);
    let analysis_hop = nfft / 4;
    let synthesis_hop = analysis_hop as f64 * stretch.max(0.01);
    let mut padded = vec![0.0f32; input.len() + 2 * nfft + analysis_hop];
    let start = nfft / 2;
    padded[start..start + input.len()].copy_from_slice(input);

    let frame_count = (input.len() + analysis_hop - 1) / analysis_hop + 1;
    let output_capacity = target_len.saturating_add(nfft * 2);
    let mut accumulation = vec![0.0f64; output_capacity];
    let mut normalization = vec![0.0f64; output_capacity];
    let window: Vec<f32> = (0..nfft)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / nfft as f64).cos() as f32)
        .collect();

    let mut planner = FftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(nfft);
    let inverse = planner.plan_fft_inverse(nfft);
    let mut previous_phase = vec![0.0f64; nfft / 2 + 1];
    let mut accumulated_phase = vec![0.0f64; nfft / 2 + 1];
    let mut spectrum = vec![Complex::<f32>::new(0.0, 0.0); nfft];

    for frame in 0..frame_count {
        let input_start = frame * analysis_hop;
        for i in 0..nfft {
            spectrum[i] = Complex::new(padded[input_start + i] * window[i], 0.0);
        }
        forward.process(&mut spectrum);

        let mut output_spectrum = vec![Complex::<f32>::new(0.0, 0.0); nfft];
        for bin in 0..=nfft / 2 {
            let re = spectrum[bin].re as f64;
            let im = spectrum[bin].im as f64;
            let magnitude = (re * re + im * im).sqrt();
            let phase = im.atan2(re);
            if frame == 0 {
                accumulated_phase[bin] = phase;
            } else {
                let expected = 2.0 * PI * bin as f64 * analysis_hop as f64 / nfft as f64;
                let delta =
                    (phase - previous_phase[bin] - expected + PI).rem_euclid(2.0 * PI) - PI;
                let true_frequency = 2.0 * PI * bin as f64 / nfft as f64
                    + delta / analysis_hop as f64;
                accumulated_phase[bin] += true_frequency * synthesis_hop;
            }
            previous_phase[bin] = phase;

            let value = Complex::new(
                (magnitude * accumulated_phase[bin].cos()) as f32,
                (magnitude * accumulated_phase[bin].sin()) as f32,
            );
            output_spectrum[bin] = value;
            if bin > 0 && bin < nfft / 2 {
                output_spectrum[nfft - bin] = value.conj();
            }
        }

        inverse.process(&mut output_spectrum);
        let output_start = (frame as f64 * synthesis_hop).round() as usize;
        for i in 0..nfft {
            let index = output_start + i;
            if index < output_capacity {
                let window_value = window[i] as f64;
                accumulation[index] +=
                    output_spectrum[i].re as f64 / nfft as f64 * window_value;
                normalization[index] += window_value * window_value;
            }
        }
    }

    let mut output = vec![0.0f32; target_len];
    for i in 0..target_len {
        let index = start + i;
        if index < accumulation.len() && normalization[index] > 1.0e-12 {
            output[i] = (accumulation[index] / normalization[index]) as f32;
        }
    }
    output
}

/// Returns one default instance of each effect in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![
        Box::new(Echo::default()),
        Box::new(Phaser::default()),
        Box::new(Wahwah::default()),
        Box::new(Distortion::default()),
        Box::new(Tremolo::default()),
        Box::new(Delay::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_buffer() -> AudioBuffer {
        let left: Vec<f32> = (0..512)
            .map(|i| (2.0 * PI * 440.0 * i as f64 / 44_100.0).sin() as f32)
            .collect();
        let right: Vec<f32> = (0..512)
            .map(|i| (2.0 * PI * 660.0 * i as f64 / 44_100.0).sin() as f32)
            .collect();
        AudioBuffer::from_channels(44_100, vec![left, right])
    }

    fn run(effect: &mut dyn BuiltinEffect, buffer: &mut AudioBuffer) {
        let mut context = EffectContext::default();
        effect.process(buffer, &mut context).unwrap();
    }

    #[test]
    fn defaults_empty_buffers_repeatability_and_clamping() {
        for mut effect in all() {
            let definitions = effect.params();
            let input = sine_buffer();

            let mut first = input.clone();
            run(effect.as_mut(), &mut first);

            let mut second = input.clone();
            run(effect.as_mut(), &mut second);
            assert_eq!(first.channels, second.channels, "{} repeatability", effect.id());

            let mut empty = AudioBuffer::from_channels(44_100, vec![vec![], vec![]]);
            run(effect.as_mut(), &mut empty);
            assert!(empty.channels.iter().all(Vec::is_empty), "{} empty buffer", effect.id());

            let definition = &definitions[0];
            assert!(set_param(effect.as_mut(), definition.key, f64::NEG_INFINITY));
            assert_eq!(
                effect.get(definition.key).unwrap(),
                definition.min,
                "{} lower clamp",
                effect.id()
            );
            assert!(set_param(effect.as_mut(), definition.key, f64::INFINITY));
            assert_eq!(
                effect.get(definition.key).unwrap(),
                definition.max,
                "{} upper clamp",
                effect.id()
            );
        }
    }

    #[test]
    fn echo_delays_an_impulse() {
        let mut effect = Echo::default();
        effect.delay = 0.01;
        let mut buffer = AudioBuffer::from_channels(1_000, vec![{
            let mut samples = vec![0.0; 32];
            samples[0] = 1.0;
            samples
        }]);
        run(&mut effect, &mut buffer);
        assert!((buffer.channels[0][10] - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn phaser_dry_mix_is_scaled_by_output_gain() {
        let mut effect = Phaser::default();
        effect.dry_wet = 0;
        let input = vec![0.25f32; 64];
        let mut buffer = AudioBuffer::from_channels(44_100, vec![input.clone()]);
        run(&mut effect, &mut buffer);
        let expected = 0.25 * 10.0_f64.powf(-6.0 / 20.0) as f32;
        assert!((buffer.channels[0][0] - expected).abs() < 1.0e-6);
    }

    #[test]
    fn wahwah_and_distortion_keep_silence_silent() {
        let mut silence = AudioBuffer::from_channels(44_100, vec![vec![0.0; 128]]);
        run(&mut Wahwah::default(), &mut silence);
        assert!(silence.channels[0].iter().all(|sample| *sample == 0.0));

        let mut silence = AudioBuffer::from_channels(44_100, vec![vec![0.0; 128]]);
        run(&mut Distortion::default(), &mut silence);
        assert!(silence.channels[0].iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn tremolo_modulates_and_delay_adds_echo() {
        let mut tremolo = Tremolo::default();
        tremolo.wave = 0;
        tremolo.phase = 0;
        tremolo.wet = 100;
        tremolo.lfo = 1.0;
        let mut buffer = AudioBuffer::from_channels(4, vec![vec![1.0; 4]]);
        run(&mut tremolo, &mut buffer);
        assert!(buffer.channels[0][0].abs() < 1.0e-6);
        assert!((buffer.channels[0][1] - 0.5).abs() < 1.0e-6 && (buffer.channels[0][2] - 1.0).abs() < 1.0e-6);

        let mut delay = Delay::default();
        delay.delay = 0.01;
        delay.number = 1;
        delay.dgain = 0.0;
        let mut buffer = AudioBuffer::from_channels(1_000, vec![{
            let mut samples = vec![0.0; 32];
            samples[0] = 1.0;
            samples
        }]);
        run(&mut delay, &mut buffer);
        assert!((buffer.channels[0][10] - 1.0).abs() < 1.0e-6);
    }
}
