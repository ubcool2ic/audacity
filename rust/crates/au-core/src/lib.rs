// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

//! Core audio data types and editing operations.

pub mod timeline;

/// A single audio sample.
pub type Sample = f32;

/// Errors returned by core audio and editing operations.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum CoreError {
    /// A frame range is invalid for the requested buffer.
    #[error("invalid range {0}..{1}")]
    InvalidRange(usize, usize),
    /// Buffers have different channel counts.
    #[error("channel mismatch")]
    ChannelMismatch,
    /// Buffers have different sample rates.
    #[error("sample rate mismatch")]
    RateMismatch,
    /// There is no earlier state in the undo history.
    #[error("nothing to undo")]
    NothingToUndo,
    /// There is no later state in the undo history.
    #[error("nothing to redo")]
    NothingToRedo,
    /// No track exists at the requested index.
    #[error("track not found {0}")]
    TrackNotFound(usize),
}

/// The result type used by this crate.
pub type Result<T> = std::result::Result<T, CoreError>;

/// A collection of audio samples arranged by channel.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBuffer {
    /// The number of samples per second.
    pub sample_rate: u32,
    /// Per-channel sample data.
    pub channels: Vec<Vec<Sample>>,
}

impl AudioBuffer {
    /// Creates an empty buffer with `n_channels` channels.
    pub fn new(sample_rate: u32, n_channels: usize) -> Self {
        Self {
            sample_rate,
            channels: vec![Vec::new(); n_channels],
        }
    }

    /// Creates a buffer from per-channel sample data.
    pub fn from_channels(sample_rate: u32, channels: Vec<Vec<Sample>>) -> Self {
        Self {
            sample_rate,
            channels,
        }
    }

    /// Returns the number of channels.
    pub fn n_channels(&self) -> usize {
        self.channels.len()
    }

    /// Returns the frame count, using the shortest channel.
    pub fn len(&self) -> usize {
        self.channels.iter().map(Vec::len).min().unwrap_or(0)
    }

    /// Returns whether the buffer contains no frames.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the duration in seconds.
    pub fn duration_secs(&self) -> f64 {
        frames_to_secs(self.len(), self.sample_rate)
    }

    /// Returns a copy of the frames in `start..end`.
    pub fn slice(&self, start: usize, end: usize) -> Result<AudioBuffer> {
        if start > end || end > self.len() {
            return Err(CoreError::InvalidRange(start, end));
        }
        Ok(AudioBuffer::from_channels(
            self.sample_rate,
            self.channels
                .iter()
                .map(|channel| channel[start..end].to_vec())
                .collect(),
        ))
    }

    /// Removes and returns the frames in `start..end`.
    pub fn remove(&mut self, start: usize, end: usize) -> Result<AudioBuffer> {
        if start > end || end > self.len() {
            return Err(CoreError::InvalidRange(start, end));
        }
        let removed = self.slice(start, end)?;
        for channel in &mut self.channels {
            channel.drain(start..end);
        }
        Ok(removed)
    }

    /// Inserts `other` at frame `at`.
    pub fn insert(&mut self, at: usize, other: &AudioBuffer) -> Result<()> {
        if self.n_channels() != other.n_channels() {
            return Err(CoreError::ChannelMismatch);
        }
        if self.sample_rate != other.sample_rate {
            return Err(CoreError::RateMismatch);
        }
        if at > self.len() {
            return Err(CoreError::InvalidRange(at, at));
        }
        for (channel, inserted) in self.channels.iter_mut().zip(&other.channels) {
            channel.splice(at..at, inserted.iter().copied());
        }
        Ok(())
    }

    /// Appends `other` to this buffer.
    pub fn append(&mut self, other: &AudioBuffer) -> Result<()> {
        let at = self.len();
        self.insert(at, other)
    }

    /// Returns the maximum absolute sample value, or zero for an empty buffer.
    pub fn peak(&self) -> Sample {
        self.channels
            .iter()
            .flat_map(|channel| channel.iter())
            .fold(0.0, |peak, sample| peak.max(sample.abs()))
    }

    /// Returns samples interleaved frame by frame.
    pub fn interleaved(&self) -> Vec<Sample> {
        let frames = self.len();
        let mut result = Vec::with_capacity(frames.saturating_mul(self.n_channels()));
        for frame in 0..frames {
            for channel in &self.channels {
                result.push(channel[frame]);
            }
        }
        result
    }

