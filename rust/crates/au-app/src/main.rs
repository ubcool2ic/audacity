// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.
//! `audacity-rs` entry point.
//!
//! * `audacity-rs` (built with `--features gui`) opens the egui editor.
//! * `audacity-rs --selftest` runs a headless edit pipeline (CI-friendly, no audio device).
//! * `audacity-rs process <in> <out.wav|out.flac> [effect...]` applies effects headlessly.
//!   Effects: amplify:<dB>, normalize:<dB>, fadein, fadeout, reverse.

use au_core::{AudioBuffer, Editor, ProjectState, Selection, Track};
use au_effects::{Amplify, Effect, FadeIn, FadeOut, Normalize, Reverse};
use std::path::Path;

pub mod fx_bridge;
pub mod plugin_bridge;
pub mod transport;
#[cfg(feature = "gui")]
mod gui;
#[cfg(feature = "gui")]
mod script_bridge;
#[cfg(feature = "gui")]
mod plugin_manager;
#[cfg(feature = "gui")]
mod spectrogram_view;
#[cfg(feature = "gui")]
mod fx_ui;
#[cfg(feature = "gui")]
mod analyze_ui;
#[cfg(feature = "gui")]
mod room_noise;

/// Crash-report folder (`AUDACITY_RS_CRASH_DIR` overrides the per-OS default).
pub fn crash_dir() -> std::path::PathBuf {
    std::env::var_os("AUDACITY_RS_CRASH_DIR").map(Into::into).unwrap_or_else(au_runtime::crash::default_crash_dir)
}

fn main() {
    au_runtime::crash::install(au_runtime::crash::CrashConfig {
        dir: crash_dir(),
        app_name: "audacity-rs".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    });
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("--selftest") => report(selftest()),
        Some("process") if args.len() >= 3 => report(process(&args[1], &args[2], &args[3..])),
        Some("aup3-info") if args.len() == 2 => report(aup3_info(&args[1])),
        Some("--help") | Some("-h") => {
            println!("usage: audacity-rs [--selftest | aup3-info <file.aup3> | process <in> <out.wav|out.flac> [amplify:<dB>|normalize:<dB>|fadein|fadeout|reverse]...]");
            0
        }
        Some(f) if !f.starts_with('-') && std::path::Path::new(f).exists() => run_gui_with(Some(f.into())),
        _ => run_gui(),
    };
    std::process::exit(code);
}

/// Prints what is in an Audacity `.aup3` project and whether it needs recovery.
fn aup3_info(path: &str) -> Result<String, String> {
    use au_project::aup3;
    let p = std::path::Path::new(path);
    let info = aup3::inspect(p).map_err(|e| e.to_string())?;
    let src = if info.needs_recovery { aup3::DocSource::Autosave } else { aup3::DocSource::Saved };
    let proj = aup3::load_from(p, src).map_err(|e| e.to_string())?;
    let mut out = format!(
        "format {}.{}.{}  rate {}  needs_recovery {}  integrity {}\n",
        info.user_version >> 24, (info.user_version >> 16) & 0xff, (info.user_version >> 8) & 0xff,
        proj.state.sample_rate, info.needs_recovery, aup3::integrity_check(p).map_err(|e| e.to_string())?
    );
    for (i, t) in proj.state.tracks.iter().enumerate() {
        out += &format!(
            "track {i} {:?}: {} ch, {} clips, {:?}, gain {} pan {} mute {} solo {}\n",
            t.name, t.clips.n_channels, t.clips.clips.len(), proj.extras.track_formats[i], t.gain, t.pan, t.mute, t.solo
        );
    }
    for l in &proj.state.labels {
        out += &format!("labels {:?}: {}\n", l.name, l.labels.len());
    }
    Ok(out.trim_end().to_owned())
}

