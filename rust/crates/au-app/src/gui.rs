// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

use au_core::{
    timeline::{LabelTrack, SnapMode, Snapper, WaveClips, ZoomInfo},
    AudioBuffer, Editor, Selection, Track,
};
use au_effects::{EffectKind, Normalize};
use au_io::{
    device::{InputKind, Player, Recorder},
    dsp::Dither,
    engine::{EngineCommand, TrackParams},
    record::{RecordMode, SoundActivation, TimerRecord},
};
use eframe::egui;
use std::path::PathBuf;

// egui/eframe desktop UI (feature `gui`).

/// Runs the app, optionally opening a project (`.aup3` or `.aurs`) at start.
pub fn run_with(open: Option<PathBuf>) -> eframe::Result<()> {
    eframe::run_native(
        "Audacity-RS",
        eframe::NativeOptions::default(),
        Box::new(move |_cc| {
            let mut app = AudacityApp::default();
            app.crash_reports = au_runtime::crash::pending_reports(&crate::crash_dir());
            // After a crash, reopen the project that was open so the Stage 4
            // recovery dialog offers its flushed autosave.
            let open = open.or_else(|| {
                app.crash_reports.iter().filter_map(|r| r.project()).find(|p| {
                    p.exists() && au_project::aup3::inspect(p).is_ok_and(|i| i.needs_recovery)
                })
            });
            if let Some(path) = open {
                app.open_path(path);
            }
            Ok(Box::new(app))
        }),
    )
}

#[allow(dead_code)]
fn selftest() -> eframe::Result<()> {
    let buffer = AudioBuffer::from_channels(44_100, vec![vec![0.0, 0.25, -0.5, 1.0]]);
    let mut app = AudacityApp::default();
    app.editor.add_track(Track::new("Selftest", buffer));
    app.editor
        .apply(Selection { track: 0, start: 0, end: 4 }, "Normalize", |buf| {
            Normalize::new(-1.0, false).process(buf);
        })
        .expect("selftest edit should succeed");
    app.editor.undo().expect("selftest undo should succeed");
    app.editor.redo().expect("selftest redo should succeed");
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Selection,
    Envelope,
    TimeShift,
    Draw,
}

#[derive(Clone, Copy)]
enum PointerDrag {
    Selection {
        track: usize,
        start: usize,
        end: usize,
    },
    Clip {
        track: usize,
        clip: usize,
        original_start: f64,
        pointer_start: f64,
        current_start: f64,
    },
    Envelope {
        track: usize,
        point: usize,
        time: f64,
        value: f64,
    },
    Label {
        track: usize,
        label: usize,
        pointer_start: f64,
        original_start: f64,
        original_end: f64,
        current_start: f64,
        current_end: f64,
    },
    Draw {
        track: usize,
        last_frame: usize,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RecordModeUi {
    NewTrack,
    Overdub,
    PunchAndRoll,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RecordTriggerUi {
    Immediate,
    Timer,
    SoundActivated,
}

struct AudacityApp {
    editor: Editor,
    selected_track: Option<usize>,
    selection: Option<Selection>,
    player: Option<Player>,
    recorder: Option<Recorder>,
    record_controller: Option<crate::transport::RecordController>,
    record_buffer: Vec<f32>,
    record_peak: f32,
    record_opts_window: bool,
    record_mode: RecordModeUi,
    record_trigger: RecordTriggerUi,
    input_kind: InputKind,
    record_preroll_secs: f64,
    record_latency_ms: f64,
    record_crossfade_ms: f64,
    timer_start_secs: f64,
    timer_duration_secs: f64,
    sound_threshold_db: f32,
    sound_hold_secs: f64,
    status: String,
    project_path: Option<PathBuf>,
    /// `.aup3` data not modelled by the editor (formats, version, selection).
    aup3_extras: au_project::aup3::Aup3Extras,
    /// File whose unclean autosave awaits the user's recover/discard choice.
    recovery_prompt: Option<PathBuf>,
    /// Unviewed crash reports from earlier sessions (Help ▸ crash dialog on launch).
    crash_reports: Vec<au_runtime::crash::CrashReport>,
    /// Signature of the history at the last autosave, and when it happened.
    autosave_sig: (usize, Option<String>),
    autosave_at: std::time::Instant,
    tool: Tool,
    zoom: ZoomInfo,
    snapper: Snapper,
    sync_lock: bool,
    drag: Option<PointerDrag>,
    draw_edits: Vec<(usize, f32)>,
    selected_label: Option<(usize, usize)>,
    label_edit: Option<LabelEdit>,
    speed_window: bool,
    speed_percent: f64,
    loop_play: bool,
    play_speed: f64,
    master_gain: f32,
    track_mix: Vec<TrackParams>,
    mixer_visible: bool,
    export_options_window: bool,
    export_bits: u16,
    export_dither: Dither,
    selection_start_text: String,
    selection_end_text: String,
    selection_length_text: String,
    fx_dialog: Option<crate::fx_ui::EffectDialog>,
    spectrum_win: Option<crate::analyze_ui::SpectrumWindow>,
    contrast_win: Option<crate::analyze_ui::ContrastWindow>,
    rt_window: bool,
    rt_target: crate::fx_ui::RtTarget,
    rt_editing: Option<usize>,
}

struct LabelEdit {
    track: usize,
    label: usize,
    text: String,
}

impl Default for AudacityApp {
    fn default() -> Self {
        Self {
            editor: Editor::new(au_core::ProjectState::default()),
            selected_track: None,
            selection: None,
            player: None,
            recorder: None,
            record_controller: None,
            record_buffer: Vec::new(),
            record_peak: 0.0,
            record_opts_window: false,
            record_mode: RecordModeUi::NewTrack,
            record_trigger: RecordTriggerUi::Immediate,
            input_kind: InputKind::Default,
            record_preroll_secs: 2.0,
            record_latency_ms: 0.0,
            record_crossfade_ms: 10.0,
            timer_start_secs: 0.0,
            timer_duration_secs: 60.0,
            sound_threshold_db: -26.0,
            sound_hold_secs: 1.0,
            status: "Ready".to_owned(),
            project_path: None,
            aup3_extras: Default::default(),
            recovery_prompt: None,
            crash_reports: Vec::new(),
            autosave_sig: (1, None),
            autosave_at: std::time::Instant::now(),
            tool: Tool::Selection,
            zoom: ZoomInfo {
                h_offset: 0.0,
                pixels_per_sec: 100.0,
            },
            snapper: Snapper {
                mode: SnapMode::Off,
                pixel_tolerance: 6.0,
            },
            sync_lock: false,
            drag: None,
            draw_edits: Vec::new(),
            selected_label: None,
            label_edit: None,
            speed_window: false,
            speed_percent: 100.0,
            loop_play: false,
            play_speed: 1.0,
            master_gain: 1.0,
            track_mix: Vec::new(),
            mixer_visible: false,
            export_options_window: false,
            export_bits: 16,
            export_dither: Dither::Triangle,
            selection_start_text: "00:00:00.000".to_owned(),
            selection_end_text: "00:00:00.000".to_owned(),
            selection_length_text: "00:00:00.000".to_owned(),
            fx_dialog: None,
            rt_window: false,
            rt_target: crate::fx_ui::RtTarget::Master,
            spectrum_win: None,
            contrast_win: None,
            rt_editing: None,
        }
    }
}

impl AudacityApp {
    fn report(&mut self, result: au_core::Result<()>) {
        self.status = match result {
            Ok(()) => "Ready".to_owned(),
            Err(error) => error.to_string(),
        };
    }

    fn selection_for_track(&self, track: usize) -> Selection {
        self.selection
            .filter(|selection| selection.track == track)
            .unwrap_or_else(|| {
                let len = self
                    .editor
                    .state()
                    .tracks
                    .get(track)
                    .map_or(0, |track| track.len());
                Selection {
                    track,
                    start: 0,
                    end: len,
                }
            })
            .normalized()
    }

    fn selected_rate(&self) -> u32 {
        self.selected_track
            .and_then(|index| self.editor.state().tracks.get(index))
            .map_or(self.editor.state().sample_rate, Track::sample_rate)
            .max(1)
    }

    fn project_rate(&self) -> u32 {
        self.editor
            .state()
            .tracks
            .first()
            .map_or(self.editor.state().sample_rate, Track::sample_rate)
            .max(1)
    }

    fn snap_time(&self, time: f64, rate: u32, clips: Option<&WaveClips>) -> f64 {
        self.snapper.snap(
            time.max(0.0),
            &self.zoom,
            rate.max(1),
            &self.editor.state().labels,
            clips,
        )
    }

    fn project_duration(&self) -> f64 {
        let state = self.editor.state();
        let track_duration = state
            .tracks
            .iter()
            .map(|track| {
                if track.sample_rate() == 0 {
                    0.0
                } else {
                    au_core::frames_to_secs(track.len(), track.sample_rate())
                }
            })
            .fold(0.0_f64, f64::max);
        state
            .labels
            .iter()
            .flat_map(|track| track.labels.iter().map(|label| label.end.max(label.start)))
            .fold(track_duration, f64::max)
    }

    fn ensure_track_mix(&mut self) {
        let count = self.editor.state().tracks.len();
        self.track_mix.resize(
            count,
            TrackParams {
                gain: 1.0,
                pan: 0.0,
                mute: false,
                solo: false,
            },
        );
        self.track_mix.truncate(count);
    }

    fn import_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            match au_io::import_file(&path) {
                Ok(buffer) => {
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Imported audio")
                        .to_owned();
                    self.editor.add_track(Track::new(name, buffer));
                    self.selected_track = Some(self.editor.state().tracks.len() - 1);
                    self.ensure_track_mix();
                    self.status = "Imported audio".to_owned();
                }
                Err(error) => self.status = format!("Import failed: {error:?}"),
            }
        }
    }

    fn is_aup3(path: &std::path::Path) -> bool {
        path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("aup3"))
    }

    fn history_sig(&self) -> (usize, Option<String>) {
        let h = &self.editor.history;
        (h.len(), h.undo_description().map(str::to_owned))
    }

    fn current_aup3(&self) -> au_project::aup3::Aup3Project {
        let mut extras = self.aup3_extras.clone();
        if let Some(sel) = self.selection {
            let sel = sel.normalized();
            let rate = self.editor.state().tracks.get(sel.track).map_or(44100, |t| t.sample_rate()).max(1) as f64;
            extras.selection = (sel.start as f64 / rate, sel.end as f64 / rate);
        }
        extras.track_formats.resize(self.editor.state().tracks.len(), au_project::blocks::SampleFormat::Float);
        extras.track_selected = (0..self.editor.state().tracks.len()).map(|i| self.selected_track == Some(i)).collect();
        au_project::aup3::Aup3Project { state: self.editor.state().clone(), extras }
    }

    fn save_project(&mut self) {
        match self.project_path.clone() {
            Some(path) => self.save_to(path),
            None => self.save_project_as(),
        }
    }

    fn save_project_as(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Audacity project", &["aup3"])
            .add_filter("Audacity-RS project (legacy)", &["aurs"])
            .set_file_name("Untitled.aup3")
            .save_file()
        {
            self.save_to(path);
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        let result = if Self::is_aup3(&path) {
            au_project::aup3::save(&path, &self.current_aup3())
        } else {
            au_project::save_project(&path, self.editor.state())
        };
        match result {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.project_path = Some(path);
                self.autosave_sig = self.history_sig();
            }
            Err(error) => self.status = format!("Save failed: {error}"),
        }
    }

