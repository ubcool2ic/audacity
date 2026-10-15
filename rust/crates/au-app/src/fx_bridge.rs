// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.
//! Glue between built-in effects ([`au_effects`]), the project/editor, the
//! realtime engine and `.aup3` realtime-effect lists. Headless and unit tested;
//! the GUI only adds dialogs on top.

use au_core::{AudioBuffer, Editor, ProjectState, Selection};
use au_effects::core::{BuiltinEffect, EffectContext, EffectKind, OutLabel};
use au_effects::stack::{EffectStack, SlotDesc};
use au_io::engine::{render_offline_with_effects, MixTrack};
use au_project::aup3::{Aup3Extras, RealtimeEffect, RealtimeEffectList};


/// Converts a saved list to a live stack. Unknown effects and plugins that fail
/// to load are skipped, while the remaining slots retain their original order.
pub fn stack_from_list(l: &RealtimeEffectList) -> EffectStack {
    let descs: Vec<SlotDesc> = l
        .effects
        .iter()
        .map(|e| SlotDesc { plugin_id: e.id.clone(), active: e.active, params: e.params.clone() })
        .collect();
    // Hosted plugins (VST3/CLAP) come from the plugin factory; built-ins from the registry.
    let mut factory = |d: &SlotDesc| {
        let e = RealtimeEffect { id: d.plugin_id.clone(), active: d.active, params: d.params.clone() };
        crate::plugin_bridge::slot_from_saved(&e).map(|slot| slot.effect)
    };
    EffectStack::from_desc_with(l.active, &descs, &mut factory).0
}

/// Converts a live stack to the saved form.
pub fn list_from_stack(s: &mut EffectStack) -> RealtimeEffectList {
    let (active, described) = s.describe();
    let effects = s
        .slots
        .iter_mut()
        .zip(described)
        .filter_map(|(slot, desc)| {
            if crate::plugin_bridge::is_plugin_id(&desc.plugin_id) {
                // Plugins save their opaque state blobs (as Audacity does), not parameter values.
                crate::plugin_bridge::saved_from_effect(slot.effect.as_mut(), slot.active)
            } else {
                Some(RealtimeEffect { id: desc.plugin_id, active: desc.active, params: desc.params })
            }
        })
        .collect();

    RealtimeEffectList { active, effects }
}
/// Builds engine stacks for every track plus master from `.aup3` extras.
pub fn stacks_for(extras: &Aup3Extras, n_tracks: usize) -> (Vec<Option<EffectStack>>, Option<EffectStack>) {
    let tracks = (0..n_tracks).map(|i| extras.track_effects.get(i).filter(|l| !l.effects.is_empty()).map(stack_from_list)).collect();
    let master = Some(&extras.master_effects).filter(|l| !l.effects.is_empty()).map(stack_from_list);
    (tracks, master)
}

/// Renders the whole project mix with realtime effects applied (export path).
pub fn render_mix(state: &ProjectState, extras: &Aup3Extras, master_gain: f32, channels: usize) -> AudioBuffer {
    let tracks: Vec<MixTrack> = crate::transport::mix_tracks(state);
    let (tfx, mfx) = stacks_for(extras, tracks.len());
    render_offline_with_effects(tracks, tfx, mfx, master_gain, channels, 4096)
}

/// Renders one track through its own realtime stack (single-track export).
pub fn render_track(state: &ProjectState, extras: &Aup3Extras, index: usize) -> Option<AudioBuffer> {
    let mut buf = state.tracks.get(index)?.render();
    if let Some(l) = extras.track_effects.get(index).filter(|l| !l.effects.is_empty()) {
        stack_from_list(l).render(&mut buf);
    }
    Some(buf)
}

/// Result of running a built-in destructively.
#[derive(Debug, Default)]
pub struct ApplyOutcome {
    /// Labels (absolute project seconds) produced by analyzers.
    pub labels: Vec<OutLabel>,
    /// Text report.
    pub report: String,
}

