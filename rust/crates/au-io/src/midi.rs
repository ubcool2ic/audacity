use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use au_core::notes::{Note, NoteTrack};
use midly::{
    num::{u24, u4, u7},
    Format, Fps, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind,
};

use crate::IoError;

#[derive(Clone, Copy)]
enum MidiEvent {
    NoteOn { channel: u8, key: u8, velocity: u8 },
    NoteOff { channel: u8, key: u8 },
}

#[derive(Clone, Copy)]
struct TimedEvent {
    tick: u64,
    track: usize,
    order: usize,
    event: MidiEvent,
}

#[derive(Clone, Copy)]
struct TimedTempo {
    tick: u64,
    track: usize,
    order: usize,
    microseconds_per_quarter: u32,
}

pub fn import_midi(path: &Path) -> Result<NoteTrack, IoError> {
    let bytes = std::fs::read(path).map_err(IoError::Io)?;
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "MIDI Track".to_string());
    parse_midi(&bytes, &name)
}

pub fn parse_midi(bytes: &[u8], name: &str) -> Result<NoteTrack, IoError> {
    let smf = Smf::parse(bytes).map_err(|error| IoError::Decode(error.to_string()))?;
    let timing = smf.header.timing;

    let mut events = Vec::new();
    let mut tempos = Vec::new();
    let mut last_event_tick = 0_u64;

    for (track_index, track) in smf.tracks.iter().enumerate() {
        let mut tick = 0_u64;
        for (order, event) in track.iter().enumerate() {
            tick = tick.saturating_add(u64::from(event.delta.as_int()));
            last_event_tick = last_event_tick.max(tick);

            match event.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(tempo)) => {
                    let microseconds_per_quarter = tempo.as_int();
                    if microseconds_per_quarter != 0 {
                        tempos.push(TimedTempo {
                            tick,
                            track: track_index,
                            order,
                            microseconds_per_quarter,
                        });
                    }
                }
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOn { key, vel },
                } => {
                    let velocity = vel.as_int();
                    let event = if velocity == 0 {
                        MidiEvent::NoteOff {
                            channel: channel.as_int(),
                            key: key.as_int(),
                        }
                    } else {
                        MidiEvent::NoteOn {
                            channel: channel.as_int(),
                            key: key.as_int(),
                            velocity,
                        }
                    };
                    events.push(TimedEvent {
                        tick,
                        track: track_index,
                        order,
                        event,
                    });
                }
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOff { key, .. },
                } => events.push(TimedEvent {
                    tick,
                    track: track_index,
                    order,
                    event: MidiEvent::NoteOff {
                        channel: channel.as_int(),
                        key: key.as_int(),
                    },
                }),
                _ => {}
            }
        }
    }

    tempos.sort_by_key(|tempo| (tempo.tick, tempo.track, tempo.order));
    let first_tempo = tempos
        .first()
        .map(|tempo| f64::from(tempo.microseconds_per_quarter))
        .filter(|tempo| *tempo > 0.0)
        .map(|tempo| 60_000_000.0 / tempo)
        .unwrap_or(120.0);

    // If multiple tracks change tempo at the same tick, the last event in the
    // stable track/order ordering is the tempo used after that tick.
    let mut tempo_map = Vec::<(u64, u32)>::new();
    for tempo in tempos {
        if let Some((last_tick, last_tempo)) = tempo_map.last_mut() {
            if *last_tick == tempo.tick {
                *last_tempo = tempo.microseconds_per_quarter;
                continue;
            }
        }
        tempo_map.push((tempo.tick, tempo.microseconds_per_quarter));
    }

    events.sort_by_key(|event| (event.tick, event.track, event.order));
    let last_event_time = tick_to_seconds(last_event_tick, timing, &tempo_map);
    let mut active = BTreeMap::<(u8, u8), VecDeque<(f64, u8)>>::new();
    let mut notes = Vec::new();

    for event in events.iter().cloned() {
        let time = tick_to_seconds(event.tick, timing, &tempo_map);
        match event.event {
            MidiEvent::NoteOn {
                channel,
                key,
                velocity,
            } => active
                .entry((channel, key))
                .or_default()
                .push_back((time, velocity)),
            MidiEvent::NoteOff { channel, key } => {
                if let Some(queue) = active.get_mut(&(channel, key)) {
                    if let Some((start, velocity)) = queue.pop_front() {
                        notes.push(Note {
                            start,
                            duration: (time - start).max(0.0),
                            pitch: f64::from(key),
                            loudness: f64::from(velocity),
                            channel: i32::from(channel),
                        });
                    }
                }
            }
        }
    }

    for ((channel, key), starts) in active {
        for (start, velocity) in starts {
            notes.push(Note {
                start,
                duration: (last_event_time - start).max(0.0),
                pitch: f64::from(key),
                loudness: f64::from(velocity),
                channel: i32::from(channel),
            });
        }
    }

    notes.sort_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then_with(|| a.pitch.total_cmp(&b.pitch))
            .then_with(|| a.channel.cmp(&b.channel))
    });

    let mut track = NoteTrack::new(name);
    track.notes = notes;
    track.tempo_bpm = first_tempo;
    Ok(track)
}

