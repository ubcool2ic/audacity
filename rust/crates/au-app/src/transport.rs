// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

//! Device-independent transport glue between the project ([`au_core`]) and
//! the realtime engine ([`au_io::engine`]) / recording logic
//! ([`au_io::record`]). Everything here runs headless and is unit tested
//! with the null backends; the GUI only adds cpal streams on top.

use au_core::{AudioBuffer, Editor, ProjectState, Track};
use au_io::engine::{MixTrack, TrackParams};
use au_io::record::{RecordMode, RecordSession, SaState, SoundActivated, SoundActivation, TimerRecord, TimerState};
use std::sync::Arc;

/// Builds engine tracks from project state (each track rendered once,
/// envelope applied, gaps as silence).
pub fn mix_tracks(state: &ProjectState) -> Vec<MixTrack> {
    state
        .tracks
        .iter()
        .map(|t| MixTrack {
            audio: Arc::new(t.render()),
            offset: 0,
            params: TrackParams { gain: t.gain, pan: t.pan, mute: t.mute, solo: t.solo },
        })
        .collect()
}

/// Gate applied before samples reach the take.
#[derive(Clone, Debug)]
pub enum RecordTrigger {
    /// Record immediately.
    Immediate,
    /// Record only inside a timer window (frames relative to session start).
    Timer(TimerRecord),
    /// Record only while input is above a threshold.
    SoundActivated(SoundActivation),
}

/// Recording options chosen in the UI.
#[derive(Clone, Debug)]
pub struct RecordOptions {
    /// New track / overdub / punch-and-roll.
    pub mode: RecordMode,
    /// Gate.
    pub trigger: RecordTrigger,
    /// Round-trip latency to compensate, in frames (positive = late).
    pub latency: i64,
    /// Punch crossfade length in frames.
    pub crossfade: usize,
    /// Track replaced by punch-and-roll.
    pub punch_track: Option<usize>,
}

impl Default for RecordOptions {
    fn default() -> Self {
        Self { mode: RecordMode::NewTrack, trigger: RecordTrigger::Immediate, latency: 0, crossfade: 64, punch_track: None }
    }
}

/// Non-realtime side of a recording: receives interleaved blocks drained
/// from the capture ring, applies the trigger, and commits to the editor.
pub struct RecordController {
    opts: RecordOptions,
    session: RecordSession,
    sa: Option<SoundActivated>,
    elapsed: u64,
    channels: usize,
    scratch: Vec<f32>,
    /// Timeline frame where the take starts.
    pub start: usize,
}

impl RecordController {
    /// Starts a controller for `channels` at `rate`, placing the take at `start`.
    pub fn new(opts: RecordOptions, channels: usize, rate: u32, start: usize) -> Self {
        let sa = match &opts.trigger {
            RecordTrigger::SoundActivated(cfg) => Some(SoundActivated::new(cfg.clone())),
            _ => None,
        };
        let session = RecordSession::new(opts.mode.clone(), channels, rate, opts.latency);
        Self { opts, session, sa, elapsed: 0, channels: channels.max(1), scratch: Vec::new(), start }
    }

    /// Feeds an interleaved block. Returns true while still accepting input
    /// (false once a timer window has finished).
    pub fn feed(&mut self, block: &[f32]) -> bool {
        let frames = block.len() / self.channels;
        let keep = match &self.opts.trigger {
            RecordTrigger::Immediate => {
                self.session.push_interleaved(block);
                true
            }
            RecordTrigger::Timer(t) => {
                let r = t.frames_to_capture(self.elapsed, frames);
                self.session.push_interleaved(&block[r.start * self.channels..r.end * self.channels]);
                t.state_at(self.elapsed + frames as u64) != TimerState::Done
            }
            RecordTrigger::SoundActivated(_) => {
                self.scratch.clear();
                if let Some(sa) = self.sa.as_mut() {
                    sa.feed(block, self.channels, &mut self.scratch);
                }
                self.session.push_interleaved(&self.scratch);
                true
            }
        };
        self.elapsed += frames as u64;
        keep
    }

    /// Whether the sound-activation gate is currently open.
    pub fn gate_open(&self) -> bool {
        self.sa.as_ref().map_or(true, |s| s.state() == SaState::Recording)
    }