    /// Builds a buffer from interleaved samples.
    ///
    /// Any trailing samples that do not make a complete frame are ignored.
    pub fn from_interleaved(
        sample_rate: u32,
        n_channels: usize,
        samples: &[Sample],
    ) -> AudioBuffer {
        if n_channels == 0 {
            return AudioBuffer::new(sample_rate, 0);
        }
        let frames = samples.len() / n_channels;
        let mut channels = vec![Vec::with_capacity(frames); n_channels];
        for frame in 0..frames {
            for channel in 0..n_channels {
                channels[channel].push(samples[frame * n_channels + channel]);
            }
        }
        AudioBuffer::from_channels(sample_rate, channels)
    }
}

/// A track in a project.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// The track's display name.
    pub name: String,
    /// The track's clips (multiple clips with gaps) and volume envelope.
    pub clips: timeline::WaveClips,
    /// Linear playback gain.
    pub gain: f32,
    /// Stereo pan, conventionally in the range -1 to 1.
    pub pan: f32,
    /// Whether playback of this track is muted.
    pub mute: bool,
    /// Whether this track is soloed.
    pub solo: bool,
}

impl Track {
    /// Creates a track with unity gain, centered pan, and no mute or solo.
    pub fn new(name: impl Into<String>, buffer: AudioBuffer) -> Self {
        Self {
            name: name.into(),
            clips: timeline::WaveClips::from_buffer("Clip", buffer),
            gain: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
        }
    }

    /// Renders the track's clips (gaps as silence, envelope applied).
    pub fn render(&self) -> AudioBuffer {
        self.clips.render()
    }

    /// Returns the track's sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.clips.sample_rate
    }

    /// Returns the timeline length in frames.
    pub fn len(&self) -> usize {
        self.clips.end()
    }

    /// Returns whether the track has no audio.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A frame selection on a track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Selection {
    /// The selected track index.
    pub track: usize,
    /// The selection's start frame.
    pub start: usize,
    /// The selection's end frame.
    pub end: usize,
}

impl Selection {
    /// Returns whether the selection has zero length.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Returns the selection with its endpoints in ascending order.
    pub fn normalized(self) -> Self {
        if self.start <= self.end {
            self
        } else {
            Self {
                track: self.track,
                start: self.end,
                end: self.start,
            }
        }
    }
}

/// The editable state of a project.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectState {
    /// Tracks in the project.
    pub tracks: Vec<Track>,
    /// The project's sample rate.
    pub sample_rate: u32,
    /// Label tracks.
    pub labels: Vec<timeline::LabelTrack>,
    /// Whether sync-lock is on (edits ripple across all tracks and labels).
    pub sync_lock: bool,
}

/// Linear, unlimited snapshot history for project states.
#[derive(Clone, Debug)]
pub struct UndoManager {
    entries: Vec<(String, ProjectState)>,
    cursor: usize,
}

impl UndoManager {
    /// Creates a history containing the initial project state.
    pub fn new(initial: ProjectState) -> Self {
        Self {
            entries: vec![(String::new(), initial)],
            cursor: 0,
        }
    }

    /// Adds a state and discards any redo branch.
    pub fn push(&mut self, description: &str, state: ProjectState) {
        self.entries.truncate(self.cursor + 1);
        self.entries.push((description.to_owned(), state));
        self.cursor = self.entries.len() - 1;
    }

    /// Moves to and returns the preceding state.
    pub fn undo(&mut self) -> Result<&ProjectState> {
        if !self.can_undo() {
            return Err(CoreError::NothingToUndo);
        }
        self.cursor -= 1;
        Ok(&self.entries[self.cursor].1)
    }

    /// Moves to and returns the following state.
    pub fn redo(&mut self) -> Result<&ProjectState> {
        if !self.can_redo() {
            return Err(CoreError::NothingToRedo);
        }
        self.cursor += 1;
        Ok(&self.entries[self.cursor].1)
    }

    /// Returns the current project state.
    pub fn current(&self) -> &ProjectState {
        &self.entries[self.cursor].1
    }

