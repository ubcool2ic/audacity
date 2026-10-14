// SPDX-License-Identifier: GPL-3.0-or-later
//! Registry of all built-in effects, generators and analyzers.
use crate::core::*;
use crate::fx;

/// One default instance of every built-in.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    let mut v = Vec::new();
    v.extend(fx::simple::all());
    v.extend(fx::filters::all());
    v.extend(fx::modulation::all());
    v.extend(fx::reverb::all());
    v.extend(fx::dynamics::all());
    v.extend(fx::autoduck::all());
    v.extend(fx::eq::all());
    v.extend(fx::loudness::all());
    v.extend(fx::noisered::all());
    v.extend(fx::repair::all());
    v.extend(fx::stretch::all());
    v.extend(fx::silence::all());
    v.extend(fx::generators::all());
    v.extend(fx::analyzers::all());
    v
}

/// Ids of all built-ins of a kind.
pub fn ids(kind: EffectKind) -> Vec<&'static str> {
    all().iter().filter(|e| e.kind() == kind).map(|e| e.id()).collect()
}

/// Creates a built-in by Audacity id (case-insensitive, spaces/hyphens ignored).
pub fn create(id: &str) -> Option<Box<dyn BuiltinEffect>> {
    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    let alias = match norm(id).as_str() {
        "changespeedandpitch" => "changespeed", "dtmftones" | "dtmf" => "dtmftones", "filtercurve" => "filtercurveeq",
        "graphiceq" => "graphiceq", "measurerms" => "rms", _ => "",
    }.to_string();
    let want = norm(id);
    all().into_iter().find(|e| { let (a, b) = (norm(e.id()), norm(e.name())); a == want || b == want || (!alias.is_empty() && (a == alias || b == alias)) })
}
