#![allow(dead_code)]
// SPDX-License-Identifier: GPL-3.0-or-later
//! Simple built-in Audacity effects and Nyquist effect ports.

use crate::core::*;
use au_core::AudioBuffer;

const AMPLIFY_ALLOW_CLIPPING: &[&str] = &["false", "true"];
const ADJUSTABLE_FADE_TYPES: &[&str] = &["Up", "Down", "SCurveUp", "SCurveDown"];
const ADJUSTABLE_FADE_UNITS: &[&str] = &["Percent", "dB"];
const ADJUSTABLE_FADE_PRESETS: &[&str] = &[
    "None",
    "LinearIn",
    "LinearOut",
    "ExponentialIn",
    "ExponentialOut",
    "LogarithmicIn",
    "LogarithmicOut",
    "RoundedIn",
    "RoundedOut",
    "CosineIn",
    "CosineOut",
    "SCurveIn",
    "SCurveOut",
];
const CROSSFADE_TYPES: &[&str] = &[
    "ConstantGain",
    "ConstantPower1",
    "ConstantPower2",
    "CustomCurve",
];
const CROSSFADE_DIRECTIONS: &[&str] = &["Automatic", "OutIn", "InOut"];

fn parameter_value(params: &[ParamDef], key: &str, value: f64) -> Option<f64> {
    params
        .iter()
        .find(|param| param.key == key)
        .map(|param| param.clamp(value))
}