    /// Returns whether an undo operation is available.
    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    /// Returns whether a redo operation is available.
    pub fn can_redo(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    /// Returns the description of the operation that undo would reverse.
    pub fn undo_description(&self) -> Option<&str> {
        self.can_undo()
            .then(|| self.entries[self.cursor].0.as_str())
    }

    /// Returns the description of the operation that redo would apply.
    pub fn redo_description(&self) -> Option<&str> {
        self.can_redo()
            .then(|| self.entries[self.cursor + 1].0.as_str())
    }

    /// Returns the number of snapshots in the history, including the initial state.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// High-level editing operations over project state and undo history.
pub struct Editor {
    /// The editor's project history.
    pub history: UndoManager,
    /// The audio buffer currently held on the clipboard.
    pub clipboard: Option<AudioBuffer>,
}

impl Editor {
    /// Creates an editor with the given initial project state.
    pub fn new(initial: ProjectState) -> Self {
        Self {
            history: UndoManager::new(initial),
            clipboard: None,
        }
    }

    /// Returns the current project state.
    pub fn state(&self) -> &ProjectState {
        self.history.current()
    }

    /// Adds a track and records an undo entry.
    pub fn add_track(&mut self, track: Track) {
        let mut state = self.state().clone();
        state.tracks.push(track);
        self.history.push("Add Track", state);
    }

    /// Removes a track and records an undo entry.
    pub fn remove_track(&mut self, idx: usize) -> Result<()> {
        if idx >= self.state().tracks.len() {
            return Err(CoreError::TrackNotFound(idx));
        }
        let mut state = self.state().clone();
        state.tracks.remove(idx);
        self.history.push("Remove Track", state);
        Ok(())
    }

    /// Copies the selected audio to the clipboard and removes it from the track.
    pub fn cut(&mut self, selection: Selection) -> Result<()> {
        let selection = selection.normalized();
        let mut state = self.state().clone();
        let track = state
            .tracks
            .get_mut(selection.track)
            .ok_or(CoreError::TrackNotFound(selection.track))?;
        let removed = track.clips.remove_range(selection.start, selection.end)?;
        ripple_remove(&mut state, selection);
        self.history.push("Cut", state);
        self.clipboard = Some(removed);
        Ok(())
    }

    /// Copies the selected audio to the clipboard without changing project history.
    pub fn copy(&mut self, selection: Selection) -> Result<()> {
        let selection = selection.normalized();
        let track = self
            .state()
            .tracks
            .get(selection.track)
            .ok_or(CoreError::TrackNotFound(selection.track))?;
        self.clipboard = Some(track.clips.render_range(selection.start, selection.end)?);
        Ok(())
    }

    /// Inserts the clipboard at the requested frame and records an undo entry.
    ///
    /// If the clipboard is empty, this is an error-free no-op.
    pub fn paste(&mut self, track: usize, at: usize) -> Result<()> {
        let Some(clipboard) = self.clipboard.as_ref() else {
            return Ok(());
        };
        let mut state = self.state().clone();
        let target = state
            .tracks
            .get_mut(track)
            .ok_or(CoreError::TrackNotFound(track))?;
        target.clips.insert(at, clipboard)?;
        let len = clipboard.len();
        ripple_insert(&mut state, track, at, len);
        self.history.push("Paste", state);
        Ok(())
    }

    /// Deletes the selected audio and records an undo entry.
    pub fn delete(&mut self, selection: Selection) -> Result<()> {
        let selection = selection.normalized();
        let mut state = self.state().clone();
        let track = state
            .tracks
            .get_mut(selection.track)
            .ok_or(CoreError::TrackNotFound(selection.track))?;
        track.clips.remove_range(selection.start, selection.end)?;
        ripple_remove(&mut state, selection);
        self.history.push("Delete", state);
        Ok(())
    }

    /// Replaces the selected audio with silence and records an undo entry.
    pub fn silence(&mut self, selection: Selection) -> Result<()> {
        self.apply(selection, "Silence", |buffer| {
            for channel in &mut buffer.channels {
                channel.fill(0.0);
            }
        })
    }

    /// Applies a transformation to a selection and replaces that range with its result.
    pub fn apply(
        &mut self,
        selection: Selection,
        description: &str,
        f: impl FnOnce(&mut AudioBuffer),
    ) -> Result<()> {
        let selection = selection.normalized();
        let mut state = self.state().clone();
        let track = state
            .tracks
            .get_mut(selection.track)
            .ok_or(CoreError::TrackNotFound(selection.track))?;
        track.clips.apply_range(selection.start, selection.end, f)?;
        self.history.push(description, state);
        Ok(())
    }

    fn edit_track(
        &mut self,
        track: usize,
        description: &str,
        f: impl FnOnce(&mut Track) -> Result<()>,
    ) -> Result<()> {
        let mut state = self.state().clone();
        let t = state.tracks.get_mut(track).ok_or(CoreError::TrackNotFound(track))?;
        f(t)?;
        self.history.push(description, state);
        Ok(())
    }

    /// Splits the clip under `frame` (Audacity "Split").
    pub fn split_clip(&mut self, track: usize, frame: usize) -> Result<()> {
        self.edit_track(track, "Split", |t| t.clips.split(frame))
    }

    /// Joins all clips intersecting `start..end` (Audacity "Join").
    pub fn join_clips(&mut self, track: usize, start: usize, end: usize) -> Result<()> {
        self.edit_track(track, "Join", |t| t.clips.join(start, end))
    }

    /// Time-shifts a clip to `new_start` frames; overlaps are rejected.
    pub fn move_clip(&mut self, track: usize, clip: usize, new_start: i64) -> Result<()> {
        self.edit_track(track, "Time-Shift", |t| t.clips.move_clip(clip, new_start).map(|_| ()))
    }

    /// Sets a clip's time-stretch ratio (pitch preserved).
    pub fn stretch_clip(&mut self, track: usize, clip: usize, ratio: f64) -> Result<()> {
        self.edit_track(track, "Stretch Clip", |t| t.clips.stretch_clip(clip, ratio))
    }

    /// Adds or replaces a volume envelope point.
    pub fn add_envelope_point(&mut self, track: usize, time: f64, value: f64) -> Result<()> {
        self.edit_track(track, "Adjusted Envelope", |t| {
            t.clips.envelope.insert(time, value);
            Ok(())
        })
    }

    /// Moves a volume envelope point.
    pub fn move_envelope_point(&mut self, track: usize, idx: usize, time: f64, value: f64) -> Result<()> {
        self.edit_track(track, "Adjusted Envelope", |t| t.clips.envelope.move_point(idx, time, value).map(|_| ()))
    }

    /// Removes a volume envelope point.
    pub fn remove_envelope_point(&mut self, track: usize, idx: usize) -> Result<()> {
        self.edit_track(track, "Adjusted Envelope", |t| t.clips.envelope.remove(idx).map(|_| ()))
    }

    fn edit_labels(
        &mut self,
        description: &str,
        f: impl FnOnce(&mut Vec<timeline::LabelTrack>) -> Result<()>,
    ) -> Result<()> {
        let mut state = self.state().clone();
        f(&mut state.labels)?;
        self.history.push(description, state);
        Ok(())
    }

    /// Adds a label track and returns its index.
    pub fn add_label_track(&mut self, name: &str) -> usize {
        let n = self.state().labels.len();
        let _ = self.edit_labels("New Label Track", |l| {
            l.push(timeline::LabelTrack::new(name));
            Ok(())
        });
        n
    }

    /// Adds a point (`start == end`) or region label.
    pub fn add_label(&mut self, lt: usize, start: f64, end: f64, text: &str) -> Result<()> {
        self.edit_labels("Added Label", |l| {
            l.get_mut(lt).ok_or(CoreError::TrackNotFound(lt))?.add(start, end, text);
            Ok(())
        })
    }

    /// Edits a label's text.
    pub fn edit_label(&mut self, lt: usize, idx: usize, text: &str) -> Result<()> {
        self.edit_labels("Edited Label", |l| l.get_mut(lt).ok_or(CoreError::TrackNotFound(lt))?.edit_text(idx, text))
    }

    /// Changes a label's times.
    pub fn set_label_times(&mut self, lt: usize, idx: usize, start: f64, end: f64) -> Result<()> {
        self.edit_labels("Moved Label", |l| l.get_mut(lt).ok_or(CoreError::TrackNotFound(lt))?.set_times(idx, start, end).map(|_| ()))
    }

    /// Deletes a label.
    pub fn delete_label(&mut self, lt: usize, idx: usize) -> Result<()> {
        self.edit_labels("Deleted Label", |l| l.get_mut(lt).ok_or(CoreError::TrackNotFound(lt))?.remove(idx).map(|_| ()))
    }

    /// Imports labels from Audacity's label text format into a new label track.
    pub fn import_labels(&mut self, name: &str, text: &str) -> Result<usize> {
        let track = timeline::LabelTrack::from_audacity_txt(name, text)?;
        let n = self.state().labels.len();
        self.edit_labels("Imported Labels", |l| {
            l.push(track);
            Ok(())
        })?;
        Ok(n)
    }

    /// Turns sync-lock on or off (not an undoable edit in Audacity, but kept in state).
    pub fn set_sync_lock(&mut self, on: bool) {
        let mut state = self.state().clone();
        state.sync_lock = on;
        self.history.push("Sync-Lock", state);
    }

    /// Undoes the most recent edit.
    pub fn undo(&mut self) -> Result<()> {
        self.history.undo().map(|_| ())
    }

    /// Redoes the next edit.
    pub fn redo(&mut self) -> Result<()> {
        self.history.redo().map(|_| ())
    }
}

fn ripple_remove(state: &mut ProjectState, sel: Selection) {
    if !state.sync_lock {
        return;
    }
    let rate = state.tracks[sel.track].sample_rate();
    let (a, b) = (frames_to_secs(sel.start, rate), frames_to_secs(sel.end, rate));
    for (i, t) in state.tracks.iter_mut().enumerate() {
        if i != sel.track {
            let end = sel.end.min(t.len().max(sel.start));
            if sel.start < end {
                let _ = t.clips.remove_range(sel.start, end);
            }
        }
    }
    for l in &mut state.labels {
        l.remove_range(a, b);
    }
}

fn ripple_insert(state: &mut ProjectState, track: usize, at: usize, len: usize) {
    if !state.sync_lock || len == 0 {
        return;
    }
    let rate = state.tracks[track].sample_rate();
    for (i, t) in state.tracks.iter_mut().enumerate() {
        if i != track && at < t.len() {
            let silence = AudioBuffer::from_channels(t.sample_rate(), vec![vec![0.0; len]; t.clips.n_channels]);
            let _ = t.clips.insert(at, &silence);
        }
    }
    for l in &mut state.labels {
        l.insert_space(frames_to_secs(at, rate), frames_to_secs(len, rate));
    }
}

/// Converts a duration in seconds to a frame count, rounding to the nearest frame.
pub fn secs_to_frames(secs: f64, rate: u32) -> usize {
    (secs * f64::from(rate)).round().max(0.0) as usize
}

/// Converts a frame count to a duration in seconds.
pub fn frames_to_secs(frames: usize, rate: u32) -> f64 {
    if rate == 0 {
        0.0
    } else {
        frames as f64 / f64::from(rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(samples: &[f32]) -> AudioBuffer {
        AudioBuffer::from_channels(48_000, vec![samples.to_vec()])
    }

    fn editor_with(samples: &[f32]) -> Editor {
        Editor::new(ProjectState {
            tracks: vec![Track::new("track", buffer(samples))],
            sample_rate: 48_000,
            ..Default::default()
        })
    }

    #[test]
    fn cut_copy_paste_roundtrip() {
        let mut editor = editor_with(&[1.0, 2.0, 3.0, 4.0]);
        editor
            .copy(Selection {
                track: 0,
                start: 1,
                end: 3,
            })
            .unwrap();
        assert_eq!(editor.clipboard.as_ref().unwrap().channels[0], vec![2.0, 3.0]);
        editor
            .cut(Selection {
                track: 0,
                start: 1,
                end: 3,
            })
            .unwrap();
        assert_eq!(editor.state().tracks[0].render().channels[0], vec![1.0, 4.0]);
        editor.paste(0, 1).unwrap();
        assert_eq!(
            editor.state().tracks[0].render().channels[0],
            vec![1.0, 2.0, 3.0, 4.0]
        );
    }

    #[test]
    fn undo_redo_chain_and_branch_truncation() {
        let mut editor = editor_with(&[1.0]);
        editor.add_track(Track::new("second", buffer(&[2.0])));
        editor.add_track(Track::new("third", buffer(&[3.0])));
        assert_eq!(editor.history.len(), 3);
        editor.undo().unwrap();
        assert_eq!(editor.state().tracks.len(), 2);
        editor.undo().unwrap();
        assert_eq!(editor.state().tracks.len(), 1);
        editor.redo().unwrap();
        assert_eq!(editor.state().tracks.len(), 2);
        editor.add_track(Track::new("replacement", buffer(&[4.0])));
        assert!(!editor.history.can_redo());
        assert_eq!(editor.history.len(), 3);
        assert_eq!(editor.state().tracks[1].name, "second");
        assert_eq!(editor.state().tracks[2].name, "replacement");
    }

    #[test]
    fn insert_reports_channel_rate_and_range_errors() {
        let mut mono = buffer(&[1.0]);
        assert!(matches!(
            mono.insert(0, &AudioBuffer::new(48_000, 2)),
            Err(CoreError::ChannelMismatch)
        ));
        assert!(matches!(
            mono.insert(0, &AudioBuffer::from_channels(44_100, vec![vec![2.0]])),
            Err(CoreError::RateMismatch)
        ));
        assert!(matches!(
            mono.insert(2, &buffer(&[2.0])),
            Err(CoreError::InvalidRange(2, 2))
        ));
    }

    #[test]
    fn interleave_roundtrip() {
        let original =
            AudioBuffer::from_channels(44_100, vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
        let interleaved = original.interleaved();
        assert_eq!(interleaved, vec![1.0, 3.0, 2.0, 4.0]);
        assert_eq!(
            AudioBuffer::from_interleaved(44_100, 2, &interleaved),
            original
        );
    }

    #[test]
    fn apply_can_change_selection_length() {
        let mut editor = editor_with(&[1.0, 2.0, 3.0, 4.0]);
        editor
            .apply(
                Selection {
                    track: 0,
                    start: 1,
                    end: 3,
                },
                "Expand",
                |selected| selected.channels[0].push(9.0),
            )
            .unwrap();
        assert_eq!(
            editor.state().tracks[0].render().channels[0],
            vec![1.0, 2.0, 3.0, 9.0, 4.0]
        );
        editor.undo().unwrap();
        assert_eq!(
            editor.state().tracks[0].render().channels[0],
            vec![1.0, 2.0, 3.0, 4.0]
        );
    }

    #[test]
    fn buffer_ranges_and_time_conversions() {
        let mut audio = buffer(&[1.0, -3.0, 2.0]);
        assert_eq!(audio.peak(), 3.0);
        assert_eq!(audio.duration_secs(), 3.0 / 48_000.0);
        assert_eq!(audio.slice(1, 2).unwrap().channels[0], vec![-3.0]);
        assert!(matches!(audio.slice(2, 4), Err(CoreError::InvalidRange(2, 4))));
        assert_eq!(audio.remove(1, 2).unwrap().channels[0], vec![-3.0]);
        assert_eq!(audio.channels[0], vec![1.0, 2.0]);
        assert_eq!(secs_to_frames(0.5, 48_000), 24_000);
        assert_eq!(frames_to_secs(24_000, 48_000), 0.5);
    }

    #[test]
    fn undo_and_redo_errors_are_reported() {
        let mut history = UndoManager::new(ProjectState::default());
        assert!(matches!(history.undo(), Err(CoreError::NothingToUndo)));
        assert!(matches!(history.redo(), Err(CoreError::NothingToRedo)));
    }

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32 + 1.0).collect()
    }

    #[test]
    fn timeline_golden_undo_redo_restores_clip_label_envelope_state() {
        let mut editor = Editor::new(ProjectState {
            tracks: vec![Track::new("t", AudioBuffer::from_channels(10, vec![ramp(10)]))],
            sample_rate: 10,
            ..Default::default()
        });
        let mut snapshots = vec![editor.state().clone()];
        editor.split_clip(0, 4).unwrap();
        snapshots.push(editor.state().clone());
        editor.move_clip(0, 1, 8).unwrap();
        snapshots.push(editor.state().clone());
        editor.add_envelope_point(0, 0.0, 1.0).unwrap();
        editor.add_envelope_point(0, 1.0, 0.5).unwrap();
        snapshots.push(editor.state().clone());
        let lt = editor.add_label_track("Labels");
        editor.add_label(lt, 0.2, 0.2, "point").unwrap();
        editor.add_label(lt, 0.5, 0.9, "region").unwrap();
        editor.edit_label(lt, 1, "renamed").unwrap();
        snapshots.push(editor.state().clone());
        editor.join_clips(0, 0, 14).unwrap();
        editor.stretch_clip(0, 0, 2.0).unwrap();
        snapshots.push(editor.state().clone());
        assert_eq!(editor.state().tracks[0].clips.clips.len(), 1);
        assert_eq!(editor.state().tracks[0].len(), 28);
        // Undo back to the start, checking each golden snapshot, then redo forward.
        let undo_steps = [2usize, 4, 2, 1, 1];
        let mut idx = snapshots.len() - 1;
        for steps in undo_steps {
            for _ in 0..steps {
                editor.undo().unwrap();
            }
            idx -= 1;
            assert_eq!(editor.state(), &snapshots[idx]);
        }
        for steps in undo_steps.iter().rev() {
            for _ in 0..*steps {
                editor.redo().unwrap();
            }
            idx += 1;
            assert_eq!(editor.state(), &snapshots[idx]);
        }
    }

    #[test]
    fn timeline_gap_renders_silence_and_envelope_applies_in_render() {
        let mut editor = Editor::new(ProjectState {
            tracks: vec![Track::new("t", AudioBuffer::from_channels(10, vec![vec![1.0; 4]]))],
            sample_rate: 10,
            ..Default::default()
        });
        editor.split_clip(0, 2).unwrap();
        editor.move_clip(0, 1, 5).unwrap();
        assert_eq!(editor.state().tracks[0].render().channels[0], vec![1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0]);
        editor.add_envelope_point(0, 0.0, 0.0).unwrap();
        editor.add_envelope_point(0, 0.6, 0.6).unwrap();
        let out = editor.state().tracks[0].render().channels[0].clone();
        assert!((out[0] - 0.0).abs() < 1e-6 && (out[1] - 0.1).abs() < 1e-6);
        assert!((out[6] - 0.6).abs() < 1e-6);
    }

    #[test]
    fn timeline_sync_lock_cut_and_paste_ripple_labels_and_tracks() {
        let mut editor = Editor::new(ProjectState {
            tracks: vec![
                Track::new("a", AudioBuffer::from_channels(10, vec![ramp(10)])),
                Track::new("b", AudioBuffer::from_channels(10, vec![ramp(10)])),
            ],
            sample_rate: 10,
            ..Default::default()
        });
        let lt = editor.add_label_track("L");
        editor.add_label(lt, 0.8, 0.8, "x").unwrap();
        editor.set_sync_lock(true);
        editor.cut(Selection { track: 0, start: 2, end: 4 }).unwrap();
        assert_eq!(editor.state().tracks[1].len(), 8);
        assert!((editor.state().labels[0].labels[0].start - 0.6).abs() < 1e-9);
        editor.paste(0, 2).unwrap();
        assert_eq!(editor.state().tracks[0].render().channels[0], ramp(10));
        assert_eq!(editor.state().tracks[1].len(), 10);
        assert!((editor.state().labels[0].labels[0].start - 0.8).abs() < 1e-9);
        // Without sync-lock other tracks are untouched.
        editor.set_sync_lock(false);
        editor.delete(Selection { track: 0, start: 0, end: 1 }).unwrap();
        assert_eq!(editor.state().tracks[1].len(), 10);
    }

    #[test]
    fn timeline_label_import_export_through_editor() {
        let mut editor = editor_with(&[0.0]);
        let lt = editor.import_labels("Imported", "1.5\t2.5\tverse\n\\\t100\t200\n3.0\t3.0\tdrop\n").unwrap();
        let txt = editor.state().labels[lt].to_audacity_txt();
        assert_eq!(txt, "1.500000\t2.500000\tverse\n3.000000\t3.000000\tdrop\n");
        editor.delete_label(lt, 0).unwrap();
        assert_eq!(editor.state().labels[lt].labels.len(), 1);
        editor.undo().unwrap();
        assert_eq!(editor.state().labels[lt].labels.len(), 2);
        assert!(editor.import_labels("bad", "abc\n").is_err());
    }
}