    /// Journals unsaved edits into the `.aup3` autosave table (at most every
    /// 2 s), so a crash can be recovered on the next open.
    fn maybe_autosave(&mut self) {
        let Some(path) = self.project_path.clone().filter(|p| Self::is_aup3(p)) else { return };
        let sig = self.history_sig();
        if sig == self.autosave_sig || self.autosave_at.elapsed().as_secs_f64() < 2.0 {
            return;
        }
        self.autosave_at = std::time::Instant::now();
        if let Some(d) = &sig.1 { au_runtime::crash::log_action(format!("edit: {d}")); }
        self.autosave_sig = sig;
        if let Err(e) = au_project::aup3::autosave(&path, &self.current_aup3()) {
            self.status = format!("Autosave failed: {e}");
        }
    }

    fn open_project(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("All projects", &["aup3", "aurs"])
            .add_filter("Audacity project", &["aup3"])
            .add_filter("Audacity-RS project (legacy)", &["aurs"])
            .pick_file()
        {
            self.open_path(path);
        }
    }

    fn open_path(&mut self, path: PathBuf) {
        if Self::is_aup3(&path) || au_project::is_aup3(&path) {
            match au_project::aup3::inspect(&path) {
                Ok(info) if info.needs_recovery => {
                    self.recovery_prompt = Some(path);
                    return;
                }
                Ok(_) => {}
                Err(e) => {
                    self.status = format!("Open failed: {e}");
                    return;
                }
            }
            self.open_aup3(path, au_project::aup3::DocSource::Saved);
            return;
        }
        match au_project::load_project(&path) {
            Ok(state) => {
                self.aup3_extras = Default::default();
                self.install_state(state, path);
            }
            Err(error) => self.status = format!("Open failed: {error}"),
        }
    }

    fn open_aup3(&mut self, path: PathBuf, source: au_project::aup3::DocSource) {
        match au_project::aup3::load_from(&path, source) {
            Ok(p) => {
                let (s0, s1) = p.extras.selection;
                self.aup3_extras = p.extras;
                self.install_state(p.state, path);
                if s1 > s0 && !self.editor.state().tracks.is_empty() {
                    let rate = self.editor.state().tracks[0].sample_rate() as f64;
                    self.selection = Some(Selection { track: 0, start: (s0 * rate) as usize, end: (s1 * rate) as usize });
                }
                if source == au_project::aup3::DocSource::Autosave {
                    // Unsaved until the user saves, like Audacity after recovery.
                    self.autosave_sig = (0, None);
                    self.status = "Recovered unsaved changes from autosave — save to keep them".to_owned();
                }
            }
            Err(e) => self.status = format!("Open failed: {e}"),
        }
    }

    fn install_state(&mut self, state: au_core::ProjectState, path: PathBuf) {
        self.editor = Editor::new(state);
        self.selected_track = if self.editor.state().tracks.is_empty() { None } else { Some(0) };
        self.selection = None;
        self.selected_label = None;
        self.project_path = Some(path);
        self.sync_lock = self.editor.state().sync_lock;
        self.track_mix.clear();
        self.ensure_track_mix();
        self.autosave_sig = self.history_sig();
        self.status = "Project opened".to_owned();
        self.fit_zoom(1000.0);
    }

    /// Forces an autosave of unsaved edits now (crash path; ignores the 2 s throttle).
    fn flush_autosave(&mut self) {
        let Some(path) = self.project_path.clone().filter(|p| Self::is_aup3(p)) else { return };
        if self.history_sig() != self.autosave_sig {
            let _ = au_project::aup3::autosave(&path, &self.current_aup3());
        }
    }

