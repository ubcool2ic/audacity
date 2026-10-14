#![allow(dead_code)]
use crate::core::*;
use au_core::AudioBuffer;

/// Audacity's Compressor dynamic-range processor.
pub struct Compressor {
    /// Compression threshold in dB.
    pub threshold_db: f64,
    /// Makeup gain in dB.
    pub makeup_gain_db: f64,
    /// Soft-knee width in dB.
    pub knee_width_db: f64,
    /// Compression ratio.
    pub compression_ratio: f64,
    /// Lookahead time in milliseconds.
    pub lookahead_ms: f64,
    /// Attack time in milliseconds.
    pub attack_ms: f64,
    /// Release time in milliseconds.
    pub release_ms: f64,
    processor: DynamicsProcessor,
}

impl Default for Compressor {
    fn default() -> Self {
        Self {
            threshold_db: -12.0,
            makeup_gain_db: 9.0,
            knee_width_db: 6.0,
            compression_ratio: 4.0,
            lookahead_ms: 3.0,
            attack_ms: 3.0,
            release_ms: 100.0,
            processor: DynamicsProcessor::default(),
        }
    }
}

/// Audacity's Limiter dynamic-range processor.
pub struct Limiter {
    /// Limiter threshold in dB.
    pub threshold_db: f64,
    /// Target output level in dB.
    pub makeup_target_db: f64,
    /// Soft-knee width in dB.
    pub knee_width_db: f64,
    /// Lookahead time in milliseconds.
    pub lookahead_ms: f64,
    /// Release time in milliseconds.
    pub release_ms: f64,
    processor: DynamicsProcessor,
}

impl Default for Limiter {
    fn default() -> Self {
        Self {
            threshold_db: -6.0,
            makeup_target_db: -1.0,
            knee_width_db: 2.0,
            lookahead_ms: 1.0,
            release_ms: 20.0,
            processor: DynamicsProcessor::default(),
        }
    }
}

/// Audacity's offline Noise Gate effect.
pub struct NoiseGate {
    /// Selects Gate or Analyze mode.
    pub mode: usize,
    /// Selects linked or independent stereo detection.
    pub stereo_link: usize,
    /// Gate threshold in dB.
    pub threshold: f64,
    /// Frequency above which the gate detector operates, in kHz.
    pub gate_freq: f64,
    /// Level reduction below the threshold, in dB.
    pub level_reduction: f64,
    /// Attack time in milliseconds.
    pub attack: f64,
    /// Hold time in milliseconds.
    pub hold: f64,
    /// Decay time in milliseconds.
    pub decay: f64,
}

impl Default for NoiseGate {
    fn default() -> Self {
        Self {
            mode: 0,
            stereo_link: 0,
            threshold: -40.0,
            gate_freq: 0.0,
            level_reduction: -24.0,
            attack: 10.0,
            hold: 50.0,
            decay: 100.0,
        }
    }
}

const COMPRESSOR_PARAMS: &[ParamDef] = &[
    ParamDef::float("thresholdDb", "Threshold (dB)", -60.0, 0.0, -12.0),
    ParamDef::float("makeupGainDb", "Makeup gain (dB)", -20.0, 20.0, 9.0),
    ParamDef::float("kneeWidthDb", "Knee width (dB)", 0.0, 20.0, 6.0),
    ParamDef::float("compressionRatio", "Compression ratio", 1.0, 20.0, 4.0),
    ParamDef::float("lookaheadMs", "Lookahead (ms)", 0.0, 100.0, 3.0),
    ParamDef::float("attackMs", "Attack (ms)", 0.0, 1000.0, 3.0),
    ParamDef::float("releaseMs", "Release (ms)", 0.0, 5000.0, 100.0),
];

const LIMITER_PARAMS: &[ParamDef] = &[
    ParamDef::float("thresholdDb", "Threshold (dB)", -20.0, 0.0, -6.0),
    ParamDef::float("makeupTargetDb", "Makeup target (dB)", -20.0, 0.0, -1.0),
    ParamDef::float("kneeWidthDb", "Knee width (dB)", 0.0, 20.0, 2.0),
    ParamDef::float("lookaheadMs", "Lookahead (ms)", 0.0, 50.0, 1.0),
    ParamDef::float("releaseMs", "Release (ms)", 0.0, 5000.0, 20.0),
];

