use std::path::Path;

use au_core::{notes::NoteTrack, ProjectState};
use au_project::aup3::{self, Aup3Extras, Aup3Project, DocSource};

fn midi_fixture() -> Vec<u8> {
    let tempo_track = [
        0, 0xFF, 0x51, 3, 0x07, 0xA1, 0x20, // 120 BPM
        0, 0xFF, 0x58, 4, 4, 2, 24, 8, // 4/4
        0, 0xFF, 0x2F, 0,
    ];
    let note_track = [
        0, 0xFF, 0x03, 5, b'N', b'o', b't', b'e', b's', // track name
        0, 0x90, 60, 100, // C4 on
        0x83, 0x60, 0x80, 60, 0, // C4 off at 0.5 s
        0, 0x90, 64, 90, // E4 on
        0x81, 0x70, 0x80, 64, 0, // E4 off at 0.75 s
        0, 0x91, 67, 80, // G4 on, channel 1
        0x85, 0x50, 0x81, 67, 0, // G4 off at 1.5 s
        0, 0xFF, 0x2F, 0,
    ];

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"MThd");
    bytes.extend_from_slice(&6_u32.to_be_bytes());
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&2_u16.to_be_bytes());
    bytes.extend_from_slice(&480_u16.to_be_bytes());

    bytes.extend_from_slice(b"MTrk");
    bytes.extend_from_slice(&(tempo_track.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&tempo_track);

    bytes.extend_from_slice(b"MTrk");
    bytes.extend_from_slice(&(note_track.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&note_track);
    bytes
}

fn assert_notes_equal_with_tolerance(actual: &NoteTrack, expected: &NoteTrack) {
    assert_eq!(actual.notes.len(), expected.notes.len());
    for (left, right) in actual.notes.iter().zip(&expected.notes) {
        assert!((left.start - right.start).abs() < 1.0e-4);
        assert!((left.duration - right.duration).abs() < 1.0e-4);
        assert_eq!(left.pitch, right.pitch);
        assert_eq!(left.loudness, right.loudness);
        assert_eq!(left.channel, right.channel);
    }
}

#[test]
fn midi_aup3_native_and_midi_round_trips_preserve_notes() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let midi_path = directory.path().join("source.mid");
    std::fs::write(&midi_path, midi_fixture()).expect("write MIDI fixture");

    let track = au_io::midi::import_midi(&midi_path).expect("import fixture MIDI");
    assert_eq!(track.name, "source");
    assert_eq!(track.notes.len(), 3);

    let state = ProjectState {
        sample_rate: 44_100,
        note_tracks: vec![track.clone()],
        ..ProjectState::default()
    };

    let aup3_path = directory.path().join("roundtrip.aup3");
    aup3::save(
        &aup3_path,
        &Aup3Project {
            state: state.clone(),
            extras: Aup3Extras::default(),
        },
    )
    .expect("save AUP3 project");
    let loaded_aup3 = aup3::load(&aup3_path).expect("load AUP3 project");
    let aup3_track = &loaded_aup3.state.note_tracks[0];
    assert_eq!(aup3_track.name, track.name);
    assert_eq!(aup3_track.offset, track.offset);
    assert_notes_equal_with_tolerance(aup3_track, &track);

    let exported_midi = directory.path().join("exported.mid");
    au_io::midi::export_midi(&exported_midi, aup3_track).expect("export MIDI");
    let imported_again =
        au_io::midi::import_midi(&exported_midi).expect("re-import exported MIDI");
    assert_notes_equal_with_tolerance(&imported_again, &track);

    let native_path = directory.path().join("roundtrip.aurs");
    au_project::save_project(&native_path, &state).expect("save native project");
    let loaded_native = au_project::load_project(&native_path).expect("load native project");
    let native_track = &loaded_native.note_tracks[0];
    assert_eq!(native_track.name, track.name);
    assert_eq!(native_track.offset, track.offset);
    assert_notes_equal_with_tolerance(native_track, &track);
}

#[test]
fn audacity_allegro_notetrack_is_written_to_aup3_xml() {
    let allegro = "#offset 0\n\
#track 0\n\
T0.0000 -tempor:120\n\
T1.5000 -tempor:120\n\
T0.0000 V0 K60 P60 U0.5000 L100\n\
T0.5000 V0 K64 P64 U0.2500 L90\n\
T0.7500 V1 K67 P67 U0.7500 L80\n";
    let track = NoteTrack::from_allegro(allegro).expect("parse Audacity Allegro data");
    assert_eq!(track.notes.len(), 3);

    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("allegro.aup3");
    let state = ProjectState {
        sample_rate: 44_100,
        note_tracks: vec![track],
        ..ProjectState::default()
    };
    aup3::save(
        &path,
        &Aup3Project {
            state,
            extras: Aup3Extras::default(),
        },
    )
    .expect("save project");

    let xml = aup3::dump_xml(&path, DocSource::Saved).expect("dump saved XML");
    assert!(xml.contains("<notetrack name="));
    assert!(xml.contains("U0.5000"));
}

#[allow(dead_code)]
fn _path_is_used_as_expected(_: &Path) {}

