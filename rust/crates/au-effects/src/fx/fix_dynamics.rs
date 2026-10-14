//! Audacity 3.7.3 compressor and limiter.
//!
//! Both effects use one linked detector for all channels. The processor runs
//! in blocks of at most 512 frames; offline output is aligned by discarding
//! the lookahead latency and processing zero input to flush the delayed audio.

use crate::core::*;
use au_core::AudioBuffer;

const MAX_BLOCK_SIZE: usize = 512;
const LOG2_TO_DB: f32 = 20.0_f32 / 3.321_928_094_887_362_f32;

/// Compressor settings.
#[derive(Clone, Debug)]
pub struct Compressor {
    /// Input threshold in dB.
    pub threshold_db: f64,
    /// Make-up gain in dB.
    pub makeup_gain_db: f64,
    /// Soft-knee width in dB.
    pub knee_width_db: f64,
    /// Compression ratio.
    pub compression_ratio: f64,
    /// Lookahead in milliseconds.
    pub lookahead_ms: f64,
    /// Attack in milliseconds.
    pub attack_ms: f64,
    /// Release in milliseconds.
    pub release_ms: f64,
    rt_processor: Option<CompressorProcessor>,
    rt_sample_rate: u32,
}

impl Default for Compressor {
    fn default() -> Self {
        Self {
            threshold_db: -10.0,
            makeup_gain_db: 0.0,
            knee_width_db: 5.0,
            compression_ratio: 10.0,
            lookahead_ms: 1.0,
            attack_ms: 30.0,
            release_ms: 150.0,
            rt_processor: None,
            rt_sample_rate: 0,
        }
    }
}

impl BuiltinEffect for Compressor {
    fn id(&self) -> &'static str {
        "Compressor"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("thresholdDb", "Threshold (dB)", -60.0, 0.0, -10.0),
            ParamDef::float("makeupGainDb", "Make-up gain (dB)", 0.0, 30.0, 0.0),
            ParamDef::float("kneeWidthDb", "Knee width (dB)", 0.0, 40.0, 5.0),
            ParamDef::float("compressionRatio", "Ratio", 1.0, 100.0, 10.0),
            ParamDef::float("lookaheadMs", "Lookahead (ms)", 0.0, 1000.0, 1.0),
            ParamDef::float("attackMs", "Attack (ms)", 0.0, 1000.0, 30.0),
            ParamDef::float("releaseMs", "Release (ms)", 0.0, 1000.0, 150.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "thresholdDb" => Some(self.threshold_db),
            "makeupGainDb" => Some(self.makeup_gain_db),
            "kneeWidthDb" => Some(self.knee_width_db),
            "compressionRatio" => Some(self.compression_ratio),
            "lookaheadMs" => Some(self.lookahead_ms),
            "attackMs" => Some(self.attack_ms),
            "releaseMs" => Some(self.release_ms),
            _ => None,
        }
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
        process_audio(buf, DynamicSettings::from_compressor(self))
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        self.rt_sample_rate = sample_rate;
        self.rt_processor = make_realtime_processor(
            DynamicSettings::from_compressor(self),
            sample_rate,
            channels,
        );
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        let settings = DynamicSettings::from_compressor(self);
        rt_process_audio(&mut self.rt_processor, block, settings);
    }

    fn rt_latency(&self) -> usize {
        reported_latency(self.lookahead_ms, self.rt_sample_rate)
    }
}

/// Limiter settings.
#[derive(Clone, Debug)]
pub struct Limiter {
    /// Input threshold in dB.
    pub threshold_db: f64,
    /// Output target in dB.
    pub makeup_target_db: f64,
    /// Soft-knee width in dB.
    pub knee_width_db: f64,
    /// Lookahead in milliseconds.
    pub lookahead_ms: f64,
    /// Release in milliseconds.
    pub release_ms: f64,
    rt_processor: Option<CompressorProcessor>,
    rt_sample_rate: u32,
}

impl Default for Limiter {
    fn default() -> Self {
        Self {
            threshold_db: -5.0,
            makeup_target_db: -1.0,
            knee_width_db: 2.0,
            lookahead_ms: 1.0,
            release_ms: 20.0,
            rt_processor: None,
            rt_sample_rate: 0,
        }
    }
}

