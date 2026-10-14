// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.
//! Common framework for built-in effects, generators and analyzers.

use au_core::AudioBuffer;

/// What an effect does to the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    /// Transforms existing audio.
    Process,
    /// Replaces the selection with generated audio (length = selection length).
    Generate,
    /// Reads audio and reports measurements / labels without modifying it.
    Analyze,
}

/// Parameter type, mirroring Audacity's CommandParameters.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamType {
    /// Real-valued.
    Float,
    /// Integer-valued.
    Int,
    /// Boolean stored as 0.0 / 1.0.
    Bool,
    /// Enumeration; the value is the index into the symbol list.
    Choice(&'static [&'static str]),
}

/// Describes one effect parameter. `key` is Audacity's automation key.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamDef {
    /// Automation key as used by Audacity macros / .aup3 (e.g. "Delay").
    pub key: &'static str,
    /// Human label.
    pub label: &'static str,
    /// Parameter type.
    pub ty: ParamType,
    /// Minimum.
    pub min: f64,
    /// Maximum.
    pub max: f64,
    /// Default.
    pub default: f64,
}

impl ParamDef {
    /// Float parameter.
    pub const fn float(key: &'static str, label: &'static str, min: f64, max: f64, default: f64) -> Self {
        Self { key, label, ty: ParamType::Float, min, max, default }
    }
    /// Integer parameter.
    pub const fn int(key: &'static str, label: &'static str, min: f64, max: f64, default: f64) -> Self {
        Self { key, label, ty: ParamType::Int, min, max, default }
    }
    /// Boolean parameter.
    pub const fn boolean(key: &'static str, label: &'static str, default: bool) -> Self {
        Self { key, label, ty: ParamType::Bool, min: 0.0, max: 1.0, default: if default { 1.0 } else { 0.0 } }
    }
    /// Choice parameter.
    pub const fn choice(key: &'static str, label: &'static str, symbols: &'static [&'static str], default: usize) -> Self {
        Self { key, label, ty: ParamType::Choice(symbols), min: 0.0, max: (symbols.len() - 1) as f64, default: default as f64 }
    }
    /// Clamps (and rounds for Int/Bool/Choice) a value into range.
    pub fn clamp(&self, v: f64) -> f64 {
        let v = if v.is_nan() { self.default } else { v.clamp(self.min, self.max) };
        match self.ty { ParamType::Float => v, _ => v.round() }
    }
}

/// A label produced by an analyzer (times in seconds relative to selection start).
#[derive(Clone, Debug, PartialEq)]
pub struct OutLabel {
    /// Start time.
    pub t0: f64,
    /// End time.
    pub t1: f64,
    /// Text.
    pub text: String,
}

/// Extra inputs / results for processing.
#[derive(Default, Debug)]
pub struct EffectContext<'a> {
    /// Control track (Auto Duck) or second track (Crossfade Tracks).
    pub aux: Option<&'a AudioBuffer>,
    /// Absolute time of the selection start in the project (seconds).
    pub t0: f64,
    /// Labels created by analyzers (e.g. Beat Finder, Label Sounds, Find Clipping).
    pub labels: Vec<OutLabel>,
    /// Free-text report (e.g. Contrast, Sample Data Export, loudness measurement).
    pub report: String,
}

/// Errors reported by effects.
#[derive(Debug, Clone, PartialEq)]
pub enum EffectError {
    /// Bad / insufficient input (message explains).
    Invalid(String),
}

impl std::fmt::Display for EffectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { EffectError::Invalid(s) => write!(f, "{s}") }
    }
}
impl std::error::Error for EffectError {}

/// A built-in Audacity effect, generator or analyzer.
///
/// Parameters are exchanged as `f64` keyed by Audacity's automation keys so
/// presets, macros and `.aup3` realtime-effect state can round-trip them.
pub trait BuiltinEffect: Send {
    /// Audacity command id (e.g. "Echo", "Normalize", "Tone").
    fn id(&self) -> &'static str;
    /// Display name.
    fn name(&self) -> &'static str { self.id() }
    /// Kind of effect.
    fn kind(&self) -> EffectKind { EffectKind::Process }
    /// Parameter definitions.
    fn params(&self) -> Vec<ParamDef>;
    /// Reads a parameter.
    fn get(&self, key: &str) -> Option<f64>;
    /// Writes a parameter (value is clamped by the caller via [`ParamDef::clamp`]). Returns false for unknown keys.
    fn set(&mut self, key: &str, value: f64) -> bool;
    /// Offline (destructive) processing of the whole selection. May change the buffer length.
    fn process(&mut self, buf: &mut AudioBuffer, ctx: &mut EffectContext) -> Result<(), EffectError>;
    /// Two-step effects (Noise Reduction): learn a profile from `noise`. Others return an error.
    fn learn_profile(&mut self, _noise: &AudioBuffer) -> Result<(), EffectError> {
        Err(EffectError::Invalid("effect has no profile step".into()))
    }
    /// Whether this effect can run in a realtime stack.
    fn supports_realtime(&self) -> bool { false }
    /// Prepares realtime state.
    fn rt_reset(&mut self, _sample_rate: u32, _channels: usize) {}
    /// Processes one block in place (all channels same length). Only called if `supports_realtime`.
    fn rt_process(&mut self, _block: &mut [Vec<f32>]) {}
    /// Latency (frames) introduced by realtime processing.
    fn rt_latency(&self) -> usize { 0 }
}

/// Sets a parameter with clamping. Returns false for unknown keys.
pub fn set_param(e: &mut dyn BuiltinEffect, key: &str, value: f64) -> bool {
    match e.params().into_iter().find(|p| p.key == key) {
        Some(p) => e.set(key, p.clamp(value)),
        None => false,
    }
}

/// Snapshot of all parameters, in definition order.
pub fn get_params(e: &dyn BuiltinEffect) -> Vec<(String, f64)> {
    e.params().iter().filter_map(|p| e.get(p.key).map(|v| (p.key.to_string(), v))).collect()
}

/// Resets all parameters to their defaults.
pub fn reset_defaults(e: &mut dyn BuiltinEffect) {
    for p in e.params() { e.set(p.key, p.default); }
}

/// Helper used by most effects: run `rt_reset` + `rt_process` over the whole buffer in blocks.
pub fn process_via_realtime(e: &mut dyn BuiltinEffect, buf: &mut AudioBuffer) {
    let ch = buf.channels.len();
    e.rt_reset(buf.sample_rate, ch);
    let n = buf.len();
    let bs = 4096;
    let mut pos = 0;
    while pos < n {
        let m = bs.min(n - pos);
        let mut block: Vec<Vec<f32>> = buf.channels.iter().map(|c| c[pos..pos + m].to_vec()).collect();
        e.rt_process(&mut block);
        for (c, b) in buf.channels.iter_mut().zip(block) { c[pos..pos + m].copy_from_slice(&b); }
        pos += m;
    }
}
