use crate::core::*;
use au_core::AudioBuffer;
use std::collections::VecDeque;

const STEPS: usize = 1024;
const TABLE_SIZE: usize = 2049;
const SKIP_SAMPLES: usize = 1000;

/// The Audacity waveshaping distortion effect.
#[derive(Clone, Debug)]
pub struct Distortion {
    /// Distortion type, represented as an index into the Audacity choice list.
    pub distortion_type: f64,
    /// Whether to remove DC using the rolling-average filter.
    pub dc_block: bool,
    /// Clipping threshold in decibels.
    pub threshold_db: f64,
    /// Noise floor in decibels, used by the Leveller type.
    pub noise_floor: f64,
    /// First distortion parameter.
    pub parameter1: f64,
    /// Second distortion parameter.
    pub parameter2: f64,
    /// Number of repeats.
    pub repeats: u32,
    rt_states: Vec<DistortionState>,
}

impl Default for Distortion {
    fn default() -> Self {
        Self {
            distortion_type: 0.0,
            dc_block: false,
            threshold_db: -6.0,
            noise_floor: -70.0,
            parameter1: 50.0,
            parameter2: 50.0,
            repeats: 1,
            rt_states: Vec::new(),
        }
    }
}

impl Distortion {
    fn settings(&self) -> DistortionSettings {
        let table_choice = ParamDef::choice("Type", "Type", DISTORTION_TYPES, 0)
            .clamp(self.distortion_type) as usize;

        DistortionSettings {
            table_choice,
            dc_block: self.dc_block,
            threshold_db: ParamDef::float(
                "Threshold dB",
                "Threshold dB",
                -100.0,
                0.0,
                -6.0,
            )
            .clamp(self.threshold_db),
            noise_floor: ParamDef::float(
                "Noise Floor",
                "Noise Floor",
                -80.0,
                -20.0,
                -70.0,
            )
            .clamp(self.noise_floor),
            parameter1: ParamDef::float(
                "Parameter 1",
                "Parameter 1",
                0.0,
                100.0,
                50.0,
            )
            .clamp(self.parameter1),
            parameter2: ParamDef::float(
                "Parameter 2",
                "Parameter 2",
                0.0,
                100.0,
                50.0,
            )
            .clamp(self.parameter2),
            repeats: self.repeats.min(5) as usize,
        }
    }

    fn process_channel(
        samples: &mut [f32],
        settings: &DistortionSettings,
        state: &mut DistortionState,
    ) {
        // Parameter 2 is deliberately not copied into the cached state below:
        // InstanceProcess in Audacity compares it here but does not update it.
        let update = settings.table_choice != state.table_choice
            || settings.noise_floor != state.noise_floor
            || settings.threshold_db != state.threshold_db
            || settings.parameter1 != state.parameter1
            || settings.parameter2 != state.parameter2
            || settings.repeats != state.repeats;

        let p1 = settings.parameter1 / 100.0;
        let p2 = settings.parameter2 / 100.0;

        state.table_choice = settings.table_choice;
        state.threshold_db = settings.threshold_db;
        state.noise_floor = settings.noise_floor;
        state.parameter1 = settings.parameter1;
        state.repeats = settings.repeats;

        for sample in samples {
            if update && state.skip_count % SKIP_SAMPLES == 0 {
                make_table(state, settings);
            }
            if update {
                state.skip_count += 1;
            }

            let input = *sample;
            let shaped = wave_shaper(input, state, settings);
            let output = match settings.table_choice {
                0 | 1 => {
                    let gain = (1.0 - p2) + (state.makeup_gain * p2);
                    (shaped as f64 * gain) as f32
                }
                2 | 3 | 4 | 5 | 7 => (shaped as f64 * p2) as f32,
                6 | 8 | 9 => shaped,
                10 => ((shaped as f64 * (p1 - p2)) + (input as f64 * p2)) as f32,
                _ => shaped,
            };

            *sample = if settings.dc_block {
                dc_filter(state, output)
            } else {
                output
            };
        }
    }
}