fn tick_to_seconds(tick: u64, timing: Timing, tempo_map: &[(u64, u32)]) -> f64 {
    match timing {
        Timing::Timecode(fps, ticks_per_frame) => {
            let frames_per_second = match fps {
                Fps::Fps24 => 24.0,
                Fps::Fps25 => 25.0,
                Fps::Fps29 => 30_000.0 / 1_001.0,
                Fps::Fps30 => 30.0,
            };
            if ticks_per_frame == 0 {
                0.0
            } else {
                tick as f64 / (frames_per_second * f64::from(ticks_per_frame))
            }
        }
        Timing::Metrical(ticks_per_quarter) => {
            let ppq = u64::from(ticks_per_quarter.as_int()).max(1);
            let mut previous_tick = 0_u64;
            let mut microseconds_per_quarter = 500_000_u32;
            let mut seconds = 0.0;

            for &(change_tick, new_tempo) in tempo_map {
                if change_tick > tick {
                    break;
                }
                seconds += (change_tick.saturating_sub(previous_tick)) as f64
                    * f64::from(microseconds_per_quarter)
                    / (ppq as f64 * 1_000_000.0);
                previous_tick = change_tick;
                microseconds_per_quarter = new_tempo;
            }

            seconds
                + (tick.saturating_sub(previous_tick)) as f64
                    * f64::from(microseconds_per_quarter)
                    / (ppq as f64 * 1_000_000.0)
        }
    }
}

fn seconds_to_ticks(seconds: f64, tempo_bpm: f64) -> u64 {
    const MAX_TICK: f64 = 268_435_455.0;
    let seconds = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    let bpm = if tempo_bpm.is_finite() && tempo_bpm > 0.0 {
        tempo_bpm
    } else {
        120.0
    };
    (seconds * bpm / 60.0 * 480.0)
        .round()
        .clamp(0.0, MAX_TICK) as u64
}

pub fn export_midi(path: &Path, track: &NoteTrack) -> Result<(), IoError> {
    let bytes = write_midi(track)?;
    std::fs::write(path, bytes).map_err(IoError::Io)
}