    /// Shows crash reports left by earlier sessions: view, copy, dismiss.
    fn crash_report_dialog(&mut self, ctx: &egui::Context) {
        let Some(report) = self.crash_reports.first().cloned() else { return };
        let mut action = None;
        egui::Window::new("Audacity-RS crashed last time")
            .collapsible(false)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(format!(
                    "A crash report was saved locally (nothing was uploaded):\n{}",
                    report.path.display()
                ));
                if let Some(m) = report.message() { ui.label(format!("Reason: {m}")); }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    let mut text = report.text.as_str();
                    ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(f32::INFINITY));
                });
                ui.horizontal(|ui| {
                    if ui.button("Copy report").clicked() { action = Some(0); }
                    if ui.button("Dismiss").clicked() { action = Some(1); }
                    if self.crash_reports.len() > 1 { ui.label(format!("{} more", self.crash_reports.len() - 1)); }
                });
            });
        match action {
            Some(0) => { ctx.copy_text(report.text.clone()); self.status = "Crash report copied".to_owned(); }
            Some(1) => {
                let _ = au_runtime::crash::dismiss(&report);
                self.crash_reports.remove(0);
            }
            _ => {}
        }
    }

    fn recovery_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = self.recovery_prompt.clone() else { return };
        let mut choice = None;
        egui::Window::new("Automatic Crash Recovery")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} was not saved properly the last time it was open.\n\
                     Recover the unsaved changes from its autosave?",
                    path.display()
                ));
                ui.horizontal(|ui| {
                    if ui.button("Recover").clicked() { choice = Some(0); }
                    if ui.button("Discard changes").clicked() { choice = Some(1); }
                    if ui.button("Cancel").clicked() { choice = Some(2); }
                });
            });
        match choice {
            Some(0) => {
                self.recovery_prompt = None;
                self.open_aup3(path, au_project::aup3::DocSource::Autosave);
            }
            Some(1) => {
                self.recovery_prompt = None;
                match au_project::aup3::discard_autosave(&path) {
                    Ok(()) => self.open_aup3(path, au_project::aup3::DocSource::Saved),
                    Err(e) => self.status = format!("Discard failed: {e}"),
                }
            }
            Some(_) => self.recovery_prompt = None,
            None => {}
        }
    }

    fn export(&mut self, flac: bool) {
        let Some(index) = self.selected_track else {
            self.status = "Select a track to export".to_owned();
            return;
        };
        let Some(track) = self.editor.state().tracks.get(index) else {
            self.status = "Selected track not found".to_owned();
            return;
        };
        let _ = track;
        // Realtime effects are non-destructive: they are rendered into the export only.
        let Some(buffer) = crate::fx_bridge::render_track(self.editor.state(), &self.aup3_extras, index) else { return; };
        let filter = if flac { "FLAC audio" } else { "WAV audio" };
        let extension = if flac { "flac" } else { "wav" };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(filter, &[extension])
            .save_file()
        {
            let result = if flac {
                au_io::export_flac(&path, &buffer, usize::from(self.export_bits.min(24)))
            } else {
                au_io::export_wav_dithered(
                    &path,
                    &buffer,
                    self.export_bits,
                    self.export_dither,
                )
            };
            self.status = match result {
                Ok(()) => "Audio exported".to_owned(),
                Err(error) => format!("Export failed: {error:?}"),
            };
        }
    }

    fn selection_project_frames(&self) -> Option<(usize, usize)> {
        let selection = self.selection?;
        let track = self.editor.state().tracks.get(selection.track)?;
        let rate = track.sample_rate().max(1);
        let project_rate = self.project_rate();
        let start_secs = au_core::frames_to_secs(selection.start, rate);
        let end_secs = au_core::frames_to_secs(selection.end, rate);
        Some((
            au_core::secs_to_frames(start_secs, project_rate),
            au_core::secs_to_frames(end_secs, project_rate),
        ))
    }

    fn play(&mut self) {
        if let Some(player) = self.player.as_mut() {
            player.handle.send(EngineCommand::Play);
            self.status = "Playing".to_owned();
            return;
        }

        let state = self.editor.state();
        let rate = self.project_rate();
        let selection = self.selection_project_frames();
        let start = selection.map_or(0, |range| range.0);
        let loop_range = if self.loop_play {
            selection.filter(|(start, end)| end > start)
        } else {
            None
        };
        let tracks = crate::transport::mix_tracks(state);
        let (track_fx, master_fx) = crate::fx_bridge::stacks_for(&self.aup3_extras, tracks.len());
        match Player::start_with_effects(
            tracks,
            track_fx,
            master_fx,
            rate,
            self.master_gain,
            start,
            self.play_speed,
            loop_range,
        ) {
            Ok(player) => {
                self.player = Some(player);
                self.status = "Playing".to_owned();
            }
            Err(error) => self.status = format!("Playback failed: {error:?}"),
        }
    }

    fn toggle_pause(&mut self) {
        if let Some(player) = self.player.as_mut() {
            let command = if player.handle.is_playing() {
                EngineCommand::Pause
            } else {
                EngineCommand::Play
            };
            player.handle.send(command);
            self.status = if player.handle.is_playing() {
                "Playing".to_owned()
            } else {
                "Paused".to_owned()
            };
        } else {
            self.play();
        }
    }

    fn stop_playback(&mut self) {
        if let Some(mut player) = self.player.take() {
            player.stop();
        }
        self.status = "Stopped".to_owned();
    }

    fn seek_transport(&mut self, project_frame: usize) {
        if let Some(player) = self.player.as_mut() {
            let device_frame = (project_frame as f64 * player.rate_ratio).round() as usize;
            player.handle.send(EngineCommand::Seek(device_frame));
        }
    }

    fn set_play_speed(&mut self, speed: f64) {
        self.play_speed = speed.clamp(0.1, 3.0);
        if let Some(player) = self.player.as_mut() {
            player.handle.send(EngineCommand::SetSpeed(self.play_speed));
        }
    }

    fn send_track_mix(&mut self, index: usize, params: TrackParams, commit: bool) {
        if let Some(player) = self.player.as_mut() {
            player
                .handle
                .send(EngineCommand::SetTrack(index, params));
        }
        if commit {
            let result = self.editor.set_track_mix(
                index,
                params.gain,
                params.pan,
                params.mute,
                params.solo,
            );
            if let Err(error) = result {
                self.status = error.to_string();
            }
        }
    }

    fn open_record_options(&mut self) {
        self.record_opts_window = true;
    }

    fn begin_recording(&mut self) {
        let recorder = match Recorder::start(self.input_kind) {
            Ok(recorder) => recorder,
            Err(error) => {
                self.status = format!("Recording failed: {error:?}");
                return;
            }
        };
        let rate = recorder.sample_rate.max(1);
        let channels = recorder.channels.max(1) as usize;
        let selected_rate = self.selected_rate();
        let cursor_project_frame = self
            .selection
            .filter(|selection| Some(selection.track) == self.selected_track)
            .map_or(0, |selection| {
                let track_rate = self
                    .editor
                    .state()
                    .tracks
                    .get(selection.track)
                    .map_or(selected_rate, Track::sample_rate)
                    .max(1);
                let cursor_secs = au_core::frames_to_secs(selection.start, track_rate);
                au_core::secs_to_frames(cursor_secs, rate)
            });

        let selected_range = self.selection.filter(|selection| {
            Some(selection.track) == self.selected_track && selection.end > selection.start
        });
        let range_record_frames = selected_range.map(|selection| {
            let track_rate = self
                .editor
                .state()
                .tracks
                .get(selection.track)
                .map_or(selected_rate, Track::sample_rate)
                .max(1);
            (
                au_core::secs_to_frames(
                    au_core::frames_to_secs(selection.start, track_rate),
                    rate,
                ),
                au_core::secs_to_frames(
                    au_core::frames_to_secs(selection.end, track_rate),
                    rate,
                ),
            )
        });

        let preroll = au_core::secs_to_frames(self.record_preroll_secs.max(0.0), rate);
        let (start, mode) = match self.record_mode {
            RecordModeUi::NewTrack => (cursor_project_frame, RecordMode::NewTrack),
            RecordModeUi::Overdub => (cursor_project_frame, RecordMode::Overdub),
            RecordModeUi::PunchAndRoll => {
                let (punch_in, punch_out) = range_record_frames
                    .unwrap_or((cursor_project_frame, cursor_project_frame));
                let start = punch_in.saturating_sub(preroll);
                (
                    start,
                    RecordMode::PunchAndRoll {
                        punch_in,
                        punch_out: Some(punch_out),
                        preroll,
                    },
                )
            }
        };
        let trigger = match self.record_trigger {
            RecordTriggerUi::Immediate => crate::transport::RecordTrigger::Immediate,
            RecordTriggerUi::Timer => crate::transport::RecordTrigger::Timer(TimerRecord {
                start_at: au_core::secs_to_frames(self.timer_start_secs.max(0.0), rate) as u64,
                duration: au_core::secs_to_frames(self.timer_duration_secs.max(0.0), rate) as u64,
            }),
            RecordTriggerUi::SoundActivated => {
                crate::transport::RecordTrigger::SoundActivated(SoundActivation {
                    threshold_db: self.sound_threshold_db,
                    hold_frames: au_core::secs_to_frames(self.sound_hold_secs.max(0.0), rate),
                })
            }
        };
        let opts = crate::transport::RecordOptions {
            mode,
            trigger,
            latency: (self.record_latency_ms * rate as f64 / 1000.0).round() as i64,
            crossfade: au_core::secs_to_frames(
                self.record_crossfade_ms.max(0.0) / 1000.0,
                rate,
            ),
            punch_track: self.selected_track,
        };
        let controller = crate::transport::RecordController::new(opts, channels, rate, start);

        self.recorder = Some(recorder);
        self.record_controller = Some(controller);
        self.record_buffer.clear();
        self.record_opts_window = false;

        if matches!(
            self.record_mode,
            RecordModeUi::Overdub | RecordModeUi::PunchAndRoll
        ) {
            let project_rate = self.project_rate();
            let start_secs = au_core::frames_to_secs(start, rate);
            let playback_start = au_core::secs_to_frames(start_secs, project_rate);
            let tracks = crate::transport::mix_tracks(self.editor.state());
            match Player::start(
                tracks,
                project_rate,
                self.master_gain,
                playback_start,
                self.play_speed,
                None,
            ) {
                Ok(player) => self.player = Some(player),
                Err(error) => self.status = format!("Playback failed: {error:?}"),
            }
        }
        self.status = "Recording".to_owned();
    }

    fn stop_recording(&mut self) {
        self.recorder = None;
        self.record_buffer.clear();
        if let Some(controller) = self.record_controller.take() {
            match controller.commit(&mut self.editor) {
                Ok(track) => {
                    self.selected_track = Some(track);
                    self.selection = None;
                    self.ensure_track_mix();
                    self.status = "Recording committed".to_owned();
                }
                Err(error) => self.status = format!("Recording commit failed: {error}"),
            }
        }
        if let Some(mut player) = self.player.take() {
            player.stop();
        }
    }

    fn process_recording(&mut self) {
        let Some(recorder) = self.recorder.as_mut() else {
            return;
        };
        self.record_buffer.clear();
        recorder.poll(&mut self.record_buffer);
        self.record_peak = self.record_peak.max(recorder.take_peak()).clamp(0.0, 1.5);
        let done = self
            .record_controller
            .as_mut()
            .is_some_and(|controller| !controller.feed(&self.record_buffer));
        if done {
            self.stop_recording();
        } else if self
            .record_controller
            .as_ref()
            .is_some_and(|controller| !controller.gate_open())
        {
            self.status = "Waiting for sound".to_owned();
        } else {
            self.status = "Recording".to_owned();
        }
    }

    fn edit_selection(&mut self, operation: impl FnOnce(&mut Editor) -> au_core::Result<()>) {
        let result = operation(&mut self.editor);
        self.report(result);
    }

    fn open_spectrum(&mut self) {
        let Some(track) = self.selected_track else { self.status = "Plot Spectrum: select a track first".into(); return; };
        let sel = self.selection_for_track(track).normalized();
        match self.editor.state().tracks[sel.track].clips.render_range(sel.start, sel.end) {
            Ok(buf) => self.spectrum_win = Some(crate::analyze_ui::SpectrumWindow::new(buf)),
            Err(e) => self.status = format!("Plot Spectrum: {e}"),
        }
    }

    fn analyzer_windows(&mut self, ctx: &egui::Context) {
        if let Some(w) = self.spectrum_win.as_mut() {
            if !w.show(ctx) { self.spectrum_win = None; }
        }
        if let Some(mut w) = self.contrast_win.take() {
            let track = self.selected_track;
            let audio = track.and_then(|i| self.editor.state().tracks.get(i)).map(|t| t.render());
            let selection = track.and_then(|i| {
                let s = self.selection.filter(|s| s.track == i)?.normalized();
                let rate = audio.as_ref()?.sample_rate as f64;
                Some((s.start as f64 / rate, s.end as f64 / rate))
            });
            let mut measure = |t0: f64, t1: f64| match audio.as_ref() {
                Some(b) => au_effects::fx::fix_contrast::measure(b, t0, t1),
                None => Err("Please select a track.".to_string()),
            };
            if w.show(ctx, &mut measure, selection) { self.contrast_win = Some(w); }
        }
    }

    fn effect_windows(&mut self, ctx: &egui::Context) {
        self.analyzer_windows(ctx);
        let names: Vec<String> = self.editor.state().tracks.iter().map(|t| t.name.clone()).collect();
        if let Some(mut dlg) = self.fx_dialog.take() {
            let action = dlg.show(ctx, &names);
            let sel = self.selected_track.map(|i| self.selection_for_track(i));
            let aux = dlg.aux_track.and_then(|i| self.editor.state().tracks.get(i)).map(|t| t.render());
            let mut keep = true;
            match action {
                crate::fx_ui::DialogAction::None => {}
                crate::fx_ui::DialogAction::Close => keep = false,
                _ if sel.is_none() => dlg.report = "Select a track (and a time range) first".into(),
                crate::fx_ui::DialogAction::GetProfile => {
                    let sel = sel.unwrap().normalized();
                    let t = &self.editor.state().tracks[sel.track];
                    match t.clips.render_range(sel.start, sel.end).map_err(|e| e.to_string()).and_then(|b| dlg.effect.learn_profile(&b).map_err(|e| e.to_string())) {
                        Ok(()) => { dlg.has_profile = true; dlg.report = "Noise profile captured. Now select the audio to clean and press Apply.".into(); }
                        Err(e) => dlg.report = e,
                    }
                }
                crate::fx_ui::DialogAction::Preview => {
                    match crate::fx_bridge::preview(self.editor.state(), sel.unwrap(), dlg.effect.as_mut(), 6.0) {
                        Ok(buf) => {
                            self.player = None;
                            let rate = buf.sample_rate;
                            let mt = au_io::engine::MixTrack { audio: std::sync::Arc::new(buf), offset: 0, params: Default::default() };
                            match Player::start(vec![mt], rate, self.master_gain, 0, 1.0, None) {
                                Ok(p) => { self.player = Some(p); self.status = "Previewing".into(); }
                                Err(e) => dlg.report = format!("Preview playback failed: {e:?}"),
                            }
                        }
                        Err(e) => dlg.report = e,
                    }
                }
                crate::fx_ui::DialogAction::Apply => {
                    match crate::fx_bridge::apply_builtin(&mut self.editor, sel.unwrap(), dlg.effect.as_mut(), aux.as_ref()) {
                        Ok(out) => {
                            dlg.report = out.report;
                            if !out.labels.is_empty() { dlg.report.push_str(&format!("\n{} labels added", out.labels.len())); }
                            self.status = format!("Applied {}", dlg.effect.name());
                            if dlg.effect.kind() != EffectKind::Analyze { keep = false; }
                        }
                        Err(e) => dlg.report = e,
                    }
                }
            }
            if keep { self.fx_dialog = Some(dlg); }
        }
        if self.rt_window {
            let mut open = true;
            let n = self.editor.state().tracks.len();
            self.aup3_extras.track_effects.resize_with(n, Default::default);
            let mut changed = false;
            egui::Window::new("Realtime Effects").open(&mut open).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.rt_target, crate::fx_ui::RtTarget::Master, "Master");
                    for (i, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut self.rt_target, crate::fx_ui::RtTarget::Track(i), name);
                    }
                });
                ui.separator();
                let list = match self.rt_target {
                    crate::fx_ui::RtTarget::Master => &mut self.aup3_extras.master_effects,
                    crate::fx_ui::RtTarget::Track(i) if i < n => &mut self.aup3_extras.track_effects[i],
                    _ => &mut self.aup3_extras.master_effects,
                };
                changed = crate::fx_ui::realtime_panel(ui, list, &mut self.rt_editing);
                ui.label("Effects here are non-destructive: heard on playback, rendered on export, saved in .aup3.");
            });
            if changed && self.player.is_some() {
                // Restart playback so the engine picks up the new stack.
                let pos = self.player.as_ref().map(|p| p.position()).unwrap_or(0);
                self.player = None;
                self.play();
                if let Some(p) = self.player.as_mut() { p.handle.send(EngineCommand::Seek(pos)); }
            }
            self.rt_window = open;
        }
    }

    #[allow(dead_code)]
    fn apply_effect(&mut self, description: &'static str, effect: impl FnOnce(&mut AudioBuffer)) {
        let Some(index) = self.selected_track else {
            self.status = "Select a track first".to_owned();
            return;
        };
        let selection = self.selection_for_track(index);
        let result = self.editor.apply(selection, description, effect);
        self.report(result);
    }

    fn fit_zoom(&mut self, width: f64) {
        self.zoom.fit(self.project_duration().max(0.1), width.max(1.0));
    }

    #[allow(dead_code)]
    fn zoom_to_selection(&mut self, width: f64) {
        if let Some(selection) = self.selection {
            let rate = self
                .editor
                .state()
                .tracks
                .get(selection.track)
                .map_or(self.editor.state().sample_rate.max(1), Track::sample_rate);
            let start = au_core::frames_to_secs(selection.start, rate);
            let end = au_core::frames_to_secs(selection.end, rate);
            let duration = (end - start).max(0.1);
            self.zoom.pixels_per_sec = (width.max(1.0) / duration).clamp(1.0, 10_000.0);
            self.zoom.h_offset = start;
        }
    }

    fn split_selection(&mut self) {
        let Some(track) = self.selected_track else {
            return;
        };
        let selection = self.selection_for_track(track);
        let mut result = self.editor.split_clip(track, selection.start);
        if result.is_ok() && selection.end > selection.start {
            result = self.editor.split_clip(track, selection.end);
        }
        self.report(result);
    }

    fn join_selection(&mut self) {
        let Some(track) = self.selected_track else {
            return;
        };
        let selection = self.selection_for_track(track);
        self.edit_selection(|editor| editor.join_clips(track, selection.start, selection.end));
    }

    fn apply_clip_speed(&mut self) {
        let Some(track_index) = self.selected_track else {
            self.status = "Select a track first".to_owned();
            return;
        };
        let frame = self.selection_for_track(track_index).start;
        let clip_index = self
            .editor
            .state()
            .tracks
            .get(track_index)
            .and_then(|track| track.clips.clip_at(frame));
        let Some(clip_index) = clip_index else {
            self.status = "No clip at the selection start".to_owned();
            return;
        };
        let ratio = (self.speed_percent / 100.0).max(0.01);
        let result = self.editor.stretch_clip(track_index, clip_index, ratio);
        self.report(result);
        self.speed_window = false;
    }

    fn add_label_at_selection(&mut self) {
        let selection = self.selected_track.map(|index| self.selection_for_track(index));
        let rate = self.selected_rate();
        let (start, end) = selection.map_or((0.0, 0.0), |selection| {
            (
                au_core::frames_to_secs(selection.start, rate),
                au_core::frames_to_secs(selection.end, rate),
            )
        });

        let label_track = if self.editor.state().labels.is_empty() {
            self.editor.add_label_track("Labels")
        } else {
            0
        };
        if let Err(error) = self.editor.add_label(label_track, start, end, "") {
            self.status = error.to_string();
            return;
        }
        let label_index = self.editor.state().labels[label_track].labels.len() - 1;
        self.selected_label = Some((label_track, label_index));
        self.label_edit = Some(LabelEdit {
            track: label_track,
            label: label_index,
            text: String::new(),
        });
        self.status = "Editing new label".to_owned();
    }

    fn import_labels(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Audacity label track", &["txt"])
            .pick_file()
        {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    let name = path
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Labels")
                        .to_owned();
                    match self.editor.import_labels(&name, &text) {
                        Ok(_) => self.status = "Labels imported".to_owned(),
                        Err(error) => self.status = error.to_string(),
                    }
                }
                Err(error) => self.status = format!("Label import failed: {error}"),
            }
        }
    }

    fn export_labels(&mut self) {
        let labels = &self.editor.state().labels;
        if labels.is_empty() {
            self.status = "There are no labels to export".to_owned();
            return;
        }
        let contents = labels
            .iter()
            .map(LabelTrack::to_audacity_txt)
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Audacity label track", &["txt"])
            .save_file()
        {
            self.status = match std::fs::write(path, contents) {
                Ok(()) => "Labels exported".to_owned(),
                Err(error) => format!("Label export failed: {error}"),
            };
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let shortcuts = ctx.input(|input| {
            let ctrl = input.modifiers.ctrl;
            (
                ctrl && input.key_pressed(egui::Key::X),
                ctrl && input.key_pressed(egui::Key::C),
                ctrl && input.key_pressed(egui::Key::V),
                ctrl && !input.modifiers.shift && input.key_pressed(egui::Key::Z),
                (ctrl && input.modifiers.shift && input.key_pressed(egui::Key::Z))
                    || (ctrl && input.key_pressed(egui::Key::Y)),
                input.key_pressed(egui::Key::Delete),
                input.key_pressed(egui::Key::Space),
                ctrl && input.key_pressed(egui::Key::I),
                ctrl && input.key_pressed(egui::Key::J),
                ctrl && input.key_pressed(egui::Key::B),
                ctrl && input.key_pressed(egui::Key::Num1),
                ctrl && input.key_pressed(egui::Key::Num3),
                ctrl && input.key_pressed(egui::Key::F),
            )
        });

        let (
            cut,
            copy,
            paste,
            undo,
            redo,
            delete,
            space,
            split,
            join,
            add_label,
            zoom_in,
            zoom_out,
            fit,
        ) = shortcuts;

        if delete {
            if let Some((label_track, label)) = self.selected_label.take() {
                let result = self.editor.delete_label(label_track, label);
                self.report(result);
            } else if let Some(index) = self.selected_track {
                let selection = self.selection_for_track(index);
                self.edit_selection(|editor| editor.delete(selection));
            }
        } else if let Some(index) = self.selected_track {
            let selection = self.selection_for_track(index);
            if cut {
                self.edit_selection(|editor| editor.cut(selection));
            } else if copy {
                let result = self.editor.copy(selection);
                self.report(result);
            } else if paste {
                let at = selection.start;
                self.edit_selection(|editor| editor.paste(index, at));
            }
        }

        if undo {
            let result = self.editor.undo();
            self.report(result);
        } else if redo {
            let result = self.editor.redo();
            self.report(result);
        }

        if space {
            self.toggle_pause();
        }
        if split {
            self.split_selection();
        }
        if join {
            self.join_selection();
        }
        if add_label {
            self.add_label_at_selection();
        }
        if zoom_in {
            self.zoom.zoom_in();
        }
        if zoom_out {
            self.zoom.zoom_out();
        }
        if fit {
            self.fit_zoom(1000.0);
        }
    }

    fn time_at_x(&self, x: f32, rect: egui::Rect) -> f64 {
        self.zoom.x_to_time((x - rect.left()) as f64).max(0.0)
    }

    fn add_draw_point(&mut self, frame: usize, value: f32) {
        if let Some((_, old_value)) = self
            .draw_edits
            .iter_mut()
            .rev()
            .find(|(edited_frame, _)| *edited_frame == frame)
        {
            *old_value = value;
        } else {
            self.draw_edits.push((frame, value));
        }
    }

    fn draw_track(&mut self, ui: &mut egui::Ui, index: usize) {
        let width = ui.available_width().max(1.0);
        let zoom = self.zoom.clone();
        let selection = self.selection.filter(|selection| selection.track == index);
        let playhead = self
            .player
            .as_ref()
            .filter(|_| self.selected_track == Some(index))
            .map(Player::position);
        let drag_preview = self.drag;

        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, 116.0), egui::Sense::click_and_drag());
        let painter = ui.painter_at(rect);

        let mut clip_bounds = Vec::new();
        let mut envelope_points = Vec::new();
        let rate;
        let frames;

        {
            let state = self.editor.state();
            let Some(track) = state.tracks.get(index) else {
                return;
            };
            rate = track.sample_rate().max(1);
            frames = track.len();
            let channels = track.clips.n_channels.max(1);
            painter.rect_filled(rect, 3.0, egui::Color32::from_gray(34));
            painter.text(
                rect.left_top() + egui::vec2(8.0, 5.0),
                egui::Align2::LEFT_TOP,
                format!(
                    "{}  ·  {} ch  ·  {:.2}s",
                    track.name,
                    channels,
                    au_core::frames_to_secs(frames, rate)
                ),
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );

            let body = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top() + 23.0),
                rect.right_bottom(),
            );
            if let Some(selection) = selection {
                let start = au_core::frames_to_secs(selection.start, rate);
                let end = au_core::frames_to_secs(selection.end, rate);
                let left = rect.left() + zoom.time_to_x(start) as f32;
                let right = rect.left() + zoom.time_to_x(end) as f32;
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(left, body.top()),
                        egui::pos2(right.max(left + 1.0), body.bottom()),
                    ),
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(60, 130, 230, 48),
                );
            }

            for (clip_index, clip) in track.clips.clips.iter().enumerate() {
                let clip_start = match drag_preview {
                    Some(PointerDrag::Clip {
                        track: drag_track,
                        clip: drag_clip,
                        current_start,
                        ..
                    }) if drag_track == index && drag_clip == clip_index => current_start,
                    _ => au_core::frames_to_secs(clip.start, rate),
                };
                let clip_len_secs = au_core::frames_to_secs(clip.len(), rate);
                let left = rect.left() + zoom.time_to_x(clip_start) as f32;
                let right = rect.left() + zoom.time_to_x(clip_start + clip_len_secs) as f32;
                let clip_rect = egui::Rect::from_min_max(
                    egui::pos2(left, body.top()),
                    egui::pos2(right.max(left + 2.0), body.bottom()),
                );
                clip_bounds.push((clip.start, clip.end()));

                if clip_rect.intersects(rect) {
                    painter.rect_filled(clip_rect, 4.0, egui::Color32::from_rgb(43, 72, 59));
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            clip_rect.left_top(),
                            egui::pos2(
                                clip_rect.right(),
                                (clip_rect.top() + 19.0).min(clip_rect.bottom()),
                            ),
                        ),
                        4.0,
                        egui::Color32::from_rgb(55, 91, 70),
                    );
                    let speed_text = if (clip.stretch_ratio - 1.0).abs() > f64::EPSILON {
                        format!("  ({:.0}%)", clip.stretch_ratio * 100.0)
                    } else {
                        String::new()
                    };
                    painter.text(
                        clip_rect.left_top() + egui::vec2(5.0, 2.0),
                        egui::Align2::LEFT_TOP,
                        format!("{}{}", clip.name, speed_text),
                        egui::FontId::proportional(11.0),
                        egui::Color32::WHITE,
                    );

                    let rendered = clip.rendered();
                    let rendered_len = rendered.len();
                    if rendered_len > 0 && clip.len() > 0 {
                        let first_x = clip_rect.left().ceil().max(rect.left()) as i32;
                        let last_x = clip_rect.right().floor().min(rect.right()) as i32;
                        let waveform_top = (body.top() + 19.0).min(body.bottom());
                        let waveform_height = (body.bottom() - waveform_top).max(1.0);
                        for channel in 0..rendered.n_channels().min(channels) {
                            let channel_top =
                                waveform_top + channel as f32 * waveform_height / channels as f32;
                            let channel_height = waveform_height / channels as f32;
                            let center = channel_top + channel_height * 0.5;
                            painter.line_segment(
                                [
                                    egui::pos2(clip_rect.left().max(rect.left()), center),
                                    egui::pos2(clip_rect.right().min(rect.right()), center),
                                ],
                                egui::Stroke::new(1.0, egui::Color32::from_gray(78)),
                            );

                            let Some(samples) = rendered.channels.get(channel) else {
                                continue;
                            };
                            for x in first_x..last_x {
                                let local_time_start =
                                    zoom.x_to_time((x as f32 - rect.left()) as f64) - clip_start;
                                let local_time_end =
                                    zoom.x_to_time((x as f32 + 1.0 - rect.left()) as f64) - clip_start;
                                let local_start = (local_time_start.max(0.0)
                                    / clip_len_secs.max(f64::EPSILON)
                                    * rendered_len as f64)
                                    .floor() as usize;
                                let local_end = (local_time_end.max(0.0)
                                    / clip_len_secs.max(f64::EPSILON)
                                    * rendered_len as f64)
                                    .ceil() as usize;
                                let start = local_start.min(samples.len());
                                let end = local_end.max(start + 1).min(samples.len());
                                if start >= end {
                                    continue;
                                }
                                let (low, high) = samples[start..end]
                                    .iter()
                                    .fold((1.0_f32, -1.0_f32), |(low, high), &sample| {
                                        (low.min(sample), high.max(sample))
                                    });
                                let scale = channel_height * 0.44;
                                painter.line_segment(
                                    [
                                        egui::pos2(x as f32, center - high * scale),
                                        egui::pos2(x as f32, center - low * scale),
                                    ],
                                    egui::Stroke::new(
                                        1.0,
                                        egui::Color32::from_rgb(100, 207, 145),
                                    ),
                                );
                            }
                        }
                    }
                }
            }

            envelope_points.extend(
                track
                    .clips
                    .envelope
                    .points
                    .iter()
                    .map(|point| (point.time, point.value)),
            );
            if !envelope_points.is_empty() {
                let value_y = |value: f64| {
                    body.top() + ((2.0 - value.clamp(0.0, 2.0)) / 2.0) as f32 * body.height()
                };
                let points = envelope_points
                    .iter()
                    .enumerate()
                    .map(|(point_index, &(time, value))| match drag_preview {
                        Some(PointerDrag::Envelope {
                            track: drag_track,
                            point,
                            time: preview_time,
                            value: preview_value,
                        }) if drag_track == index && point == point_index => {
                            (preview_time, preview_value)
                        }
                        _ => (time, value),
                    })
                    .collect::<Vec<_>>();
                for pair in points.windows(2) {
                    let (t0, v0) = pair[0];
                    let (t1, v1) = pair[1];
                    painter.line_segment(
                        [
                            egui::pos2(rect.left() + zoom.time_to_x(t0) as f32, value_y(v0)),
                            egui::pos2(rect.left() + zoom.time_to_x(t1) as f32, value_y(v1)),
                        ],
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 160, 255)),
                    );
                }
                for (time, value) in points {
                    painter.circle_filled(
                        egui::pos2(rect.left() + zoom.time_to_x(time) as f32, value_y(value)),
                        3.5,
                        egui::Color32::from_rgb(110, 185, 255),
                    );
                }
            }

            if let Some(frame) = playhead {
                let time = au_core::frames_to_secs(frame.min(frames), rate);
                let x = rect.left() + zoom.time_to_x(time) as f32;
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(2.0, egui::Color32::YELLOW),
                );
            }

            if self.tool == Tool::Envelope {
                painter.rect_stroke(
                    body,
                    0.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(45, 88, 145)),
                );
            }
        }

        if response.drag_started() {
            if let Some(pointer) = response.interact_pointer_pos() {
                self.selected_track = Some(index);
                let time = self.time_at_x(pointer.x, rect);
                match self.tool {
                    Tool::Selection => {
                        let snapped = self.snap_time(time, rate, None);
                        let frame = au_core::secs_to_frames(snapped, rate).min(frames);
                        self.drag = Some(PointerDrag::Selection {
                            track: index,
                            start: frame,
                            end: frame,
                        });
                    }
                    Tool::TimeShift => {
                        let frame = au_core::secs_to_frames(time, rate);
                        if let Some((clip_index, &(start, _end))) = clip_bounds
                            .iter()
                            .enumerate()
                            .find(|(_, (start, end))| frame >= *start && frame < *end)
                        {
                            let original_start = au_core::frames_to_secs(start, rate);
                            self.drag = Some(PointerDrag::Clip {
                                track: index,
                                clip: clip_index,
                                original_start,
                                pointer_start: time,
                                current_start: original_start,
                            });
                        }
                    }
                    Tool::Envelope => {
                        let body_top = rect.top() + 23.0;
                        let body_height = (rect.bottom() - body_top).max(1.0);
                        let nearest = envelope_points
                            .iter()
                            .enumerate()
                            .map(|(point, &(point_time, point_value))| {
                                let x = rect.left() + zoom.time_to_x(point_time) as f32;
                                let y = body_top
                                    + ((2.0 - point_value.clamp(0.0, 2.0)) / 2.0) as f32
                                        * body_height;
                                (point, (egui::pos2(x, y) - pointer).length())
                            })
                            .filter(|(_, distance)| *distance <= 6.0)
                            .min_by(|a, b| a.1.total_cmp(&b.1));
                        if let Some((point, _)) = nearest {
                            self.drag = Some(PointerDrag::Envelope {
                                track: index,
                                point,
                                time: envelope_points[point].0,
                                value: envelope_points[point].1,
                            });
                        }
                    }
                    Tool::Draw => {
                        if self.zoom.pixels_per_sec / (rate as f64) < 1.0 {
                            self.status = "Zoom in to draw samples".to_owned();
                        } else {
                            let frame =
                                au_core::secs_to_frames(time, rate).min(frames.saturating_sub(1));
                            let body_top = rect.top() + 23.0;
                            let body_height = (rect.bottom() - body_top).max(1.0);
                            let value = (1.0
                                - (pointer.y - body_top) as f64 / body_height as f64 * 2.0)
                                .clamp(-1.0, 1.0) as f32;
                            self.draw_edits.clear();
                            self.add_draw_point(frame, value);
                            self.drag = Some(PointerDrag::Draw {
                                track: index,
                                last_frame: frame,
                            });
                        }
                    }
                }
            }
        }

        if response.dragged() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let time = self.time_at_x(pointer.x, rect);
                match self.drag {
                    Some(PointerDrag::Selection { track, start, .. }) if track == index => {
                        let snapped = self.snap_time(time, rate, None);
                        let end = au_core::secs_to_frames(snapped, rate).min(frames);
                        self.drag = Some(PointerDrag::Selection { track, start, end });
                        self.selection = Some(Selection { track, start, end });
                    }
                    Some(PointerDrag::Clip {
                        track,
                        clip,
                        original_start,
                        pointer_start,
                        ..
                    }) if track == index => {
                        let requested = (original_start + time - pointer_start).max(0.0);
                        let snapped = self.snap_time(
                            requested,
                            rate,
                            Some(&self.editor.state().tracks[index].clips),
                        );
                        self.drag = Some(PointerDrag::Clip {
                            track,
                            clip,
                            original_start,
                            pointer_start,
                            current_start: snapped.max(0.0),
                        });
                    }
                    Some(PointerDrag::Envelope { track, point, .. }) if track == index => {
                        let body_top = rect.top() + 23.0;
                        let body_height = (rect.bottom() - body_top).max(1.0);
                        let value =
                            (1.0 - (pointer.y - body_top) as f64 / body_height as f64) * 2.0;
                        self.drag = Some(PointerDrag::Envelope {
                            track,
                            point,
                            time,
                            value: value.clamp(0.0, 2.0),
                        });
                    }
                    Some(PointerDrag::Draw {
                        track,
                        last_frame,
                    }) if track == index => {
                        if self.zoom.pixels_per_sec / (rate as f64) < 1.0 {
                            self.status = "Zoom in to draw samples".to_owned();
                        } else {
                            let current_frame =
                                au_core::secs_to_frames(time, rate).min(frames.saturating_sub(1));
                            let body_top = rect.top() + 23.0;
                            let body_height = (rect.bottom() - body_top).max(1.0);
                            let value = (1.0
                                - (pointer.y - body_top) as f64 / body_height as f64 * 2.0)
                                .clamp(-1.0, 1.0) as f32;
                            let previous_value = self
                                .draw_edits
                                .iter()
                                .rev()
                                .find(|(frame, _)| *frame == last_frame)
                                .map_or(value, |(_, value)| *value);
                            let distance = current_frame.abs_diff(last_frame).max(1);
                            for step in 0..=distance {
                                let frame = if current_frame >= last_frame {
                                    last_frame + step
                                } else {
                                    last_frame - step
                                };
                                let fraction = step as f32 / distance as f32;
                                self.add_draw_point(
                                    frame,
                                    previous_value + (value - previous_value) * fraction,
                                );
                            }
                            self.drag = Some(PointerDrag::Draw {
                                track,
                                last_frame: current_frame,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }

        if response.clicked() {
            self.selected_track = Some(index);
            match self.tool {
                Tool::Selection => {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        let time = self.snap_time(self.time_at_x(pointer.x, rect), rate, None);
                        let frame = au_core::secs_to_frames(time, rate).min(frames);
                        self.selection = Some(Selection {
                            track: index,
                            start: frame,
                            end: frame,
                        });
                        self.selected_label = None;
                    }
                }
                Tool::Envelope => {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        let body_top = rect.top() + 23.0;
                        let body_height = (rect.bottom() - body_top).max(1.0);
                        let value =
                            (1.0 - (pointer.y - body_top) as f64 / body_height as f64) * 2.0;
                        let time = self.time_at_x(pointer.x, rect);
                        if let Err(error) = self.editor.add_envelope_point(
                            index,
                            time,
                            value.clamp(0.0, 2.0),
                        ) {
                            self.status = error.to_string();
                        }
                    }
                }
                Tool::TimeShift | Tool::Draw => {}
            }
        }

        if self.tool == Tool::Envelope
            && (response.secondary_clicked()
                || (response.clicked() && ui.input(|input| input.modifiers.shift)))
        {
            if let Some(pointer) = response.interact_pointer_pos() {
                let nearest = envelope_points
                    .iter()
                    .enumerate()
                    .map(|(point, &(time, value))| {
                        let x = rect.left() + zoom.time_to_x(time) as f32;
                        let y = rect.top()
                            + 23.0
                            + ((2.0 - value.clamp(0.0, 2.0)) / 2.0) as f32
                                * (rect.height() - 23.0);
                        (point, (egui::pos2(x, y) - pointer).length())
                    })
                    .filter(|(_, distance)| *distance <= 8.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((point, _)) = nearest {
                    let result = self.editor.remove_envelope_point(index, point);
                    self.report(result);
                }
            }
        }
    }

    fn draw_labels(&mut self, ui: &mut egui::Ui) {
        let label_tracks = self
            .editor
            .state()
            .labels
            .iter()
            .map(|track| {
                (
                    track.name.clone(),
                    track
                        .labels
                        .iter()
                        .map(|label| (label.start, label.end, label.text.clone()))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let zoom = self.zoom.clone();

        for (track_index, (name, labels)) in label_tracks.into_iter().enumerate() {
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width().max(1.0), 48.0),
                egui::Sense::click_and_drag(),
            );
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 3.0, egui::Color32::from_gray(30));
            painter.text(
                rect.left_top() + egui::vec2(7.0, 3.0),
                egui::Align2::LEFT_TOP,
                name,
                egui::FontId::proportional(11.0),
                egui::Color32::LIGHT_GRAY,
            );

            for (label_index, (start, end, text)) in labels.iter().enumerate() {
                let mut display_start = *start;
                let mut display_end = *end;
                if let Some(PointerDrag::Label {
                    track,
                    label,
                    current_start,
                    current_end,
                    ..
                }) = self.drag
                {
                    if track == track_index && label == label_index {
                        display_start = current_start;
                        display_end = current_end;
                    }
                }
                let x0 = rect.left() + zoom.time_to_x(display_start) as f32;
                let x1 = rect.left() + zoom.time_to_x(display_end) as f32;
                let selected = self.selected_label == Some((track_index, label_index));
                if (display_end - display_start).abs() > f64::EPSILON {
                    let region = egui::Rect::from_min_max(
                        egui::pos2(x0.min(x1), rect.top() + 18.0),
                        egui::pos2(x0.max(x1).max(x0.min(x1) + 2.0), rect.bottom()),
                    );
                    painter.rect_filled(
                        region,
                        2.0,
                        if selected {
                            egui::Color32::from_rgba_unmultiplied(225, 170, 55, 95)
                        } else {
                            egui::Color32::from_rgba_unmultiplied(170, 135, 50, 65)
                        },
                    );
                    painter.line_segment(
                        [egui::pos2(x0, rect.top() + 17.0), egui::pos2(x0, rect.bottom())],
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(235, 190, 75)),
                    );
                    painter.line_segment(
                        [egui::pos2(x1, rect.top() + 17.0), egui::pos2(x1, rect.bottom())],
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(235, 190, 75)),
                    );
                } else {
                    painter.line_segment(
                        [egui::pos2(x0, rect.top() + 16.0), egui::pos2(x0, rect.bottom())],
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(235, 190, 75)),
                    );
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(x0, rect.top() + 17.0),
                            egui::pos2(x0 + 11.0, rect.top() + 19.0),
                            egui::pos2(x0, rect.top() + 29.0),
                        ],
                        if selected {
                            egui::Color32::from_rgb(250, 205, 90)
                        } else {
                            egui::Color32::from_rgb(205, 160, 55)
                        },
                        egui::Stroke::NONE,
                    ));
                }
                painter.text(
                    egui::pos2(x0 + 12.0, rect.top() + 19.0),
                    egui::Align2::LEFT_TOP,
                    text,
                    egui::FontId::proportional(11.0),
                    egui::Color32::WHITE,
                );
            }

            let hit_label = |pointer: egui::Pos2| {
                labels
                    .iter()
                    .enumerate()
                    .find_map(|(label_index, (start, end, _))| {
                        let x0 = rect.left() + zoom.time_to_x(*start) as f32;
                        let x1 = rect.left() + zoom.time_to_x(*end) as f32;
                        let hit = if (end - start).abs() <= f64::EPSILON {
                            (pointer.x - x0).abs() <= 8.0
                        } else {
                            pointer.x >= x0.min(x1) - 4.0 && pointer.x <= x0.max(x1) + 4.0
                        };
                        hit.then_some(label_index)
                    })
            };

            if response.drag_started() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    if let Some(label_index) = hit_label(pointer) {
                        let (start, end, _) = labels[label_index];
                        let time = zoom.x_to_time((pointer.x - rect.left()) as f64).max(0.0);
                        self.selected_label = Some((track_index, label_index));
                        self.drag = Some(PointerDrag::Label {
                            track: track_index,
                            label: label_index,
                            pointer_start: time,
                            original_start: start,
                            original_end: end,
                            current_start: start,
                            current_end: end,
                        });
                    }
                }
            }

            if response.dragged() {
                if let (Some(pointer), Some(PointerDrag::Label {
                    track,
                    label,
                    pointer_start,
                    original_start,
                    original_end,
                    ..
                })) = (response.interact_pointer_pos(), self.drag)
                {
                    if track == track_index {
                        let time = zoom.x_to_time((pointer.x - rect.left()) as f64).max(0.0);
                        let delta = time - pointer_start;
                        self.drag = Some(PointerDrag::Label {
                            track,
                            label,
                            pointer_start,
                            original_start,
                            original_end,
                            current_start: (original_start + delta).max(0.0),
                            current_end: (original_end + delta).max(0.0),
                        });
                    }
                }
            }

            if response.clicked() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    if let Some(label_index) = hit_label(pointer) {
                        self.selected_label = Some((track_index, label_index));
                        let (start, end, text) = &labels[label_index];
                        let wave_track = self.selected_track.unwrap_or(0);
                        let rate = self
                            .editor
                            .state()
                            .tracks
                            .get(wave_track)
                            .map_or(self.editor.state().sample_rate.max(1), Track::sample_rate);
                        self.selected_track = self
                            .editor
                            .state()
                            .tracks
                            .is_empty()
                            .then_some(None)
                            .unwrap_or(Some(wave_track));
                        self.selection = Some(Selection {
                            track: wave_track,
                            start: au_core::secs_to_frames(*start, rate),
                            end: au_core::secs_to_frames(*end, rate),
                        });
                        if response.double_clicked() {
                            self.label_edit = Some(LabelEdit {
                                track: track_index,
                                label: label_index,
                                text: text.clone(),
                            });
                        }
                    }
                }
            }
        }
    }

    fn finish_draw_stroke(&mut self, track: usize) {
        if self.draw_edits.is_empty() {
            return;
        }
        self.draw_edits.sort_by_key(|(frame, _)| *frame);
        self.draw_edits.dedup_by(|a, b| {
            if a.0 == b.0 {
                a.1 = b.1;
                true
            } else {
                false
            }
        });
        let first = self.draw_edits.first().map_or(0, |(frame, _)| *frame);
        let last = self.draw_edits.last().map_or(first, |(frame, _)| *frame);
        let mut values = vec![0.0_f32; last.saturating_sub(first) + 1];
        if let Some(samples) = self
            .editor
            .state()
            .tracks
            .get(track)
            .and_then(|wave_track| wave_track.render().channels.first().cloned())
        {
            for (offset, sample) in values.iter_mut().enumerate() {
                if let Some(value) = samples.get(first + offset) {
                    *sample = *value;
                }
            }
        }
        for &(frame, value) in &self.draw_edits {
            if let Some(sample) = values.get_mut(frame.saturating_sub(first)) {
                *sample = value;
            }
        }
        let result = self.editor.draw_samples(track, first, &values);
        self.report(result);
        self.draw_edits.clear();
    }

    fn finish_pointer_drag(&mut self, ctx: &egui::Context) {
        let released =
            ctx.input(|input| input.pointer.button_released(egui::PointerButton::Primary));
        if !released {
            return;
        }
        let Some(drag) = self.drag.take() else {
            return;
        };
        match drag {
            PointerDrag::Selection { track, start, end } => {
                self.selection = Some(Selection { track, start, end });
            }
            PointerDrag::Clip {
                track,
                clip,
                current_start,
                ..
            } => {
                let rate = self
                    .editor
                    .state()
                    .tracks
                    .get(track)
                    .map_or(1, Track::sample_rate);
                let new_start = au_core::secs_to_frames(current_start.max(0.0), rate) as i64;
                let result = self.editor.move_clip(track, clip, new_start);
                self.report(result);
            }
            PointerDrag::Envelope {
                track,
                point,
                time,
                value,
            } => {
                let result = self
                    .editor
                    .move_envelope_point(track, point, time.max(0.0), value.clamp(0.0, 2.0));
                self.report(result);
            }
            PointerDrag::Label {
                track,
                label,
                current_start,
                current_end,
                ..
            } => {
                let rate = self.selected_rate();
                let start = self.snap_time(current_start, rate, None).max(0.0);
                let end = self.snap_time(current_end, rate, None).max(start);
                let result = self.editor.set_label_times(track, label, start, end);
                self.report(result);
            }
            PointerDrag::Draw { track, .. } => self.finish_draw_stroke(track),
        }
    }

    fn draw_ruler(&self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(1.0), 28.0),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(24));
        painter.line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            egui::Stroke::new(1.0, egui::Color32::from_gray(80)),
        );

        let visible_secs = rect.width() as f64 / self.zoom.pixels_per_sec.max(0.1);
        let raw_step = visible_secs / 8.0;
        let magnitude = 10.0_f64.powf(raw_step.max(0.001).log10().floor());
        let step = [1.0, 2.0, 5.0, 10.0]
            .into_iter()
            .map(|multiplier| multiplier * magnitude)
            .find(|candidate| *candidate >= raw_step)
            .unwrap_or(10.0 * magnitude);
        let start = (self.zoom.h_offset / step).floor() * step;
        let end = self.zoom.h_offset + visible_secs + step;
        let mut time = start;
        while time <= end {
            let x = rect.left() + self.zoom.time_to_x(time) as f32;
            if x >= rect.left() && x <= rect.right() {
                painter.line_segment(
                    [egui::pos2(x, rect.bottom() - 9.0), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(1.0, egui::Color32::GRAY),
                );
                painter.text(
                    egui::pos2(x + 3.0, rect.top() + 2.0),
                    egui::Align2::LEFT_TOP,
                    format!("{time:.2}"),
                    egui::FontId::proportional(10.0),
                    egui::Color32::LIGHT_GRAY,
                );
            }
            time += step;
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        if ui.button("Open / Import").clicked() {
            self.import_file();
        }
        if ui.button("Open Project").clicked() {
            self.open_project();
        }
        if ui.button("Save Project").clicked() {
            self.save_project();
        }
        if ui.button("Export WAV").clicked() {
            self.export(false);
        }
        if ui.button("Export FLAC").clicked() {
            self.export(true);
        }
        ui.separator();
        if ui.button("Play").clicked() {
            self.play();
        }
        if ui.button("Pause").clicked() {
            self.toggle_pause();
        }
        if ui.button("Stop").clicked() {
            self.stop_playback();
        }
        if ui.button("Record").clicked() {
            self.open_record_options();
        }
        if self.recorder.is_some() && ui.button("Stop Record").clicked() {
            self.stop_recording();
        }
        if ui.button("Mixer").clicked() {
            self.mixer_visible = !self.mixer_visible;
        }
        ui.separator();
        if ui.button("Cut").clicked() {
            if let Some(index) = self.selected_track {
                let selection = self.selection_for_track(index);
                self.edit_selection(|editor| editor.cut(selection));
            }
        }
        if ui.button("Copy").clicked() {
            if let Some(index) = self.selected_track {
                let selection = self.selection_for_track(index);
                let result = self.editor.copy(selection);
                self.report(result);
            }
        }
        if ui.button("Paste").clicked() {
            if let Some(index) = self.selected_track {
                let at = self.selection_for_track(index).start;
                self.edit_selection(|editor| editor.paste(index, at));
            }
        }
        if ui.button("Delete").clicked() {
            if let Some(index) = self.selected_track {
                let selection = self.selection_for_track(index);
                self.edit_selection(|editor| editor.delete(selection));
            }
        }
        if ui.button("Undo").clicked() {
            let result = self.editor.undo();
            self.report(result);
        }
        if ui.button("Redo").clicked() {
            let result = self.editor.redo();
            self.report(result);
        }
    }

    fn draw_windows(&mut self, ctx: &egui::Context) {
        if self.speed_window {
            let mut open = self.speed_window;
            let mut apply = false;
            egui::Window::new("Change Clip Speed")
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("Speed:");
                    ui.add(
                        egui::DragValue::new(&mut self.speed_percent)
                            .speed(1.0)
                            .suffix("%"),
                    );
                    if ui.button("Apply").clicked() {
                        apply = true;
                    }
                });
            self.speed_window = open;
            if apply {
                self.apply_clip_speed();
            }
        }

        if self.record_opts_window {
            let mut open = self.record_opts_window;
            let mut start_recording = false;
            egui::Window::new("Recording options")
                .open(&mut open)
                .show(ctx, |ui| {
                    egui::ComboBox::from_label("Mode")
                        .selected_text(match self.record_mode {
                            RecordModeUi::NewTrack => "New track",
                            RecordModeUi::Overdub => "Overdub",
                            RecordModeUi::PunchAndRoll => "Punch and roll",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.record_mode,
                                RecordModeUi::NewTrack,
                                "New track",
                            );
                            ui.selectable_value(
                                &mut self.record_mode,
                                RecordModeUi::Overdub,
                                "Overdub",
                            );
                            ui.selectable_value(
                                &mut self.record_mode,
                                RecordModeUi::PunchAndRoll,
                                "Punch and roll",
                            );
                        });
                    if self.record_mode == RecordModeUi::PunchAndRoll {
                        ui.add(
                            egui::DragValue::new(&mut self.record_preroll_secs)
                                .speed(0.1)
                                .range(0.0..=60.0)
                                .suffix(" s preroll"),
                        );
                        ui.label("Punch uses the current selection on the selected track.");
                    }
                    ui.add(
                        egui::DragValue::new(&mut self.record_latency_ms)
                            .speed(0.1)
                            .range(-500.0..=500.0)
                            .suffix(" ms latency"),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.record_crossfade_ms)
                            .speed(0.1)
                            .range(0.0..=1000.0)
                            .suffix(" ms crossfade"),
                    );

                    ui.separator();
                    egui::ComboBox::from_label("Trigger")
                        .selected_text(match self.record_trigger {
                            RecordTriggerUi::Immediate => "Immediate",
                            RecordTriggerUi::Timer => "Timer",
                            RecordTriggerUi::SoundActivated => "Sound activated",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.record_trigger,
                                RecordTriggerUi::Immediate,
                                "Immediate",
                            );
                            ui.selectable_value(
                                &mut self.record_trigger,
                                RecordTriggerUi::Timer,
                                "Timer",
                            );
                            ui.selectable_value(
                                &mut self.record_trigger,
                                RecordTriggerUi::SoundActivated,
                                "Sound activated",
                            );
                        });
                    match self.record_trigger {
                        RecordTriggerUi::Immediate => {}
                        RecordTriggerUi::Timer => {
                            ui.add(
                                egui::DragValue::new(&mut self.timer_start_secs)
                                    .speed(0.1)
                                    .range(0.0..=86_400.0)
                                    .suffix(" s start in"),
                            );
                            ui.add(
                                egui::DragValue::new(&mut self.timer_duration_secs)
                                    .speed(0.1)
                                    .range(0.0..=86_400.0)
                                    .suffix(" s duration"),
                            );
                        }
                        RecordTriggerUi::SoundActivated => {
                            ui.add(
                                egui::DragValue::new(&mut self.sound_threshold_db)
                                    .speed(0.5)
                                    .range(-96.0..=0.0)
                                    .suffix(" dB threshold"),
                            );
                            ui.add(
                                egui::DragValue::new(&mut self.sound_hold_secs)
                                    .speed(0.1)
                                    .range(0.0..=60.0)
                                    .suffix(" s hold"),
                            );
                        }
                    }

                    egui::ComboBox::from_label("Input")
                        .selected_text(match self.input_kind {
                            InputKind::Default => "Default input",
                            InputKind::Loopback => "System audio loopback",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.input_kind,
                                InputKind::Default,
                                "Default input",
                            );
                            ui.selectable_value(
                                &mut self.input_kind,
                                InputKind::Loopback,
                                "System audio loopback",
                            );
                        });
                    ui.small(
                        "Windows only; on Linux pick a Monitor source; macOS unsupported",
                    );
                    if ui.button("Start Recording").clicked() {
                        start_recording = true;
                    }
                });
            self.record_opts_window = open;
            if start_recording {
                self.begin_recording();
            }
        }

        if self.export_options_window {
            let mut open = self.export_options_window;
            egui::Window::new("Export options")
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Bits:");
                        egui::ComboBox::from_id_salt("export-bits")
                            .selected_text(self.export_bits.to_string())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.export_bits, 16, "16");
                                ui.selectable_value(&mut self.export_bits, 24, "24");
                                ui.selectable_value(&mut self.export_bits, 32, "32");
                            });
                    });
                    egui::ComboBox::from_label("Dither")
                        .selected_text(match self.export_dither {
                            Dither::None => "None",
                            Dither::Rectangle => "Rectangle",
                            Dither::Triangle => "Triangle",
                            Dither::Shaped => "Shaped",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.export_dither, Dither::None, "None");
                            ui.selectable_value(
                                &mut self.export_dither,
                                Dither::Rectangle,
                                "Rectangle",
                            );
                            ui.selectable_value(
                                &mut self.export_dither,
                                Dither::Triangle,
                                "Triangle",
                            );
                            ui.selectable_value(&mut self.export_dither, Dither::Shaped, "Shaped");
                        });
                });
            self.export_options_window = open;
        }

        if self.label_edit.is_some() {
            let mut open = true;
            let mut apply = false;
            egui::Window::new("Edit Label")
                .open(&mut open)
                .show(ctx, |ui| {
                    if let Some(edit) = self.label_edit.as_mut() {
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut edit.text).hint_text("Label text"),
                        );
                        if response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter))
                        {
                            apply = true;
                        }
                        if ui.button("Apply").clicked() {
                            apply = true;
                        }
                    }
                });
            if !open {
                self.label_edit = None;
            } else if apply {
                if let Some(edit) = self.label_edit.take() {
                    let result = self.editor.edit_label(edit.track, edit.label, &edit.text);
                    self.report(result);
                }
            }
        }
    }

    fn mixer_panel(&mut self, ctx: &egui::Context) {
        if !self.mixer_visible {
            return;
        }
        self.ensure_track_mix();
        let track_info = self
            .editor
            .state()
            .tracks
            .iter()
            .map(|track| track.name.clone())
            .collect::<Vec<_>>();
        egui::SidePanel::right("mixer")
            .default_width(250.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Mixer");
                ui.separator();
                for (index, name) in track_info.iter().enumerate() {
                    let mut params = self.track_mix[index];
                    let mut commit = false;
                    ui.group(|ui| {
                        ui.label(name);
                        let gain_now = params.gain;
                        let response = ui.add(
                            egui::Slider::new(&mut params.gain, 0.0..=2.0)
                                .text(format!(
                                    "Gain ({:.1} dB)",
                                    if gain_now > 0.0 {
                                        20.0 * gain_now.log10()
                                    } else {
                                        f32::NEG_INFINITY
                                    }
                                )),
                        );
                        if response.changed() {
                            self.send_track_mix(index, params, false);
                        }
                        if response.drag_stopped() || (response.changed() && !response.dragged()) {
                            commit = true;
                        }

                        let pan_response =
                            ui.add(egui::Slider::new(&mut params.pan, -1.0..=1.0).text("Pan"));
                        if pan_response.changed() {
                            self.send_track_mix(index, params, false);
                        }
                        if pan_response.drag_stopped()
                            || (pan_response.changed() && !pan_response.dragged())
                        {
                            commit = true;
                        }

                        let mute_changed = ui.checkbox(&mut params.mute, "Mute").changed();
                        let solo_changed = ui.checkbox(&mut params.solo, "Solo").changed();
                        if mute_changed || solo_changed {
                            commit = true;
                        }
                        if mute_changed || solo_changed {
                            self.send_track_mix(index, params, false);
                        }
                    });
                    self.track_mix[index] = params;
                    if commit {
                        self.send_track_mix(index, params, true);
                    }
                }
                ui.separator();
                let master_response = ui.add(
                    egui::Slider::new(&mut self.master_gain, 0.0..=2.0).text("Master gain"),
                );
                if master_response.changed() {
                    if let Some(player) = self.player.as_mut() {
                        player
                            .handle
                            .send(EngineCommand::SetMasterGain(self.master_gain));
                    }
                }
            });
    }

    fn draw_meter_bar(ui: &mut egui::Ui, label: &str, peak: f32, rms: f32) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(190.0, 18.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, egui::Color32::from_gray(38));
        let db_position = |value: f32| {
            if value <= 0.0 {
                0.0
            } else {
                ((20.0 * value.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
            }
        };
        let rms_width = rect.width() * db_position(rms);
        let peak_width = rect.width() * db_position(peak);
        painter.rect_filled(
            egui::Rect::from_min_max(
                rect.left_top(),
                egui::pos2(rect.left() + rms_width, rect.bottom()),
            ),
            2.0,
            egui::Color32::from_rgb(55, 145, 85),
        );
        painter.line_segment(
            [
                egui::pos2(rect.left() + peak_width, rect.top()),
                egui::pos2(rect.left() + peak_width, rect.bottom()),
            ],
            egui::Stroke::new(
                2.0,
                if peak >= 1.0 {
                    egui::Color32::RED
                } else {
                    egui::Color32::LIGHT_GREEN
                },
            ),
        );
        painter.text(
            rect.left_top() + egui::vec2(4.0, 1.0),
            egui::Align2::LEFT_TOP,
            format!(
                "{label}  P {:.1} dB  R {:.1} dB",
                if peak > 0.0 {
                    20.0 * peak.log10()
                } else {
                    -60.0
                },
                if rms > 0.0 {
                    20.0 * rms.log10()
                } else {
                    -60.0
                }
            ),
            egui::FontId::proportional(10.0),
            egui::Color32::WHITE,
        );
    }

    fn selection_toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let track_rate = self
            .selection
            .and_then(|selection| self.editor.state().tracks.get(selection.track))
            .map_or(self.selected_rate(), Track::sample_rate)
            .max(1);
        let current = self.selection;
        let start_secs = current.map_or(0.0, |selection| {
            au_core::frames_to_secs(selection.start, track_rate)
        });
        let end_secs = current.map_or(0.0, |selection| {
            au_core::frames_to_secs(selection.end, track_rate)
        });
        let start_id = egui::Id::new("selection-start-field");
        let end_id = egui::Id::new("selection-end-field");
        let length_id = egui::Id::new("selection-length-field");
        if !ctx.memory(|memory| memory.has_focus(start_id)) {
            self.selection_start_text = format_time(start_secs);
        }
        if !ctx.memory(|memory| memory.has_focus(end_id)) {
            self.selection_end_text = format_time(end_secs);
        }
        if !ctx.memory(|memory| memory.has_focus(length_id)) {
            self.selection_length_text = format_time((end_secs - start_secs).max(0.0));
        }

        let mut update = None;
        ui.horizontal(|ui| {
            ui.label("Start");
            let start_response = ui.add(
                egui::TextEdit::singleline(&mut self.selection_start_text)
                    .id(start_id)
                    .desired_width(94.0),
            );
            if start_response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
            {
                update = parse_time(&self.selection_start_text).map(|start| {
                    let end = parse_time(&self.selection_end_text).unwrap_or(end_secs);
                    (start, end)
                });
                if update.is_none() {
                    self.status = "Invalid Start time; expected hh:mm:ss.mmm".to_owned();
                }
            }

            ui.label("End");
            let end_response = ui.add(
                egui::TextEdit::singleline(&mut self.selection_end_text)
                    .id(end_id)
                    .desired_width(94.0),
            );
            if end_response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
            {
                update = parse_time(&self.selection_end_text).map(|end| {
                    let start = parse_time(&self.selection_start_text).unwrap_or(start_secs);
                    (start, end)
                });
                if update.is_none() {
                    self.status = "Invalid End time; expected hh:mm:ss.mmm".to_owned();
                }
            }

            ui.label("Length");
            let length_response = ui.add(
                egui::TextEdit::singleline(&mut self.selection_length_text)
                    .id(length_id)
                    .desired_width(94.0),
            );
            if length_response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
            {
                update = parse_time(&self.selection_length_text).map(|length| {
                    let start = parse_time(&self.selection_start_text).unwrap_or(start_secs);
                    (start, start + length)
                });
                if update.is_none() {
                    self.status = "Invalid Length; expected hh:mm:ss.mmm".to_owned();
                }
            }
            ui.separator();
            ui.label(format!("Snap: {}", snap_mode_name(self.snapper.mode.clone())));
        });
        if let Some((start, end)) = update {
            if let Some(track) = self.selected_track {
                let start = start.max(0.0);
                let end = end.max(start);
                self.selection = Some(Selection {
                    track,
                    start: au_core::secs_to_frames(start, track_rate),
                    end: au_core::secs_to_frames(end, track_rate),
                });
            } else {
                self.status = "Select a track before editing the selection".to_owned();
            }
        }
    }
}