impl BuiltinEffect for Limiter {
    fn id(&self) -> &'static str {
        "Limiter"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("thresholdDb", "Threshold (dB)", -20.0, 0.0, -5.0),
            ParamDef::float("makeupTargetDb", "Make-up target (dB)", -20.0, 0.0, -1.0),
            ParamDef::float("kneeWidthDb", "Knee width (dB)", 0.0, 10.0, 2.0),
            ParamDef::float("lookaheadMs", "Lookahead (ms)", 0.0, 50.0, 1.0),
            ParamDef::float("releaseMs", "Release (ms)", 0.0, 1000.0, 20.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "thresholdDb" => Some(self.threshold_db),
            "makeupTargetDb" => Some(self.makeup_target_db),
            "kneeWidthDb" => Some(self.knee_width_db),
            "lookaheadMs" => Some(self.lookahead_ms),
            "releaseMs" => Some(self.release_ms),
            _ => None,
        }
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
        process_audio(buf, DynamicSettings::from_limiter(self))
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        self.rt_sample_rate = sample_rate;
        self.rt_processor = make_realtime_processor(
            DynamicSettings::from_limiter(self),
            sample_rate,
            channels,
        );
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        let settings = DynamicSettings::from_limiter(self);
        rt_process_audio(&mut self.rt_processor, block, settings);
    }

    fn rt_latency(&self) -> usize {
        reported_latency(self.lookahead_ms, self.rt_sample_rate)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DynamicSettings {
    in_threshold_db: f64,
    out_threshold_db: f64,
    knee_width_db: f64,
    ratio: f64,
    lookahead_ms: f64,
    attack_ms: f64,
    release_ms: f64,
}

impl DynamicSettings {
    fn from_compressor(settings: &Compressor) -> Self {
        Self {
            in_threshold_db: settings.threshold_db,
            out_threshold_db: settings.threshold_db + settings.makeup_gain_db,
            knee_width_db: settings.knee_width_db,
            ratio: settings.compression_ratio,
            lookahead_ms: settings.lookahead_ms,
            attack_ms: settings.attack_ms,
            release_ms: settings.release_ms,
        }
    }

    fn from_limiter(settings: &Limiter) -> Self {
        Self {
            in_threshold_db: settings.threshold_db,
            out_threshold_db: settings.makeup_target_db,
            knee_width_db: settings.knee_width_db,
            ratio: f64::INFINITY,
            lookahead_ms: settings.lookahead_ms,
            attack_ms: 0.0,
            release_ms: settings.release_ms,
        }
    }
}

fn process_audio(buf: &mut AudioBuffer, settings: DynamicSettings) -> Result<(), EffectError> {
    let frames = buf.len();
    let channels = buf.channels.len();
    if frames == 0 || channels == 0 || buf.sample_rate == 0 {
        return Ok(());
    }

    // CompressorInstance constructs its processor with compressor defaults,
    // applies the effect settings, and then initializes it.
    let defaults = DynamicSettings::from_compressor(&Compressor::default());
    let mut processor = CompressorProcessor::new(defaults);
    processor.apply_settings_if_needed(settings);
    processor.init(buf.sample_rate as usize, channels, MAX_BLOCK_SIZE);

    let latency = processor.lookahead.delay_in_samples;
    let total = frames.saturating_add(latency);
    let input: Vec<Vec<f32>> = buf
        .channels
        .iter()
        .map(|channel| {
            let mut padded = vec![0.0; total];
            let count = frames.min(channel.len());
            padded[..count].copy_from_slice(&channel[..count]);
            padded
        })
        .collect();
    let mut output = vec![vec![0.0; total]; channels];

    // Keep the final input block and the flush as separate calls, as they
    // are separate ProcessBlock calls in the offline path.
    let mut position = 0;
    while position < frames {
        let count = (frames - position).min(MAX_BLOCK_SIZE);
        processor.process_block(&input, &mut output, position, count);
        position += count;
    }
    while position < total {
        let count = (total - position).min(MAX_BLOCK_SIZE);
        processor.process_block(&input, &mut output, position, count);
        position += count;
    }

    for (channel, processed) in buf.channels.iter_mut().zip(output) {
        *channel = processed[latency..latency + frames].to_vec();
    }
    Ok(())
}

fn make_realtime_processor(
    settings: DynamicSettings,
    sample_rate: u32,
    channels: usize,
) -> Option<CompressorProcessor> {
    if sample_rate == 0 || channels == 0 {
        return None;
    }

    let defaults = DynamicSettings::from_compressor(&Compressor::default());
    let mut processor = CompressorProcessor::new(defaults);
    processor.apply_settings_if_needed(settings);
    processor.init(sample_rate as usize, channels, MAX_BLOCK_SIZE);
    Some(processor)
}

fn rt_process_audio(
    processor: &mut Option<CompressorProcessor>,
    block: &mut [Vec<f32>],
    settings: DynamicSettings,
) {
    let Some(processor) = processor.as_mut() else {
        return;
    };

    processor.apply_settings_if_needed(settings);
    let channels = processor.num_channels;
    if channels == 0 || block.len() < channels {
        return;
    }

    let frames = block[..channels]
        .iter()
        .map(Vec::len)
        .min()
        .unwrap_or(0);
    let mut position = 0;

    while position < frames {
        let count = (frames - position).min(MAX_BLOCK_SIZE);
        let input: Vec<Vec<f32>> = block[..channels]
            .iter()
            .map(|channel| channel[position..position + count].to_vec())
            .collect();
        let mut output = vec![vec![0.0; count]; channels];

        processor.process_block(&input, &mut output, 0, count);

        for (channel, processed) in block[..channels].iter_mut().zip(output) {
            channel[position..position + count].copy_from_slice(&processed);
        }
        position += count;
    }
}

fn reported_latency(lookahead_ms: f64, sample_rate: u32) -> usize {
    if sample_rate == 0 {
        0
    } else {
        (lookahead_ms * sample_rate as f64 / 1000.0) as usize
    }
}

#[derive(Clone, Debug)]
struct GainReductionComputer {
    sample_rate: f64,
    knee: f32,
    threshold: f32,
    attack_time: f32,
    release_time: f32,
    slope: f32,
    makeup_gain: f32,
    state: f32,
    alpha_attack: f32,
    alpha_release: f32,
}

impl GainReductionComputer {
    fn new() -> Self {
        Self {
            sample_rate: 0.0,
            knee: 0.0,
            threshold: -10.0,
            attack_time: 0.01,
            release_time: 0.15,
            slope: 1.0_f32 / 2.0 - 1.0,
            makeup_gain: 0.0,
            state: 0.0,
            alpha_attack: 0.0,
            alpha_release: 0.0,
        }
    }

    fn time_to_gain(&self, seconds: f32) -> f32 {
        (-1.0_f32 / (self.sample_rate as f32 * seconds)).exp()
    }

    fn prepare(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        self.alpha_attack = 1.0 - self.time_to_gain(self.attack_time);
        self.alpha_release = 1.0 - self.time_to_gain(self.release_time);
    }

    fn set_attack_time(&mut self, seconds: f32) {
        self.attack_time = seconds;
        self.alpha_attack = 1.0 - self.time_to_gain(seconds);
    }

    fn set_release_time(&mut self, seconds: f32) {
        self.release_time = seconds;
        self.alpha_release = 1.0 - self.time_to_gain(seconds);
    }

    fn characteristic_overshoot(overshoot: f32, knee: f32, slope: f32) -> f32 {
        let knee_half = knee / 2.0;
        if overshoot <= -knee_half {
            0.0
        } else if overshoot > -knee_half && overshoot <= knee_half {
            0.5 * slope * (overshoot + knee_half) * (overshoot + knee_half) / knee
        } else {
            slope * overshoot
        }
    }

    fn compute_gain_in_decibels(&mut self, samples: &mut [f32]) {
        for sample in samples {
            let level_db = LOG2_TO_DB * fast_log2(sample.abs());
            let overshoot = level_db - self.threshold;
            let reduction = Self::characteristic_overshoot(overshoot, self.knee, self.slope);
            let diff = reduction - self.state;
            if diff < 0.0 {
                self.state += self.alpha_attack * diff;
            } else {
                self.state += self.alpha_release * diff;
            }
            *sample = self.state;
        }
    }
}

#[derive(Clone, Debug)]
struct LookAheadGainReduction {
    sample_rate: f64,
    block_size: usize,
    delay: f32,
    delay_in_samples: usize,
    write_position: usize,
    last_pushed_samples: usize,
    buffer: Vec<f32>,
}

impl LookAheadGainReduction {
    fn new() -> Self {
        Self {
            sample_rate: 0.0,
            block_size: 0,
            delay: 0.0,
            delay_in_samples: 0,
            write_position: 0,
            last_pushed_samples: 0,
            buffer: Vec::new(),
        }
    }

    fn set_delay_time(&mut self, seconds: f32) {
        self.delay = if seconds <= 0.0 { 0.0 } else { seconds };
        if self.sample_rate != 0.0 {
            self.prepare(self.sample_rate, self.block_size);
        }
    }

    fn prepare(&mut self, sample_rate: f64, block_size: usize) {
        self.sample_rate = sample_rate;
        self.block_size = block_size;
        self.delay_in_samples = (self.delay as f64 * sample_rate) as usize;
        self.buffer = vec![0.0; block_size + self.delay_in_samples];
        self.write_position = 0;
    }

    fn push_samples(&mut self, source: &[f32]) {
        let length = self.buffer.len();
        let start = self.write_position;
        let first = (length - start).min(source.len());
        self.buffer[start..start + first].copy_from_slice(&source[..first]);
        if first < source.len() {
            self.buffer[..source.len() - first].copy_from_slice(&source[first..]);
        }
        self.write_position = (self.write_position + source.len()) % length;
        self.last_pushed_samples = source.len();
    }

    fn process(&mut self) {
        let mut next_gain_reduction_value = 0.0_f32;
        let mut step = 0.0_f32;
        let length = self.buffer.len();

        let mut index = self.write_position as isize - 1;
        if index < 0 {
            index += length as isize;
        }

        let (first, second) = process_positions(index as usize, self.last_pushed_samples);
        for _ in 0..first {
            let sample = self.buffer[index as usize];
            if sample > next_gain_reduction_value {
                self.buffer[index as usize] = next_gain_reduction_value;
                next_gain_reduction_value += step;
            } else {
                step = -sample / self.delay_in_samples as f32;
                next_gain_reduction_value = sample + step;
            }
            index -= 1;
        }

        if second > 0 {
            index = length as isize - 1;
            for _ in 0..second {
                let sample = self.buffer[index as usize];
                if sample > next_gain_reduction_value {
                    self.buffer[index as usize] = next_gain_reduction_value;
                    next_gain_reduction_value += step;
                } else {
                    step = -sample / self.delay_in_samples as f32;
                    next_gain_reduction_value = sample + step;
                }
                index -= 1;
            }
        }

        if index < 0 {
            index = length as isize - 1;
        }

        let (first, second) = process_positions(index as usize, self.delay_in_samples);
        let mut broke = false;
        for _ in 0..first {
            let sample = self.buffer[index as usize];
            if sample > next_gain_reduction_value {
                self.buffer[index as usize] = next_gain_reduction_value;
                next_gain_reduction_value += step;
            } else {
                broke = true;
                break;
            }
            index -= 1;
        }

        if !broke && second > 0 {
            index = length as isize - 1;
            for _ in 0..second {
                let sample = self.buffer[index as usize];
                if sample > next_gain_reduction_value {
                    self.buffer[index as usize] = next_gain_reduction_value;
                    next_gain_reduction_value += step;
                } else {
                    break;
                }
                index -= 1;
            }
        }
    }

    fn read_samples(&self, destination: &mut [f32]) {
        let length = self.buffer.len();
        let position = self.write_position as isize
            - self.last_pushed_samples as isize
            - self.delay_in_samples as isize;
        let start = position.rem_euclid(length as isize) as usize;
        let first = (length - start).min(destination.len());
        destination[..first].copy_from_slice(&self.buffer[start..start + first]);
        if first < destination.len() {
            let rest = destination.len() - first;
            destination[first..].copy_from_slice(&self.buffer[..rest]);
        }
    }
}

fn process_positions(start: usize, count: usize) -> (usize, usize) {
    let first = (start + 1).min(count);
    (first, count - first)
}

#[derive(Clone, Debug)]
struct CompressorProcessor {
    gain_reduction: GainReductionComputer,
    lookahead: LookAheadGainReduction,
    settings: DynamicSettings,
    sample_rate: usize,
    num_channels: usize,
    block_size: usize,
    envelope: Vec<f32>,
    delayed_input: Vec<Vec<f32>>,
}

impl CompressorProcessor {
    fn new(settings: DynamicSettings) -> Self {
        Self {
            gain_reduction: GainReductionComputer::new(),
            lookahead: LookAheadGainReduction::new(),
            settings,
            sample_rate: 0,
            num_channels: 0,
            block_size: 0,
            envelope: vec![0.0; MAX_BLOCK_SIZE],
            delayed_input: Vec::new(),
        }
    }

    fn apply_settings_if_needed(&mut self, settings: DynamicSettings) {
        if settings == self.settings {
            return;
        }

        let reinit_lookahead = settings.lookahead_ms != self.settings.lookahead_ms;
        self.settings = settings;
        self.gain_reduction.threshold = settings.in_threshold_db as f32;
        self.gain_reduction.knee = settings.knee_width_db as f32;
        self.gain_reduction
            .set_attack_time((settings.attack_ms / 1000.0) as f32);
        self.gain_reduction
            .set_release_time((settings.release_ms / 1000.0) as f32);
        self.gain_reduction.slope = 1.0_f32 / settings.ratio as f32 - 1.0;
        self.gain_reduction.makeup_gain =
            (settings.out_threshold_db - settings.in_threshold_db) as f32;

        if reinit_lookahead {
            self.lookahead
                .set_delay_time((settings.lookahead_ms / 1000.0) as f32);
            self.reinit();
        }
    }

    fn init(&mut self, sample_rate: usize, num_channels: usize, block_size: usize) {
        self.sample_rate = sample_rate;
        self.num_channels = num_channels;
        self.block_size = block_size.min(MAX_BLOCK_SIZE);
        self.reinit();
    }

    fn reinit(&mut self) {
        if self.sample_rate == 0 || self.num_channels == 0 || self.block_size == 0 {
            return;
        }

        self.gain_reduction.prepare(self.sample_rate as f64);
        self.lookahead
            .set_delay_time((self.settings.lookahead_ms / 1000.0) as f32);
        self.lookahead
            .prepare(self.sample_rate as f64, self.block_size);

        let length = self.lookahead.delay_in_samples + self.block_size;
        self.delayed_input = vec![vec![0.0; length]; self.num_channels];
        self.envelope.fill(0.0);
    }

    fn process_block(
        &mut self,
        input: &[Vec<f32>],
        output: &mut [Vec<f32>],
        start: usize,
        block_len: usize,
    ) {
        let mut processed = 0;
        while processed < block_len {
            let count = (block_len - processed).min(self.block_size);
            let offset = start + processed;

            for frame in 0..count {
                let mut maximum = 0.0_f32;
                for channel in input.iter().take(self.num_channels) {
                    let sample = channel[offset + frame].abs();
                    if sample > maximum {
                        maximum = sample;
                    }
                }
                self.envelope[frame] = maximum;
            }

            self.gain_reduction
                .compute_gain_in_decibels(&mut self.envelope[..count]);

            if self.settings.lookahead_ms > 0.0 {
                self.lookahead.push_samples(&self.envelope[..count]);
                self.lookahead.process();
                self.lookahead.read_samples(&mut self.envelope[..count]);
            }

            let delay = self.lookahead.delay_in_samples;
            for (channel, delayed) in input
                .iter()
                .zip(self.delayed_input.iter_mut())
                .take(self.num_channels)
            {
                delayed[delay..delay + count].copy_from_slice(&channel[offset..offset + count]);
            }

            let makeup_gain = self.gain_reduction.makeup_gain;
            for (delayed, out) in self
                .delayed_input
                .iter_mut()
                .zip(output.iter_mut())
                .take(self.num_channels)
            {
                for frame in 0..count {
                    out[offset + frame] = delayed[frame]
                        * 10.0_f32.powf(0.05_f32 * (self.envelope[frame] + makeup_gain));
                }
                delayed.copy_within(count..count + delay, 0);
            }

            processed += count;
        }
    }
}

// MathApprox.h::FastLog2. Its polynomial and exponent adjustment are
// intentionally approximate; replacing them with f32::log2 changes the
// detector's gain-reduction values.
fn fast_log2(value: f32) -> f32 {
    let mut bits = value.to_bits() as i32;
    let mut log_2 = (((bits >> 23) & 255) - 128) as f32;
    bits &= !(255_i32 << 23);
    bits += 127_i32 << 23;
    let mantissa = f32::from_bits(bits as u32);
    log_2 += ((-0.335_828_781_1_f32) * mantissa + 2.0_f32) * mantissa
        - 0.658_717_593_166_67_f32;
    log_2
}

/// Returns the compressor and limiter.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(Compressor::default()), Box::new(Limiter::default())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{set_param, EffectContext};

    fn stereo_sine() -> AudioBuffer {
        let mut left = Vec::new();
        let mut right = Vec::new();
        for frame in 0..1024 {
            let phase = frame as f32 * 0.071;
            left.push(0.8 * phase.sin());
            right.push(0.6 * (phase + 0.4).sin());
        }
        AudioBuffer::from_channels(48_000, vec![left, right])
    }

    #[test]
    fn defaults_process_short_stereo_without_panic() {
        let mut context = EffectContext::default();
        for mut effect in all() {
            let mut audio = stereo_sine();
            effect.process(&mut audio, &mut context).unwrap();
            assert_eq!(audio.channels.len(), 2);
            assert_eq!(audio.len(), 1024);
            assert!(audio.channels.iter().flatten().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn empty_buffer_does_not_panic() {
        let mut audio = AudioBuffer::from_channels(48_000, vec![vec![], vec![]]);
        let mut context = EffectContext::default();
        for mut effect in all() {
            effect.process(&mut audio, &mut context).unwrap();
        }
        assert_eq!(audio.len(), 0);
    }

    #[test]
    fn processing_is_repeatable() {
        let mut context = EffectContext::default();
        for mut effect in all() {
            let mut first = stereo_sine();
            let mut second = stereo_sine();
            effect.process(&mut first, &mut context).unwrap();
            effect.process(&mut second, &mut context).unwrap();
            assert_eq!(first.channels, second.channels);
        }
    }

    #[test]
    fn set_param_clamps_values() {
        let mut compressor = Compressor::default();
        assert!(set_param(&mut compressor, "thresholdDb", -100.0));
        assert_eq!(compressor.threshold_db, -60.0);
        assert!(set_param(&mut compressor, "compressionRatio", 1000.0));
        assert_eq!(compressor.compression_ratio, 100.0);

        let mut limiter = Limiter::default();
        assert!(set_param(&mut limiter, "lookaheadMs", 100.0));
        assert_eq!(limiter.lookahead_ms, 50.0);
    }

    #[test]
    fn fast_log2_uses_audacity_polynomial() {
        // exponent term (127 - 128) = -1, then the polynomial correction (MathApprox.h)
        let expected_at_one = -1.0_f32
            + (((-0.335_828_781_1_f32) * 1.0_f32 + 2.0_f32) * 1.0_f32
                - 0.658_717_593_166_67_f32);
        assert_eq!(fast_log2(1.0).to_bits(), expected_at_one.to_bits());
        assert_eq!(fast_log2(2.0).to_bits(), (1.0 + expected_at_one).to_bits());
        assert_ne!(fast_log2(1.0).to_bits(), 1.0_f32.log2().to_bits());
    }

    #[test]
    fn realtime_matches_offline_after_latency_with_uneven_blocks() {
        let frames = 3000;
        let sample_rate = 48_000;
        let mut left = Vec::with_capacity(frames);
        let mut right = Vec::with_capacity(frames);
        for frame in 0..frames {
            let phase = frame as f32 * 0.037;
            left.push(0.95 * phase.sin());
            right.push(0.72 * (phase + 0.31).sin());
        }

        let source = AudioBuffer::from_channels(sample_rate, vec![left, right]);
        let mut offline = source.clone();
        let mut context = EffectContext::default();
        let mut offline_effect = Compressor::default();
        offline_effect.process(&mut offline, &mut context).unwrap();

        let mut realtime_effect = Compressor::default();
        assert!(realtime_effect.supports_realtime());
        realtime_effect.rt_reset(sample_rate, 2);
        let latency = realtime_effect.rt_latency();

        let mut rendered = vec![Vec::new(), Vec::new()];
        let uneven_sizes = [113, 777, 257, 511, 333, 1009];
        let mut position = 0;
        let mut size_index = 0;
        while position < frames {
            let count = uneven_sizes[size_index % uneven_sizes.len()].min(frames - position);
            let mut block: Vec<Vec<f32>> = source
                .channels
                .iter()
                .map(|channel| channel[position..position + count].to_vec())
                .collect();
            realtime_effect.rt_process(&mut block);
            for (destination, processed) in rendered.iter_mut().zip(block) {
                destination.extend(processed);
            }
            position += count;
            size_index += 1;
        }

        let mut remaining = latency;
        while remaining > 0 {
            let count = remaining.min(uneven_sizes[size_index % uneven_sizes.len()]);
            let mut block = vec![vec![0.0; count]; 2];
            realtime_effect.rt_process(&mut block);
            for (destination, processed) in rendered.iter_mut().zip(block) {
                destination.extend(processed);
            }
            remaining -= count;
            size_index += 1;
        }

        for channel in 0..2 {
            let realtime_aligned = &rendered[channel][latency..latency + frames];
            for (actual, expected) in realtime_aligned.iter().zip(&offline.channels[channel]) {
                assert!(
                    (actual - expected).abs() < 1.0e-6,
                    "realtime/offline mismatch: {actual} != {expected}"
                );
            }
        }
    }
}