/// Applies a built-in (effect / generator / analyzer) to a selection through the
/// editor (one undo step). Analyzers do not modify audio; their labels are
/// shifted to absolute time and added to a new label track named after the effect.
pub fn apply_builtin(editor: &mut Editor, sel: Selection, effect: &mut dyn BuiltinEffect, aux: Option<&AudioBuffer>) -> Result<ApplyOutcome, String> {
    let sel = sel.normalized();
    let rate = editor.state().tracks.get(sel.track).ok_or("no such track")?.sample_rate().max(1) as f64;
    let t0 = sel.start as f64 / rate;
    let mut out = ApplyOutcome::default();
    if effect.kind() == EffectKind::Analyze {
        let track = &editor.state().tracks[sel.track];
        let mut buf = track.clips.render_range(sel.start, sel.end.max(sel.start)).map_err(|e| e.to_string())?;
        let mut ctx = EffectContext { aux, t0, ..Default::default() };
        au_effects::offline::process_isolated(effect, &mut buf, &mut ctx).map_err(|e| e.to_string())?;
        out.report = ctx.report;
        out.labels = ctx.labels.into_iter().map(|l| OutLabel { t0: l.t0 + t0, t1: l.t1 + t0, text: l.text }).collect();
        if !out.labels.is_empty() {
            let lt = editor.add_label_track(&effect.name());
            for l in &out.labels { editor.add_label(lt, l.t0, l.t1, &l.text).map_err(|e| e.to_string())?; }
        }
        return Ok(out);
    }
    let mut err = None;
    let mut report = String::new();
    editor.apply(sel, &effect.name(), |buf| {
        let mut ctx = EffectContext { aux, t0, ..Default::default() };
        // Runs on the shared au-runtime pool with panic isolation: a buggy effect reports an error instead of crashing.
        if let Err(e) = au_effects::offline::process_isolated(&mut *effect, buf, &mut ctx) { err = Some(e.to_string()); }
        report = ctx.report;
    }).map_err(|e| e.to_string())?;
    if let Some(e) = err { let _ = editor.undo(); return Err(e); }
    // Audio written into a 16/24-bit track is stored in that format (Audacity 3.7.3
    // verified for Generate > Tone into a 16-bit track: values land on the int16 grid).
    if editor.state().tracks.get(sel.track).is_some_and(|t| t.int_bits.is_some()) {
        let dither = hq_dither();
        editor.history.modify_current(|st| {
            if let Some(t) = st.tracks.get_mut(sel.track) {
                let len = t.len();
                quantize_track_range(t, sel.start, len, dither);
            }
        });
    }
    out.report = report;
    Ok(out)
}

/// Processes a copy of (at most `max_secs` of) a selection for preview playback.
pub fn preview(state: &ProjectState, sel: Selection, effect: &mut dyn BuiltinEffect, max_secs: f64) -> Result<AudioBuffer, String> {
    let sel = sel.normalized();
    let track = state.tracks.get(sel.track).ok_or("no such track")?;
    let rate = track.sample_rate().max(1);
    let end = sel.end.min(sel.start + (max_secs * rate as f64) as usize).max(sel.start);
    let mut buf = track.clips.render_range(sel.start, end).map_err(|e| e.to_string())?;
    au_effects::offline::process_isolated(effect, &mut buf, &mut EffectContext::default()).map_err(|e| e.to_string())?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use au_core::Track;
    use au_effects::registry;
    use au_io::engine::{Mixer, NullOutput, engine, EngineCommand};

    fn state() -> ProjectState {
        let s: Vec<f32> = (0..8000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        ProjectState { tracks: vec![Track::new("t", AudioBuffer::from_channels(8000, vec![s]))], sample_rate: 8000, ..Default::default() }
    }

    fn echo_list() -> RealtimeEffectList {
        let mut st = EffectStack::new();
        assert!(st.push("Echo"));
        au_effects::stack::params_from_strings(st.slots[0].effect.as_mut(), &[("Delay".into(), "0.1".into()), ("Decay".into(), "0.5".into())]);
        list_from_stack(&mut st)
    }

    #[test]
    fn list_round_trips_through_stack() {
        let l = echo_list();
        assert_eq!(l.effects[0].id, "Effect_Audacity_Audacity_Echo_Built-in Effect: Echo");
        assert_eq!(list_from_stack(&mut stack_from_list(&l)), l);
    }

    #[test]
    fn stack_survives_aup3_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("fx.aup3");
        let mut extras = Aup3Extras::default();
        extras.track_effects = vec![echo_list()];
        let mut master = EffectStack::new(); master.push("Amplify"); master.push("Reverb");
        master.slots[1].active = false;
        extras.master_effects = list_from_stack(&mut master);
        au_project::aup3::save(&p, &au_project::aup3::Aup3Project { state: state(), extras: extras.clone() }).unwrap();
        let back = au_project::aup3::load(&p).unwrap();
        assert_eq!(back.extras.track_effects, extras.track_effects);
        assert_eq!(back.extras.master_effects, extras.master_effects);
    }

    #[test]
    fn export_render_applies_track_stack_nondestructively() {
        let st = state();
        let mut extras = Aup3Extras::default();
        extras.track_effects = vec![echo_list()];
        let out = render_track(&st, &extras, 0).unwrap();
        let mut expect = st.tracks[0].render();
        let mut e = registry::create("Echo").unwrap();
        au_effects::stack::params_from_strings(e.as_mut(), &[("Delay".into(), "0.1".into()), ("Decay".into(), "0.5".into())]);
        e.process(&mut expect, &mut EffectContext::default()).unwrap();
        assert_eq!(out.channels, expect.channels);
        assert_ne!(st.tracks[0].render().channels, out.channels); // project audio untouched
        let mix = render_mix(&st, &extras, 1.0, 1);
        assert_eq!(mix.len(), out.len());
    }

    #[test]
    fn engine_plays_through_track_and_master_stacks() {
        let st = state();
        let mut m = Mixer::new(crate::transport::mix_tracks(&st), 1.0);
        let mut amp = EffectStack::new(); amp.push("Amplify");
        au_effects::stack::params_from_strings(amp.slots[0].effect.as_mut(), &[("Ratio".into(), "0.5".into())]);
        m.set_track_effects(0, Some(amp));
        let mut inv = EffectStack::new(); inv.push("Invert");
        m.set_master_effects(Some(inv));
        let (mut h, mut cb) = engine(m);
        let mut out = NullOutput { sample_rate: 8000, channels: 1, block: 256, captured: Vec::new() };
        h.send(EngineCommand::Play);
        out.pump(&mut cb, 4);
        let src = st.tracks[0].render();
        // mono track centre pan at unity, mono out = (l+r)/2 = sample
        for i in 0..1024 { assert!((out.captured[i] + 0.5 * src.channels[0][i]).abs() < 1e-5, "frame {i}"); }
    }

    #[test]
    fn apply_builtin_effect_generator_analyzer() {
        let mut ed = Editor::new(state());
        let sel = Selection { track: 0, start: 0, end: 4000 };
        let mut tone = registry::create("Tone").unwrap();
        apply_builtin(&mut ed, sel, tone.as_mut(), None).unwrap();
        assert!((ed.state().tracks[0].render().channels[0][1] - (2.0 * std::f32::consts::PI * 440.0 / 8000.0).sin() * 0.8).abs() < 1e-3);
        let mut fc = registry::create("FindClipping").unwrap();
        let r = apply_builtin(&mut ed, sel, fc.as_mut(), None).unwrap();
        assert!(r.labels.is_empty());
        let mut inv = registry::create("Invert").unwrap();
        let before = ed.state().tracks[0].render();
        apply_builtin(&mut ed, sel, inv.as_mut(), None).unwrap();
        assert_eq!(ed.state().tracks[0].render().channels[0][10], -before.channels[0][10]);
        ed.undo().unwrap();
        assert_eq!(ed.state().tracks[0].render().channels[0][10], before.channels[0][10]);
        let mut duck = registry::create("AutoDuck").unwrap();
        assert!(apply_builtin(&mut ed, sel, duck.as_mut(), None).is_err());
    }
}