impl eframe::App for AudacityApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // The panic hook has already written the crash report; flush unsaved
        // edits to the autosave before the panic continues to unwind.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.update_frame(ctx, frame)));
        if let Err(p) = r {
            self.flush_autosave();
            std::panic::resume_unwind(p);
        }
    }
}

impl AudacityApp {
    fn update_frame(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ctx);

        let dt = ctx.input(|input| input.unstable_dt).max(0.0);
        self.record_peak *= (-dt * 2.0).exp();
        if self.recorder.is_some() {
            self.process_recording();
        }
        if self.player.as_ref().is_some_and(Player::is_finished) {
            self.player = None;
        }
        self.ensure_track_mix();
        self.maybe_autosave();
        self.recovery_dialog(ctx);
        self.crash_report_dialog(ctx);
        match &self.project_path {
            Some(p) => au_runtime::crash::set_context("project", p.display().to_string()),
            None => au_runtime::crash::clear_context("project"),
        }
        self.effect_windows(ctx);

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Import audio…").clicked() {
                        ui.close_menu();
                        self.import_file();
                    }
                    if ui.button("Open… (.aup3 / .aurs)").clicked() {
                        ui.close_menu();
                        self.open_project();
                    }
                    if ui.button("Save Project").clicked() {
                        ui.close_menu();
                        self.save_project();
                    }
                    if ui.button("Save Project As…").clicked() {
                        ui.close_menu();
                        self.save_project_as();
                    }
                    if ui.button("Export WAV…").clicked() {
                        ui.close_menu();
                        self.export(false);
                    }
                    if ui.button("Export FLAC…").clicked() {
                        ui.close_menu();
                        self.export(true);
                    }
                    if ui.button("Export options…").clicked() {
                        ui.close_menu();
                        self.export_options_window = true;
                    }
                    ui.separator();
                    if ui.button("Import Labels…").clicked() {
                        ui.close_menu();
                        self.import_labels();
                    }
                    if ui.button("Export Labels…").clicked() {
                        ui.close_menu();
                        self.export_labels();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui
                        .selectable_label(self.mixer_visible, "Mixer")
                        .clicked()
                    {
                        self.mixer_visible = !self.mixer_visible;
                        ui.close_menu();
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.button("Cut").clicked() {
                        ui.close_menu();
                        if let Some(index) = self.selected_track {
                            let selection = self.selection_for_track(index);
                            self.edit_selection(|editor| editor.cut(selection));
                        }
                    }
                    if ui.button("Copy").clicked() {
                        ui.close_menu();
                        if let Some(index) = self.selected_track {
                            let selection = self.selection_for_track(index);
                            let result = self.editor.copy(selection);
                            self.report(result);
                        }
                    }
                    if ui.button("Paste").clicked() {
                        ui.close_menu();
                        if let Some(index) = self.selected_track {
                            let at = self.selection_for_track(index).start;
                            self.edit_selection(|editor| editor.paste(index, at));
                        }
                    }
                    if ui.button("Delete").clicked() {
                        ui.close_menu();
                        if let Some(index) = self.selected_track {
                            let selection = self.selection_for_track(index);
                            self.edit_selection(|editor| editor.delete(selection));
                        }
                    }
                    ui.separator();
                    if ui.button("Split  (Ctrl+I)").clicked() {
                        ui.close_menu();
                        self.split_selection();
                    }
                    if ui.button("Join  (Ctrl+J)").clicked() {
                        ui.close_menu();
                        self.join_selection();
                    }
                    if ui.button("Change Clip Speed…").clicked() {
                        ui.close_menu();
                        self.speed_window = true;
                    }
                    ui.separator();
                    if ui.button("Undo").clicked() {
                        ui.close_menu();
                        let result = self.editor.undo();
                        self.report(result);
                    }
                    if ui.button("Redo").clicked() {
                        ui.close_menu();
                        let result = self.editor.redo();
                        self.report(result);
                    }
                });
                ui.menu_button("Tracks", |ui| {
                    if ui.button("Add Label Track").clicked() {
                        ui.close_menu();
                        self.editor.add_label_track("Labels");
                        self.status = "Label track added".to_owned();
                    }
                    if ui.button("Add Label at Selection  (Ctrl+B)").clicked() {
                        ui.close_menu();
                        self.add_label_at_selection();
                    }
                    if ui.button("Import Labels…").clicked() {
                        ui.close_menu();
                        self.import_labels();
                    }
                    if ui.button("Export Labels…").clicked() {
                        ui.close_menu();
                        self.export_labels();
                    }
                    ui.separator();
                    let mut sync_lock = self.sync_lock;
                    if ui.checkbox(&mut sync_lock, "Sync-Lock").changed() {
                        self.sync_lock = sync_lock;
                        self.editor.set_sync_lock(sync_lock);
                    }
                });
                for (menu, kind) in [("Generate", EffectKind::Generate), ("Effect", EffectKind::Process), ("Analyze", EffectKind::Analyze)] {
                    ui.menu_button(menu, |ui| {
                        if kind == EffectKind::Process && ui.button("Realtime Effects…").clicked() {
                            ui.close_menu();
                            self.rt_window = true;
                        }
                        egui::ScrollArea::vertical().max_height(500.0).show(ui, |ui| {
                            let mut items: Vec<(String, &'static str)> = au_effects::registry::all().iter()
                                .filter(|e| e.kind() == kind).map(|e| (e.name().to_string(), e.id())).collect();
                            items.sort();
                            for (name, id) in items {
                                if ui.button(format!("{name}…")).clicked() {
                                    ui.close_menu();
                                    match id {
                                        "PlotSpectrum" => self.open_spectrum(),
                                        "Contrast" => self.contrast_win = Some(crate::analyze_ui::ContrastWindow::new()),
                                        _ => self.fx_dialog = crate::fx_ui::EffectDialog::open(id),
                                    }
                                }
                            }
                        });
                    });
                }
                self.toolbar(ui);
            });

            ui.horizontal(|ui| {
                ui.label("Tool:");
                ui.selectable_value(&mut self.tool, Tool::Selection, "Selection");
                ui.selectable_value(&mut self.tool, Tool::Envelope, "Envelope");
                ui.selectable_value(&mut self.tool, Tool::TimeShift, "Time Shift");
                ui.selectable_value(&mut self.tool, Tool::Draw, "Draw");
                ui.separator();
                if ui.button("Zoom In").clicked() {
                    self.zoom.zoom_in();
                }
                if ui.button("Zoom Out").clicked() {
                    self.zoom.zoom_out();
                }
                if ui.button("Fit").clicked() {
                    self.fit_zoom(1000.0);
                }
                ui.separator();
                egui::ComboBox::from_id_salt("snap-mode")
                    .selected_text(match self.snapper.mode {
                        SnapMode::Off => "Snap: Off",
                        SnapMode::Grid { secs } if (secs - 1.0).abs() < f64::EPSILON => {
                            "Snap: Grid 1s"
                        }
                        SnapMode::Grid { .. } => "Snap: Grid 0.1s",
                        SnapMode::Samples => "Snap: Samples",
                        SnapMode::ZeroCrossing => "Snap: Zero crossings",
                        SnapMode::Labels => "Snap: Labels",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.snapper.mode, SnapMode::Off, "Off");
                        ui.selectable_value(
                            &mut self.snapper.mode,
                            SnapMode::Grid { secs: 1.0 },
                            "Grid 1s",
                        );
                        ui.selectable_value(
                            &mut self.snapper.mode,
                            SnapMode::Grid { secs: 0.1 },
                            "Grid 0.1s",
                        );
                        ui.selectable_value(&mut self.snapper.mode, SnapMode::Samples, "Samples");
                        ui.selectable_value(
                            &mut self.snapper.mode,
                            SnapMode::ZeroCrossing,
                            "Zero crossings",
                        );
                        ui.selectable_value(&mut self.snapper.mode, SnapMode::Labels, "Labels");
                    });
                if ui.checkbox(&mut self.sync_lock, "Sync-Lock").changed() {
                    self.editor.set_sync_lock(self.sync_lock);
                }
                ui.separator();
                ui.checkbox(&mut self.loop_play, "Loop");
                let mut speed = self.play_speed;
                let speed_response =
                    ui.add(egui::Slider::new(&mut speed, 0.1..=3.0).text("Play-at-speed"));
                if speed_response.changed() {
                    self.set_play_speed(speed);
                }
                if ui.button("Rewind").clicked() {
                    self.seek_transport(0);
                }
                if ui.button("End").clicked() {
                    let end = au_core::secs_to_frames(self.project_duration(), self.project_rate());
                    self.seek_transport(end);
                }
            });

            ui.horizontal(|ui| {
                let duration = self.project_duration();
                let visible = ui.available_width() as f64 / self.zoom.pixels_per_sec.max(0.1);
                let max_offset = (duration - visible).max(0.0);
                self.zoom.h_offset = self.zoom.h_offset.clamp(0.0, max_offset);
                ui.label("Horizontal scroll:");
                ui.add(
                    egui::Slider::new(&mut self.zoom.h_offset, 0.0..=max_offset)
                        .show_value(false),
                );
            });
        });

        self.mixer_panel(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            let canvas_left = ui.min_rect().left();
            let wheel = ctx.input(|input| {
                (
                    input.modifiers.ctrl,
                    input.raw_scroll_delta.y,
                    input.pointer.hover_pos(),
                )
            });
            if wheel.0 && wheel.1.abs() > 0.0 {
                if let Some(pointer) = wheel.2 {
                    let anchor_x = (pointer.x - canvas_left).max(0.0) as f64;
                    self.zoom
                        .zoom_by((wheel.1 as f64 * 0.01).exp(), anchor_x);
                }
            }

            self.draw_ruler(ui);
            egui::ScrollArea::vertical().show(ui, |ui| {
                let count = self.editor.state().tracks.len();
                for index in 0..count {
                    self.draw_track(ui, index);
                    ui.add_space(5.0);
                }
                self.draw_labels(ui);
                if count == 0 && self.editor.state().labels.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label("Open or import audio to begin.");
                    });
                }
            });
            self.finish_pointer_drag(ctx);
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.separator();
                if let Some((track, label)) = self.selected_label {
                    ui.label(format!("Label {}:{}", track + 1, label + 1));
                }
                if self.player.is_some() {
                    ui.label("Playhead active");
                }
            });

            self.selection_toolbar(ui, ctx);

            ui.horizontal(|ui| {
                let meters = self
                    .player
                    .as_ref()
                    .map(|player| player.handle.meters.load());
                let reading = meters.unwrap_or(au_io::engine::MeterReading {
                    peak: [0.0; 2],
                    rms: [0.0; 2],
                });
                Self::draw_meter_bar(ui, "L", reading.peak[0], reading.rms[0]);
                Self::draw_meter_bar(ui, "R", reading.peak[1], reading.rms[1]);
                Self::draw_meter_bar(ui, "REC", self.record_peak, self.record_peak * 0.7);
            });
        });

        self.draw_windows(ctx);

        if self.player.is_some() || self.recorder.is_some() {
            ctx.request_repaint();
        }
    }
}

