// SPDX-License-Identifier: GPL-3.0-or-later
//! `aup3_tool roundtrip IN OUT` | `aup3_tool dump IN` | `aup3_tool autosave-edit IN`
//! Used by tools/verify_in_audacity.sh to hand Rust-written files to real Audacity.
use au_project::aup3::{self, DocSource};
use au_core::timeline::{Clip, EnvPoint, Label, LabelTrack, WaveClips};
use au_core::{AudioBuffer, ProjectState, Track};
use au_project::aup3::{Aup3Extras, Aup3Project};
use std::path::Path;

/// A project built entirely in Rust: stereo, gain/pan, two clips (one
/// time-stretched), an envelope point inside the second clip, labels.
fn synth() -> Aup3Project {
    let sine = |f: f32, n: usize| (0..n).map(|i| 0.4 * (i as f32 * f * std::f32::consts::TAU / 44100.0).sin()).collect::<Vec<_>>();
    let mut wc = WaveClips::new(44100, 2);
    wc.clips.push(Clip::new("A", 0, AudioBuffer::from_channels(44100, vec![sine(330.0, 22050), sine(660.0, 22050)])));
    let mut b = Clip::new("B", 44100, AudioBuffer::from_channels(44100, vec![sine(500.0, 44100), sine(250.0, 44100)]));
    b.stretch_ratio = 1.25;
    wc.clips.push(b);
    wc.envelope.points.push(EnvPoint { time: 1.5, value: 0.7 });
    let mut t = Track::new("RustMade", AudioBuffer::new(44100, 2));
    t.clips = wc;
    t.gain = 0.5;
    t.pan = -0.3;
    let mut lt = LabelTrack::new("Marks");
    lt.labels.push(Label { start: 0.1, end: 0.4, text: "região ✓".into() });
    Aup3Project {
        state: ProjectState { tracks: vec![t], sample_rate: 44100, labels: vec![lt], sync_lock: false, note_tracks: Vec::new() },
        extras: Aup3Extras { selection: (0.1, 0.3), track_formats: vec![au_project::blocks::SampleFormat::Int24], track_selected: vec![true], ..Default::default() },
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let r = match a.get(1).map(String::as_str) {
        Some("roundtrip") => aup3::load(Path::new(&a[2])).and_then(|p| aup3::save(Path::new(&a[3]), &p)),
        Some("dump") => aup3::dump_xml(Path::new(&a[2]), DocSource::Saved).map(|x| println!("{x}")),
        Some("autosave-edit") => aup3::load(Path::new(&a[2])).and_then(|mut p| {
            p.state.tracks[0].name = "Recovered".into();
            aup3::autosave(Path::new(&a[2]), &p)
        }),
        Some("eq") => aup3::load(Path::new(&a[2])).and_then(|x| {
            let y = aup3::load(Path::new(&a[3]))?;
            println!("state equal: {}  selection {:?} vs {:?}  formats {:?} vs {:?}", x.state == y.state,
                x.extras.selection, y.extras.selection, x.extras.track_formats, y.extras.track_formats);
            Ok(())
        }),
        Some("synth") => aup3::save(Path::new(&a[2]), &synth()),
        _ => {
            eprintln!("usage: aup3_tool roundtrip IN OUT | dump IN | autosave-edit IN");
            std::process::exit(2)
        }
    };
    if let Err(e) = r {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
