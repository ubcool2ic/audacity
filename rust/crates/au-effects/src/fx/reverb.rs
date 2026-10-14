//! Audacity-compatible implementation of the Reverb effect.

use crate::core::*;
use au_core::AudioBuffer;
use std::collections::VecDeque;

const COMB_LENGTHS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_LENGTHS: [usize; 4] = [225, 341, 441, 556];
const STEREO_ADJUST: f64 = 12.0;

#[derive(Clone)]
struct Filter {
    buffer: Vec<f32>,
    ptr: usize,
    store: f32,
}

impl Filter {
    fn new(size: usize) -> Self {
        let size = size.max(1);
        Self {
            buffer: vec![0.0; size],
            ptr: 0,
            store: 0.0,
        }
    }

    fn advance(&mut self) {
        if self.ptr == 0 {
            self.ptr = self.buffer.len() - 1;
        } else {
            self.ptr -= 1;
        }
    }

    fn comb_process(&mut self, input: f32, feedback: f32, hf_damping: f32) -> f32 {
        let output = self.buffer[self.ptr];
        self.store = output + (self.store - output) * hf_damping;
        self.buffer[self.ptr] = input + self.store * feedback;
        self.advance();
        output
    }

    fn allpass_process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.ptr];
        self.buffer[self.ptr] = (input as f64 + output as f64 * 0.5) as f32;
        self.advance();
        output - input
    }
}

#[derive(Clone, Copy, Default)]
struct OnePole {
    b0: f64,
    b1: f64,
    a1: f64,
    i1: f64,
    o1: f64,
}

impl OnePole {
    fn process(&mut self, input: f32) -> f32 {
        let output = (input as f64 * self.b0 + self.i1 * self.b1 - self.o1 * self.a1) as f32;
        self.i1 = input as f64;
        self.o1 = output as f64;
        output
    }
}

#[derive(Clone)]
struct FilterArray {
    comb: Vec<Filter>,
    allpass: Vec<Filter>,
    one_pole: [OnePole; 2],
}

impl FilterArray {
    fn new(sample_rate: f64, scale: f64, mut offset: f64) -> Self {
        let rate_ratio = sample_rate / 44100.0;
        let mut comb = Vec::with_capacity(COMB_LENGTHS.len());
        for &length in &COMB_LENGTHS {
            let size = (scale * rate_ratio * (length as f64 + STEREO_ADJUST * offset) + 0.5)
                as usize;
            comb.push(Filter::new(size));
            offset = -offset;
        }

        let mut allpass = Vec::with_capacity(ALLPASS_LENGTHS.len());
        for &length in &ALLPASS_LENGTHS {
            let size = (rate_ratio * (length as f64 + STEREO_ADJUST * offset) + 0.5) as usize;
            allpass.push(Filter::new(size));
            offset = -offset;
        }

        Self {
            comb,
            allpass,
            one_pole: [OnePole::default(); 2],
        }
    }

    fn init_tone_filters(&mut self, sample_rate: f64, tone_low: f64, tone_high: f64) {
        let fc_highpass = 440.0 * 2.0_f64.powf(((72.0 - tone_low / 100.0 * 48.0) - 69.0) / 12.0);
        let fc_lowpass = 440.0 * 2.0_f64.powf(((72.0 + tone_high / 100.0 * 48.0) - 69.0) / 12.0);

        let a1_high = -(-2.0 * std::f64::consts::PI * fc_highpass / sample_rate).exp();
        let b0_high = (1.0 - a1_high) / 2.0;
        self.one_pole[0] = OnePole {
            a1: a1_high,
            b0: b0_high,
            b1: -b0_high,
            i1: 0.0,
            o1: 0.0,
        };

        let a1_low = -(-2.0 * std::f64::consts::PI * fc_lowpass / sample_rate).exp();
        self.one_pole[1] = OnePole {
            a1: a1_low,
            b0: 1.0 + a1_low,
            b1: 0.0,
            i1: 0.0,
            o1: 0.0,
        };
    }

    fn process(&mut self, input: f32, feedback: f32, hf_damping: f32, gain: f32) -> f32 {
        let mut output = 0.0_f32;

        for index in (0..self.comb.len()).rev() {
            output += self.comb[index].comb_process(input, feedback, hf_damping);
        }
        for index in (0..self.allpass.len()).rev() {
            output = self.allpass[index].allpass_process(output);
        }

        output = self.one_pole[0].process(output);
        output = self.one_pole[1].process(output);
        output * gain
    }
}