fn format_time(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1000.0).round() as u64;
    let hours = millis / 3_600_000;
    let minutes = (millis / 60_000) % 60;
    let secs = (millis / 1_000) % 60;
    let remainder = millis % 1_000;
    format!("{hours:02}:{minutes:02}:{secs:02}.{remainder:03}")
}

fn parse_time(text: &str) -> Option<f64> {
    let mut parts = text.trim().split(':');
    let hours = parts.next()?.parse::<u64>().ok()?;
    let minutes = parts.next()?.parse::<u64>().ok()?;
    let seconds = parts.next()?.parse::<f64>().ok()?;
    if parts.next().is_some() || minutes >= 60 || !(0.0..60.0).contains(&seconds) {
        return None;
    }
    Some(hours as f64 * 3600.0 + minutes as f64 * 60.0 + seconds)
}

fn snap_mode_name(mode: SnapMode) -> &'static str {
    match mode {
        SnapMode::Off => "Off",
        SnapMode::Grid { secs } if (secs - 1.0).abs() < f64::EPSILON => "Grid 1s",
        SnapMode::Grid { .. } => "Grid 0.1s",
        SnapMode::Samples => "Samples",
        SnapMode::ZeroCrossing => "Zero crossings",
        SnapMode::Labels => "Labels",
    }
}

trait ProcessEffect {
    fn process(&mut self, buffer: &mut AudioBuffer);
}

impl<T: au_effects::Effect + ?Sized> ProcessEffect for T {
    fn process(&mut self, buffer: &mut AudioBuffer) {
        au_effects::Effect::process(self, buffer);
    }
}
