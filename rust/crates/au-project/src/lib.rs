// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

//! Stage-1 native `.aurs` project storage. The real Audacity `.aup3`
//! reader and writer will be implemented in Stage 4.

use au_core::{AudioBuffer, CoreError, ProjectState, Track};
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
         DROP TABLE IF EXISTS tracks;
         DROP TABLE IF EXISTS project;
         CREATE TABLE project(key TEXT PRIMARY KEY, value TEXT);
         CREATE TABLE tracks(
             idx INTEGER PRIMARY KEY,
             name TEXT,
             gain REAL,
             pan REAL,
             mute INTEGER,
             solo INTEGER,
             sample_rate INTEGER,
             n_channels INTEGER
         );
         CREATE TABLE samples(
             track INTEGER,
             channel INTEGER,
             data BLOB
         );",
    )?;

    transaction.execute(
        "INSERT INTO project(key, value) VALUES (?1, ?2)",
        params!["format_version", "1"],
    )?;
    transaction.execute(
        "INSERT INTO project(key, value) VALUES (?1, ?2)",
        params!["sample_rate", state.sample_rate.to_string()],
    )?;

    let mut track_statement = transaction.prepare(
        "INSERT INTO tracks(idx, name, gain, pan, mute, solo, sample_rate, n_channels)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    let mut sample_statement =
        transaction.prepare("INSERT INTO samples(track, channel, data) VALUES (?1, ?2, ?3)")?;

    for (track_index, track) in state.tracks.iter().enumerate() {
        let index = i64::try_from(track_index)
            .map_err(|_| ProjectError::BadFormat("too many tracks".into()))?;
        let n_channels = i64::try_from(track.buffer.n_channels())
            .map_err(|_| ProjectError::BadFormat("too many channels".into()))?;

        track_statement.execute(params![
            index,
            track.name,
            track.gain as f64,
            track.pan as f64,
            i64::from(track.mute),
            i64::from(track.solo),
            i64::from(track.buffer.sample_rate),
            n_channels,
        ])?;

        for (channel_index, channel) in track.buffer.channels.iter().enumerate() {
            let channel_index = i64::try_from(channel_index)
                .map_err(|_| ProjectError::BadFormat("too many channels".into()))?;
            let mut bytes = Vec::with_capacity(channel.len().saturating_mul(4));
            for sample in channel {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
            sample_statement.execute(params![index, channel_index, bytes])?;
        }
    }

    Ok(())
}

/// Load a native `.aurs` SQLite project.
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
    if format_version.as_deref() != Some("1") {
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

    let mut tracks = Vec::new();
    let mut track_statement = connection.prepare(
        "SELECT idx, name, gain, pan, mute, solo, sample_rate, n_channels
         FROM tracks ORDER BY idx",
    )?;
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
        ))
    })?;

    for row in track_rows {
        let (track_index, name, gain, pan, mute, solo, track_rate, n_channels) = row?;
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

        let mut channels = vec![None; n_channels];
        let mut sample_statement = connection.prepare(
            "SELECT channel, data FROM samples WHERE track=?1 ORDER BY channel",
        )?;
        let sample_rows = sample_statement.query_map(params![track_index as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;

        for sample_row in sample_rows {
            let (channel_index, bytes) = sample_row?;
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
            let samples = bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();
            channels[channel_index] = Some(samples);
        }

        let channels = channels
            .into_iter()
            .map(|channel| {
                channel.ok_or_else(|| ProjectError::BadFormat("missing sample channel".into()))
            })
            .collect::<Result<Vec<_>>>()?;

        while tracks.len() <= track_index {
            tracks.push(None);
        }
        if tracks[track_index].is_some() {
            return Err(ProjectError::BadFormat("duplicate track index".into()));
        }
        let mut track = Track::new(name, AudioBuffer::from_channels(track_rate, channels));
        track.gain = gain as f32;
        track.pan = pan as f32;
        track.mute = mute != 0;
        track.solo = solo != 0;
        tracks[track_index] = Some(track);
    }

    let tracks = tracks
        .into_iter()
        .map(|track| track.ok_or_else(|| ProjectError::BadFormat("non-contiguous track indices".into())))
        .collect::<Result<Vec<_>>>()?;

    Ok(ProjectState {
        tracks,
        sample_rate,
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
        };

        save_project(&path, &state).unwrap();
        assert_eq!(load_project(&path).unwrap(), state);
    }
}