/// Quality > High-quality conversion dither, read when audio is written into
/// an integer track (Audacity's Dither::BestSetting, default Shaped).
pub fn hq_dither() -> au_io::export::intfmt::AudDither {
    use au_io::export::intfmt::AudDither;
    match HQ_DITHER.load(std::sync::atomic::Ordering::Relaxed) {
        0 => AudDither::None,
        1 => AudDither::Rectangle,
        2 => AudDither::Triangle,
        _ => AudDither::Shaped,
    }
}

static HQ_DITHER: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(3);

/// Sets the dither used when writing into integer tracks (from Preferences).
pub fn set_hq_dither(symbol: &str) {
    let d = match symbol {
        "None" => 0,
        "Rectangle" => 1,
        "Triangle" => 2,
        _ => 3,
    };
    HQ_DITHER.store(d, std::sync::atomic::Ordering::Relaxed);
}

/// Converts `[start, end)` of an integer-format track to its format with
/// `dither`, as Audacity does when float audio is written into a 16/24-bit track.
pub fn quantize_track_range(t: &mut au_core::Track, start: usize, end: usize, dither: au_io::export::intfmt::AudDither) {
    let Some(bits) = t.int_bits else { return };
    if dither == au_io::export::intfmt::AudDither::None {
        t.quantize_range(start, end);
        return;
    }
    let scale = (1u32 << (bits - 1)) as f32;
    for clip in &mut t.clips.clips {
        let (c0, c1) = (clip.start, clip.end());
        if c1 <= start || c0 >= end || (clip.stretch_ratio - 1.0).abs() > f64::EPSILON {
            continue;
        }
        let a = start.saturating_sub(c0);
        let b = (end - c0).min(clip.buffer.len());
        for ch in &mut clip.buffer.channels {
            let e = b.min(ch.len());
            let seg = &mut ch[a.min(e)..e];
            let mut ints = vec![0i32; seg.len()];
            let mut state = au_io::export::intfmt::DitherState::new();
            state.convert(seg, bits, dither, &mut ints);
            for (v, i) in seg.iter_mut().zip(ints) {
                *v = i as f32 / scale;
            }
        }
    }
}
