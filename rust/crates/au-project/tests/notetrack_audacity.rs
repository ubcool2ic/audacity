use std::fs;
use std::path::{Path, PathBuf};

use au_core::notes::{Note, NoteTrack};

const EXPECTED_NOTES: [(f64, f64, f64, f64, i32); 3] = [
    (0.0, 0.5, 60.0, 100.0, 0),
    (0.5, 0.25, 64.0, 90.0, 0),
    (0.75, 0.75, 67.0, 80.0, 1),
];

fn fixture_path(filename: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/aup3")
        .join(filename)
}

fn copy_aup3_fixture(filename: &str, dir: &tempfile::TempDir) -> PathBuf {
    let destination = dir.path().join(filename);
    fs::copy(fixture_path(filename), &destination).unwrap();
    destination
}

fn assert_close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, got {actual} (tolerance {tolerance})"
    );
}

fn assert_expected_notes(notes: &[Note], tolerance: f64) {
    assert_eq!(notes.len(), EXPECTED_NOTES.len());

    for (index, (note, expected)) in notes.iter().zip(EXPECTED_NOTES).enumerate() {
        let (start, duration, pitch, loudness, channel) = expected;
        assert_close(note.start, start, tolerance);
        assert_close(note.duration, duration, tolerance);
        assert_close(note.pitch, pitch, tolerance);
        assert_close(note.loudness, loudness, tolerance);
        assert_eq!(note.channel, channel, "channel mismatch for note {index}");
    }
}

fn assert_expected_track(track: &NoteTrack) {
    assert_eq!(track.name, "ref");
    assert_expected_notes(&track.notes, 1e-4);
}

#[test]
fn reads_note_track_written_by_audacity() {
    let temp_dir = tempfile::tempdir().unwrap();
    let project_path = copy_aup3_fixture("notetrack_audacity373.aup3", &temp_dir);
    let project = au_project::aup3::load(&project_path).unwrap();

    assert_eq!(project.state.note_tracks.len(), 1);
    assert_expected_track(&project.state.note_tracks[0]);
}

#[test]
fn audacity_reads_and_rewrites_our_note_track() {
    let temp_dir = tempfile::tempdir().unwrap();
    let project_path =
        copy_aup3_fixture("notetrack_ours_resaved_by_audacity373.aup3", &temp_dir);
    let project = au_project::aup3::load(&project_path).unwrap();

    assert_eq!(project.state.note_tracks.len(), 1);
    assert_expected_track(&project.state.note_tracks[0]);
}

#[test]
fn midi_import_matches_audacity_import() {
    let temp_dir = tempfile::tempdir().unwrap();
    let project_path = copy_aup3_fixture("notetrack_audacity373.aup3", &temp_dir);
    let project = au_project::aup3::load(&project_path).unwrap();
    let midi_track =
        au_io::midi::import_midi(&fixture_path("notetrack_ref.mid")).unwrap();

    assert_eq!(project.state.note_tracks.len(), 1);
    assert_expected_notes(&midi_track.notes, 1e-4);
    assert_expected_notes(&project.state.note_tracks[0].notes, 1e-4);

    assert_eq!(midi_track.notes.len(), project.state.note_tracks[0].notes.len());
    for (midi_note, project_note) in midi_track
        .notes
        .iter()
        .zip(&project.state.note_tracks[0].notes)
    {
        assert_close(midi_note.start, project_note.start, 1e-4);
        assert_close(midi_note.duration, project_note.duration, 1e-4);
        assert_close(midi_note.pitch, project_note.pitch, 1e-4);
        assert_close(midi_note.loudness, project_note.loudness, 1e-4);
        assert_eq!(midi_note.channel, project_note.channel);
    }
}
