// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.
//! Realtime (non-destructive) effect stacks and presets.

use crate::core::*;
use crate::registry;
use au_core::AudioBuffer;
use std::path::{Path, PathBuf};

/// One slot in a realtime stack, mirroring Audacity's `<effect>` element.
pub struct StackSlot {
    /// Whether the slot processes audio.
    pub active: bool,
    /// The effect instance.
    pub effect: Box<dyn BuiltinEffect>,
}

impl std::fmt::Debug for StackSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StackSlot").field("id", &self.effect.id()).field("active", &self.active)
            .field("params", &get_params(self.effect.as_ref())).finish()
    }
}

/// Serializable description of a stack slot (what `.aup3` stores).
#[derive(Clone, Debug, PartialEq)]
pub struct SlotDesc {
    /// Audacity plugin id, e.g. `Effect_Audacity_Audacity_Echo_Built-in Effect: Echo`.
    pub plugin_id: String,
    /// Active flag.
    pub active: bool,
    /// Parameters `(name, value-string)` in Audacity's CommandParameters format.
    pub params: Vec<(String, String)>,
}

/// A list of realtime effects (per track or master), like Audacity's `RealtimeEffectList`.
#[derive(Debug, Default)]
pub struct EffectStack {
    /// Whole-list bypass (`<effects active="...">`).
    pub active: bool,
    /// Slots, processed in order.
    pub slots: Vec<StackSlot>,
    rate: u32,
    channels: usize,
}

/// Audacity plugin id for a built-in effect symbol.
pub fn plugin_id(symbol: &str) -> String {
    format!("Effect_Audacity_Audacity_{symbol}_Built-in Effect: {symbol}")
}

/// Extracts the built-in symbol from an Audacity plugin id (5 `_`-separated fields, field 3).
pub fn symbol_from_plugin_id(id: &str) -> Option<String> {
    let parts: Vec<&str> = id.splitn(5, '_').collect();
    if parts.len() == 5 { Some(parts[3].to_string()) } else { None }
}

fn fmt_value(def: &ParamDef, v: f64) -> String {
    match &def.ty {
        ParamType::Bool => if v != 0.0 { "1".into() } else { "0".into() },
        ParamType::Int => format!("{}", v.round() as i64),
        ParamType::Choice(s) => s.get(v as usize).unwrap_or(&"").to_string(),
        ParamType::Float => format!("{v}"),
    }
}

fn parse_value(def: &ParamDef, s: &str) -> Option<f64> {
    match &def.ty {
        ParamType::Bool => Some(if s == "1" || s.eq_ignore_ascii_case("true") { 1.0 } else { 0.0 }),
        ParamType::Choice(sy) => sy.iter().position(|x| *x == s).or_else(|| sy.iter().position(|x| x.eq_ignore_ascii_case(s))).map(|i| i as f64).or_else(|| s.parse().ok()),
        _ => s.parse().ok(),
    }
}

/// Parameters of an effect as Audacity-style strings.
pub fn params_to_strings(e: &dyn BuiltinEffect) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = e.params().iter().filter_map(|p| e.get(p.key).map(|v| (p.key.to_string(), fmt_value(p, v)))).collect();
    for k in e.text_keys() { if let Some(t) = e.get_text(k) { v.push((k.to_string(), t)); } }
    v
}

/// Applies Audacity-style string parameters; unknown keys are ignored.
pub fn params_from_strings(e: &mut dyn BuiltinEffect, ps: &[(String, String)]) {
    let defs = e.params();
    for (k, v) in ps {
        if let Some(d) = defs.iter().find(|d| d.key == k).or_else(|| defs.iter().find(|d| d.key.eq_ignore_ascii_case(k))) {
            if let Some(i) = e.choice_symbols(d.key).and_then(|syms| syms.iter().position(|s| s == v)) {
                e.set(d.key, i as f64);
            } else if let Some(x) = parse_value(d, v) { e.set(d.key, d.clamp(x)); }
        } else if let Some(t) = e.text_keys().into_iter().find(|t| t.eq_ignore_ascii_case(k)) {
            e.set_text(t, v);
        }
    }
}

impl EffectStack {
    /// New, active, empty stack.
    pub fn new() -> Self { Self { active: true, ..Default::default() } }