struct ReverbCore {
    feedback: f32,
    hf_damping: f32,
    gain: f32,
    input_fifo: VecDeque<f32>,
    chan: [FilterArray; 2],
    initialized_with_zero_depth: bool,
}

impl ReverbCore {
    fn new(
        sample_rate: f64,
        wet_gain_db: f64,
        room_scale: f64,
        reverberance: f64,
        hf_damping: f64,
        pre_delay_ms: f64,
        stereo_depth: f64,
        tone_low: f64,
        tone_high: f64,
    ) -> Self {
        let mut result = Self {
            feedback: 0.0,
            hf_damping: 0.0,
            gain: 0.0,
            input_fifo: VecDeque::new(),
            chan: [
                FilterArray::new(sample_rate, 1.0, 0.0),
                FilterArray::new(sample_rate, 1.0, 1.0),
            ],
            initialized_with_zero_depth: false,
        };
        result.init(
            sample_rate,
            wet_gain_db,
            room_scale,
            reverberance,
            hf_damping,
            pre_delay_ms,
            stereo_depth,
            tone_low,
            tone_high,
        );
        result
    }

    fn set_simple_params(
        &mut self,
        sample_rate: f64,
        wet_gain_db: f64,
        reverberance: f64,
        hf_damping: f64,
        tone_low: f64,
        tone_high: f64,
    ) {
        let a = -1.0 / (1.0_f64 - 0.3).ln();
        let b = 100.0 / ((1.0_f64 - 0.98).ln() * a + 1.0);
        self.feedback = (1.0 - ((reverberance - b) / (a * b)).exp()) as f32;
        self.hf_damping = (hf_damping / 100.0 * 0.3 + 0.2) as f32;
        self.gain = (10.0_f64.powf(wet_gain_db * 0.05) * 0.015) as f32;

        self.chan[0].init_tone_filters(sample_rate, tone_low, tone_high);
        self.chan[1].init_tone_filters(sample_rate, tone_low, tone_high);
    }

    fn init(
        &mut self,
        sample_rate: f64,
        wet_gain_db: f64,
        room_scale: f64,
        reverberance: f64,
        hf_damping: f64,
        pre_delay_ms: f64,
        stereo_depth: f64,
        tone_low: f64,
        tone_high: f64,
    ) {
        self.input_fifo.clear();
        let delay = (pre_delay_ms / 1000.0 * sample_rate + 0.5) as usize;
        self.input_fifo.resize(delay, 0.0);

        self.set_simple_params(
            sample_rate,
            wet_gain_db,
            reverberance,
            hf_damping,
            tone_low,
            tone_high,
        );

        let scale = room_scale / 100.0 * 0.9 + 0.1;
        let depth = stereo_depth / 100.0;
        for index in 0..=(depth.ceil() as usize) {
            self.chan[index] = FilterArray::new(sample_rate, scale, index as f64 * depth);
            self.chan[index].init_tone_filters(sample_rate, tone_low, tone_high);
        }
        self.initialized_with_zero_depth = stereo_depth == 0.0;
    }

    fn process_sample(&mut self, input: f32) -> [f32; 2] {
        self.input_fifo.push_back(input);
        let delayed_input = self.input_fifo.pop_front().unwrap_or(0.0);

        let wet0 = self.chan[0].process(
            delayed_input,
            self.feedback,
            self.hf_damping,
            self.gain,
        );
        let wet1 = if self.initialized_with_zero_depth {
            0.0
        } else {
            self.chan[1].process(
                delayed_input,
                self.feedback,
                self.hf_damping,
                self.gain,
            )
        };
        [wet0, wet1]
    }
}

/// Audacity's Schroeder-style reverb effect.
pub struct Reverb {
    /// Room size, in percent.
    pub room_size: f64,
    /// Pre-delay, in milliseconds.
    pub pre_delay: f64,
    /// Reverberance, in percent.
    pub reverberance: f64,
    /// High-frequency damping, in percent.
    pub hf_damping: f64,
    /// Low tone control, in percent.
    pub tone_low: f64,
    /// High tone control, in percent.
    pub tone_high: f64,
    /// Wet signal gain, in decibels.
    pub wet_gain: f64,
    /// Dry signal gain, in decibels.
    pub dry_gain: f64,
    /// Stereo width, in percent.
    pub stereo_width: f64,
    /// Whether to output only the wet signal.
    pub wet_only: bool,
    cores: Vec<ReverbCore>,
}

