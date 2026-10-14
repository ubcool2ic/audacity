// SPDX-License-Identifier: GPL-3.0-or-later
//! Verification helper: apply built-in effects to WAVs and extract audio from .aup3.
//! fx_tool apply <id> in.wav out.wav [Key=value ...] [aux=path.wav] [profile=noise.wav]
//! fx_tool gen <id> out.wav <rate> <channels> <seconds> [Key=value ...]
//! fx_tool analyze <id> in.wav [Key=value ...]
//! fx_tool aup3wav in.aup3 out.wav [track]
use au_effects::core::*;
use au_effects::registry;
use std::path::Path;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let kv = |rest: &[String], e: &mut dyn BuiltinEffect| -> (Option<String>, Option<String>) {
        let (mut aux, mut prof) = (None, None);
        let mut strs = Vec::new();
        for s in rest {
            let (k, v) = s.split_once('=').expect("Key=value");
            match k { "aux" => aux = Some(v.to_string()), "profile" => prof = Some(v.to_string()), _ => strs.push((k.to_string(), v.to_string())) }
        }
        au_effects::stack::params_from_strings(e, &strs);
        (aux, prof)
    };
    match a[1].as_str() {
        "apply" | "analyze" => {
            let mut e = registry::create(&a[2]).expect("unknown effect");
            let mut buf = au_io::import_file(Path::new(&a[3])).unwrap();
            let rest = if a[1] == "apply" { &a[5..] } else { &a[4..] };
            let (aux, prof) = kv(rest, e.as_mut());
            let auxb = aux.map(|p| au_io::import_file(Path::new(&p)).unwrap());
            if let Some(p) = prof {
                let nb = au_io::import_file(Path::new(&p)).unwrap();
                e.learn_profile(&nb).unwrap();
            }
            let mut ctx = EffectContext { aux: auxb.as_ref(), ..Default::default() };
            e.process(&mut buf, &mut ctx).unwrap();
            if a[1] == "apply" { au_io::export_wav(Path::new(&a[4]), &buf, 32).unwrap(); }
            for l in &ctx.labels { println!("LABEL\t{:.6}\t{:.6}\t{}", l.t0, l.t1, l.text); }
            if !ctx.report.is_empty() { println!("{}", ctx.report); }
        }
        "gen" => {
            let mut e = registry::create(&a[2]).expect("unknown");
            let rate: u32 = a[4].parse().unwrap(); let ch: usize = a[5].parse().unwrap(); let secs: f64 = a[6].parse().unwrap();
            kv(&a[7..], e.as_mut());
            let n = (secs * rate as f64).round() as usize;
            let mut buf = au_core::AudioBuffer::from_channels(rate, vec![vec![0.0; n]; ch]);
            e.process(&mut buf, &mut EffectContext::default()).unwrap();
            au_io::export_wav(Path::new(&a[3]), &buf, 32).unwrap();
        }
        "aup3wav" => {
            let p = au_project::aup3::load(Path::new(&a[2])).unwrap();
            let ti: usize = a.get(4).map(|s| s.parse().unwrap()).unwrap_or(0);
            let buf = p.state.tracks[ti].clips.render();
            au_io::export_wav(Path::new(&a[3]), &buf, 32).unwrap();
            for lt in &p.state.labels { for l in &lt.labels { println!("LABEL\t{:.6}\t{:.6}\t{}", l.start, l.end, l.text); } }
        }
        "list" => {
            for e in registry::all() {
                let ps: Vec<String> = e.params().iter().map(|p| format!("{}={}", p.key, p.default)).collect();
                println!("{:?}\t{}\t{}\t{}", e.kind(), e.id(), e.supports_realtime(), ps.join(" "));
            }
        }
        _ => eprintln!("unknown command"),
    }
}