    /// Appends an effect by id. Returns false if unknown or not realtime-capable.
    pub fn push(&mut self, id: &str) -> bool {
        match registry::create(id) {
            Some(mut e) if e.supports_realtime() => {
                if self.rate > 0 { e.rt_reset(self.rate, self.channels); }
                self.slots.push(StackSlot { active: true, effect: e });
                true
            }
            _ => false,
        }
    }

    /// Moves a slot.
    pub fn move_slot(&mut self, from: usize, to: usize) {
        if from < self.slots.len() && to < self.slots.len() { let s = self.slots.remove(from); self.slots.insert(to, s); }
    }

    /// Removes a slot.
    pub fn remove(&mut self, i: usize) { if i < self.slots.len() { self.slots.remove(i); } }

    /// Prepares all effects for streaming.
    pub fn reset(&mut self, sample_rate: u32, channels: usize) {
        self.rate = sample_rate; self.channels = channels;
        for s in &mut self.slots { s.effect.rt_reset(sample_rate, channels); }
    }

    /// Total latency of active slots.
    pub fn latency(&self) -> usize {
        if !self.active { return 0; }
        self.slots.iter().filter(|s| s.active).map(|s| s.effect.rt_latency()).sum()
    }

    /// Processes a block in place (channels must match `reset`).
    pub fn process_block(&mut self, block: &mut [Vec<f32>]) {
        if !self.active { return; }
        if block.len() != self.channels { self.reset(self.rate.max(1), block.len()); }
        for s in &mut self.slots { if s.active { s.effect.rt_process(block); } }
    }

    /// Renders a whole buffer through the stack (used for export / mixdown).
    /// Latency is compensated so output aligns with input; length is preserved.
    pub fn render(&mut self, buf: &mut AudioBuffer) {
        if !self.active || self.slots.iter().all(|s| !s.active) { return; }
        let ch = buf.channels.len();
        self.reset(buf.sample_rate, ch);
        let lat = self.latency();
        let n = buf.len();
        let total = n + lat;
        let mut out: Vec<Vec<f32>> = vec![Vec::with_capacity(total); ch];
        let mut pos = 0;
        while pos < total {
            let m = 4096.min(total - pos);
            let mut block: Vec<Vec<f32>> = buf.channels.iter().map(|c| {
                (pos..pos + m).map(|i| c.get(i).copied().unwrap_or(0.0)).collect()
            }).collect();
            self.process_block(&mut block);
            for (o, b) in out.iter_mut().zip(block) { o.extend(b); }
            pos += m;
        }
        for (c, o) in buf.channels.iter_mut().zip(out) { *c = o[lat..lat + n].to_vec(); }
    }

    /// Describes the stack for saving.
    pub fn describe(&self) -> (bool, Vec<SlotDesc>) {
        (self.active, self.slots.iter().map(|s| SlotDesc {
            plugin_id: plugin_id(s.effect.id()), active: s.active, params: params_to_strings(s.effect.as_ref()),
        }).collect())
    }

    /// Rebuilds a stack from a saved description. Unknown effects are skipped and returned.
    pub fn from_desc(active: bool, slots: &[SlotDesc]) -> (Self, Vec<String>) {
        let mut st = Self::new(); st.active = active;
        let mut missing = Vec::new();
        for d in slots {
            let sym = symbol_from_plugin_id(&d.plugin_id).unwrap_or_else(|| d.plugin_id.clone());
            match registry::create(&sym) {
                Some(mut e) => { params_from_strings(e.as_mut(), &d.params); st.slots.push(StackSlot { active: d.active, effect: e }); }
                None => missing.push(d.plugin_id.clone()),
            }
        }
        (st, missing)
    }
}

/// A named parameter set.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    /// Preset name.
    pub name: String,
    /// Parameter strings.
    pub params: Vec<(String, String)>,
}

impl Preset {
    /// Captures an effect's current settings.
    pub fn capture(name: &str, e: &dyn BuiltinEffect) -> Self { Self { name: name.into(), params: params_to_strings(e) } }
    /// Applies to an effect.
    pub fn apply(&self, e: &mut dyn BuiltinEffect) { params_from_strings(e, &self.params) }
    /// Serializes as Audacity's macro-parameter string: `Key="value" Key2="v2"`.
    pub fn to_string_params(&self) -> String {
        self.params.iter().map(|(k, v)| format!("{k}=\"{}\"", v.replace('"', "'"))).collect::<Vec<_>>().join(" ")
    }
    /// Parses a macro-parameter string.
    pub fn parse_params(name: &str, s: &str) -> Self {
        let mut params = Vec::new();
        let mut rest = s.trim();
        while let Some(eq) = rest.find('=') {
            let k = rest[..eq].trim().to_string();
            let after = &rest[eq + 1..];
            let (v, r) = if let Some(stripped) = after.strip_prefix('"') {
                let end = stripped.find('"').unwrap_or(stripped.len());
                (stripped[..end].to_string(), stripped.get(end + 1..).unwrap_or(""))
            } else {
                let end = after.find(' ').unwrap_or(after.len());
                (after[..end].to_string(), &after[end..])
            };
            params.push((k, v));
            rest = r.trim();
        }
        Self { name: name.into(), params }
    }
}