impl Default for Reverb {
    fn default() -> Self {
        Self {
            room_size: 75.0,
            pre_delay: 10.0,
            reverberance: 50.0,
            hf_damping: 50.0,
            tone_low: 100.0,
            tone_high: 100.0,
            wet_gain: -1.0,
            dry_gain: -1.0,
            stereo_width: 100.0,
            wet_only: false,
            cores: Vec::new(),
        }
    }
}

impl Reverb {
    fn set_value(&mut self, key: &str, value: f64) -> bool {
        let (slot, def) = match key {
            "RoomSize" => (&mut self.room_size, ParamDef::float("RoomSize", "", 0.0, 100.0, 75.0)),
            "Delay" => (&mut self.pre_delay, ParamDef::float("Delay", "", 0.0, 200.0, 10.0)),
            "Reverberance" => (
                &mut self.reverberance,
                ParamDef::float("Reverberance", "", 0.0, 100.0, 50.0),
            ),
            "HfDamping" => (&mut self.hf_damping, ParamDef::float("HfDamping", "", 0.0, 100.0, 50.0)),
            "ToneLow" => (&mut self.tone_low, ParamDef::float("ToneLow", "", 0.0, 100.0, 100.0)),
            "ToneHigh" => (&mut self.tone_high, ParamDef::float("ToneHigh", "", 0.0, 100.0, 100.0)),
            "WetGain" => (&mut self.wet_gain, ParamDef::float("WetGain", "", -20.0, 10.0, -1.0)),
            "DryGain" => (&mut self.dry_gain, ParamDef::float("DryGain", "", -20.0, 10.0, -1.0)),
            "StereoWidth" => (
                &mut self.stereo_width,
                ParamDef::float("StereoWidth", "", 0.0, 100.0, 100.0),
            ),
            "WetOnly" => {
                self.wet_only = ParamDef::boolean("WetOnly", "", false).clamp(value) != 0.0;
                return true;
            }
            _ => return false,
        };
        *slot = def.clamp(value);
        true
    }

    fn make_core(&self, sample_rate: u32, stereo: bool) -> ReverbCore {
        ReverbCore::new(
            sample_rate as f64,
            self.wet_gain,
            self.room_size,
            self.reverberance,
            self.hf_damping,
            self.pre_delay,
            if stereo { self.stereo_width } else { 0.0 },
            self.tone_low,
            self.tone_high,
        )
    }
}

impl BuiltinEffect for Reverb {
    fn id(&self) -> &'static str {
        "Reverb"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("RoomSize", "Room Size (%)", 0.0, 100.0, 75.0),
            ParamDef::float("Delay", "Pre-delay (ms)", 0.0, 200.0, 10.0),
            ParamDef::float("Reverberance", "Reverberance (%)", 0.0, 100.0, 50.0),
            ParamDef::float("HfDamping", "Damping (%)", 0.0, 100.0, 50.0),
            ParamDef::float("ToneLow", "Tone Low (%)", 0.0, 100.0, 100.0),
            ParamDef::float("ToneHigh", "Tone High (%)", 0.0, 100.0, 100.0),
            ParamDef::float("WetGain", "Wet Gain (dB)", -20.0, 10.0, -1.0),
            ParamDef::float("DryGain", "Dry Gain (dB)", -20.0, 10.0, -1.0),
            ParamDef::float("StereoWidth", "Stereo Width (%)", 0.0, 100.0, 100.0),
            ParamDef::boolean("WetOnly", "Wet Only", false),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "RoomSize" => Some(self.room_size),
            "Delay" => Some(self.pre_delay),
            "Reverberance" => Some(self.reverberance),
            "HfDamping" => Some(self.hf_damping),
            "ToneLow" => Some(self.tone_low),
            "ToneHigh" => Some(self.tone_high),
            "WetGain" => Some(self.wet_gain),
            "DryGain" => Some(self.dry_gain),
            "StereoWidth" => Some(self.stereo_width),
            "WetOnly" => Some(if self.wet_only { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        self.set_value(key, value)
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
        self.cores.clear();
        let stereo = channels == 2;
        let cores: Vec<_> = (0..channels).map(|_| self.make_core(sample_rate, stereo)).collect();
        self.cores = cores;
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        if block.is_empty() {
            return;
        }
        let frames = block[0].len();
        let dry_gain = if self.wet_only {
            0.0_f32
        } else {
            10.0_f64.powf(self.dry_gain * 0.05) as f32
        };

        for frame in 0..frames {
            let mut wet = Vec::with_capacity(block.len());
            for (channel, samples) in block.iter().enumerate() {
                if let Some(core) = self.cores.get_mut(channel) {
                    wet.push(core.process_sample(samples[frame]));
                } else {
                    wet.push([0.0, 0.0]);
                }
            }

            if block.len() == 2 {
                for channel in 0..2 {
                    let output = dry_gain * block[channel][frame]
                        + (0.5_f64
                            * (wet[0][channel] as f64 + wet[1][channel] as f64))
                            as f32;
                    block[channel][frame] = output;
                }
            } else {
                for channel in 0..block.len() {
                    block[channel][frame] =
                        dry_gain * block[channel][frame] + wet[channel][0];
                }
            }
        }
    }
}

/// Returns the built-in effects implemented in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(Reverb::default())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{set_param, EffectContext};