fn scale_db(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

fn channels_len(buf: &AudioBuffer) -> usize {
    buf.channels.first().map_or(0, Vec::len)
}

/// Audacity's Amplify effect.
#[derive(Clone, Debug)]
pub struct Amplify {
    /// Linear gain ratio.
    pub ratio: f64,
    /// Whether clipping is permitted by the effect UI.
    pub allow_clipping: bool,
}

impl Default for Amplify {
    fn default() -> Self {
        Self {
            ratio: 1.0,
            allow_clipping: false,
        }
    }
}

impl BuiltinEffect for Amplify {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Amplify"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Ratio", "Ratio", 0.0, 10.0, 1.0),
            ParamDef::boolean("AllowClipping", "Allow clipping", false),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Ratio" => Some(self.ratio),
            "AllowClipping" => Some(if self.allow_clipping { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let Some(value) = parameter_value(&self.params(), key, value) else {
            return false;
        };
        match key {
            "Ratio" => self.ratio = value,
            "AllowClipping" => self.allow_clipping = value != 0.0,
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

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        for channel in block {
            for sample in channel {
                *sample = (*sample as f64 * self.ratio) as f32;
            }
        }
    }
}

/// Audacity's Normalize effect.
#[derive(Clone, Debug)]
pub struct Normalize {
    /// Target peak level in dB.
    pub peak_level: f64,
    /// Whether to normalize the peak amplitude.
    pub apply_volume: bool,
    /// Whether to remove each channel's DC offset.
    pub remove_dc_offset: bool,
    /// Whether to normalize stereo channels independently.
    pub stereo_independent: bool,
}

impl Default for Normalize {
    fn default() -> Self {
        Self {
            peak_level: -1.0,
            apply_volume: true,
            remove_dc_offset: true,
            stereo_independent: false,
        }
    }
}

impl BuiltinEffect for Normalize {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Normalize"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("PeakLevel", "Peak level", -20.0, 0.0, -1.0),
            ParamDef::boolean("ApplyVolume", "Apply volume", true),
            ParamDef::boolean("RemoveDcOffset", "Remove DC offset", true),
            ParamDef::boolean("StereoIndependent", "Stereo channels independently", false),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "PeakLevel" => Some(self.peak_level),
            "ApplyVolume" => Some(if self.apply_volume { 1.0 } else { 0.0 }),
            "RemoveDcOffset" => Some(if self.remove_dc_offset { 1.0 } else { 0.0 }),
            "StereoIndependent" => Some(if self.stereo_independent { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let Some(value) = parameter_value(&self.params(), key, value) else {
            return false;
        };
        match key {
            "PeakLevel" => self.peak_level = value,
            "ApplyVolume" => self.apply_volume = value != 0.0,
            "RemoveDcOffset" => self.remove_dc_offset = value != 0.0,
            "StereoIndependent" => self.stereo_independent = value != 0.0,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if !self.apply_volume && !self.remove_dc_offset {
            return Ok(());
        }

        let mut offsets = Vec::with_capacity(buf.channels.len());
        let mut peaks = Vec::with_capacity(buf.channels.len());

        for channel in &buf.channels {
            let offset = if self.remove_dc_offset && !channel.is_empty() {
                -(channel.iter().map(|&sample| sample as f64).sum::<f64>()
                    / channel.len() as f64) as f32
            } else {
                0.0
            };
            let peak = channel.iter().fold(0.0_f32, |peak, &sample| {
                peak.max((sample + offset).abs())
            });
            offsets.push(offset);
            peaks.push(peak);
        }

        let shared_peak = peaks.iter().copied().fold(0.0_f32, f32::max);
        let target = scale_db(self.peak_level) as f32;

        for (index, channel) in buf.channels.iter_mut().enumerate() {
            let peak = if self.stereo_independent {
                peaks[index]
            } else {
                shared_peak
            };
            let multiplier = if self.apply_volume && peak > 0.0 {
                target / peak
            } else {
                1.0
            };
            let offset = offsets[index];
            for sample in channel {
                *sample = (*sample + offset) * multiplier;
            }
        }
        Ok(())
    }
}

/// Audacity's Invert effect.
#[derive(Clone, Debug, Default)]
pub struct Invert;

impl BuiltinEffect for Invert {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Invert"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        Vec::new()
    }

    fn get(&self, _key: &str) -> Option<f64> {
        None
    }

    fn set(&mut self, _key: &str, _value: f64) -> bool {
        false
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

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        for channel in block {
            for sample in channel {
                *sample = -*sample;
            }
        }
    }
}

/// Audacity's Reverse effect.
#[derive(Clone, Debug, Default)]
pub struct Reverse;

impl BuiltinEffect for Reverse {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Reverse"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        Vec::new()
    }

    fn get(&self, _key: &str) -> Option<f64> {
        None
    }

    fn set(&mut self, _key: &str, _value: f64) -> bool {
        false
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        for channel in &mut buf.channels {
            channel.reverse();
        }
        Ok(())
    }
}

/// Audacity's Fade In effect.
#[derive(Clone, Debug, Default)]
pub struct FadeIn;

impl BuiltinEffect for FadeIn {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "FadeIn"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        Vec::new()
    }

    fn get(&self, _key: &str) -> Option<f64> {
        None
    }

    fn set(&mut self, _key: &str, _value: f64) -> bool {
        false
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        for channel in &mut buf.channels {
            let len = channel.len();
            if len == 0 {
                continue;
            }
            for (index, sample) in channel.iter_mut().enumerate() {
                *sample *= index as f32 / len as f32;
            }
        }
        Ok(())
    }
}

/// Audacity's Fade Out effect.
#[derive(Clone, Debug, Default)]
pub struct FadeOut;

impl BuiltinEffect for FadeOut {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "FadeOut"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        Vec::new()
    }

    fn get(&self, _key: &str) -> Option<f64> {
        None
    }

    fn set(&mut self, _key: &str, _value: f64) -> bool {
        false
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        for channel in &mut buf.channels {
            let len = channel.len();
            if len == 0 {
                continue;
            }
            for (index, sample) in channel.iter_mut().enumerate() {
                *sample *= (len - index) as f32 / len as f32;
            }
        }
        Ok(())
    }
}

/// Audacity's Repeat effect. `count` is the number of additional copies.
#[derive(Clone, Debug)]
pub struct Repeat {
    /// Number of additional repetitions.
    pub count: i64,
}

impl Default for Repeat {
    fn default() -> Self {
        Self { count: 1 }
    }
}

impl BuiltinEffect for Repeat {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Repeat"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        vec![ParamDef::int(
            "Count",
            "Repeat count",
            1.0,
            i32::MAX as f64,
            1.0,
        )]
    }

    fn get(&self, key: &str) -> Option<f64> {
        (key == "Count").then_some(self.count as f64)
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let Some(value) = parameter_value(&self.params(), key, value) else {
            return false;
        };
        if key != "Count" {
            return false;
        }
        self.count = value as i64;
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if self.count <= 0 || channels_len(buf) == 0 {
            return Ok(());
        }
        for channel in &mut buf.channels {
            let original = channel.clone();
            let additional = original.len().saturating_mul(self.count as usize);
            channel.reserve(additional);
            for _ in 0..self.count {
                channel.extend_from_slice(&original);
            }
        }
        Ok(())
    }
}

