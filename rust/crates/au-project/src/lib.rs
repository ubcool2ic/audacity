// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

//! Native `.aurs` project storage plus Audacity 3.x `.aup3` support.

pub mod aup3;
pub mod binxml;
pub mod blocks;
pub mod noise_profiles;

use au_core::{
    timeline::{Clip, EnvPoint, Envelope, Label, LabelTrack, WaveClips},
    AudioBuffer, CoreError, ProjectState, Track,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("bad project format: {0}")]
    BadFormat(String),
    #[error(transparent)]
    Core(#[from] CoreError),
}

type Result<T> = std::result::Result<T, ProjectError>;

/// Save a project to a native `.aurs` SQLite file, replacing its contents.
pub fn save_project(path: &Path, state: &ProjectState) -> Result<()> {
    let mut connection = Connection::open(path)?;
    let transaction = connection.transaction()?;
    save_in_transaction(&transaction, state)?;
    transaction.commit()?;
    Ok(())
}

fn save_in_transaction(transaction: &Transaction<'_>, state: &ProjectState) -> Result<()> {
    transaction.execute_batch(
        "DROP TABLE IF EXISTS samples;
         DROP TABLE IF EXISTS envelope;
         DROP TABLE IF EXISTS clips;
         DROP TABLE IF EXISTS labels;
         DROP TABLE IF EXISTS label_tracks;
         DROP TABLE IF EXISTS tracks;
         DROP TABLE IF EXISTS project;
         DROP TABLE IF EXISTS note_tracks;
         CREATE TABLE note_tracks(idx INTEGER PRIMARY KEY, name TEXT, offset REAL, mute INTEGER, solo INTEGER, velocity REAL, visible INTEGER, tempo REAL, data TEXT);
         CREATE TABLE project(key TEXT PRIMARY KEY, value TEXT);
         CREATE TABLE tracks(
             idx INTEGER PRIMARY KEY,
             name TEXT,
             gain REAL,
             pan REAL,
             mute INTEGER,
             solo INTEGER,
             sample_rate INTEGER,
             n_channels INTEGER,
             env_default REAL
         );
         CREATE TABLE clips(
             track INTEGER,
             idx INTEGER,
             name TEXT,
             start INTEGER,
             stretch_ratio REAL,
             sample_rate INTEGER,
             PRIMARY KEY(track, idx)
         );
         CREATE TABLE samples(
             track INTEGER,
             clip INTEGER,
             channel INTEGER,
             data BLOB
         );
         CREATE TABLE envelope(
             track INTEGER,
             idx INTEGER,
             time REAL,
             value REAL
         );
         CREATE TABLE label_tracks(idx INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE labels(
             lt INTEGER,
             idx INTEGER,
             start REAL,
             end REAL,
             text TEXT
         );",
    )?;

    transaction.execute(
        "INSERT INTO project(key, value) VALUES (?1, ?2)",
        params!["format_version", "2"],
    )?;
    transaction.execute(
        "INSERT INTO project(key, value) VALUES (?1, ?2)",
        params!["sample_rate", state.sample_rate.to_string()],
    )?;
    transaction.execute(
        "INSERT INTO project(key, value) VALUES (?1, ?2)",
        params!["sync_lock", if state.sync_lock { "1" } else { "0" }],
    )?;

    let mut track_statement = transaction.prepare(
        "INSERT INTO tracks
         (idx, name, gain, pan, mute, solo, sample_rate, n_channels, env_default)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?;
    let mut clip_statement = transaction.prepare(
        "INSERT INTO clips(track, idx, name, start, stretch_ratio, sample_rate)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let mut sample_statement =
        transaction.prepare("INSERT INTO samples(track, clip, channel, data) VALUES (?1, ?2, ?3, ?4)")?;
    let mut envelope_statement =
        transaction.prepare("INSERT INTO envelope(track, idx, time, value) VALUES (?1, ?2, ?3, ?4)")?;

    for (track_index, track) in state.tracks.iter().enumerate() {
        let index = i64::try_from(track_index)
            .map_err(|_| ProjectError::BadFormat("too many tracks".into()))?;
        let n_channels = i64::try_from(track.clips.n_channels)
            .map_err(|_| ProjectError::BadFormat("too many channels".into()))?;

        track_statement.execute(params![
            index,
            track.name,
            track.gain as f64,
            track.pan as f64,
            i64::from(track.mute),
            i64::from(track.solo),
            i64::from(track.clips.sample_rate),
            n_channels,
            track.clips.envelope.default_value,
        ])?;

        for (clip_index, clip) in track.clips.clips.iter().enumerate() {
            let clip_idx = i64::try_from(clip_index)
                .map_err(|_| ProjectError::BadFormat("too many clips".into()))?;
            let start = i64::try_from(clip.start)
                .map_err(|_| ProjectError::BadFormat("clip start is too large".into()))?;
            clip_statement.execute(params![
                index,
                clip_idx,
                clip.name,
                start,
                clip.stretch_ratio,
                i64::from(clip.buffer.sample_rate),
            ])?;

            for (channel_index, channel) in clip.buffer.channels.iter().enumerate() {
                let channel_index = i64::try_from(channel_index)
                    .map_err(|_| ProjectError::BadFormat("too many channels".into()))?;
                let mut bytes = Vec::with_capacity(channel.len().saturating_mul(4));
                for sample in channel {
                    bytes.extend_from_slice(&sample.to_le_bytes());
                }
                sample_statement.execute(params![index, clip_idx, channel_index, bytes])?;
            }
        }

        for (point_index, point) in track.clips.envelope.points.iter().enumerate() {
            let point_idx = i64::try_from(point_index)
                .map_err(|_| ProjectError::BadFormat("too many envelope points".into()))?;
            envelope_statement.execute(params![
                index,
                point_idx,
                point.time,
                point.value
            ])?;
        }
    }

    let mut label_track_statement =
        transaction.prepare("INSERT INTO label_tracks(idx, name) VALUES (?1, ?2)")?;
    let mut label_statement =
        transaction.prepare("INSERT INTO labels(lt, idx, start, end, text) VALUES (?1, ?2, ?3, ?4, ?5)")?;
    for (track_index, label_track) in state.labels.iter().enumerate() {
        let index = i64::try_from(track_index)
            .map_err(|_| ProjectError::BadFormat("too many label tracks".into()))?;
        label_track_statement.execute(params![index, label_track.name])?;
        for (label_index, label) in label_track.labels.iter().enumerate() {
            let label_idx = i64::try_from(label_index)
                .map_err(|_| ProjectError::BadFormat("too many labels".into()))?;
            label_statement.execute(params![
                index,
                label_idx,
                label.start,
                label.end,
                label.text
            ])?;
        }
    }

for (idx, nt) in state.note_tracks.iter().enumerate() {
    transaction.execute(
        "INSERT INTO note_tracks(idx, name, offset, mute, solo, velocity, visible, tempo, data)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            idx as i64,
            nt.name,
            nt.offset,
            i64::from(nt.mute),
            i64::from(nt.solo),
            f64::from(nt.velocity),
            i64::from(nt.visible_channels),
            nt.tempo_bpm,
            nt.to_allegro()
        ],
    )?;
}


    Ok(())
}

fn read_samples(
    connection: &Connection,
    track_index: usize,
    clip_index: usize,
    n_channels: usize,
) -> Result<Vec<Vec<f32>>> {
    let mut channels = vec![None; n_channels];
    // v1 files have no `clip` column; `clip_index == usize::MAX` selects that layout.
    let (sql, clip) = if clip_index == usize::MAX {
        ("SELECT channel, data FROM samples WHERE track=?1 AND ?2=?2 ORDER BY channel", 0i64)
    } else {
        ("SELECT channel, data FROM samples WHERE track=?1 AND clip=?2 ORDER BY channel", clip_index as i64)
    };
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map(
        params![track_index as i64, clip],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
    )?;

    for row in rows {
        let (channel_index, bytes) = row?;
        let channel_index = usize::try_from(channel_index)
            .map_err(|_| ProjectError::BadFormat("invalid channel index".into()))?;
        if channel_index >= n_channels || channels[channel_index].is_some() {
            return Err(ProjectError::BadFormat(
                "invalid or duplicate sample channel".into(),
            ));
        }
        if bytes.len() % 4 != 0 {
            return Err(ProjectError::BadFormat(
                "sample data length is not a multiple of four".into(),
            ));
        }
        channels[channel_index] = Some(
            bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect(),
        );
    }

    channels
        .into_iter()
        .map(|channel| {
            channel.ok_or_else(|| ProjectError::BadFormat("missing sample channel".into()))
        })
        .collect()
}

/// Load a native `.aurs` SQLite project, including legacy version 1 files.
pub fn load_project(path: &Path) -> Result<ProjectState> {
    let connection = Connection::open(path)?;

    let has_project_table: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='project')",
        [],
        |row| row.get(0),
    )?;
    let has_tracks_table: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='tracks')",
        [],
        |row| row.get(0),
    )?;
    let has_samples_table: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='samples')",
        [],
        |row| row.get(0),
    )?;

    if !has_project_table || !has_tracks_table || !has_samples_table {
        return Err(ProjectError::BadFormat(
            "missing required project tables".into(),
        ));
    }

    let format_version: Option<String> = connection
        .query_row(
            "SELECT value FROM project WHERE key='format_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let version = format_version
        .ok_or_else(|| ProjectError::BadFormat("unsupported or missing format version".into()))?;
    if version != "1" && version != "2" {
        return Err(ProjectError::BadFormat(
            "unsupported or missing format version".into(),
        ));
    }

    let sample_rate_text: Option<String> = connection
        .query_row(
            "SELECT value FROM project WHERE key='sample_rate'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let sample_rate = sample_rate_text
        .ok_or_else(|| ProjectError::BadFormat("missing project sample rate".into()))?
        .parse::<u32>()
        .map_err(|_| ProjectError::BadFormat("invalid project sample rate".into()))?;

    let sync_lock = if version == "2" {
        match connection
            .query_row(
                "SELECT value FROM project WHERE key='sync_lock'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .as_deref()
        {
            None | Some("0") => false,
            Some("1") => true,
            Some(_) => return Err(ProjectError::BadFormat("invalid sync lock value".into())),
        }
    } else {
        false
    };

    let mut tracks = Vec::new();
    let mut track_statement = if version == "1" {
        connection.prepare(
            "SELECT idx, name, gain, pan, mute, solo, sample_rate, n_channels, 1.0
             FROM tracks ORDER BY idx",
        )?
    } else {
        connection.prepare(
            "SELECT idx, name, gain, pan, mute, solo, sample_rate, n_channels, env_default
             FROM tracks ORDER BY idx",
        )?
    };

    let track_rows = track_statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
            row.get::<_, f64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, f64>(8)?,
        ))
    })?;

    for row in track_rows {
        let (track_index, name, gain, pan, mute, solo, track_rate, n_channels, env_default) = row?;
        if track_index < 0 || n_channels < 0 {
            return Err(ProjectError::BadFormat(
                "negative track index or channel count".into(),
            ));
        }
        let track_index = usize::try_from(track_index)
            .map_err(|_| ProjectError::BadFormat("invalid track index".into()))?;
        let n_channels = usize::try_from(n_channels)
            .map_err(|_| ProjectError::BadFormat("invalid channel count".into()))?;
        let track_rate = u32::try_from(track_rate)
            .map_err(|_| ProjectError::BadFormat("invalid track sample rate".into()))?;

        let mut clips = WaveClips::new(track_rate, n_channels);
        clips.envelope = Envelope {
            points: Vec::new(),
            default_value: env_default,
        };

        if version == "1" {
            let channels = read_samples(&connection, track_index, usize::MAX, n_channels)?;
            clips.clips.push(Clip {
                name: "Clip".to_string(),
                start: 0,
                buffer: AudioBuffer::from_channels(track_rate, channels),
                stretch_ratio: 1.0,
            });
        } else {
            let mut clip_statement = connection.prepare(
                "SELECT idx, name, start, stretch_ratio, sample_rate
                 FROM clips WHERE track=?1 ORDER BY idx",
            )?;
            let clip_rows = clip_statement.query_map(params![track_index as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            let mut expected_index = 0usize;
            for clip_row in clip_rows {
                let (clip_idx, clip_name, start, stretch_ratio, clip_rate) = clip_row?;
                let clip_idx = usize::try_from(clip_idx)
                    .map_err(|_| ProjectError::BadFormat("invalid clip index".into()))?;
                if clip_idx != expected_index {
                    return Err(ProjectError::BadFormat(
                        "non-contiguous clip indices".into(),
                    ));
                }
                expected_index += 1;
                let start = usize::try_from(start)
                    .map_err(|_| ProjectError::BadFormat("invalid clip start".into()))?;
                let clip_rate = u32::try_from(clip_rate)
                    .map_err(|_| ProjectError::BadFormat("invalid clip sample rate".into()))?;
                let channels = read_samples(&connection, track_index, clip_idx, n_channels)?;
                clips.clips.push(Clip {
                    name: clip_name,
                    start,
                    buffer: AudioBuffer::from_channels(clip_rate, channels),
                    stretch_ratio,
                });
            }

            let mut envelope_statement = connection.prepare(
                "SELECT idx, time, value FROM envelope WHERE track=?1 ORDER BY idx",
            )?;
            let envelope_rows = envelope_statement.query_map(params![track_index as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            })?;
            for (expected_index, point_row) in envelope_rows.enumerate() {
                let (point_index, time, value) = point_row?;
                if usize::try_from(point_index).ok() != Some(expected_index) {
                    return Err(ProjectError::BadFormat(
                        "non-contiguous envelope point indices".into(),
                    ));
                }
                clips.envelope.points.push(EnvPoint { time, value });
            }
        }

        let mut track = Track::new(name, AudioBuffer::from_channels(track_rate, vec![vec![]; n_channels]));
        track.clips = clips;
        track.gain = gain as f32;
        track.pan = pan as f32;
        track.mute = mute != 0;
        track.solo = solo != 0;

        while tracks.len() <= track_index {
            tracks.push(None);
        }
        if tracks[track_index].is_some() {
            return Err(ProjectError::BadFormat("duplicate track index".into()));
        }
        tracks[track_index] = Some(track);
    }

    let tracks = tracks
        .into_iter()
        .map(|track| {
            track.ok_or_else(|| ProjectError::BadFormat("non-contiguous track indices".into()))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut labels = Vec::new();
    if version == "2" {
        let mut label_track_statement =
            connection.prepare("SELECT idx, name FROM label_tracks ORDER BY idx")?;
        let label_track_rows = label_track_statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        for (expected_index, label_track_row) in label_track_rows.enumerate() {
            let (label_track_index, name) = label_track_row?;
            if usize::try_from(label_track_index).ok() != Some(expected_index) {
                return Err(ProjectError::BadFormat(
                    "non-contiguous label track indices".into(),
                ));
            }
            let mut label_track = LabelTrack {
                name,
                labels: Vec::new(),
            };
            let mut statement = connection.prepare(
                "SELECT idx, start, end, text FROM labels WHERE lt=?1 ORDER BY idx",
            )?;
            let rows = statement.query_map(params![label_track_index], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            for (expected_label_index, row) in rows.enumerate() {
                let (label_index, start, end, text) = row?;
                if usize::try_from(label_index).ok() != Some(expected_label_index) {
                    return Err(ProjectError::BadFormat(
                        "non-contiguous label indices".into(),
                    ));
                }
                label_track.labels.push(Label { start, end, text });
            }
            labels.push(label_track);
        }
    }

let mut note_tracks = Vec::new();
let has_note_tracks = connection
    .query_row(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='note_tracks'",
        [],
        |row| row.get::<_, String>(0),
    )
    .is_ok();

if has_note_tracks {
    let mut statement = connection.prepare(
        "SELECT name, offset, mute, solo, velocity, visible, tempo, data
         FROM note_tracks ORDER BY idx",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, f64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, f64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, f64>(6)?,
            row.get::<_, String>(7)?,
        ))
    })?;

    for row in rows {
        let (name, offset, mute, solo, velocity, visible, tempo, data) = row?;
        let mut nt = au_core::notes::NoteTrack::from_allegro(&data)
            .unwrap_or_else(|_| au_core::notes::NoteTrack::new(name.clone()));
        nt.name = name;
        nt.offset = offset;
        nt.mute = mute != 0;
        nt.solo = solo != 0;
        nt.velocity = velocity as f32;
        nt.visible_channels = visible.clamp(0, i64::from(u32::MAX)) as u32;
        nt.tempo_bpm = tempo;
        note_tracks.push(nt);
    }
}

    Ok(ProjectState {
        tracks,
        sample_rate,
        labels,
        sync_lock,
        note_tracks,
    })
}

/// Detects an Audacity 3 `.aup3` database by checking for its characteristic
/// `project` and `sampleblocks` tables. This only detects the format; it does
/// not read or write `.aup3` project contents.
pub fn is_aup3(path: &Path) -> bool {
    let Ok(connection) = Connection::open(path) else {
        return false;
    };
    ["project", "sampleblocks"].iter().all(|table| {
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                params![table],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_stereo_and_mono_tracks() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("test.aurs");
        let state = ProjectState {
            sample_rate: 48_000,
            tracks: vec![
                Track::new(
                    "Stereo".to_string(),
                    AudioBuffer::from_channels(
                        48_000,
                        vec![vec![0.25, -0.5, 1.0], vec![-0.25, 0.5, -1.0]],
                    ),
                ),
                Track::new(
                    "Mono".to_string(),
                    AudioBuffer::from_channels(44_100, vec![vec![0.0, 0.75]]),
                ),
            ],
            ..Default::default()
        };

        save_project(&path, &state).unwrap();
        assert_eq!(load_project(&path).unwrap(), state);
    }

    #[test]
    fn v2_roundtrip_clips_gaps_envelope_labels() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("clips.aurs");
        let mut track = Track::new(
            "Clips".to_string(),
            AudioBuffer::from_channels(48_000, vec![vec![]]),
        );
        track.clips.clips = vec![
            Clip {
                name: "First".to_string(),
                start: 0,
                buffer: AudioBuffer::from_channels(48_000, vec![vec![0.1, 0.2]]),
                stretch_ratio: 1.0,
            },
            Clip {
                name: "Second".to_string(),
                start: 100,
                buffer: AudioBuffer::from_channels(48_000, vec![vec![0.3, 0.4]]),
                stretch_ratio: 1.5,
            },
        ];
        track.clips.envelope.points = vec![
            EnvPoint {
                time: 0.0,
                value: 0.5,
            },
            EnvPoint {
                time: 2.0,
                value: 1.0,
            },
        ];
        let state = ProjectState {
            tracks: vec![track],
            sample_rate: 48_000,
            note_tracks: Vec::new(),
            labels: vec![LabelTrack {
                name: "Labels".to_string(),
                labels: vec![
                    Label {
                        start: 1.0,
                        end: 1.0,
                        text: "Point".to_string(),
                    },
                    Label {
                        start: 2.0,
                        end: 3.0,
                        text: "Region".to_string(),
                    },
                ],
            }],
            sync_lock: true,
        };

        save_project(&path, &state).unwrap();
        assert_eq!(load_project(&path).unwrap(), state);
    }

    #[test]
    fn loads_v1_file() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("legacy.aurs");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE project(key TEXT PRIMARY KEY, value TEXT);
                 INSERT INTO project VALUES('format_version', '1');
                 INSERT INTO project VALUES('sample_rate', '44100');
                 CREATE TABLE tracks(
                     idx INTEGER PRIMARY KEY, name TEXT, gain REAL, pan REAL,
                     mute INTEGER, solo INTEGER, sample_rate INTEGER, n_channels INTEGER
                 );
                 INSERT INTO tracks VALUES(0, 'Legacy', 1.0, 0.0, 0, 0, 44100, 1);
                 CREATE TABLE samples(track INTEGER, channel INTEGER, data BLOB);",
            )
            .unwrap();
        let bytes: Vec<u8> = [0.25f32, -0.5]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        connection
            .execute(
                "INSERT INTO samples(track, channel, data) VALUES (0, 0, ?1)",
                params![bytes],
            )
            .unwrap();
        drop(connection);

        let state = load_project(&path).unwrap();
        assert_eq!(state.tracks.len(), 1);
        assert_eq!(state.tracks[0].clips.clips.len(), 1);
        let clip = &state.tracks[0].clips.clips[0];
        assert_eq!(clip.name, "Clip");
        assert_eq!(clip.start, 0);
        assert_eq!(clip.stretch_ratio, 1.0);
        assert_eq!(clip.buffer.channels, vec![vec![0.25, -0.5]]);
        assert!(state.tracks[0].clips.envelope.points.is_empty());
        assert!(state.labels.is_empty());
        assert!(!state.sync_lock);
    }
}