    fn sine_buffer(sample_rate: u32, frames: usize) -> AudioBuffer {
        let left: Vec<f32> = (0..frames)
            .map(|i| (i as f32 * 0.071).sin() * 0.4)
            .collect();
        let right: Vec<f32> = (0..frames)
            .map(|i| (i as f32 * 0.053).sin() * 0.3)
            .collect();
        AudioBuffer::from_channels(sample_rate, vec![left, right])
    }

    #[test]
    fn default_params_process_without_panic() {
        let mut effect = Reverb::default();
        let mut buffer = sine_buffer(44100, 256);
        effect
            .process(&mut buffer, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buffer.len(), 256);
        assert!(buffer.channels.iter().flatten().all(|sample| sample.is_finite()));
    }

    #[test]
    fn empty_buffer_does_not_panic() {
        let mut effect = Reverb::default();
        let mut buffer = AudioBuffer::from_channels(44100, vec![vec![], vec![]]);
        effect
            .process(&mut buffer, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn processing_is_repeatable() {
        let input = sine_buffer(44100, 2048);
        let mut first = input.clone();
        let mut second = input;
        Reverb::default()
            .process(&mut first, &mut EffectContext::default())
            .unwrap();
        Reverb::default()
            .process(&mut second, &mut EffectContext::default())
            .unwrap();
        assert_eq!(first.channels, second.channels);
    }

    #[test]
    fn parameter_boundaries_are_clamped() {
        let mut effect = Reverb::default();
        assert!(set_param(&mut effect, "RoomSize", -100.0));
        assert_eq!(effect.room_size, 0.0);
        assert!(set_param(&mut effect, "Delay", 1000.0));
        assert_eq!(effect.pre_delay, 200.0);
        assert!(set_param(&mut effect, "WetOnly", 2.0));
        assert!(effect.wet_only);
        assert!(!set_param(&mut effect, "Unknown", 1.0));
    }

    #[test]
    fn dry_path_and_wet_only_are_sane() {
        let mut dry = Reverb::default();
        dry.pre_delay = 0.0;
        let input = sine_buffer(44100, 64);
        let mut output = input.clone();
        dry.process(&mut output, &mut EffectContext::default())
            .unwrap();

        let expected_gain = 10.0_f32.powf(-1.0 / 20.0);
        for channel in 0..2 {
            for frame in 0..64 {
                let expected = input.channels[channel][frame] * expected_gain;
                assert!((output.channels[channel][frame] - expected).abs() < 1e-6);
            }
        }

        let mut wet_only = Reverb::default();
        wet_only.wet_only = true;
        let mut silent = sine_buffer(44100, 64);
        wet_only
            .process(&mut silent, &mut EffectContext::default())
            .unwrap();
        assert!(silent.channels.iter().flatten().all(|sample| sample.abs() < 1e-7));
    }
}