/// Audacity's Stereo To Mono effect.
#[derive(Clone, Debug, Default)]
pub struct StereoToMono;

impl BuiltinEffect for StereoToMono {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "StereoToMono"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        Vec::new()
    }

    fn get(&self, _key: &str) -> Option<f64> {
        None
    }

    fn set(&mut self, _key: &str, _value: f64) -> bool {
        false
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if buf.channels.len() <= 1 {
            return Ok(());
        }
        let len = channels_len(buf);
        let denominator = buf.channels.len() as f32;
        let mut mono = vec![0.0_f32; len];
        for channel in &buf.channels {
            for (index, &sample) in channel.iter().take(len).enumerate() {
                mono[index] += sample / denominator;
            }
        }
        buf.channels = vec![mono];
        Ok(())
    }
}

/// Audacity's Adjustable Fade Nyquist effect.
#[derive(Clone, Debug)]
pub struct AdjustableFade {
    /// Fade type: Up, Down, S-Curve Up, or S-Curve Down.
    pub fade_type: usize,
    /// Mid-fade adjustment, in percent.
    pub curve: f64,
    /// Gain units: percent or dB.
    pub units: usize,
    /// Start gain (or end gain), in the selected units.
    pub gain0: f64,
    /// End gain (or start gain), in the selected units.
    pub gain1: f64,
    /// Handy preset selection.
    pub preset: usize,
}

impl Default for AdjustableFade {
    fn default() -> Self {
        Self {
            fade_type: 0,
            curve: 0.0,
            units: 0,
            gain0: 0.0,
            gain1: 100.0,
            preset: 0,
        }
    }
}

impl AdjustableFade {
    fn gain_scale(&self, gain: f64) -> f64 {
        if self.units == 0 {
            gain / 100.0
        } else {
            scale_db(gain)
        }
    }

    fn simple_curve(g0: f64, g1: f64, curve_ratio: f64, t: f64) -> f64 {
        if g0 == g1 {
            return g0;
        }

        let linear = if g0 > g1 { 1.0 - t } else { t };
        let cosine = if g0 > g1 {
            (1.0 + (std::f64::consts::PI * t).cos()) * 0.5
        } else {
            (1.0 - (std::f64::consts::PI * t).cos()) * 0.5
        };

        let env = if curve_ratio > 0.0 && curve_ratio < 0.5 {
            let weight = curve_ratio * 2.0;
            linear * (1.0 - weight) + cosine * weight
        } else if curve_ratio > 0.0 {
            let power = 1.5 - curve_ratio;
            cosine.max(0.0).powf(power)
        } else {
            let power = 1.0 - 2.0 * curve_ratio;
            linear.max(0.0).powf(power)
        };

        g0.min(g1) + (g0 - g1).abs() * env
    }

    fn raised_cosine(g0: f64, g1: f64, curve_ratio: f64, t: f64) -> f64 {
        let adjusted = if curve_ratio > 0.0 {
            let x = 2.0 * curve_ratio;
            ((1.0 - x).exp() - std::f64::consts::E)
                / (1.0 - std::f64::consts::E)
        } else {
            let x = 1.63 * curve_ratio;
            ((1.0 - x).exp() - std::f64::consts::E)
                / (1.0 - std::f64::consts::E)
        };

        if g0 == g1 {
            return g0;
        }
        let base = if g0 > g1 {
            (1.0 + (std::f64::consts::PI * t).cos()) * 0.5
        } else {
            (1.0 - (std::f64::consts::PI * t).cos()) * 0.5
        };
        let ramp = if g0 > g1 {
            1.0 - adjusted * t
        } else {
            1.0 - adjusted * (1.0 - t)
        };
        let env = if base == 0.0 {
            0.0
        } else {
            (base.ln() * ramp).exp()
        };
        g0.min(g1) + (g0 - g1).abs() * env
    }