/// Factory presets for an effect id (from Audacity's C++ tables where they exist).
pub fn factory_presets(id: &str) -> Vec<Preset> {
    match id {
        "Reverb" => {
            // Reverb.cpp FactoryPresets: RoomSize, Delay, Reverberance, HfDamping, ToneLow, ToneHigh, WetGain, DryGain, StereoWidth, WetOnly
            let t: [(&str, [f64; 9], u8); 11] = [
                ("Vocal I", [70., 20., 40., 99., 100., 50., -12., 0., 70.], 0),
                ("Vocal II", [50., 0., 50., 99., 50., 100., -1., -1., 70.], 0),
                ("Bathroom", [16., 8., 80., 0., 0., 100., -6., 0., 100.], 0),
                ("Small Room Bright", [30., 10., 50., 50., 50., 100., -1., -1., 100.], 0),
                ("Small Room Dark", [30., 10., 50., 50., 100., 0., -1., -1., 100.], 0),
                ("Medium Room", [75., 10., 40., 50., 100., 70., -1., -1., 70.], 0),
                ("Large Room", [85., 10., 40., 50., 100., 80., 0., -6., 90.], 0),
                ("Church Hall", [90., 32., 60., 50., 100., 50., 0., -12., 100.], 0),
                ("Cathedral", [90., 16., 90., 50., 100., 0., 0., -20., 100.], 0),
                ("Big Cave", [100., 55., 100., 50., 100., 0., 0., -12., 100.], 0),
                ("Big Cave Wet Only", [100., 55., 100., 50., 100., 0., 0., -12., 100.], 1),
            ];
            let keys = ["RoomSize", "Delay", "Reverberance", "HfDamping", "ToneLow", "ToneHigh", "WetGain", "DryGain", "StereoWidth"];
            t.iter().map(|(n, v, w)| {
                let mut params: Vec<(String, String)> = keys.iter().zip(v).map(|(k, x)| (k.to_string(), format!("{x}"))).collect();
                params.push(("WetOnly".into(), if *w == 1 { "1".into() } else { "0".into() }));
                Preset { name: n.to_string(), params }
            }).collect()
        }
        _ => crate::factory_presets::FACTORY.iter().filter(|(e, _, _)| *e == id)
            .map(|(_, n, p)| Preset::parse_params(n, p)).collect(),
    }
}

/// Directory holding user presets (`$XDG_CONFIG_HOME/audacity-rs/presets`).
pub fn user_preset_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("audacity-rs").join("presets")
}

fn sanitize(s: &str) -> String { s.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect() }

/// Saves a user preset under `dir/<effect id>/<name>.preset`.
pub fn save_user_preset(dir: &Path, effect_id: &str, p: &Preset) -> std::io::Result<()> {
    let d = dir.join(sanitize(effect_id));
    std::fs::create_dir_all(&d)?;
    std::fs::write(d.join(format!("{}.preset", sanitize(&p.name))), format!("{}\n{}\n", p.name, p.to_string_params()))
}

/// Lists user presets for an effect, sorted by name.
pub fn load_user_presets(dir: &Path, effect_id: &str) -> Vec<Preset> {
    let d = dir.join(sanitize(effect_id));
    let mut v: Vec<Preset> = std::fs::read_dir(d).into_iter().flatten().flatten().filter_map(|e| {
        let s = std::fs::read_to_string(e.path()).ok()?;
        let mut l = s.lines();
        let name = l.next()?.to_string();
        Some(Preset::parse_params(&name, l.next().unwrap_or("")))
    }).collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// Deletes a user preset.
pub fn delete_user_preset(dir: &Path, effect_id: &str, name: &str) -> std::io::Result<()> {
    std::fs::remove_file(dir.join(sanitize(effect_id)).join(format!("{}.preset", sanitize(name))))
}