const NOISE_GATE_PARAMS: &[ParamDef] = &[
    ParamDef::choice("MODE", "Select Function", &["Gate", "Analyze"], 0),
    ParamDef::choice(
        "STEREO-LINK",
        "Stereo Linking",
        &["LinkStereo", "DoNotLink"],
        0,
    ),
    ParamDef::float("THRESHOLD", "Gate threshold (dB)", -96.0, -6.0, -40.0),
    ParamDef::float("GATE-FREQ", "Gate frequencies above (kHz)", 0.0, 10.0, 0.0),
    ParamDef::float("LEVEL-REDUCTION", "Level reduction (dB)", -100.0, 0.0, -24.0),
    ParamDef::float("ATTACK", "Attack (ms)", 1.0, 1000.0, 10.0),
    ParamDef::float("HOLD", "Hold (ms)", 0.0, 2000.0, 50.0),
    ParamDef::float("DECAY", "Decay (ms)", 10.0, 4000.0, 100.0),
];

fn read_param(params: &[ParamDef], key: &str, values: &[f64]) -> Option<f64> {
    params
        .iter()
        .position(|p| p.key == key)
        .and_then(|i| values.get(i).copied())
}

fn write_param(params: &[ParamDef], key: &str, value: f64, values: &mut [f64]) -> bool {
    if let Some(i) = params.iter().position(|p| p.key == key) {
        if let Some(slot) = values.get_mut(i) {
            *slot = value;
            return true;
        }
    }
    false
}

impl BuiltinEffect for Compressor {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Compressor"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        COMPRESSOR_PARAMS.to_vec()
    }

    fn get(&self, key: &str) -> Option<f64> {
        read_param(
            COMPRESSOR_PARAMS,
            key,
            &[
                self.threshold_db,
                self.makeup_gain_db,
                self.knee_width_db,
                self.compression_ratio,
                self.lookahead_ms,
                self.attack_ms,
                self.release_ms,
            ],
        )
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "thresholdDb" => self.threshold_db = value,
            "makeupGainDb" => self.makeup_gain_db = value,
            "kneeWidthDb" => self.knee_width_db = value,
            "compressionRatio" => self.compression_ratio = value,
            "lookaheadMs" => self.lookahead_ms = value,
            "attackMs" => self.attack_ms = value,
            "releaseMs" => self.release_ms = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        process_dynamics_offline(self, buf)
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        self.processor.reset(
            sample_rate,
            channels,
            self.threshold_db,
            self.threshold_db + self.makeup_gain_db,
            self.knee_width_db,
            self.compression_ratio,
            self.lookahead_ms,
            self.attack_ms,
            self.release_ms,
        );
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        self.processor.process(block);
    }

    fn rt_latency(&self) -> usize {
        self.processor.delay_samples
    }
}

impl BuiltinEffect for Limiter {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Limiter"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        LIMITER_PARAMS.to_vec()
    }

    fn get(&self, key: &str) -> Option<f64> {
        read_param(
            LIMITER_PARAMS,
            key,
            &[
                self.threshold_db,
                self.makeup_target_db,
                self.knee_width_db,
                self.lookahead_ms,
                self.release_ms,
            ],
        )
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "thresholdDb" => self.threshold_db = value,
            "makeupTargetDb" => self.makeup_target_db = value,
            "kneeWidthDb" => self.knee_width_db = value,
            "lookaheadMs" => self.lookahead_ms = value,
            "releaseMs" => self.release_ms = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        process_dynamics_offline(self, buf)
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        self.processor.reset(
            sample_rate,
            channels,
            self.threshold_db,
            self.makeup_target_db,
            self.knee_width_db,
            f64::INFINITY,
            self.lookahead_ms,
            0.0,
            self.release_ms,
        );
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        self.processor.process(block);
    }

    fn rt_latency(&self) -> usize {
        self.processor.delay_samples
    }
}