    /// Commits the take to the editor as one undoable step. Returns the
    /// index of the track that received audio.
    pub fn commit(self, editor: &mut Editor) -> au_core::Result<usize> {
        let opts = self.opts;
        match (&opts.mode, opts.punch_track) {
            (RecordMode::PunchAndRoll { .. }, Some(idx)) => {
                let existing = editor
                    .state()
                    .tracks
                    .get(idx)
                    .ok_or(au_core::CoreError::TrackNotFound(idx))?
                    .render();
                let buf = self.session.finish(Some(&existing), self.start, opts.crossfade);
                editor.replace_track_audio(idx, buf, "Punch and Roll Record")?;
                Ok(idx)
            }
            _ => {
                let buf: AudioBuffer = self.session.finish(None, self.start, opts.crossfade);
                let name = if matches!(opts.mode, RecordMode::Overdub) { "Overdub" } else { "Recording" };
                editor.add_track(Track::new(name, buf));
                Ok(editor.state().tracks.len() - 1)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au_io::engine::{engine, EngineCommand, InputSource, Mixer, NullInput, NullOutput};

    fn state_with(tracks: Vec<Vec<f32>>) -> ProjectState {
        ProjectState {
            tracks: tracks
                .into_iter()
                .enumerate()
                .map(|(i, d)| Track::new(format!("t{i}"), AudioBuffer::from_channels(8000, vec![d])))
                .collect(),
            sample_rate: 8000,
            ..Default::default()
        }
    }

    #[test]
    fn transport_playback_headless_mixes_project() {
        let mut st = state_with(vec![vec![0.25; 8], vec![0.25; 8]]);
        st.tracks[1].mute = true;
        let (mut h, mut cb) = engine(Mixer::new(mix_tracks(&st), 1.0));
        let mut out = NullOutput { sample_rate: 8000, channels: 1, block: 4, captured: Vec::new() };
        h.send(EngineCommand::Play);
        out.pump(&mut cb, 3);
        assert!(out.captured[..8].iter().all(|v| (v - 0.25).abs() < 1e-5));
        assert!(out.captured[8..].iter().all(|v| *v == 0.0));
        assert!(!h.is_playing());
    }

    #[test]
    fn record_new_track_through_null_input_is_undoable() {
        let mut ed = Editor::new(state_with(vec![vec![0.0; 4]]));
        let mut input = NullInput { data: vec![0.5; 6], pos: 0 };
        let mut rc = RecordController::new(RecordOptions { latency: 2, ..Default::default() }, 1, 8000, 3);
        let mut block = [0.0f32; 4];
        for _ in 0..2 {
            let n = input.read(&mut block, 1);
            rc.feed(&block[..n]);
        }
        let idx = rc.commit(&mut ed).unwrap();
        let r = ed.state().tracks[idx].render();
        // 6 captured - 2 latency = 4 frames, placed at frame 3.
        assert_eq!(r.channels[0], vec![0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 0.5]);
        ed.undo().unwrap();
        assert_eq!(ed.state().tracks.len(), 1);
    }

    #[test]
    fn record_punch_replaces_region() {
        let mut ed = Editor::new(state_with(vec![vec![0.1; 20]]));
        let opts = RecordOptions {
            mode: RecordMode::PunchAndRoll { punch_in: 5, punch_out: Some(10), preroll: 2 },
            crossfade: 0,
            punch_track: Some(0),
            ..Default::default()
        };
        let mut rc = RecordController::new(opts, 1, 8000, 3);
        rc.feed(&[0.9; 12]);
        rc.commit(&mut ed).unwrap();
        let r = ed.state().tracks[0].render().channels[0].clone();
        assert!(r[..5].iter().all(|v| (*v - 0.1).abs() < 1e-6));
        assert!(r[5..10].iter().all(|v| (*v - 0.9).abs() < 1e-6));
        assert!(r[10..20].iter().all(|v| (*v - 0.1).abs() < 1e-6));
    }

    #[test]
    fn record_timer_window_only() {
        let opts = RecordOptions { trigger: RecordTrigger::Timer(TimerRecord { start_at: 4, duration: 4 }), ..Default::default() };
        let mut rc = RecordController::new(opts, 1, 8000, 0);
        let data: Vec<f32> = (0..12).map(|i| i as f32 / 100.0).collect();
        assert!(rc.feed(&data[..6]));
        assert!(!rc.feed(&data[6..]));
        let mut ed = Editor::new(state_with(vec![]));
        let idx = rc.commit(&mut ed).unwrap();
        let r = ed.state().tracks[idx].render().channels[0].clone();
        assert_eq!(r, vec![0.04, 0.05, 0.06, 0.07]);
    }

    #[test]
    fn record_sound_activated_skips_silence() {
        let opts = RecordOptions {
            trigger: RecordTrigger::SoundActivated(SoundActivation { threshold_db: -20.0, hold_frames: 2 }),
            ..Default::default()
        };
        let mut rc = RecordController::new(opts, 1, 8000, 0);
        rc.feed(&[0.0, 0.0, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0]);
        assert!(!rc.gate_open());
        let mut ed = Editor::new(state_with(vec![]));
        let idx = rc.commit(&mut ed).unwrap();
        let r = ed.state().tracks[idx].render().channels[0].clone();
        assert_eq!(r, vec![0.5, 0.5, 0.0, 0.0]);
    }

    #[test]
    fn track_mix_is_undoable() {
        let mut ed = Editor::new(state_with(vec![vec![0.0; 2]]));
        ed.set_track_mix(0, 0.5, -0.5, true, false).unwrap();
        assert_eq!(ed.state().tracks[0].gain, 0.5);
        assert!(ed.state().tracks[0].mute);
        ed.undo().unwrap();
        assert_eq!(ed.state().tracks[0].gain, 1.0);
    }
}