    fn gain_at(&self, t: f64) -> f64 {
        if self.preset != 0 {
            return match self.preset {
                1 => t,
                2 => 1.0 - t,
                3 | 4 => {
                    let x = scale_db(-60.0);
                    let env = if self.preset == 3 {
                        x.powf(1.0 - t)
                    } else {
                        x.powf(t)
                    };
                    (env - x) / (1.0 - x)
                }
                5 | 6 => {
                    let x = scale_db(15.311);
                    let env = if self.preset == 5 {
                        x.powf(1.0 - t)
                    } else {
                        x.powf(t)
                    };
                    (env - x) / (1.0 - x)
                }
                7 => t.sqrt(),
                8 => (1.0 - t).sqrt(),
                9 => (1.0 - (std::f64::consts::PI * t).cos()) * 0.5,
                10 => (1.0 + (std::f64::consts::PI * t).cos()) * 0.5,
                11 => (1.0 - (std::f64::consts::PI * t).cos()) * 0.5,
                _ => (1.0 + (std::f64::consts::PI * t).cos()) * 0.5,
            };
        }

        let g0 = self.gain_scale(self.gain0);
        let g1 = self.gain_scale(self.gain1);
        let curve_ratio = self.curve / 100.0;

        match self.fade_type {
            0 => Self::simple_curve(g0.min(g1), g0.max(g1), curve_ratio, t),
            1 => Self::simple_curve(g0.max(g1), g0.min(g1), curve_ratio, t),
            2 => Self::raised_cosine(g0.min(g1), g0.max(g1), curve_ratio, t),
            _ => Self::raised_cosine(g0.max(g1), g0.min(g1), curve_ratio, t),
        }
    }
}

impl BuiltinEffect for AdjustableFade {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "AdjustableFade"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::choice("type", "Fade type", ADJUSTABLE_FADE_TYPES, 0),
            ParamDef::float("curve", "Mid-fade adjust", -100.0, 100.0, 0.0),
            ParamDef::choice("units", "Gain units", ADJUSTABLE_FADE_UNITS, 0),
            ParamDef::float("gain0", "Start gain", -1000.0, 1000.0, 0.0),
            ParamDef::float("gain1", "End gain", -1000.0, 1000.0, 100.0),
            ParamDef::choice("preset", "Preset", ADJUSTABLE_FADE_PRESETS, 0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "type" => Some(self.fade_type as f64),
            "curve" => Some(self.curve),
            "units" => Some(self.units as f64),
            "gain0" => Some(self.gain0),
            "gain1" => Some(self.gain1),
            "preset" => Some(self.preset as f64),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let Some(value) = parameter_value(&self.params(), key, value) else {
            return false;
        };
        match key {
            "type" => self.fade_type = value as usize,
            "curve" => self.curve = value,
            "units" => self.units = value as usize,
            "gain0" => self.gain0 = value,
            "gain1" => self.gain1 = value,
            "preset" => self.preset = value as usize,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        for channel in &mut buf.channels {
            let len = channel.len();
            if len == 0 {
                continue;
            }
            for (index, sample) in channel.iter_mut().enumerate() {
                let gain = self.gain_at(index as f64 / len as f64);
                *sample = (*sample as f64 * gain) as f32;
            }
        }
        Ok(())
    }
}

/// Audacity's Studio Fade Out Nyquist effect.
#[derive(Clone, Debug, Default)]
pub struct StudioFadeOut;

impl BuiltinEffect for StudioFadeOut {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "StudioFadeOut"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        Vec::new()
    }

    fn get(&self, _key: &str) -> Option<f64> {
        None
    }

    fn set(&mut self, _key: &str, _value: f64) -> bool {
        false
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let len = channels_len(buf);
        if len == 0 {
            return Ok(());
        }
        if len < 3 {
            return Err(EffectError::Invalid(
                "Studio Fade Out requires more than 2 samples".to_string(),
            ));
        }

        let sample_rate = buf.sample_rate.max(1) as f64;
        let duration = len as f64 / sample_rate;
        let filter_duration = (duration / 2.0).min(0.5);

        for channel in &mut buf.channels {
            let mut low_pass = 0.0_f64;
            for (index, sample) in channel.iter_mut().enumerate() {
                let t = index as f64 / sample_rate;
                let fade = if duration < 0.2 {
                    0.5 * (1.0 + (std::f64::consts::PI * t / duration).cos())
                } else if t <= filter_duration && filter_duration > 0.0 {
                    0.5 * (1.0 + (std::f64::consts::PI * t / filter_duration).cos())
                } else {
                    0.0
                };

                if duration >= 0.2 {
                    let cutoff =
                        (sample_rate * 0.5) + (100.0 - sample_rate * 0.5) * (t / duration);
                    let alpha = 1.0 - (-2.0 * std::f64::consts::PI * cutoff.max(0.0)
                        / sample_rate)
                        .exp();
                    low_pass += alpha * (*sample as f64 - low_pass);
                    let filter_in = 1.0 - fade;
                    *sample = ((*sample as f64 * fade) + low_pass * filter_in) as f32;
                } else {
                    *sample = (*sample as f64 * fade) as f32;
                }
            }
        }
        Ok(())
    }
}