impl BuiltinEffect for NoiseGate {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "NoiseGate"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        NOISE_GATE_PARAMS.to_vec()
    }

    fn get(&self, key: &str) -> Option<f64> {
        read_param(
            NOISE_GATE_PARAMS,
            key,
            &[
                self.mode as f64,
                self.stereo_link as f64,
                self.threshold,
                self.gate_freq,
                self.level_reduction,
                self.attack,
                self.hold,
                self.decay,
            ],
        )
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "MODE" => self.mode = value.round() as usize,
            "STEREO-LINK" => self.stereo_link = value.round() as usize,
            "THRESHOLD" => self.threshold = value,
            "GATE-FREQ" => self.gate_freq = value,
            "LEVEL-REDUCTION" => self.level_reduction = value,
            "ATTACK" => self.attack = value,
            "HOLD" => self.hold = value,
            "DECAY" => self.decay = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if self.mode != 0 {
            let frames = buf.len().min((buf.sample_rate as usize) / 2);
            let peak = buf
                .channels
                .iter()
                .filter_map(|c| c.iter().take(frames).map(|x| x.abs()).reduce(f32::max))
                .fold(0.0f32, f32::max);
            let peak_db = if peak == 0.0 {
                f32::NEG_INFINITY
            } else {
                20.0 * peak.log10()
            };
            ctx.report = format!(
                "Peak based on first {:.2} seconds {:.2} dB\nSuggested Threshold Setting {:.0} dB.",
                frames as f64 / buf.sample_rate.max(1) as f64,
                peak_db,
                peak_db + 1.0
            );
            return Ok(());
        }

        if buf.len() == 0 || buf.channels.is_empty() {
            return Ok(());
        }
        if buf.len() < 100 {
            return Err(EffectError::Invalid(
                "Insufficient audio selected; select at least 100 samples.".to_string(),
            ));
        }

        let sr = buf.sample_rate as f32;
        let frequency = self.gate_freq as f32 * 1000.0;
        if frequency > 20.0 && frequency >= sr * 0.45 {
            return Err(EffectError::Invalid(
                "Gate frequency is too high for the selected sample rate.".to_string(),
            ));
        }

        apply_noise_gate(self, buf);
        Ok(())
    }
}

impl NoiseGate {
    fn params_values(&self) -> [f64; 8] {
        [
            self.mode as f64,
            self.stereo_link as f64,
            self.threshold,
            self.gate_freq,
            self.level_reduction,
            self.attack,
            self.hold,
            self.decay,
        ]
    }
}

fn process_dynamics_offline(
    effect: &mut dyn BuiltinEffect,
    buf: &mut AudioBuffer,
) -> Result<(), EffectError> {
    let frames = buf.len();
    if frames == 0 || buf.channels.is_empty() {
        process_via_realtime(effect, buf);
        return Ok(());
    }

    effect.rt_reset(buf.sample_rate, buf.channels.len());
    let latency = effect.rt_latency();
    let mut channels: Vec<Vec<f32>> = buf
        .channels
        .iter()
        .map(|channel| {
            let mut extended = channel.clone();
            extended.resize(extended.len().saturating_add(latency), 0.0);
            extended
        })
        .collect();

    let mut extended = AudioBuffer::from_channels(buf.sample_rate, std::mem::take(&mut channels));
    process_via_realtime(effect, &mut extended);

    for (dest, src) in buf.channels.iter_mut().zip(extended.channels.iter()) {
        if src.len() >= latency.saturating_add(frames) && dest.len() >= frames {
            dest[..frames].copy_from_slice(&src[latency..latency + frames]);
        }
    }
    Ok(())
}

#[derive(Default)]
struct DynamicsProcessor {
    sample_rate: u32,
    channels: usize,
    delay_samples: usize,
    threshold_db: f32,
    makeup_db: f32,
    knee_db: f32,
    slope: f32,
    alpha_attack: f32,
    alpha_release: f32,
    gain_state: f32,
    lookahead: Vec<f32>,
    lookahead_write: usize,
    last_pushed: usize,
    delayed_input: Vec<Vec<f32>>,
}

impl DynamicsProcessor {
    fn reset(
        &mut self,
        sample_rate: u32,
        channels: usize,
        threshold_db: f64,
        output_threshold_db: f64,
        knee_db: f64,
        ratio: f64,
        lookahead_ms: f64,
        attack_ms: f64,
        release_ms: f64,
    ) {
        self.sample_rate = sample_rate;
        self.channels = channels;
        self.threshold_db = threshold_db as f32;
        self.makeup_db = (output_threshold_db - threshold_db) as f32;
        self.knee_db = knee_db as f32;
        self.slope = (1.0 / ratio as f32) - 1.0;

        let sr = sample_rate as f32;
        let time_to_gain = |seconds: f32| -> f32 {
            (-1.0 / (sr * seconds)).exp()
        };
        self.alpha_attack = 1.0 - time_to_gain(attack_ms as f32 / 1000.0);
        self.alpha_release = 1.0 - time_to_gain(release_ms as f32 / 1000.0);

        self.delay_samples = ((lookahead_ms / 1000.0) * sample_rate as f64) as usize;
        self.gain_state = 0.0;
        self.lookahead_write = 0;
        self.last_pushed = 0;

        let ring_len = self.delay_samples.saturating_add(512).max(1);
        self.lookahead = vec![0.0; ring_len];
        self.delayed_input = (0..channels)
            .map(|_| vec![0.0; self.delay_samples.saturating_add(512)])
            .collect();
    }