pub fn write_midi(track: &NoteTrack) -> Result<Vec<u8>, IoError> {
    let bpm = if track.tempo_bpm.is_finite() && track.tempo_bpm > 0.0 {
        track.tempo_bpm
    } else {
        120.0
    };
    let microseconds_per_quarter =
        (60_000_000.0 / bpm).round().clamp(1.0, 16_777_215.0) as u32;

    let tempo_track = vec![
        TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(
                microseconds_per_quarter,
            ))),
        },
        TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::TimeSignature(4, 2, 24, 8)),
        },
        TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        },
    ];

    let mut events = Vec::<(u64, u8, usize, TrackEventKind<'_>)>::new();
    events.push((
        0,
        0,
        0,
        TrackEventKind::Meta(MetaMessage::TrackName(track.name.as_bytes())),
    ));

    for (index, note) in track.notes.iter().enumerate() {
        let channel = note.channel.clamp(0, 15) as u8;
        if track.visible_channels & (1_u32 << channel) == 0 {
            continue;
        }

        let pitch = if note.pitch.is_finite() {
            note.pitch.round().clamp(0.0, 127.0) as u8
        } else {
            0
        };
        let velocity = if note.loudness.is_finite() {
            note.loudness.round().clamp(1.0, 127.0) as u8
        } else {
            1
        };
        let start_seconds = track.offset + note.start;
        let start_tick = seconds_to_ticks(start_seconds, bpm);
        let end_tick = seconds_to_ticks(
            start_seconds + if note.duration.is_finite() {
                note.duration.max(0.0)
            } else {
                0.0
            },
            bpm,
        )
        .max(start_tick);

        events.push((
            start_tick,
            2,
            index.saturating_mul(2),
            TrackEventKind::Midi {
                channel: u4::new(channel),
                message: MidiMessage::NoteOn {
                    key: u7::new(pitch),
                    vel: u7::new(velocity),
                },
            },
        ));
        events.push((
            end_tick,
            1,
            index.saturating_mul(2).saturating_add(1),
            TrackEventKind::Midi {
                channel: u4::new(channel),
                message: MidiMessage::NoteOff {
                    key: u7::new(pitch),
                    vel: u7::new(0),
                },
            },
        ));
    }

    events.sort_by_key(|(tick, priority, sequence, _)| (*tick, *priority, *sequence));

    let mut note_track = Vec::with_capacity(events.len() + 1);
    let mut previous_tick = 0_u64;
    for (tick, _, _, kind) in events {
        let delta = tick.saturating_sub(previous_tick).min(268_435_455) as u32;
        note_track.push(TrackEvent {
            delta: delta.into(),
            kind,
        });
        previous_tick = tick;
    }
    note_track.push(TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let smf = Smf {
        header: Header {
            format: Format::Parallel,
            timing: Timing::Metrical(480.into()),
        },
        tracks: vec![tempo_track, note_track],
    };
    let mut bytes = Vec::new();
    smf.write_std(&mut bytes)
        .map_err(|error| IoError::Encode(error.to_string()))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_track(tempo_bpm: f64) -> NoteTrack {
        let mut track = NoteTrack::new("three notes");
        track.tempo_bpm = tempo_bpm;
        track.notes = vec![
            Note {
                start: 0.0,
                duration: 0.5,
                pitch: 60.0,
                loudness: 100.0,
                channel: 0,
            },
            Note {
                start: 0.5,
                duration: 0.25,
                pitch: 64.0,
                loudness: 90.0,
                channel: 0,
            },
            Note {
                start: 0.75,
                duration: 0.75,
                pitch: 67.0,
                loudness: 80.0,
                channel: 1,
            },
        ];
        track
    }

    fn assert_notes_close(actual: &NoteTrack, expected: &NoteTrack) {
        assert_eq!(actual.notes.len(), expected.notes.len());
        for (left, right) in actual.notes.iter().zip(&expected.notes) {
            assert!((left.start - right.start).abs() <= 0.001);
            assert!((left.duration - right.duration).abs() <= 0.001);
            assert_eq!(left.pitch, right.pitch);
            assert_eq!(left.loudness, right.loudness);
            assert_eq!(left.channel, right.channel);
        }
    }

    #[test]
    fn write_then_parse_preserves_three_notes_at_multiple_tempos() {
        for tempo in [120.0, 90.0] {
            let expected = sample_track(tempo);
            let bytes = write_midi(&expected).expect("MIDI encoding should succeed");
            let actual = parse_midi(&bytes, "round trip").expect("MIDI decoding should succeed");
            assert_notes_close(&actual, &expected);
            assert!((actual.tempo_bpm - tempo).abs() < 0.01);
        }
    }

    #[test]
    fn zero_velocity_note_on_is_a_note_off() {
        // Format 0, 480 PPQ: C4 on at tick 0 and velocity-zero on at tick 480.
        let bytes = [
            b'M', b'T', b'h', b'd', 0, 0, 0, 6, 0, 0, 0, 1, 1, 0xE0, b'M', b'T', b'r', b'k',
            0, 0, 0, 19, 0, 0x90, 60, 100, 0x83, 0x60, 0x90, 60, 0, 0, 0xFF, 0x2F, 0,
        ];
        let track = parse_midi(&bytes, "velocity zero").expect("MIDI decoding should succeed");
        assert_eq!(track.notes.len(), 1);
        assert!((track.notes[0].duration - 0.5).abs() < 0.001);
        assert_eq!(track.notes[0].loudness, 100.0);
    }

    #[test]
    fn tempo_change_mid_file_changes_seconds_conversion() {
        // Format 0, 480 PPQ. The first 480 ticks take 0.5 s at 120 BPM;
        // the next 480 ticks take 1.0 s at 60 BPM.
        let bytes = [
            b'M', b'T', b'h', b'd', 0, 0, 0, 6, 0, 0, 0, 1, 1, 0xE0, b'M', b'T', b'r', b'k',
            0, 0, 0, 30, 0, 0xFF, 0x51, 3, 0x07, 0xA1, 0x20, 0, 0x90, 60, 100, 0x83, 0x60,
            0xFF, 0x51, 3, 0x0F, 0x42, 0x40, 0x83, 0x60, 0x80, 60, 0, 0, 0xFF, 0x2F, 0,
        ];
        let track = parse_midi(&bytes, "tempo change").expect("MIDI decoding should succeed");
        assert_eq!(track.notes.len(), 1);
        assert!((track.notes[0].start - 0.0).abs() < 0.001);
        assert!((track.notes[0].duration - 1.5).abs() < 0.001);
        assert!((track.tempo_bpm - 120.0).abs() < 0.01);
    }
}