/// Audacity's Clip Fix Nyquist effect.
#[derive(Clone, Debug)]
pub struct ClipFix {
    /// Threshold as a percentage of the channel's peak.
    pub threshold: f64,
    /// Output gain in dB.
    pub gain: f64,
}

impl Default for ClipFix {
    fn default() -> Self {
        Self {
            threshold: 95.0,
            gain: -9.0,
        }
    }
}

impl ClipFix {
    fn process_chunk(buffer: &mut [f32], threshold: f64) {
        const SLOPE_LENGTH: usize = 4;
        if buffer.len() < SLOPE_LENGTH * 2 {
            return;
        }

        let mut exits = Vec::new();
        let mut returns = Vec::new();
        let last_sample = buffer.len() - SLOPE_LENGTH;

        for index in SLOPE_LENGTH..last_sample {
            let current = (buffer[index] as f64).abs();
            let previous = (buffer[index - 1] as f64).abs();
            if current >= threshold {
                if previous < threshold {
                    exits.push(index - 1);
                }
            } else if previous >= threshold {
                returns.push(index);
            }
        }

        if (buffer[SLOPE_LENGTH - 1] as f64).abs() >= threshold && !returns.is_empty() {
            returns.remove(0);
        }

        for (&t0, &t1) in exits.iter().zip(returns.iter()) {
            if t1 <= t0 || t1 >= buffer.len() || t0 < SLOPE_LENGTH - 1 {
                continue;
            }
            let distance = (t1 - t0) as f64;
            let d0 = (buffer[t0] as f64 - buffer[t0 - (SLOPE_LENGTH - 1)] as f64)
                / (SLOPE_LENGTH - 1) as f64;
            let d1 = (buffer[t1 + (SLOPE_LENGTH - 1)] as f64 - buffer[t1] as f64)
                / (SLOPE_LENGTH - 1) as f64;
            let m = (d1 + d0) / (distance * distance);
            let b = d1 / distance - m * t1 as f64;

            for index in (t0 + 1)..t1 {
                let j = index as f64;
                let value = (t1 - index) as f64 * (buffer[t0] as f64 / distance)
                    + (index - t0) as f64 * (buffer[t1] as f64 / distance)
                    + (index - t0) as f64
                        * (index as isize - t1 as isize) as f64
                        * (m * j + b);
                buffer[index] = value as f32;
            }
        }
    }
}

impl BuiltinEffect for ClipFix {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "ClipFix"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("threshold", "Threshold of clipping", 0.0, 100.0, 95.0),
            ParamDef::float("gain", "Reduce amplitude", -30.0, 0.0, -9.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "threshold" => Some(self.threshold),
            "gain" => Some(self.gain),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let Some(value) = parameter_value(&self.params(), key, value) else {
            return false;
        };
        match key {
            "threshold" => self.threshold = value,
            "gain" => self.gain = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        const BUFFER_SIZE: usize = 100_000;
        const SLOPE_LENGTH: usize = 4;
        let gain = scale_db(self.gain) as f32;

        for channel in &mut buf.channels {
            let peak = channel
                .iter()
                .fold(0.0_f32, |peak, &sample| peak.max(sample.abs()));
            let threshold = self.threshold / 100.0 * peak as f64;
            let mut position = 0;

            while channel.len().saturating_sub(position) >= BUFFER_SIZE {
                Self::process_chunk(
                    &mut channel[position..position + BUFFER_SIZE],
                    threshold,
                );
                position += BUFFER_SIZE;
            }

            let remainder = channel.len() - position;
            if remainder >= SLOPE_LENGTH {
                Self::process_chunk(&mut channel[position..], threshold);
            }

            for sample in channel {
                *sample *= gain;
            }
        }
        Ok(())
    }
}

/// Audacity's Crossfade Clips Nyquist effect.
#[derive(Clone, Debug, Default)]
pub struct CrossfadeClips;

impl BuiltinEffect for CrossfadeClips {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "CrossfadeClips"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        Vec::new()
    }