fn report(r: Result<String, String>) -> i32 {
    match r {
        Ok(msg) => {
            println!("{msg}");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(feature = "gui")]
fn run_gui() -> i32 {
    run_gui_with(None)
}

#[cfg(feature = "gui")]
fn run_gui_with(open: Option<std::path::PathBuf>) -> i32 {
    match gui::run_with(open) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(not(feature = "gui"))]
fn run_gui_with(_open: Option<std::path::PathBuf>) -> i32 {
    run_gui()
}

#[cfg(not(feature = "gui"))]
fn run_gui() -> i32 {
    eprintln!("built without the `gui` feature; rebuild with `cargo run -p au-app --features gui`, or use --selftest / process");
    2
}

fn parse_effect(spec: &str) -> Result<Box<dyn Effect>, String> {
    let (name, arg) = spec.split_once(':').unwrap_or((spec, ""));
    let num = |d: f32| if arg.is_empty() { Ok(d) } else { arg.parse::<f32>().map_err(|e| e.to_string()) };
    Ok(match name {
        "amplify" => Box::new(Amplify::new(num(0.0)?)),
        "normalize" => Box::new(Normalize { peak_db: num(-1.0)?, remove_dc: true }),
        "fadein" => Box::new(FadeIn),
        "fadeout" => Box::new(FadeOut),
        "reverse" => Box::new(Reverse),
        other => return Err(format!("unknown effect {other}")),
    })
}

fn process(input: &str, output: &str, effects: &[String]) -> Result<String, String> {
    let buf = au_io::import_file(Path::new(input)).map_err(|e| e.to_string())?;
    let mut editor = Editor::new(ProjectState { tracks: vec![], sample_rate: buf.sample_rate, ..Default::default() });
    let frames = buf.len();
    editor.add_track(Track::new("input", buf));
    let sel = Selection { track: 0, start: 0, end: frames };
    for spec in effects {
        let mut fx = parse_effect(spec)?;
        editor.apply(sel, spec, |b: &mut AudioBuffer| fx.process(b)).map_err(|e| e.to_string())?;
    }
    let out = &editor.state().tracks[0].render();
    let lower = output.to_ascii_lowercase();
    if lower.ends_with(".flac") {
        au_io::export_flac(Path::new(output), out, 16).map_err(|e| e.to_string())?;
    } else {
        au_io::export_wav(Path::new(output), out, 16).map_err(|e| e.to_string())?;
    }
    Ok(format!("wrote {output}: {} frames, {} ch, {} Hz, peak {:.3}", out.len(), out.n_channels(), out.sample_rate, out.peak()))
}

fn selftest() -> Result<String, String> {
    let rate = 44_100;
    let tone: Vec<f32> = (0..rate).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.25).collect();
    let mut editor = Editor::new(ProjectState { tracks: vec![], sample_rate: rate as u32, ..Default::default() });
    editor.add_track(Track::new("tone", AudioBuffer::from_channels(rate as u32, vec![tone])));
    let e = |x: au_core::CoreError| x.to_string();
    editor.cut(Selection { track: 0, start: 0, end: 1000 }).map_err(e)?;
    editor.paste(0, 0).map_err(e)?;
    editor
        .apply(Selection { track: 0, start: 0, end: rate }, "Normalize", |b| Normalize { peak_db: -1.0, remove_dc: false }.process(b))
        .map_err(e)?;
    editor.undo().map_err(e)?;
    editor.redo().map_err(e)?;
    let dir = std::env::temp_dir().join(format!("au-selftest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|x| x.to_string())?;
    let wav = dir.join("t.wav");
    au_io::export_wav(&wav, &editor.state().tracks[0].render(), 16).map_err(|x| x.to_string())?;
    let back = au_io::import_file(&wav).map_err(|x| x.to_string())?;
    let proj = dir.join("t.aurs");
    au_project::save_project(&proj, editor.state()).map_err(|x| x.to_string())?;
    let loaded = au_project::load_project(&proj).map_err(|x| x.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    if back.len() != rate || loaded.tracks.len() != 1 {
        return Err("selftest mismatch".into());
    }
    Ok(format!("selftest ok: {} frames, peak {:.3} (expected ~0.891)", back.len(), back.peak()))
}
