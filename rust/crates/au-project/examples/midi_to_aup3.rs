//! Verification helper: `midi_to_aup3 <in.mid> <out.aup3>` writes a project with one note track.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let track = au_io::midi::import_midi(std::path::Path::new(&a[1])).expect("import MIDI");
    let state = au_core::ProjectState { sample_rate: 44100, note_tracks: vec![track], ..Default::default() };
    let project = au_project::aup3::Aup3Project { state, extras: Default::default() };
    au_project::aup3::save(std::path::Path::new(&a[2]), &project).expect("save aup3");
}