    fn get(&self, _key: &str) -> Option<f64> {
        None
    }

    fn set(&mut self, _key: &str, _value: f64) -> bool {
        false
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let aux = ctx
            .aux
            .ok_or_else(|| EffectError::Invalid("Crossfade Clips requires a second clip".into()))?;
        let len = channels_len(buf).min(channels_len(aux));
        let channels = buf.channels.len().max(aux.channels.len());
        let mut output = Vec::with_capacity(channels);

        for channel_index in 0..channels {
            let first = buf.channels.get(channel_index);
            let second = aux.channels.get(channel_index);
            let mut channel = Vec::with_capacity(len);

            for index in 0..len {
                let t = if len == 0 {
                    0.0
                } else {
                    index as f64 / len as f64
                };
                let (fade_out, fade_in) = if len < 2 {
                    (1.0 - t, t)
                } else {
                    (
                        (std::f64::consts::FRAC_PI_2 * t).cos(),
                        (std::f64::consts::FRAC_PI_2 * t).sin(),
                    )
                };
                let a = first
                    .and_then(|samples| samples.get(index))
                    .copied()
                    .unwrap_or(0.0);
                let b = second
                    .and_then(|samples| samples.get(index))
                    .copied()
                    .unwrap_or(0.0);
                channel.push((a as f64 * fade_out + b as f64 * fade_in) as f32);
            }
            output.push(channel);
        }

        buf.channels = output;
        Ok(())
    }
}

fn crossfade_track_gain(effect_type: usize, curve: f64, fade_out: bool, t: f64) -> f64 {
    match effect_type {
        0 => {
            if fade_out {
                1.0 - t
            } else {
                t
            }
        }
        1 => {
            if fade_out {
                (std::f64::consts::FRAC_PI_2 * t).cos()
            } else {
                (std::f64::consts::FRAC_PI_2 * t).sin()
            }
        }
        2 => {
            if fade_out {
                (1.0 - t).sqrt()
            } else {
                t.sqrt()
            }
        }
        _ => {
            let ccurve = 0.99 - 0.98 * curve;
            let epsilon = ccurve.powf(2.7);
            let envelope = if fade_out {
                epsilon.powf(1.0 - t)
            } else {
                epsilon.powf(t)
            };
            if fade_out {
                1.0 - (envelope - epsilon) / (1.0 - epsilon)
            } else {
                (envelope - epsilon) / (1.0 - epsilon)
            }
        }
    }
}

/// Audacity's Crossfade Tracks Nyquist effect.
#[derive(Clone, Debug)]
pub struct CrossfadeTracks {
    /// Crossfade curve type.
    pub fade_type: usize,
    /// Custom curve amount.
    pub curve: f64,
    /// Fade direction.
    pub direction: usize,
}

impl Default for CrossfadeTracks {
    fn default() -> Self {
        Self {
            fade_type: 0,
            curve: 0.0,
            direction: 0,
        }
    }
}