    fn process(&mut self, block: &mut [Vec<f32>]) {
        if self.sample_rate == 0 || self.channels == 0 || block.is_empty() {
            return;
        }

        let block_len = block.iter().map(Vec::len).min().unwrap_or(0);
        let mut processed = 0;
        while processed < block_len {
            let n = (block_len - processed).min(512);
            let mut envelope = vec![0.0f32; n];

            for (i, level) in envelope.iter_mut().enumerate() {
                let mut max_sample = 0.0f32;
                for channel in block.iter().take(self.channels) {
                    max_sample = max_sample.max(channel[processed + i].abs());
                }
                let level_db = 6.020_599_913_f32 * max_sample.log2();
                let overshoot = level_db - self.threshold_db;
                let reduction = characteristic(overshoot, self.knee_db, self.slope);

                let diff = reduction - self.gain_state;
                if diff < 0.0 {
                    self.gain_state += self.alpha_attack * diff;
                } else {
                    self.gain_state += self.alpha_release * diff;
                }
                *level = self.gain_state;
            }

            if self.delay_samples > 0 {
                self.lookahead_push_process_read(&mut envelope);
            }

            let delay = self.delay_samples;
            for channel_index in 0..self.channels.min(block.len()) {
                let input = &mut self.delayed_input[channel_index];
                let channel = &mut block[channel_index];

                for i in 0..n {
                    input[delay + i] = channel[processed + i];
                }
                for i in 0..n {
                    let sample = input[i];
                    channel[processed + i] =
                        sample * (10.0f32).powf(0.05 * (envelope[i] + self.makeup_db));
                }

                input.copy_within(n..n + delay, 0);
            }

            processed += n;
        }
    }

    fn lookahead_push_process_read(&mut self, envelope: &mut [f32]) {
        let len = self.lookahead.len();
        if len == 0 {
            return;
        }

        let n = envelope.len();
        for (i, value) in envelope.iter().copied().enumerate() {
            self.lookahead[(self.lookahead_write + i) % len] = value;
        }
        self.lookahead_write = (self.lookahead_write + n) % len;
        self.last_pushed = n;

        let mut next_value = 0.0f32;
        let mut step = 0.0f32;
        let mut index = (self.lookahead_write + len - 1) % len;

        for _ in 0..n {
            let sample = self.lookahead[index];
            if sample > next_value {
                self.lookahead[index] = next_value;
                next_value += step;
            } else {
                step = if self.delay_samples == 0 {
                    0.0
                } else {
                    -sample / self.delay_samples as f32
                };
                next_value = sample + step;
            }
            index = (index + len - 1) % len;
        }

        for _ in 0..self.delay_samples {
            let sample = self.lookahead[index];
            if sample > next_value {
                self.lookahead[index] = next_value;
                next_value += step;
            } else {
                break;
            }
            index = (index + len - 1) % len;
        }

        let start = (self.lookahead_write + len - self.last_pushed % len
            + len
            - self.delay_samples % len)
            % len;
        for (i, out) in envelope.iter_mut().enumerate() {
            *out = self.lookahead[(start + i) % len];
        }
    }
}

fn characteristic(overshoot: f32, knee: f32, slope: f32) -> f32 {
    let knee_half = knee / 2.0;
    if overshoot <= -knee_half {
        0.0
    } else if overshoot <= knee_half && knee != 0.0 {
        0.5 * slope * (overshoot + knee_half) * (overshoot + knee_half) / knee
    } else {
        slope * overshoot
    }
}

#[derive(Clone, Copy)]
struct BiquadCoefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

#[derive(Default, Clone, Copy)]
struct BiquadState {
    z1: f32,
    z2: f32,
}

