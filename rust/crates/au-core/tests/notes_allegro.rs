use au_core::notes::NoteTrack;

const AUDACITY_DATA: &str = "#offset 0\n\
#track 0\n\
T0.0000 -tempor:120\n\
T1.5000 -tempor:120\n\
T0.0000 V0 K60 P60 U0.5000 L100\n\
T0.5000 V0 K64 P64 U0.2500 L90\n\
T0.7500 V1 K67 P67 U0.7500 L80\n";

fn assert_notes_close(left: &NoteTrack, right: &NoteTrack) {
    assert_eq!(left.notes.len(), right.notes.len());
    for (a, b) in left.notes.iter().zip(&right.notes) {
        assert!((a.start - b.start).abs() < 1.0e-9);
        assert!((a.duration - b.duration).abs() < 1.0e-9);
        assert!((a.pitch - b.pitch).abs() < 1.0e-9);
        assert!((a.loudness - b.loudness).abs() < 1.0e-9);
        assert_eq!(a.channel, b.channel);
    }
}

#[test]
fn parses_audacity_allegro_notes() {
    let track = NoteTrack::from_allegro(AUDACITY_DATA).expect("parse Allegro data");
    assert_eq!(track.notes.len(), 3);

    let expected = [
        (0.0, 0.5, 60.0, 100.0, 0),
        (0.5, 0.25, 64.0, 90.0, 0),
        (0.75, 0.75, 67.0, 80.0, 1),
    ];
    for (note, (start, duration, pitch, loudness, channel)) in
        track.notes.iter().zip(expected.into_iter())
    {
        assert!((note.start - start).abs() < 1.0e-9);
        assert!((note.duration - duration).abs() < 1.0e-9);
        assert!((note.pitch - pitch).abs() < 1.0e-9);
        assert!((note.loudness - loudness).abs() < 1.0e-9);
        assert_eq!(note.channel, channel);
    }
}

#[test]
fn allegro_output_reparses_and_contains_expected_lines() {
    let track = NoteTrack::from_allegro(AUDACITY_DATA).expect("parse Allegro data");
    let encoded = track.to_allegro();
    assert!(encoded.contains("#track 0"));
    assert!(encoded.contains("T0.0000 V0 K60 P60 U0.5000 L100"));
    assert_notes_close(
        &track,
        &NoteTrack::from_allegro(&encoded).expect("reparse Allegro output"),
    );
}

#[test]
fn fractional_pitch_survives_allegro_round_trip() {
    let mut track = NoteTrack::new("fractional pitch");
    track.notes.push(au_core::notes::Note {
        start: 0.0,
        duration: 0.5,
        pitch: 60.5,
        loudness: 100.0,
        channel: 0,
    });
    let reparsed =
        NoteTrack::from_allegro(&track.to_allegro()).expect("reparse fractional pitch");
    assert_eq!(reparsed.notes.len(), 1);
    assert!((reparsed.notes[0].pitch - 60.5).abs() < 1.0e-9);
}

#[test]
fn rendering_is_non_silent_and_mute_renders_silence() {
    let mut track = NoteTrack::from_allegro(AUDACITY_DATA).expect("parse Allegro data");
    let rendered = track.render(44_100);
    let expected_frames = ((track.end_time() + 0.1) * 44_100.0).round() as usize;
    assert!(rendered.len().abs_diff(expected_frames) <= 1);
    assert!(rendered.channels[0].iter().any(|sample| sample.abs() > 1.0e-6));

    track.mute = true;
    let muted = track.render(44_100);
    assert!(muted.channels[0].iter().all(|sample| sample.abs() <= 1.0e-12));
}