impl BuiltinEffect for CrossfadeTracks {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "CrossfadeTracks"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::choice("type", "Fade type", CROSSFADE_TYPES, 0),
            ParamDef::float("curve", "Custom curve", 0.0, 1.0, 0.0),
            ParamDef::choice("direction", "Fade direction", CROSSFADE_DIRECTIONS, 0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "type" => Some(self.fade_type as f64),
            "curve" => Some(self.curve),
            "direction" => Some(self.direction as f64),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let Some(value) = parameter_value(&self.params(), key, value) else {
            return false;
        };
        match key {
            "type" => self.fade_type = value as usize,
            "curve" => self.curve = value,
            "direction" => self.direction = value as usize,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let aux = ctx
            .aux
            .ok_or_else(|| EffectError::Invalid("Crossfade Tracks requires a second track".into()))?;
        let len = channels_len(buf);

        // The first input fades out by default; OutIn also fades the first
        // input out, while InOut reverses the direction.
        let first_fades_out = self.direction != 2;
        for (channel_index, channel) in buf.channels.iter_mut().enumerate() {
            let other = aux
                .channels
                .get(channel_index)
                .or_else(|| aux.channels.first());
            for (index, sample) in channel.iter_mut().enumerate() {
                let t = if len == 0 {
                    0.0
                } else {
                    index as f64 / len as f64
                };
                let first_gain =
                    crossfade_track_gain(self.fade_type, self.curve, first_fades_out, t);
                let other_gain =
                    crossfade_track_gain(self.fade_type, self.curve, !first_fades_out, t);
                let second = other
                    .and_then(|samples| samples.get(index))
                    .copied()
                    .unwrap_or(0.0);
                *sample = (*sample as f64 * first_gain + second as f64 * other_gain) as f32;
            }
        }
        Ok(())
    }
}

/// Returns one default instance of each effect in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![
        Box::new(Amplify::default()),
        Box::new(Normalize::default()),
        Box::new(Invert),
        Box::new(Reverse),
        Box::new(FadeIn),
        Box::new(FadeOut),
        Box::new(Repeat::default()),
        Box::new(StereoToMono),
        Box::new(AdjustableFade::default()),
        Box::new(StudioFadeOut),
        Box::new(ClipFix::default()),
        Box::new(CrossfadeClips),
        Box::new(CrossfadeTracks::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_buffer() -> AudioBuffer {
        let left: Vec<f32> = (0..64)
            .map(|i| (i as f32 * 0.17).sin() * 0.5)
            .collect();
        let right: Vec<f32> = (0..64)
            .map(|i| (i as f32 * 0.13).sin() * 0.3)
            .collect();
        AudioBuffer::from_channels(44_100, vec![left, right])
    }

    fn aux_buffer() -> AudioBuffer {
        let left: Vec<f32> = (0..64)
            .map(|i| (i as f32 * 0.11).cos() * 0.4)
            .collect();
        let right: Vec<f32> = (0..64)
            .map(|i| (i as f32 * 0.09).cos() * 0.2)
            .collect();
        AudioBuffer::from_channels(44_100, vec![left, right])
    }

    fn same_samples(a: &AudioBuffer, b: &AudioBuffer) -> bool {
        a.channels == b.channels
    }

    fn exercise_effect(mut effect: Box<dyn BuiltinEffect>, needs_aux: bool) {
        let mut input = sine_buffer();
        let mut context = EffectContext::default();
        let aux = aux_buffer();
        if needs_aux {
            context.aux = Some(&aux);
        }
        effect.process(&mut input, &mut context).unwrap();

        let mut empty = AudioBuffer::from_channels(44_100, vec![vec![], vec![]]);
        let mut empty_context = EffectContext::default();
        let empty_aux = AudioBuffer::from_channels(44_100, vec![vec![], vec![]]);
        if needs_aux {
            empty_context.aux = Some(&empty_aux);
        }
        let _ = effect.process(&mut empty, &mut empty_context);

        let mut first = sine_buffer();
        let mut second = sine_buffer();
        let mut first_context = EffectContext::default();
        let mut second_context = EffectContext::default();
        if needs_aux {
            first_context.aux = Some(&aux);
            second_context.aux = Some(&aux);
        }
        let mut first_effect = all()
            .into_iter()
            .find(|candidate| candidate.id() == effect.id())
            .unwrap();
        let mut second_effect = all()
            .into_iter()
            .find(|candidate| candidate.id() == effect.id())
            .unwrap();
        first_effect.process(&mut first, &mut first_context).unwrap();
        second_effect.process(&mut second, &mut second_context).unwrap();
        assert!(same_samples(&first, &second), "{}", effect.id());

        for param in effect.params() {
            assert!(set_param(
                effect.as_mut(),
                param.key,
                param.min - 100.0
            ));
            assert_eq!(effect.get(param.key), Some(param.min));
            assert!(set_param(
                effect.as_mut(),
                param.key,
                param.max + 100.0
            ));
            assert_eq!(effect.get(param.key), Some(param.max));
        }
    }

    #[test]
    fn all_effects_run_and_clamp_parameters() {
        for effect in all() {
            let needs_aux = matches!(&*effect.id(), "CrossfadeClips" | "CrossfadeTracks");
            exercise_effect(effect, needs_aux);
        }
    }

    #[test]
    fn amplify_and_invert_sanity() {
        let mut amplify = Amplify { ratio: 2.0, allow_clipping: false };
        let mut buf = AudioBuffer::from_channels(44_100, vec![vec![0.25, -0.5]]);
        amplify
            .process(&mut buf, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buf.channels[0], vec![0.5, -1.0]);

        let mut invert = Invert;
        invert
            .process(&mut buf, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buf.channels[0], vec![-0.5, 1.0]);
    }

    #[test]
    fn normalize_reverse_and_fades_sanity() {
        let mut normalize = Normalize::default();
        normalize.remove_dc_offset = false;
        normalize.peak_level = 0.0;
        let mut buf = AudioBuffer::from_channels(44_100, vec![vec![0.25, -0.5]]);
        normalize
            .process(&mut buf, &mut EffectContext::default())
            .unwrap();
        assert!((buf.channels[0][0] - 0.5).abs() < 1.0e-6);
        assert!((buf.channels[0][1] + 1.0).abs() < 1.0e-6);

        let mut reverse = Reverse;
        let mut buf = AudioBuffer::from_channels(44_100, vec![vec![1.0, 2.0, 3.0]]);
        reverse
            .process(&mut buf, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buf.channels[0], vec![3.0, 2.0, 1.0]);

        let mut fade_in = FadeIn;
        let mut buf = AudioBuffer::from_channels(44_100, vec![vec![1.0; 4]]);
        fade_in
            .process(&mut buf, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buf.channels[0], vec![0.0, 0.25, 0.5, 0.75]);

        let mut fade_out = FadeOut;
        let mut buf = AudioBuffer::from_channels(44_100, vec![vec![1.0; 4]]);
        fade_out
            .process(&mut buf, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buf.channels[0], vec![1.0, 0.75, 0.5, 0.25]);
    }

    #[test]
    fn repeat_and_mono_sanity() {
        let mut repeat = Repeat::default();
        let mut buf = AudioBuffer::from_channels(44_100, vec![vec![1.0, 2.0]]);
        repeat
            .process(&mut buf, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buf.channels[0], vec![1.0, 2.0, 1.0, 2.0]);

        let mut mono = StereoToMono;
        let mut buf = AudioBuffer::from_channels(
            44_100,
            vec![vec![1.0, 0.0], vec![0.0, 1.0]],
        );
        mono.process(&mut buf, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buf.channels, vec![vec![0.5, 0.5]]);
    }

    #[test]
    fn nyquist_effects_sanity() {
        let mut adjustable = AdjustableFade {
            preset: 1,
            ..AdjustableFade::default()
        };
        let mut buf = AudioBuffer::from_channels(44_100, vec![vec![1.0; 4]]);
        adjustable
            .process(&mut buf, &mut EffectContext::default())
            .unwrap();
        assert_eq!(buf.channels[0], vec![0.0, 0.25, 0.5, 0.75]);

        let mut studio = StudioFadeOut;
        let mut buf = AudioBuffer::from_channels(44_100, vec![vec![1.0; 128]]);
        studio
            .process(&mut buf, &mut EffectContext::default())
            .unwrap();
        assert!(buf.channels[0][0] > buf.channels[0][127]);

        let mut clip_fix = ClipFix::default();
        let mut buf =
            AudioBuffer::from_channels(44_100, vec![vec![0.2, 0.4, 0.8, 0.8, 0.8, 0.4, 0.2]]);
        clip_fix
            .process(&mut buf, &mut EffectContext::default())
            .unwrap();
        assert!(buf.channels[0].iter().all(|sample| sample.abs() <= 1.0));
    }

    #[test]
    fn crossfade_effects_sanity() {
        let mut clips = CrossfadeClips;
        let mut buf = AudioBuffer::from_channels(44_100, vec![vec![1.0; 4]]);
        let aux = AudioBuffer::from_channels(44_100, vec![vec![0.0; 4]]);
        let mut context = EffectContext {
            aux: Some(&aux),
            ..EffectContext::default()
        };
        clips.process(&mut buf, &mut context).unwrap();
        assert_eq!(buf.len(), 4);
        assert!(buf.channels[0][0] > buf.channels[0][3]);

        let mut tracks = CrossfadeTracks::default();
        let mut buf = AudioBuffer::from_channels(44_100, vec![vec![1.0; 4]]);
        let aux = AudioBuffer::from_channels(44_100, vec![vec![1.0; 4]]);
        let mut context = EffectContext {
            aux: Some(&aux),
            ..EffectContext::default()
        };
        tracks.process(&mut buf, &mut context).unwrap();
        assert!((buf.channels[0][0] - 1.0).abs() < 1.0e-6);
        assert!((buf.channels[0][3] - 1.0).abs() < 1.0e-6);
    }
}