fn butterworth_coefficients(cutoff: f32, sample_rate: f32, highpass: bool) -> [BiquadCoefficients; 4] {
    let q_values = [
        0.509_795_6f32,
        0.601_344_9,
        0.900_968_85,
        2.562_915,
    ];
    let safe_cutoff = cutoff.clamp(1.0, sample_rate * 0.499);
    let omega = std::f32::consts::TAU * safe_cutoff / sample_rate.max(1.0);
    let cos_omega = omega.cos();
    let sin_omega = omega.sin();

    std::array::from_fn(|i| {
        let alpha = sin_omega / (2.0 * q_values[i]);
        let (b0, b1, b2) = if highpass {
            (
                (1.0 + cos_omega) / 2.0,
                -(1.0 + cos_omega),
                (1.0 + cos_omega) / 2.0,
            )
        } else {
            (
                (1.0 - cos_omega) / 2.0,
                1.0 - cos_omega,
                (1.0 - cos_omega) / 2.0,
            )
        };
        let a0 = 1.0 + alpha;
        BiquadCoefficients {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: -2.0 * cos_omega / a0,
            a2: (1.0 - alpha) / a0,
        }
    })
}

fn filter_sample(
    mut sample: f32,
    coeffs: &[BiquadCoefficients; 4],
    states: &mut [BiquadState; 4],
) -> f32 {
    for (coeff, state) in coeffs.iter().zip(states.iter_mut()) {
        let output = coeff.b0 * sample + state.z1;
        state.z1 = coeff.b1 * sample - coeff.a1 * output + state.z2;
        state.z2 = coeff.b2 * sample - coeff.a2 * output;
        sample = output;
    }
    sample
}

fn apply_noise_gate(effect: &NoiseGate, buf: &mut AudioBuffer) {
    let frames = buf.len();
    if frames == 0 || buf.channels.is_empty() {
        return;
    }

    let sample_rate = buf.sample_rate.max(1) as f32;
    let threshold = 10.0f32.powf(effect.threshold as f32 / 20.0);
    let floor = 10.0f32.powf(effect.level_reduction as f32 / 20.0);
    let silence_flag = if effect.level_reduction > -96.0 {
        0.0
    } else {
        1.0
    };
    let hold_samples = ((effect.hold as f32 / 1000.0) * sample_rate) as usize;
    let attack_samples = ((effect.attack as f32 / 1000.0) * sample_rate).max(1.0) as usize;
    let attack_seconds = (effect.attack as f32 / 1000.0).max(1.0 / sample_rate);
    let decay_seconds = (effect.decay as f32 / 1000.0).max(1.0 / sample_rate);
    let attack_alpha = (-1.0 / (attack_seconds * sample_rate)).exp();
    let decay_alpha = (-1.0 / (decay_seconds * sample_rate)).exp();

    let channels = buf.channels.len();
    let mut detector = vec![vec![0.0f32; frames]; channels];
    for (ch, channel) in buf.channels.iter().enumerate() {
        for i in 0..frames.min(channel.len()) {
            detector[ch][i] = channel[i].abs();
        }
    }

    let linked = channels > 1 && effect.stereo_link == 0;
    let mut follows = if linked {
        let mut mono = vec![0.0f32; frames];
        for i in 0..frames {
            mono[i] = detector
                .iter()
                .map(|channel| channel.get(i).copied().unwrap_or(0.0))
                .fold(0.0, f32::max);
        }
        vec![mono]
    } else {
        detector
    };

    let mut open = vec![vec![false; frames]; follows.len()];
    for (ch, follow) in follows.iter_mut().enumerate() {
        let mut remaining_hold = 0usize;
        for i in 0..frames {
            if follow[i] > threshold {
                remaining_hold = hold_samples;
                open[ch][i] = true;
            } else if remaining_hold > 0 {
                open[ch][i] = true;
                remaining_hold -= 1;
            }
        }
    }

    let mut envelopes = vec![vec![floor; frames]; follows.len()];
    for ch in 0..follows.len() {
        let mut gain_envelope = floor;
        for i in 0..frames {
            let future_open_distance = (0..=attack_samples.min(frames - 1 - i))
                .find(|distance| open[ch][i + *distance]);
            let target = if open[ch][i] {
                1.0
            } else if let Some(distance) = future_open_distance {
                1.0 - distance as f32 / attack_samples.max(1) as f32
            } else {
                floor
            };

            if target > gain_envelope {
                if open[ch][i] {
                    gain_envelope = 1.0;
                } else {
                    gain_envelope = target + attack_alpha * (gain_envelope - target);
                    gain_envelope = gain_envelope.max(target);
                }
            } else {
                gain_envelope = target + decay_alpha * (gain_envelope - target);
            }
            envelopes[ch][i] = gain_envelope.min(1.0);
        }
    }

    let gate_gain = 1.0 - silence_flag * floor;
    let frequency = effect.gate_freq as f32 * 1000.0;
    let split_frequency = frequency > 20.0;
    let low_coeffs = if split_frequency {
        Some(butterworth_coefficients(frequency * 0.91, sample_rate, false))
    } else {
        None
    };
    let high_coeffs = if split_frequency {
        Some(butterworth_coefficients(frequency, sample_rate, true))
    } else {
        None
    };

    let mut low_states = vec![[BiquadState::default(); 4]; channels];
    let mut high_states = vec![[BiquadState::default(); 4]; channels];

    for ch in 0..channels {
        let env_index = if linked { 0 } else { ch };
        for i in 0..frames.min(buf.channels[ch].len()) {
            let gate_env = envelopes[env_index][i].min(1.0);
            let gain = gate_gain * (gate_env - silence_flag * floor);
            if let (Some(low), Some(high)) = (&low_coeffs, &high_coeffs) {
                let sample = buf.channels[ch][i];
                let low_sample = filter_sample(sample, low, &mut low_states[ch]);
                let high_sample = filter_sample(sample, high, &mut high_states[ch]);
                buf.channels[ch][i] = low_sample + high_sample * gain;
            } else {
                buf.channels[ch][i] *= gain;
            }
        }
    }

    follows.clear();
}