impl BuiltinEffect for Distortion {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Distortion"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::choice("Type", "Type", DISTORTION_TYPES, 0),
            ParamDef::boolean("DC Block", "DC Block", false),
            ParamDef::float("Threshold dB", "Threshold dB", -100.0, 0.0, -6.0),
            ParamDef::float("Noise Floor", "Noise Floor", -80.0, -20.0, -70.0),
            ParamDef::float("Parameter 1", "Parameter 1", 0.0, 100.0, 50.0),
            ParamDef::float("Parameter 2", "Parameter 2", 0.0, 100.0, 50.0),
            ParamDef::int("Repeats", "Repeats", 0.0, 5.0, 1.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Type" => Some(self.distortion_type),
            "DC Block" => Some(if self.dc_block { 1.0 } else { 0.0 }),
            "Threshold dB" => Some(self.threshold_db),
            "Noise Floor" => Some(self.noise_floor),
            "Parameter 1" => Some(self.parameter1),
            "Parameter 2" => Some(self.parameter2),
            "Repeats" => Some(self.repeats as f64),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let Some(def) = self.params().into_iter().find(|param| param.key == key) else {
            return false;
        };
        let value = def.clamp(value);

        match key {
            "Type" => self.distortion_type = value,
            "DC Block" => self.dc_block = value != 0.0,
            "Threshold dB" => self.threshold_db = value,
            "Noise Floor" => self.noise_floor = value,
            "Parameter 1" => self.parameter1 = value,
            "Parameter 2" => self.parameter2 = value,
            "Repeats" => self.repeats = value as u32,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let settings = self.settings();
        for channel in &mut buf.channels {
            let mut state = DistortionState::new(buf.sample_rate as f32, &settings);
            Self::process_channel(channel, &settings, &mut state);
        }
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        let settings = self.settings();
        self.rt_states = (0..channels)
            .map(|_| DistortionState::new(sample_rate as f32, &settings))
            .collect();
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        let settings = self.settings();
        for (channel, state) in block.iter_mut().zip(self.rt_states.iter_mut()) {
            Self::process_channel(channel, &settings, state);
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

#[derive(Clone, Copy)]
struct DistortionSettings {
    table_choice: usize,
    dc_block: bool,
    threshold_db: f64,
    noise_floor: f64,
    parameter1: f64,
    parameter2: f64,
    repeats: usize,
}

#[derive(Clone, Debug)]
struct DistortionState {
    sample_rate: f32,
    skip_count: usize,
    table_choice: usize,
    threshold_db: f64,
    noise_floor: f64,
    parameter1: f64,
    parameter2: f64,
    repeats: usize,
    samples: VecDeque<f32>,
    queue_total: f64,
    makeup_gain: f64,
    table: [f64; TABLE_SIZE],
}

impl DistortionState {
    fn new(sample_rate: f32, settings: &DistortionSettings) -> Self {
        let mut state = Self {
            sample_rate,
            skip_count: 0,
            table_choice: settings.table_choice,
            threshold_db: settings.threshold_db,
            noise_floor: settings.noise_floor,
            parameter1: settings.parameter1,
            parameter2: settings.parameter2,
            repeats: settings.repeats,
            samples: VecDeque::new(),
            queue_total: 0.0,
            makeup_gain: 1.0,
            table: [0.0; TABLE_SIZE],
        };
        make_table(&mut state, settings);
        state
    }
}

fn db_to_linear(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

fn make_table(state: &mut DistortionState, settings: &DistortionSettings) {
    match settings.table_choice {
        0 => hard_clip(state, settings),
        1 => soft_clip(state, settings),
        2 => half_sin_table(&mut state.table, settings),
        3 => exponential_table(&mut state.table, settings),
        4 => logarithmic_table(&mut state.table, settings),
        5 => cubic_table(&mut state.table, settings),
        6 => even_harmonic_table(&mut state.table, settings),
        7 => sine_table(&mut state.table, settings),
        8 => leveller_table(&mut state.table, settings),
        9 => rectifier_table(&mut state.table, settings),
        10 => hard_limiter(state, settings),
        _ => {}
    }
}

fn hard_clip(state: &mut DistortionState, settings: &DistortionSettings) {
    let threshold = db_to_linear(settings.threshold_db);
    let low_threshold = 1.0 - threshold;
    let high_threshold = 1.0 + threshold;

    for n in 0..TABLE_SIZE {
        state.table[n] = if (n as f64) < STEPS as f64 * low_threshold {
            -threshold
        } else if (n as f64) > STEPS as f64 * high_threshold {
            threshold
        } else {
            n as f64 / STEPS as f64 - 1.0
        };
        state.makeup_gain = 1.0 / threshold;
    }
}

fn soft_clip(state: &mut DistortionState, settings: &DistortionSettings) {
    let threshold_linear = db_to_linear(settings.threshold_db);
    let threshold = 1.0 + threshold_linear;
    let amount = 2.0_f64.powf(7.0 * settings.parameter1 / 100.0);
    let peak = log_curve(threshold_linear, 1.0, amount);
    state.makeup_gain = 1.0 / peak as f64;
    state.table[STEPS] = 0.0;

    for n in STEPS..TABLE_SIZE {
        state.table[n] = if (n as f64) < STEPS as f64 * threshold {
            (n as f32 / STEPS as f32 - 1.0) as f64
        } else {
            log_curve(
                threshold_linear,
                (n as f64 / STEPS as f64 - 1.0) as f32,
                amount,
            ) as f64
        };
    }
    copy_half_table(&mut state.table);
}

fn log_curve(threshold: f64, value: f32, ratio: f64) -> f32 {
    (threshold + ((ratio * (threshold - value as f64)).exp() - 1.0) / -ratio) as f32
}

fn exponential_table(table: &mut [f64; TABLE_SIZE], settings: &DistortionSettings) {
    let amount = 0.999_f64.min(db_to_linear(-settings.parameter1));
    for n in STEPS..TABLE_SIZE {
        let linear_value = n as f32 / STEPS as f32;
        let scale = -1.0 / (1.0 - amount);
        let curve = (((linear_value as f64) - 1.0) * amount.ln()).exp();
        table[n] = scale * (curve - 1.0);
    }
    copy_half_table(table);
}

fn logarithmic_table(table: &mut [f64; TABLE_SIZE], settings: &DistortionSettings) {
    let amount = settings.parameter1;
    let step_size = 1.0 / STEPS as f64;
    let mut linear_value = 0.0;

    if amount == 0.0 {
        for value in table.iter_mut().skip(STEPS) {
            *value = linear_value;
            linear_value += step_size;
        }
    } else {
        for value in table.iter_mut().skip(STEPS) {
            *value = (1.0 + amount * linear_value).ln() / (1.0 + amount).ln();
            linear_value += step_size;
        }
    }
    copy_half_table(table);
}

fn half_sin_table(table: &mut [f64; TABLE_SIZE], settings: &DistortionSettings) {
    let iterations = (settings.parameter1 / 20.0).floor() as usize;
    let fractional = settings.parameter1 / 20.0 - iterations as f64;
    let step_size = 1.0 / STEPS as f64;
    let mut linear_value = 0.0;

    for n in STEPS..TABLE_SIZE {
        table[n] = linear_value;
        for _ in 0..iterations {
            table[n] = (table[n] * std::f64::consts::FRAC_PI_2).sin();
        }
        table[n] +=
            ((table[n] * std::f64::consts::FRAC_PI_2).sin() - table[n]) * fractional;
        linear_value += step_size;
    }
    copy_half_table(table);
}

fn cubic(x: f64, parameter1: f64) -> f64 {
    if parameter1 == 0.0 {
        x
    } else {
        x - x.powf(3.0) / 3.0
    }
}

fn cubic_table(table: &mut [f64; TABLE_SIZE], settings: &DistortionSettings) {
    let amount = settings.parameter1 * 3.0_f64.sqrt() / 100.0;
    let mut gain = 1.0;
    if amount != 0.0 {
        gain = 1.0 / cubic(amount.min(1.0), settings.parameter1);
    }

    let step_size = amount / STEPS as f64;
    let mut x = -amount;

    if amount == 0.0 {
        for (i, value) in table.iter_mut().enumerate() {
            *value = i as f64 / STEPS as f64 - 1.0;
        }
    } else {
        for value in table.iter_mut() {
            *value = gain * cubic(x, settings.parameter1);
            for _ in 0..settings.repeats {
                *value = gain * cubic(*value * amount, settings.parameter1);
            }
            x += step_size;
        }
    }
}

fn even_harmonic_table(table: &mut [f64; TABLE_SIZE], settings: &DistortionSettings) {
    let amount = settings.parameter1 / -100.0;
    let c = settings.parameter2.max(0.001) / 10.0;
    let step = 1.0 / STEPS as f64;
    let mut x = -1.0;

    for value in table.iter_mut() {
        *value = ((1.0 + amount) * x) - (x * (amount / c.tanh()) * (c * x).tanh());
        x += step;
    }
}

fn sine_table(table: &mut [f64; TABLE_SIZE], settings: &DistortionSettings) {
    let iterations = (settings.parameter1 / 20.0).floor() as usize;
    let fractional = settings.parameter1 / 20.0 - iterations as f64;
    let step_size = 1.0 / STEPS as f64;
    let mut linear_value = 0.0;

    for n in STEPS..TABLE_SIZE {
        table[n] = linear_value;
        for _ in 0..iterations {
            table[n] = (1.0
                + (table[n] * std::f64::consts::PI - std::f64::consts::FRAC_PI_2).sin())
                / 2.0;
        }
        table[n] += ((1.0
            + (table[n] * std::f64::consts::PI - std::f64::consts::FRAC_PI_2).sin())
            / 2.0
            - table[n])
            * fractional;
        linear_value += step_size;
    }
    copy_half_table(table);
}

fn leveller_table(table: &mut [f64; TABLE_SIZE], settings: &DistortionSettings) {
    let noise_floor = db_to_linear(settings.noise_floor);
    let gain_factors = [0.80, 1.00, 1.20, 1.20, 1.00, 0.80];
    let mut gain_limits = [0.0001, 0.0, 0.1, 0.3, 0.5, 1.0];
    let mut add_on_values = [0.0; 6];

    gain_limits[1] = noise_floor;
    for i in 0..5 {
        add_on_values[i + 1] =
            add_on_values[i] + gain_limits[i] * (gain_factors[i] - gain_factors[i + 1]);
    }

    let fractional_pass = settings.parameter1 / 100.0;
    for n in STEPS..TABLE_SIZE {
        table[n] = (n - STEPS) as f64 / STEPS as f64;

        for _ in 0..settings.repeats {
            let mut index = 5usize;
            let mut i = 5isize;
            while i >= 0 && table[n] < gain_limits[i as usize] {
                index = i as usize;
                i -= 1;
            }
            table[n] = table[n] * gain_factors[index] + add_on_values[index];
        }

        if fractional_pass > 0.001 {
            let mut index = 5usize;
            let mut i = 5isize;
            while i >= 0 && table[n] < gain_limits[i as usize] {
                index = i as usize;
                i -= 1;
            }
            table[n] += fractional_pass
                * (table[n] * (gain_factors[index] - 1.0) + add_on_values[index]);
        }
    }
    copy_half_table(table);
}

fn rectifier_table(table: &mut [f64; TABLE_SIZE], settings: &DistortionSettings) {
    let amount = settings.parameter1 / 50.0 - 1.0;
    let step_size = 1.0 / STEPS as f64;
    let mut index = STEPS;

    for n in 0..=STEPS {
        table[index] = n as f64 * step_size;
        index += 1;
    }

    index = STEPS - 1;
    for n in 1..=STEPS {
        table[index] = n as f64 * step_size * amount;
        index -= 1;
    }
}

fn hard_limiter(state: &mut DistortionState, settings: &DistortionSettings) {
    hard_clip(state, settings);
}

fn copy_half_table(table: &mut [f64; TABLE_SIZE]) {
    let mut count = TABLE_SIZE - 1;
    for n in 0..STEPS {
        table[n] = -table[count];
        count -= 1;
    }
}

fn wave_shaper(sample: f32, state: &DistortionState, settings: &DistortionSettings) -> f32 {
    let mut sample = sample;

    if settings.table_choice == 0 {
        let amount = settings.parameter1 / 100.0;
        sample = (sample as f64 * (1.0 + amount)) as f32;
    }

    let mut index = (sample * STEPS as f32).floor() as isize + STEPS as isize;
    index = index.clamp(0, (2 * STEPS - 1) as isize);
    let mut x_offset = ((1.0_f32 + sample) * STEPS as f32) as f64 - index as f64;
    x_offset = x_offset.clamp(0.0, 1.0);

    (state.table[index as usize]
        + (state.table[index as usize + 1] - state.table[index as usize]) * x_offset)
        as f32
}

fn dc_filter(state: &mut DistortionState, sample: f32) -> f32 {
    let queue_length = (state.sample_rate / 20.0).floor() as usize;

    state.queue_total += sample as f64;
    state.samples.push_back(sample);

    if state.samples.len() > queue_length {
        if let Some(oldest) = state.samples.pop_front() {
            state.queue_total -= oldest as f64;
        }
    }

    let average = state.queue_total / state.samples.len() as f64;
    (sample as f64 - average) as f32
}

/// Returns the built-in distortion effect.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(Distortion::default())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::set_param;

    fn sine_buffer() -> AudioBuffer {
        let left: Vec<f32> = (0..128)
            .map(|i| (i as f32 * 0.13).sin() * 0.8)
            .collect();
        let right: Vec<f32> = (0..128)
            .map(|i| (i as f32 * 0.17).sin() * 0.7)
            .collect();
        AudioBuffer::from_channels(44_100, vec![left, right])
    }

    #[test]
    fn default_processes_stereo_without_panicking() {
        let mut effect = Distortion::default();
        effect
            .process(&mut sine_buffer(), &mut EffectContext::default())
            .unwrap();
    }

    #[test]
    fn empty_buffer_does_not_panic() {
        let mut effect = Distortion::default();
        let mut buffer = AudioBuffer::from_channels(44_100, vec![Vec::new(), Vec::new()]);
        effect
            .process(&mut buffer, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn processing_is_repeatable() {
        let mut first = sine_buffer();
        let mut second = sine_buffer();
        Distortion::default()
            .process(&mut first, &mut EffectContext::default())
            .unwrap();
        Distortion::default()
            .process(&mut second, &mut EffectContext::default())
            .unwrap();
        assert_eq!(first.channels, second.channels);
    }

    #[test]
    fn parameters_are_clamped() {
        let mut effect = Distortion::default();
        assert!(set_param(&mut effect, "Type", 100.0));
        assert_eq!(effect.distortion_type, 10.0);
        assert!(set_param(&mut effect, "Threshold dB", -150.0));
        assert_eq!(effect.threshold_db, -100.0);
        assert!(set_param(&mut effect, "Repeats", 99.0));
        assert_eq!(effect.repeats, 5);
    }
}
