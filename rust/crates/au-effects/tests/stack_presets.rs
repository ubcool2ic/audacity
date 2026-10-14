// SPDX-License-Identifier: GPL-3.0-or-later
//! Stage 5 acceptance: preset and effect-stack save/load preserve order and values;
//! every built-in survives defaults / empty / short input / boundary values.
use au_core::AudioBuffer;
use au_effects::core::*;
use au_effects::registry;
use au_effects::stack::*;

fn sine(n: usize) -> AudioBuffer {
    let c: Vec<f32> = (0..n).map(|i| (i as f32 * 0.07).sin() * 0.5).collect();
    AudioBuffer::from_channels(44100, vec![c.clone(), c])
}

#[test]
fn registry_ids_unique_and_creatable() {
    let all = registry::all();
    assert!(all.len() >= 50, "{} built-ins", all.len());
    let mut ids: Vec<_> = all.iter().map(|e| e.id()).collect();
    ids.sort();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate ids");
    for id in ids { assert_eq!(registry::create(&id).unwrap().id(), id); }
    // Audacity scripting ids resolve too
    for alias in ["ChangeSpeedAndPitch", "DtmfTones", "FilterCurve", "GraphicEq", "MeasureRms"] {
        assert!(registry::create(alias).is_some(), "{alias}");
    }
}

#[test]
fn every_builtin_handles_edge_inputs_without_panic() {
    for mut e in registry::all() {
        let id = e.id().into_owned();
        let id = id.as_str();
        let aux = sine(2000);
        for n in [0usize, 1, 7, 3000] {
            for extreme in [false, true] {
                let mut e2 = registry::create(id).unwrap();
                if extreme { for p in e2.params() { set_param(e2.as_mut(), p.key, p.max); } }
                for k in ["Duration", "DUR", "BARS", "DECAY"] { set_param(e2.as_mut(), k, 0.05); }
                if id == "NoiseReduction" { let _ = e2.learn_profile(&sine(44100)); }
                // keep generators/slow effects short
                if matches!(id, "Paulstretch" | "Repeat") && extreme { continue; }
                let mut b = sine(n);
                let mut ctx = EffectContext { aux: Some(&aux), ..Default::default() };
                let _ = e2.process(&mut b, &mut ctx);
                for c in &b.channels { assert!(c.iter().all(|v| v.is_finite()), "{id} n={n} extreme={extreme} non-finite"); }
            }
        }
        reset_defaults(e.as_mut());
        for p in e.params() { assert_eq!(e.get(p.key), Some(p.default), "{id}.{} default", p.key); }
    }
}

#[test]
fn stack_desc_round_trip_preserves_order_and_values() {
    let mut st = EffectStack::new();
    for id in ["Echo", "Phaser", "Reverb", "Compressor"] { assert!(st.push(id), "{id}"); }
    assert!(!st.push("Paulstretch"), "offline-only effects are rejected");
    params_from_strings(st.slots[0].effect.as_mut(), &[("Delay".into(), "0.25".into())]);
    params_from_strings(st.slots[2].effect.as_mut(), &[("RoomSize".into(), "33".into())]);
    st.slots[1].active = false;
    st.move_slot(3, 0);
    let (active, desc) = st.describe();
    let (mut back, missing) = EffectStack::from_desc(active, &desc);
    assert!(missing.is_empty());
    assert_eq!(back.describe(), (active, desc.clone()));
    let order: Vec<_> = back.slots.iter().map(|s| s.effect.id()).collect();
    assert_eq!(order, ["Compressor", "Echo", "Phaser", "Reverb"]);
    assert_eq!(back.slots[1].effect.get("Delay"), Some(0.25));
    assert!(!back.slots[2].active);
    assert_eq!(symbol_from_plugin_id(&desc[0].plugin_id).as_deref(), Some("Compressor"));
}

#[test]
fn stack_render_equals_sequential_offline_processing_for_zero_latency_chain() {
    let mut st = EffectStack::new();
    st.push("Amplify"); st.push("Echo");
    params_from_strings(st.slots[0].effect.as_mut(), &[("Ratio".into(), "0.5".into())]);
    params_from_strings(st.slots[1].effect.as_mut(), &[("Delay".into(), "0.01".into())]);
    let mut a = sine(20000);
    st.render(&mut a);
    let mut b = sine(20000);
    for (id, k, v) in [("Amplify", "Ratio", 0.5), ("Echo", "Delay", 0.01)] {
        let mut e = registry::create(id).unwrap();
        set_param(e.as_mut(), k, v);
        e.process(&mut b, &mut EffectContext::default()).unwrap();
    }
    assert_eq!(a.channels, b.channels);
    // bypassed stack leaves audio untouched
    st.active = false;
    let mut c = sine(100);
    st.render(&mut c);
    assert_eq!(c.channels, sine(100).channels);
}

#[test]
fn presets_factory_and_user_round_trip() {
    assert_eq!(factory_presets("Reverb").len(), 11);
    assert!(factory_presets("Compressor").iter().any(|p| p.name == "Podcast/Radio"));
    assert!(factory_presets("Limiter").len() >= 3);
    assert!(factory_presets("Distortion").iter().any(|p| p.name == "Fuzz Box"));
    assert!(factory_presets("Echo").is_empty(), "Audacity ships no Echo presets");
    for id in ["Reverb", "Compressor", "Limiter", "Distortion"] {
        for p in factory_presets(id) {
            let mut e = registry::create(id).unwrap();
            p.apply(e.as_mut());
            let cap = Preset::capture(&p.name, e.as_mut());
            for (k, v) in &p.params {
                if let Some(def) = e.params().into_iter().find(|d| d.key.eq_ignore_ascii_case(k)) {
                    let got = cap.params.iter().find(|(ck, _)| ck == def.key).unwrap();
                    if let Ok(x) = v.parse::<f64>() { assert!((got.1.parse::<f64>().unwrap_or(f64::NAN) - def.clamp(x)).abs() < 1e-9 || matches!(def.ty, ParamType::Bool | ParamType::Choice(_)), "{id}/{}: {k}", p.name); }
                }
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut e = registry::create("Phaser").unwrap();
    set_param(e.as_mut(), "Stages", 6.0);
    set_param(e.as_mut(), "Freq", 1.25);
    let p = Preset::capture("My \"wide\" phaser", e.as_mut());
    save_user_preset(dir.path(), "Phaser", &p).unwrap();
    save_user_preset(dir.path(), "Phaser", &Preset::capture("A first", e.as_mut())).unwrap();
    let list = load_user_presets(dir.path(), "Phaser");
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].name, "A first");
    let mut f = registry::create("Phaser").unwrap();
    list[1].apply(f.as_mut());
    assert_eq!(get_params(f.as_ref()), get_params(e.as_mut()));
    delete_user_preset(dir.path(), "Phaser", "A first").unwrap();
    assert_eq!(load_user_presets(dir.path(), "Phaser").len(), 1);
}