/// Returns one default instance of each effect implemented in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![
        Box::new(Compressor::default()),
        Box::new(Limiter::default()),
        Box::new(NoiseGate::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::set_param;

    fn sine_buffer(frames: usize) -> AudioBuffer {
        let sample_rate = 48_000;
        let left: Vec<f32> = (0..frames)
            .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / sample_rate as f32).sin() * 0.5)
            .collect();
        let right: Vec<f32> = left.iter().map(|x| -*x * 0.8).collect();
        AudioBuffer::from_channels(sample_rate, vec![left, right])
    }

    fn same_buffer(a: &AudioBuffer, b: &AudioBuffer) -> bool {
        a.sample_rate == b.sample_rate
            && a.channels.len() == b.channels.len()
            && a.channels.iter().zip(&b.channels).all(|(x, y)| x == y)
    }

    #[test]
    fn defaults_run_repeatably_and_handle_empty_input() {
        for mut effect in all() {
            let mut first = sine_buffer(1024);
            let mut second = sine_buffer(1024);
            let mut ctx1 = EffectContext::default();
            let mut ctx2 = EffectContext::default();
            let _ = effect.process(&mut first, &mut ctx1);
            let _ = effect.process(&mut second, &mut ctx2);
            assert!(same_buffer(&first, &second), "{}", effect.id());

            let mut empty = AudioBuffer::from_channels(48_000, vec![vec![], vec![]]);
            let _ = effect.process(&mut empty, &mut EffectContext::default());
        }
    }

    #[test]
    fn parameter_values_clamp_to_declared_boundaries() {
        for mut effect in all() {
            let params = effect.params();
            assert!(!params.is_empty());
            let first = &params[0];
            assert!(set_param(&mut *effect, first.key, f64::NEG_INFINITY));
            assert_eq!(effect.get(first.key), Some(first.min));
            assert!(set_param(&mut *effect, first.key, f64::INFINITY));
            assert_eq!(effect.get(first.key), Some(first.max));
        }
    }

    #[test]
    fn dynamics_effects_preserve_silence() {
        for mut effect in [Box::new(Compressor::default()) as Box<dyn BuiltinEffect>, Box::new(Limiter::default())] {
            let mut silence =
                AudioBuffer::from_channels(48_000, vec![vec![0.0; 1024], vec![0.0; 1024]]);
            effect
                .process(&mut silence, &mut EffectContext::default())
                .unwrap();
            assert!(silence.channels.iter().flatten().all(|x| *x == 0.0));
        }
    }

    #[test]
    fn noise_gate_reduces_a_quiet_signal() {
        let mut gate = NoiseGate::default();
        gate.threshold = -20.0;
        gate.level_reduction = -40.0;
        let mut buf = AudioBuffer::from_channels(
            48_000,
            vec![vec![0.001; 1024], vec![0.001; 1024]],
        );
        gate.process(&mut buf, &mut EffectContext::default()).unwrap();
        assert!(buf.channels[0].iter().all(|x| x.abs() <= 0.001));
        assert!(buf.channels[1].iter().all(|x| x.abs() <= 0.001));
    }
}
