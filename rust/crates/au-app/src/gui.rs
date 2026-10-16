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

/// Runs the app, optionally opening a project (`.aup3` / `.aurs`) or importing an audio file at start.
pub fn run_with(open: Option<PathBuf>) -> eframe::Result<()> {
    run_with_files(open.into_iter().collect())
}

/// Starts the GUI and opens/imports each command-line file in order.
pub fn run_with_files(files: Vec<PathBuf>) -> eframe::Result<()> {
    let mut files = files.into_iter();
    let open = files.next();
    let rest: Vec<PathBuf> = files.collect();
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
            // Untitled projects journal into the temporary directory; offer leftovers.
            let open = open.or_else(|| {
                leftover_sessions(&app.settings.temp_dir).into_iter().find(|p| {
                    au_project::aup3::inspect(p).is_ok_and(|i| i.needs_recovery)
                })
            });
            // open_path imports anything that is not a project (8c9ec90).
            if let Some(path) = open {
                app.open_path(path);
            }
            for path in rest {
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


mod accessibility;
#[cfg(test)]
mod layout_tests;
mod progress;
mod toolbars;
mod extra_glue;
pub(crate) mod waveform_cache;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Selection,
    Envelope,
    TimeShift,
    Draw,
    /// Multi-tool (Extra > Tools > Multi Tool); acts as the selection tool here.
    Multi,
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
    room_noise: crate::room_noise::RoomNoise,
    noise_capture: Option<Recorder>,
    live_denoised: Vec<f32>,
    live_take_active: bool,
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
    /// Untitled project journal in the temporary directory (Audacity's
    /// TempDirectory::UnsavedProjectFileName), removed once saved or on a clean exit.
    session_file: Option<PathBuf>,
    /// Autosave being written on a worker thread (the first journal of a long project
    /// takes seconds; the UI must keep running).
    autosave_job: Option<std::thread::JoinHandle<Result<(), String>>>,
    tool: Tool,
    zoom: ZoomInfo,
    /// Shared progress dialog / cancel UI for long jobs.
    progress: progress::ProgressCenter,
    /// Jobs started by the `ProgressTest` scripting hook.
    test_jobs: Vec<(progress::JobTicket, au_runtime::JobHandle<()>)>,
    /// File > Import / Export jobs running on `io_runtime` with progress dialogs.
    io_jobs: Vec<(progress::JobTicket, IoJob)>,
    io_runtime: au_runtime::Runtime,
    /// Audacity preferences (audacity.cfg format) and the dialog editing them.
    prefs: au_core::prefs::Prefs,
    prefs_dialog: crate::prefs_dialog::PreferencesDialog,
    /// Active keyboard shortcuts (Audacity command ids).
    keymap: crate::shortcuts::KeyMap,
    keys_filter: String,
    keys_xml_path: String,
    prefs_applied: bool,
    /// Typed view of `prefs` (refreshed by apply_prefs).
    settings: crate::settings::AppSettings,
    /// /GUI/Theme value last applied (None until startup; the theme menu owns theme.cfg).
    applied_theme_pref: Option<String>,
    /// Recording dropped samples (Audacity DropoutDetector) -> warning dialog.
    dropout_warning: bool,
    dropout_dont_show: bool,
    zoom_toggle_second: bool,
    /// Top-level menu titles as last rendered (test hook GetMenuTitles).
    menu_titles: Vec<String>,
    /// Pool for scripting test jobs.
    runtime_for_tests: au_runtime::Runtime,
    /// Waveform min/max summaries computed on the au-runtime pool.
    waveform_cache: waveform_cache::WaveformCache,
    /// Track with keyboard focus in the track panel.
    focused_track: Option<usize>,
    /// Active colour theme.
    theme: crate::theme::Theme,
    /// Custom theme loaded from components or a cache.
    custom_theme: Option<crate::theme::Theme>,
    /// Whether the theme has been applied to the egui context.
    theme_applied: bool,
    /// Toolbar docking layout (persisted to toolbars.cfg).
    toolbars: toolbars::ToolbarLayout,
    /// The selected drawing mode for each track.
    track_views: Vec<crate::spectrogram_view::TrackView>,
    /// Per-track waveform view state: height, collapsed, dB scale, vertical zoom bounds.
    wave_ui: Vec<WaveUi>,
    /// Width of the track canvas last frame (screen end time for seeking).
    canvas_width: f32,
    /// Time under the mouse in the track panel (TrackPanel::GetMostRecentXPos).
    last_pointer_time: Option<f64>,
    /// End of a bounded Extra-menu play.
    play_stop: Option<extra_glue::PlayStop>,
    /// Keyboard scrub (U/I held).
    key_scrub: Option<extra_glue::KeyScrub>,
    /// Last keyboard seek, for the 4× repeat speed-up.
    last_seek: Option<std::time::Instant>,
    /// Snap-To Prior (floor) vs Nearest, and the mode Snap-To Off replaced.
    snap_prior: bool,
    snap_restore: au_core::timeline::SnapMode,
    extra_dialog: Option<extra_glue::ExtraDialog>,
    /// Recording volume slider (no hardware mixer through cpal: display only, like Audacity without one).
    input_volume: f32,
    /// OS input mixer when available, else software gain applied in the capture callback.
    record_volume: au_io::input_gain::RecordVolume,
    /// The command being run came from a key press (key-up matters for scrubbing).
    keyboard_event: bool,
    /// Undo pushes already handled by Tracks > Auto-fit track height.
    fit_seen_pushes: u64,
    /// Spectrogram textures and background computation jobs.
    spectrogram_cache: crate::spectrogram_view::SpectrogramCache,
    /// Display settings used by spectrogram tracks.
    spectrogram_settings: crate::spectrogram_view::ViewSettings,
    spectrogram_settings_open: bool,
    macros: MacrosDialog,
    /// Spectral selection retained until it is part of the core model.
    spectral_selection: Option<crate::spectrogram_view::SpectralSelection>,
    snapper: Snapper,
    sync_lock: bool,
    drag: Option<PointerDrag>,
    draw_edits: Vec<(usize, f32)>,
    selected_label: Option<(usize, usize)>,
    label_edit: Option<LabelEdit>,
    /// Label track that has keyboard focus (last clicked), for Type to create a label.
    label_focus: Option<usize>,
    /// Tracks Behaviors > Use dialog for the name of a new label: (start, end, text).
    new_label_dialog: Option<(f64, f64, String)>,
    speed_window: bool,
    speed_percent: f64,
    loop_play: bool,
    play_speed: f64,
    master_gain: f32,
    track_mix: Vec<TrackParams>,
    mixer_visible: bool,
    export_options_window: bool,
    export_audio_dialog: Option<crate::export_glue::ExportAudioDialog>,
    export_multi_dialog: Option<crate::export_ui::ExportMultipleDialog>,
    metadata_editor: Option<crate::export_ui::MetadataEditor>,
    export_bits: u16,
    export_dither: Dither,
    selection_start_text: String,
    selection_end_text: String,
    selection_length_text: String,
    fx_dialog: Option<crate::fx_ui::EffectDialog>,
    /// Plugin Manager window state (open when Some).
    plugin_manager: Option<crate::plugin_manager::PluginManagerState>,

/// Active mod-script-pipe server, when enabled.
script_pipe: Option<crate::script_bridge::ScriptPipe>,
/// Whether the mod-script-pipe server should be enabled.
script_pipe_enabled: bool,
/// Prevents repeated server-start attempts after a startup failure.
script_pipe_failed: bool,
/// Whether the FFmpeg status window is open.
ffmpeg_window: bool,
/// Detected FFmpeg availability and version or missing-library explanation.
ffmpeg_status: au_io::ffmpeg::FfmpegStatus,
    /// Catalog of discovered CLAP and VST3 plugins.
    plugins: crate::plugin_bridge::PluginCatalog,
    /// Crash-isolated editor process for the currently edited plugin, if any.
    plugin_editor: Option<(String, au_plugin_host::editor::EditorProcess)>,
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
        let record_volume = au_io::input_gain::RecordVolume::new(
            au_io::input_gain::PulseSourceMixer::detect(None)
                .map(|m| Box::new(m) as Box<dyn au_io::input_gain::InputMixer>),
        );
        Self {
            editor: Editor::new(au_core::ProjectState::default()),
            selected_track: None,
            selection: None,
            player: None,
            recorder: None,
            record_controller: None,
            record_buffer: Vec::new(),
            room_noise: crate::room_noise::RoomNoise::new(),
            noise_capture: None,
            live_denoised: Vec::new(),
            live_take_active: false,
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
            session_file: None,
            autosave_job: None,
            tool: Tool::Selection,
            track_views: Vec::new(),
            wave_ui: Vec::new(),
            canvas_width: 800.0,
            last_pointer_time: None,
            play_stop: None,
            key_scrub: None,
            last_seek: None,
            snap_prior: false,
            snap_restore: au_core::timeline::SnapMode::Grid { secs: 1.0 },
            extra_dialog: None,
            // Start the slider at the OS mixer's level (as Audacity does), else 1.0.
            input_volume: record_volume.get(),
            record_volume,
            keyboard_event: false,
            fit_seen_pushes: 0,
            spectrogram_cache: crate::spectrogram_view::SpectrogramCache::new(),
            spectrogram_settings: crate::spectrogram_view::ViewSettings::default(),
            spectrogram_settings_open: false,
            macros: MacrosDialog::default(),
            spectral_selection: None,
            toolbars: toolbars::ToolbarLayout::load(),
            theme: crate::theme::load_saved(),
            custom_theme: None,
            theme_applied: false,
            focused_track: None,
            waveform_cache: waveform_cache::WaveformCache::new(),
            progress: progress::ProgressCenter::new(),
            test_jobs: Vec::new(),
            runtime_for_tests: au_runtime::Runtime::new(1, 4),
            io_jobs: Vec::new(),
            io_runtime: au_runtime::Runtime::new(2, 16),
            prefs: load_prefs(),
            prefs_dialog: crate::prefs_dialog::PreferencesDialog::new(),
            keymap: crate::shortcuts::KeyMap::standard(),
            keys_filter: String::new(),
            keys_xml_path: String::new(),
            prefs_applied: false,
            settings: crate::settings::AppSettings::from_prefs(&au_core::prefs::Prefs::new()),
            applied_theme_pref: None,
            dropout_warning: false,
            dropout_dont_show: false,
            zoom_toggle_second: false,
            menu_titles: Vec::new(),
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
            label_focus: None,
            new_label_dialog: None,
            speed_window: false,
            speed_percent: 100.0,
            loop_play: false,
            play_speed: 1.0,
            master_gain: 1.0,
            track_mix: Vec::new(),
            mixer_visible: false,
            export_options_window: false,
            export_audio_dialog: None,
            export_multi_dialog: None,
            metadata_editor: None,
            export_bits: 16,
            export_dither: Dither::Triangle,
            selection_start_text: "00:00:00.000".to_owned(),
            selection_end_text: "00:00:00.000".to_owned(),
            selection_length_text: "00:00:00.000".to_owned(),
            fx_dialog: None,
            plugin_manager: None,

script_pipe: None,
script_pipe_enabled: std::env::var("AU_SCRIPT_PIPE")
    .map(|value| value == "1")
    .unwrap_or(false),
script_pipe_failed: false,
ffmpeg_window: false,
ffmpeg_status: au_io::ffmpeg::status(),
            plugins: {
                let mut catalog = crate::plugin_bridge::PluginCatalog::new();
                catalog.start_scan();
                catalog
            },
            plugin_editor: None,
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
        // Extra > Selection > Snap-To Prior rounds grid snaps down.
        if let (true, au_core::timeline::SnapMode::Grid { secs }) = (self.snap_prior, &self.snapper.mode) {
            if *secs > 0.0 {
                return ((time.max(0.0) / secs).floor() * secs).max(0.0);
            }
        }
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
    if let Some(path) = self.file_dialog(crate::settings::DirOp::Import)
        .add_filter(
            "All supported",
            &[
                "wav", "flac", "mp3", "ogg", "m4a", "aac", "mp4", "mov", "mkv", "webm",
                "avi", "wmv", "wma", "ac3", "mid", "midi",
            ],
        )
        .add_filter(au_core::i18n::tr_ui("MIDI"), &["mid", "midi"])
        .add_filter(
            "Audio/video via FFmpeg",
            &[
                "m4a", "aac", "mp4", "mov", "mkv", "webm", "avi", "wmv", "wma", "ac3",
            ],
        )
        .pick_file().inspect(|p| self.remember_dir(crate::settings::DirOp::Import, p))
    {
        self.import_path_background(path);
    }
}

/// Imports an audio or MIDI file as new track(s) (File > Import and the Import2 pipe command).
fn import_path(&mut self, path: PathBuf) {
    {
        let is_midi = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| {
                extension.eq_ignore_ascii_case("mid")
                    || extension.eq_ignore_ascii_case("midi")
            })
            .unwrap_or(false);

        if is_midi {
            match au_io::midi::import_midi(&path) {
                Ok(track) => {
                    let note_count = track.notes.len();
                    self.editor.add_note_track(track);
                    self.ensure_track_mix();
                    self.status = au_core::i18n::tr_format("Imported MIDI: %d notes", &[&note_count]);
                }
                Err(error) => self.status = au_core::i18n::tr_format("Import failed: %s", &[&format!("{error:?}")]),
            }
        } else {
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
                Err(au_io::IoError::FfmpegMissing(_)) => {
                    self.status =
                        "FFmpeg is not installed — see File > FFmpeg status".to_owned();
                    self.ffmpeg_window = true;
                    self.ffmpeg_status = au_io::ffmpeg::status();
                }
                Err(error) => self.status = au_core::i18n::tr_format("Import failed: %s", &[&format!("{error:?}")]),
            }
        }
    }
}

/// File > Import on the I/O pool with a progress dialog (Cancel). MIDI and the
/// pipe's Import2 stay synchronous (scripts expect the track once the command returns).
fn import_path_background(&mut self, path: PathBuf) {
    if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("mid") || e.eq_ignore_ascii_case("midi")) {
        return self.import_path(path);
    }
    let p = path.clone();
    let handle = self.io_runtime.submit(0, move |ctx: &au_runtime::pool::JobCtx| {
        ctx.cancel.check()?;
        let mut last = 0;
        let mut report = io_progress(ctx, &mut last);
        match au_io::import_file_with_progress(&p, &mut report) {
            Err(au_io::IoError::Cancelled) => Err(au_runtime::JobError::Cancelled),
            r => Ok(r.map_err(|e| format!("{e:?}"))),
        }
    });
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_owned();
    let ticket = self.progress.register("Import", au_core::i18n::tr_format("Importing %s", &[&name]), &handle, false);
    self.io_jobs.push((ticket, IoJob::Import(path, handle)));
}

/// Applies finished import/export jobs (called every frame).
fn poll_io_jobs(&mut self) {
    let mut i = 0;
    while i < self.io_jobs.len() {
        let done = match &self.io_jobs[i].1 {
            IoJob::Import(_, h) => h.try_result().map(IoDone::Import),
            IoJob::Export(_, h) => h.try_result().map(IoDone::Export),
        };
        let Some(done) = done else { i += 1; continue };
        let (ticket, job) = self.io_jobs.remove(i);
        self.progress.finish(ticket);
        match (job, done) {
            (IoJob::Import(path, _), IoDone::Import(Ok(Ok(buffer)))) => {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("Imported audio").to_owned();
                self.editor.add_track(Track::new(name, buffer));
                self.selected_track = Some(self.editor.state().tracks.len() - 1);
                self.ensure_track_mix();
                self.status = "Imported audio".to_owned();
                self.beep_if_enabled();
            }
            (IoJob::Import(path, _), IoDone::Import(Ok(Err(e)))) => {
                if e.starts_with("FfmpegMissing") {
                    self.status = "FFmpeg is not installed — see File > FFmpeg status".to_owned();
                    self.ffmpeg_window = true;
                    self.ffmpeg_status = au_io::ffmpeg::status();
                } else {
                    self.status = au_core::i18n::tr_format("Import of %s failed: %s", &[&path.display().to_string(), &e]);
                }
            }
            (_, IoDone::Import(Err(au_runtime::JobError::Cancelled)) | IoDone::Export(Err(au_runtime::JobError::Cancelled))) => {
                self.status = "Cancelled".to_owned();
            }
            (_, IoDone::Import(Err(e))) => self.status = au_core::i18n::tr_format("Import failed: %s", &[&format!("{e:?}")]),
            (IoJob::Export(..), IoDone::Import(Ok(_))) => unreachable!("import result for an export job"),
            (_, IoDone::Export(Ok(msg))) => {
                self.status = msg;
                self.beep_if_enabled();
            }
            (IoJob::Export(what, _), IoDone::Export(Err(au_runtime::JobError::Failed(e)))) => self.status = au_core::i18n::tr_format("%s failed: %s", &[&what, &e]),
            (_, IoDone::Export(Err(e))) => self.status = au_core::i18n::tr_format("Export failed: %s", &[&format!("{e:?}")]),
        }
    }
}

fn export_ffmpeg(&mut self, format: au_io::ffmpeg::ExportFormat) {
    let ffmpeg = match au_io::ffmpeg::get() {
        Ok(ffmpeg) => ffmpeg,
        Err(_) => {
            self.status = "FFmpeg is not installed — see File > FFmpeg status".to_owned();
            self.ffmpeg_window = true;
            self.ffmpeg_status = au_io::ffmpeg::status();
            return;
        }
    };

    if format == au_io::ffmpeg::ExportFormat::Ac3
        && !matches!(self.project_rate(), 48_000 | 44_100 | 32_000)
    {
        self.status = "AC3 export requires a project rate of 48000, 44100, or 32000 Hz".to_owned();
        return;
    }

    let Some(path) = self.file_dialog(crate::settings::DirOp::Export)
        .add_filter(format.label(), &[format.extension()])
        .set_file_name(format!("export.{}", format.extension()))
        .save_file().inspect(|p| self.remember_dir(crate::settings::DirOp::Export, p))
    else {
        return;
    };

    let buffer = crate::fx_bridge::render_mix(
        self.editor.state(),
        &self.aup3_extras,
        self.master_gain,
        2,
    );

    match ffmpeg.export(&path, &buffer, format, format.default_bit_rate()) {
        Ok(()) => self.status = au_core::i18n::tr_format("Exported %s", &[&format.label()]),
        Err(error) => self.status = au_core::i18n::tr_format("Export failed: %s", &[&format!("{error:?}")]),
    }
}

fn export_midi(&mut self) {
    let note_tracks = &self.editor.state().note_tracks;
    if note_tracks.is_empty() {
        self.status = "No note track to export".to_owned();
        return;
    }

    let track_index = note_tracks
        .iter()
        .position(|track| !track.mute)
        .unwrap_or(0);
    let track = note_tracks[track_index].clone();

    let Some(path) = self.file_dialog(crate::settings::DirOp::Export)
        .add_filter(au_core::i18n::tr_ui("MIDI"), &["mid"])
        .set_file_name("export.mid")
        .save_file().inspect(|p| self.remember_dir(crate::settings::DirOp::Export, p))
    else {
        return;
    };

    match au_io::midi::export_midi(&path, &track) {
        Ok(()) => self.status = "Exported MIDI".to_owned(),
        Err(error) => self.status = au_core::i18n::tr_format("MIDI export failed: %s", &[&format!("{error:?}")]),
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
        extras.frequencies = self.spectral_selection.map_or((-1.0, -1.0), |sp| (sp.f0(), sp.f1()));
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
        if let Some(path) = self.file_dialog(crate::settings::DirOp::Save)
            .add_filter(au_core::i18n::tr_ui("Audacity project"), &["aup3"])
            .add_filter(au_core::i18n::tr_ui("Audacity-RS project (legacy)"), &["aurs"])
            .set_file_name("Untitled.aup3")
            .save_file().inspect(|p| self.remember_dir(crate::settings::DirOp::Save, p))
        {
            self.save_to(path);
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        self.wait_autosave();
        let result = if Self::is_aup3(&path) {
            au_project::aup3::save(&path, &self.current_aup3())
        } else {
            au_project::save_project(&path, self.editor.state())
        };
        match result {
            Ok(()) => {
                if let Err(e) = au_project::noise_profiles::save(&path, &self.room_noise.project_blobs()) {
                    self.status = au_core::i18n::tr_format("Saved, but noise profiles not stored: %s", &[&e]);
                }
                self.status = au_core::i18n::tr_format("Saved %s", &[&path.display().to_string()]);
                self.project_path = Some(path);
                self.autosave_sig = self.history_sig();
                self.discard_session_file();
            }
            Err(error) => self.status = au_core::i18n::tr_format("Save failed: %s", &[&error]),
        }
    }

    /// Journals unsaved edits into the `.aup3` autosave table (at most every
    /// 2 s), so a crash can be recovered on the next open.
    /// Waits for a background autosave before touching the project file.
    fn wait_autosave(&mut self) {
        if let Some(job) = self.autosave_job.take() {
            let _ = job.join();
        }
    }

    fn maybe_autosave(&mut self) {
        if let Some(job) = self.autosave_job.take() {
            if !job.is_finished() {
                self.autosave_job = Some(job);
                return;
            }
            if let Ok(Err(e)) = job.join() {
                self.status = au_core::i18n::tr_format("Autosave failed: %s", &[&e]);
            }
        }
        let path = match self.project_path.clone() {
            Some(p) if Self::is_aup3(&p) => p,
            Some(_) => return,
            // Untitled: journal into Directories > Temporary files directory.
            None if self.editor.history.can_undo() => match self.session_file.clone() {
                Some(p) => p,
                None => {
                    let dir = session_dir(&self.settings.temp_dir);
                    if std::fs::create_dir_all(&dir).is_err() {
                        return;
                    }
                    let p = dir.join(unsaved_project_name(local_now()));
                    self.session_file = Some(p.clone());
                    p
                }
            },
            None => return,
        };
        let sig = self.history_sig();
        if sig == self.autosave_sig || self.autosave_at.elapsed().as_secs_f64() < 2.0 {
            return;
        }
        self.autosave_at = std::time::Instant::now();
        if let Some(d) = &sig.1 { au_runtime::crash::log_action(format!("edit: {d}")); }
        self.autosave_sig = sig;
        let project = self.current_aup3();
        self.autosave_job = Some(std::thread::spawn(move || {
            au_project::aup3::autosave(&path, &project).map_err(|e| e.to_string())
        }));
    }

    fn open_project(&mut self) {
        if let Some(path) = self.file_dialog(crate::settings::DirOp::Open)
            .add_filter(au_core::i18n::tr_ui("All projects"), &["aup3", "aurs"])
            .add_filter(au_core::i18n::tr_ui("Audacity project"), &["aup3"])
            .add_filter(au_core::i18n::tr_ui("Audacity-RS project (legacy)"), &["aurs"])
            .pick_file().inspect(|p| self.remember_dir(crate::settings::DirOp::Open, p))
        {
            self.open_path(path);
        }
    }

/// Handles pipe commands that change application state. Returns `None` when the command is not app-level (the caller then uses the read-only interpreter), otherwise `Some((message, ok))`.
fn execute_app_command(&mut self, line: &str) -> Option<(String, bool)> {
    let (command, params) = crate::script_bridge::parse_command(line);

    match command.as_str() {
        // Audacity's GetPreference / SetPreference (PreferenceCommands.cpp).
        "GetPreference" => {
            let name = params.iter().find(|(k, _)| k == "Name").map(|(_, v)| v.clone()).unwrap_or_default();
            let value = self.prefs.read_string_or(&name, "");
            return Some((value, true));
        }
        "SetPreference" => {
            let name = params.iter().find(|(k, _)| k == "Name").map(|(_, v)| v.clone()).unwrap_or_default();
            let value = params.iter().find(|(k, _)| k == "Value").map(|(_, v)| v.clone()).unwrap_or_default();
            if name.is_empty() {
                return Some(("SetPreference requires Name".to_string(), false));
            }
            self.prefs.set(&name, au_core::prefs::PrefValue::Str(value));
            if let Some(dir) = prefs_path().parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = self.prefs.save(prefs_path());
            self.apply_prefs();
            return Some((String::new(), true));
        }
        "NewMonoTrack" | "NewStereoTrack" => {
            // Tracks > Default audio track name + TrackList::MakeUniqueTrackName.
            let existing: Vec<String> = self.editor.state().tracks.iter().map(|t| t.name.clone()).collect();
            let name = crate::settings::unique_track_name(&self.settings.default_audio_track_name, &existing);
            let channels = if command == "NewStereoTrack" { 2 } else { 1 };
            let rate = self.project_rate();
            let mut track = Track::new(name.clone(), au_core::AudioBuffer::from_channels(rate, vec![Vec::new(); channels]));
            // Quality > Default Sample Format: new tracks use it.
            track.int_bits = crate::settings::quantize_needed(self.settings.default_format);
            self.editor.add_track(track);
            self.ensure_track_mix();
            self.apply_default_view(self.editor.state().tracks.len() - 1);
            return Some((String::new(), true));
        }
        "SeekLeftShort" | "SeekRightShort" | "SeekLeftLong" | "SeekRightLong" => {
            // Playback > Seek Time when playing (short/long period).
            let long = command.ends_with("Long");
            let secs = crate::settings::seek_seconds(&self.settings, long) * if command.starts_with("SeekLeft") { -1.0 } else { 1.0 };
            let rate = self.project_rate() as f64;
            let pos = self.player.as_ref().map(|p| p.position()).unwrap_or(0) as f64;
            let target = (pos + secs * rate).max(0.0) as usize;
            if self.player.is_some() {
                self.seek_transport(target);
            }
            return Some((String::new(), true));
        }
        "ZoomToggle" => {
            // View > Zoom > Zoom Toggle between Tracks preferences Preset 1 and Preset 2.
            let preset = if self.zoom_toggle_second { self.settings.zoom_preset1 } else { self.settings.zoom_preset2 };
            self.zoom_toggle_second = !self.zoom_toggle_second;
            let width = 1000.0;
            self.zoom.pixels_per_sec = crate::settings::zoom_of_preset(preset, self.project_duration(), width, self.zoom.pixels_per_sec);
            return Some((String::new(), true));
        }
        "Import2" => {
            let filename = match params
                .iter()
                .find(|(key, _)| key == "Filename")
                .map(|(_, value)| value.as_str())
            {
                Some(filename) if !filename.is_empty() => filename,
                _ => return Some(("Import2 requires Filename".to_string(), false)),
            };

            let path = std::path::PathBuf::from(filename);
            if !path.exists() {
                return Some((format!("File not found: {filename}"), false));
            }

            let tracks_before = self.editor.state().tracks.len();
            let _status_before = self.status.clone();
            if Self::is_aup3(&path) || path.extension().is_some_and(|e| e.eq_ignore_ascii_case("aurs")) { self.open_path(path); } else { self.import_path(path); }

            let tracks_increased = self.editor.state().tracks.len() > tracks_before;
            let status_lower = self.status.to_ascii_lowercase();
            let ok = tracks_increased
                || (!status_lower.contains("fail") && !status_lower.contains("error"));

            Some((format!("Imported {filename}"), ok))
        }
        "SetTrackVisuals" | "SetTrack" => {
            let track_text = match params
                .iter()
                .find(|(key, _)| key == "Track")
                .map(|(_, value)| value.as_str())
            {
                Some(value) => value,
                None => return Some(("Track is required".to_string(), false)),
            };
            let track_index = match track_text.parse::<usize>() {
                Ok(index) => index,
                Err(_) => return Some((format!("Invalid track index: {track_text}"), false)),
            };

            let display = match params
                .iter()
                .find(|(key, _)| key == "Display")
                .map(|(_, value)| value.as_str())
            {
                Some(value) => value,
                None => return Some(("Display is required".to_string(), false)),
            };
            let track_view = match display {
                "Waveform" => crate::spectrogram_view::TrackView::Waveform,
                "Spectrogram" => crate::spectrogram_view::TrackView::Spectrogram,
                "Multiview" => crate::spectrogram_view::TrackView::Multiview,
                _ => return Some((format!("Unknown Display: {display}"), false)),
            };

            let scale = match params
                .iter()
                .find(|(key, _)| key == "SpecScale")
                .map(|(_, value)| value.as_str())
            {
                None => None,
                Some("Linear") => {
                    Some(crate::spectrogram_view::FreqScale::Linear)
                }
                Some("Logarithmic") | Some("Log") => {
                    Some(crate::spectrogram_view::FreqScale::Logarithmic)
                }
                Some("Mel") => Some(crate::spectrogram_view::FreqScale::Mel),
                Some(value) => {
                    return Some((format!("Unknown SpecScale: {value}"), false));
                }
            };

            let track_count = self.editor.state().tracks.len();
            if track_index >= track_count {
                return Some((format!("Track index {track_index} out of range"), false));
            }

            self.track_views
                .resize_with(track_count, crate::spectrogram_view::TrackView::default);

            if let Some(scale) = scale {
                self.spectrogram_settings.scale = scale;
                self.spectrogram_cache.invalidate_all();
            }

            self.track_views[track_index] = track_view;
            Some((format!("Track {track_index} display: {display}"), true))
        }
        _ => None,
    }
}

    /// Editing, file and transport commands for scripting. Returns None when the command is not handled here.
    fn execute_edit_command(&mut self, line: &str) -> Option<(String, bool)> {
        let (name, args) = crate::script_bridge::parse_command(line);
        let value = |key: &str| {
            args.iter()
                .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
                .map(|(_, value)| value.as_str())
        };
        let status_failed = |status: &str| {
            let status = status.to_ascii_lowercase();
            status.contains("fail") || status.contains("error")
        };

        if let Some(result) = self.extra_command(&name) {
            return Some(result);
        }
        match name.as_str() {
            "Select" | "SelectTime" => {
                let track = if name == "SelectTime" {
                    self.selected_track.unwrap_or(0)
                } else {
                    match value("Track").unwrap_or("0").parse::<usize>() {
                        Ok(track) => track,
                        Err(_) => return Some(("Invalid track".to_string(), false)),
                    }
                };
                let (rate, len) = match self.editor.state().tracks.get(track) {
                    Some(track) => (track.sample_rate(), track.len()),
                    None => return Some((format!("Track {} does not exist", track), false)),
                };
                let start_secs = match value("Start").and_then(|v| v.parse::<f64>().ok()) {
                    Some(value) if value.is_finite() => value.max(0.0),
                    _ => return Some(("Invalid or missing Start".to_string(), false)),
                };
                let end_secs = match value("End").and_then(|v| v.parse::<f64>().ok()) {
                    Some(value) if value.is_finite() => value.max(0.0),
                    _ => return Some(("Invalid or missing End".to_string(), false)),
                };
                // Like Audacity, a time selection may extend past the end of the audio.
                let _ = len;
                let start = au_core::secs_to_frames(start_secs, rate);
                let end = au_core::secs_to_frames(end_secs, rate);
                let selection = Selection {
                    track,
                    start: start.min(end),
                    end: start.max(end),
                };
                self.selected_track = Some(track);
                self.selection = Some(selection);
                let divisor = f64::from(rate.max(1));
                Some((
                    format!(
                        "Selected {:.6}-{:.6} s on track {}",
                        selection.start as f64 / divisor,
                        selection.end as f64 / divisor,
                        track
                    ),
                    true,
                ))
            }
            "SelectAll" => {
                let (len, exists) = match self.editor.state().tracks.first() {
                    Some(track) => (track.len(), true),
                    None => (0, false),
                };
                if !exists {
                    return Some(("No tracks available".to_string(), false));
                }
                self.selected_track = Some(0);
                self.selection = Some(Selection {
                    track: 0,
                    start: 0,
                    end: len,
                });
                Some(("Selected all of track 0".to_string(), true))
            }
            "SplitDelete" => {
                // Edit > Remove Special > Split Delete: remove the audio, leave a gap.
                let Some(track) = self.selected_track else {
                    return Some(("No track selected".to_string(), false));
                };
                let sel = self.selection_for_track(track).normalized();
                let mut state = self.editor.state().clone();
                let Some(t) = state.tracks.get_mut(track) else {
                    return Some(("No track selected".to_string(), false));
                };
                let mut clips = Vec::new();
                for clip in t.clips.clips.drain(..) {
                    let (c0, c1) = (clip.start, clip.end());
                    if c1 <= sel.start || c0 >= sel.end {
                        clips.push(clip);
                        continue;
                    }
                    let audio = clip.rendered();
                    if c0 < sel.start {
                        if let Ok(left) = audio.slice(0, sel.start - c0) {
                            clips.push(au_core::timeline::Clip::new(clip.name.clone(), c0, left));
                        }
                    }
                    if c1 > sel.end {
                        if let Ok(right) = audio.slice(sel.end - c0, audio.len()) {
                            clips.push(au_core::timeline::Clip::new(clip.name.clone(), sel.end, right));
                        }
                    }
                }
                clips.sort_by_key(|c| c.start);
                t.clips.clips = clips;
                self.editor.replace_state("Split Delete", state);
                Some(("SplitDelete completed".to_string(), true))
            }
            "Cut" | "Copy" | "Delete" | "Paste" => {
                let track = match self.selected_track {
                    Some(track) => track,
                    None => return Some(("No track selected".to_string(), false)),
                };
                let selection = self.selection_for_track(track);
                let before = self.status.clone();
                match name.as_str() {
                    "Cut" => { let behavior = self.edit_behavior(); self.edit_selection(|editor| editor.cut_with(selection, &behavior)) },
                    "Copy" => self.edit_selection(|editor| editor.copy(selection)),
                    "Delete" => {
                        let behavior = self.edit_behavior();
                        self.edit_selection(|editor| editor.delete_with(selection, &behavior));
                        // EditMenus OnDelete: selectedRegion.collapseToT0().
                        if !status_failed(&self.status) {
                            if let Some(sel) = self.selection.as_mut() { sel.end = sel.start; }
                        }
                    }
                    "Paste" => {
                        { let behavior = self.edit_behavior(); self.edit_selection(|editor| editor.paste_with(track, selection.start, &behavior)) }
                    }
                    _ => unreachable!(),
                }
                if self.status != before && status_failed(&self.status) {
                    Some((self.status.clone(), false))
                } else {
                    Some((format!("{} completed", name), true))
                }
            }
            "Undo" | "Redo" => {
                if self.selected_track.is_none() {
                    return Some(("No track selected".to_string(), false));
                }
                let result = if name == "Undo" {
                    self.editor.undo()
                } else {
                    self.editor.redo()
                };
                match result {
                    Ok(()) => Some((format!("{} completed", name), true)),
                    Err(error) => Some((error.to_string(), false)),
                }
            }
            "Export2" => {
                let filename = match value("Filename") {
                    Some(filename) if !filename.is_empty() => filename,
                    _ => return Some(("Missing Filename".to_string(), false)),
                };
                let path = std::path::PathBuf::from(filename);
                let track = self.selected_track.unwrap_or(0);
                let buffer = match crate::fx_bridge::render_track(
                    self.editor.state(),
                    &self.aup3_extras,
                    track,
                ) {
                    Some(buffer) => buffer,
                    None => return Some((format!("Unable to render track {}", track), false)),
                };
                let is_flac = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(|extension| extension.eq_ignore_ascii_case("flac"))
                    .unwrap_or(false);
                let result = if is_flac {
                    let bits = if self.export_bits == 24 {
                        usize::from(self.export_bits.min(24))
                    } else {
                        16
                    };
                    au_io::export_flac(&path, &buffer, bits)
                } else {
                    au_io::export_wav_dithered(
                        &path,
                        &buffer,
                        self.export_bits,
                        self.export_dither,
                    )
                };
                match result {
                    Ok(()) => Some((format!("Exported {}", path.display()), true)),
                    Err(error) => Some((format!("Export failed: {:?}", error), false)),
                }
            }
            "SaveProject2" => {
                let filename = match value("Filename") {
                    Some(filename) if !filename.is_empty() => filename,
                    _ => return Some(("Missing Filename".to_string(), false)),
                };
                let path = std::path::PathBuf::from(filename);
                self.save_to(path);
                if self.status.starts_with("Saved") {
                    Some((self.status.clone(), true))
                } else {
                    Some((self.status.clone(), false))
                }
            }
            "OpenProject2" => {
                let filename = match value("Filename") {
                    Some(filename) if !filename.is_empty() => filename,
                    _ => return Some(("Missing Filename".to_string(), false)),
                };
                let path = std::path::PathBuf::from(filename);
                if !path.is_file() {
                    return Some((format!("File not found: {}", path.display()), false));
                }
                self.open_path(path);
                if let Some(recovery_path) = &self.recovery_prompt {
                    Some((
                        format!("Recovery available for {}", recovery_path.display()),
                        true,
                    ))
                } else if status_failed(&self.status) {
                    Some((self.status.clone(), false))
                } else {
                    Some((self.status.clone(), true))
                }
            }
            "Recover" => {
                let action = match value("Action") {
                    Some(action) if action.eq_ignore_ascii_case("Recover") => true,
                    Some(action) if action.eq_ignore_ascii_case("Discard") => false,
                    _ => return Some(("Action must be Recover or Discard".to_string(), false)),
                };
                let path = match self.recovery_prompt.clone() {
                    Some(path) => path,
                    None => return Some(("No recovery available".to_string(), false)),
                };
                if action {
                    self.recovery_prompt.take();
                    self.open_aup3(path.clone(), au_project::aup3::DocSource::Autosave);
                    if status_failed(&self.status) {
                        Some((self.status.clone(), false))
                    } else {
                        Some((format!("Recovered {}", path.display()), true))
                    }
                } else {
                    if let Err(error) = au_project::aup3::discard_autosave(&path) {
                        return Some((format!("Unable to discard autosave: {}", error), false));
                    }
                    self.recovery_prompt.take();
                    if is_unsaved_session(&path) {
                        self.session_file = Some(path.clone());
                        self.discard_session_file();
                        return Some((format!("Discarded recovery for {}", path.display()), true));
                    }
                    self.open_aup3(path.clone(), au_project::aup3::DocSource::Saved);
                    if status_failed(&self.status) {
                        Some((self.status.clone(), false))
                    } else {
                        Some((format!("Discarded recovery for {}", path.display()), true))
                    }
                }
            }
            "Play" => {
                self.play();
                Some(("Playing".to_string(), true))
            }
            "Stop" => {
                if self.recorder.is_some() {
                    self.stop_recording();
                } else {
                    self.stop_playback();
                }
                Some(("Stopped".to_string(), true))
            }
            "Record1stChoice" => {
                self.begin_recording();
                if self.recorder.is_some() {
                    Some(("Recording".to_string(), true))
                } else {
                    let message = if self.status.is_empty() {
                        "Unable to start recording".to_string()
                    } else {
                        self.status.clone()
                    };
                    Some((message, false))
                }
            }
            _ => None,
        }
    }

    /// Tracks Behaviors / Interface editing preferences for Cut, Delete and Paste.
    fn edit_behavior(&self) -> au_core::edit_behaviors::EditBehavior {
        au_core::edit_behaviors::EditBehavior {
            enable_cut_lines: self.settings.enable_cut_lines,
            edit_clips_can_move: self.settings.edit_clips_can_move,
            paste_as_new_clips: self.settings.paste_as_new_clips,
            retain_labels: self.settings.retain_labels,
        }
    }

    /// Starts an Export Audio job on the I/O pool; the progress dialog
    /// follows it (File > Export Audio and the ExportAudioTest hook).
    fn start_export_audio(
        &mut self,
        path: std::path::PathBuf,
        fmt: au_io::export::ExportFormat,
        tags: au_core::tags::Tags,
        selection_only: bool,
    ) {
        let rate = self.project_rate().max(1) as f64;
        let (t0, t1) = match self.selection_project_frames().filter(|_| selection_only) {
            Some((a, b)) if b > a => (a as f64 / rate, b as f64 / rate),
            _ => (0.0, self.editor.state().tracks.iter()
                .map(|t| t.len() as f64 / t.sample_rate().max(1) as f64)
                .fold(0.0, f64::max)),
        };
        self.aup3_extras.tags = tags.clone();
        let (state, extras) = (self.editor.state().clone(), self.aup3_extras.clone());
        let shown = path.display().to_string();
        let handle = self.io_runtime.submit(0, move |ctx: &au_runtime::pool::JobCtx| {
            ctx.cancel.check()?;
            let mut last = 0;
            let mut report = io_progress(ctx, &mut last);
            crate::export_glue::export_range_with_progress(&path, &state, &extras, t0, t1, None, &fmt, &tags, &mut report)
                .map_err(io_job_error)?;
            Ok(au_core::i18n::tr_format("Exported %s", &[&path.display().to_string()]))
        });
        let ticket = self.progress.register("Export Audio", au_core::i18n::tr_format("Exporting %s", &[&shown]), &handle, false);
        self.io_jobs.push((ticket, IoJob::Export("Export".into(), handle)));
    }

    /// Executes test and diagnostic commands exposed by the mod-script pipe.
    ///
    /// `ProgressTest` and `CrashTest` are available only when
    /// `AU_ALLOW_TEST_HOOKS=1`.
    fn execute_diagnostic_command(&mut self, line: &str) -> Option<(String, bool)> {
        let (command, args) = crate::script_bridge::parse_command(line);
        let value = |key: &str| {
            args.iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
        };

        match command.as_str() {
            "ProgressTest" => {
                if !std::env::var("AU_ALLOW_TEST_HOOKS")
                    .map(|value| value == "1")
                    .unwrap_or(false)
                {
                    return Some(("Test hooks are disabled".to_string(), false));
                }

                let seconds = match value("Seconds").and_then(|value| value.parse::<f64>().ok()) {
                    Some(seconds) if seconds.is_finite() && seconds >= 0.0 => seconds,
                    _ => {
                        return Some((
                            "ProgressTest requires a non-negative Seconds value".to_string(),
                            false,
                        ));
                    }
                };
                let title = match value("Title") {
                    Some(title) if !title.is_empty() => title.to_string(),
                    _ => {
                        return Some(("ProgressTest requires a Title value".to_string(), false));
                    }
                };

                let delay = std::time::Duration::from_secs_f64(seconds / 100.0);
                let handle = self.runtime_for_tests.submit(
                    100,
                    move |ctx: &au_runtime::pool::JobCtx| -> Result<(), au_runtime::JobError> {
                        for _ in 0..100 {
                            ctx.cancel.check()?;
                            std::thread::sleep(delay);
                            ctx.progress.add(1);
                        }
                        Ok(())
                    },
                );
                let ticket = self.progress.register(&title, "Running test job", &handle, false);
                self.test_jobs.push((ticket, handle));
                Some((format!("Started {title}"), true))
            }
            "ExportAudioTest" => {
                if !std::env::var("AU_ALLOW_TEST_HOOKS").map(|v| v == "1").unwrap_or(false) {
                    return Some(("Test hooks are disabled".to_string(), false));
                }
                let Some(file) = value("Filename").filter(|f| !f.is_empty()) else {
                    return Some(("ExportAudioTest requires a Filename value".to_string(), false));
                };
                let fmt = crate::export_glue::FormatChooser::from_name(value("Format").unwrap_or("WAV")).format;
                let tags = self.aup3_extras.tags.clone();
                let file = file.to_string();
                self.start_export_audio(std::path::PathBuf::from(&file), fmt, tags, false);
                Some((format!("Started export to {file}"), true))
            }
            // Test hook: the current selection in seconds (Audacity reads it back via AddLabel).
            // View > Zoom > Zoom Normal (ViewMenus: ZoomInfo::GetDefaultZoom = 44100 / 512).
            "ZoomNormal" => {
                self.zoom.pixels_per_sec = 44100.0 / 512.0;
                Some((String::new(), true))
            }
            "GetSelection" => {
                let (t0, t1) = self.sel_secs();
                Some((format!("{t0:.6} {t1:.6}"), true))
            }
            "GetProgress" => {
                let snapshot = self.progress.snapshot();
                if snapshot.is_empty() {
                    return Some(("none".to_string(), true));
                }

                let lines = snapshot
                    .into_iter()
                    .map(|(title, fraction, outcome)| {
                        let status = match outcome {
                            progress::Outcome::Running => "Running",
                            progress::Outcome::Cancelled => "Cancelled",
                            progress::Outcome::Stopped => "Stopped",
                        };
                        format!("{title}\t{:.1}\t{status}", fraction * 100.0)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Some((lines, true))
            }
            "CancelProgress" => {
                let count = self.progress.len();
                self.progress.cancel_all();
                Some((format!("Cancelled {count} job(s)"), true))
            }
            "CrashTest" => {
                if !std::env::var("AU_ALLOW_TEST_HOOKS")
                    .map(|value| value == "1")
                    .unwrap_or(false)
                {
                    return Some(("Test hooks are disabled".to_string(), false));
                }
                panic!("CrashTest requested over mod-script-pipe");
            }
            "GetEffectMenu" => {
                // Test hook: the effects menu layout as "depth<TAB>label" lines (Kind=Generate|Effect|Analyze).
                let (_, params) = crate::script_bridge::parse_command(line);
                let kind = match params.iter().find(|(k, _)| k == "Kind").map(|(_, v)| v.as_str()) {
                    Some("Generate") => EffectKind::Generate,
                    Some("Analyze") => EffectKind::Analyze,
                    _ => EffectKind::Process,
                };
                fn flat(nodes: &[crate::settings::MenuNode], depth: usize, out: &mut Vec<String>) {
                    for n in nodes {
                        match n {
                            crate::settings::MenuNode::Item { label, .. } => out.push(format!("{depth}\t{label}")),
                            crate::settings::MenuNode::Submenu { label, items } => { out.push(format!("{depth}\t{label}")); flat(items, depth + 1, out); }
                            crate::settings::MenuNode::Separator => out.push(format!("{depth}\t----")),
                        }
                    }
                }
                let mut out = Vec::new();
                flat(&effects_menu_nodes(kind, &self.settings.group_by), 0, &mut out);
                return Some((out.join("\n"), true));
            }
            "GetMenuTitles" => {
                return Some((self.menu_titles.join("\t"), true));
            }
            "GetDialogs" => {
                let mut lines = Vec::new();
                lines.extend(
                    self.crash_reports
                        .iter()
                        .map(|report| format!("crash_report\t{}", report.path.display())),
                );
                if let Some(path) = &self.recovery_prompt {
                    lines.push(format!("recovery\t{}", path.display()));
                }
                lines.extend(
                    self.progress
                        .snapshot()
                        .into_iter()
                        .map(|(title, _, _)| format!("progress\t{title}")),
                );

                if lines.is_empty() {
                    Some(("none".to_string(), true))
                } else {
                    Some((lines.join("\n"), true))
                }
            }
            "DismissCrashReport" => {
                if self.crash_reports.is_empty() {
                    return Some(("No crash report to dismiss".to_string(), false));
                }

                match au_runtime::crash::dismiss(&self.crash_reports[0]) {
                    Ok(()) => {
                        self.crash_reports.remove(0);
                        Some(("Crash report dismissed".to_string(), true))
                    }
                    Err(error) => Some((au_core::i18n::tr_format("Failed to dismiss crash report: %s", &[&error]), false)),
                }
            }
            _ => None,
        }
    }

    fn open_path(&mut self, path: PathBuf) {
        self.wait_autosave();
        if Self::is_aup3(&path) || au_project::is_aup3(&path) {
            match au_project::aup3::inspect(&path) {
                Ok(info) if info.needs_recovery => {
                    self.recovery_prompt = Some(path);
                    return;
                }
                Ok(_) => {}
                Err(e) => {
                    self.status = au_core::i18n::tr_format("Open failed: %s", &[&e]);
                    return;
                }
            }
            self.open_aup3(path, au_project::aup3::DocSource::Saved);
            return;
        }
        // Like Audacity, anything that is not a project file is imported as audio.
        let is_project = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("aurs") || e.eq_ignore_ascii_case("aup3"));
        if !is_project {
            self.import_path(path);
            return;
        }
        match au_project::load_project(&path) {
            Ok(state) => {
                if let Ok(b) = au_project::noise_profiles::load(&path) { self.room_noise.load_project_blobs(&b); }
                self.aup3_extras = Default::default();
                self.install_state(state, path);
            }
            Err(error) => self.status = au_core::i18n::tr_format("Open failed: %s", &[&error]),
        }
    }

    fn open_aup3(&mut self, path: PathBuf, source: au_project::aup3::DocSource) {
        let path_for_session = path.clone();
        match au_project::aup3::load_from(&path, source) {
            Ok(p) => {
                if let Ok(b) = au_project::noise_profiles::load(&path) { self.room_noise.load_project_blobs(&b); }
                let (s0, s1) = p.extras.selection;
                self.aup3_extras = p.extras;
                self.install_state(p.state, path);
                if s1 > s0 && !self.editor.state().tracks.is_empty() {
                    let rate = self.editor.state().tracks[0].sample_rate() as f64;
                    self.selection = Some(Selection { track: 0, start: (s0 * rate) as usize, end: (s1 * rate) as usize });
                }
                let (f0, f1) = self.aup3_extras.frequencies;
                self.spectral_selection = (f0 >= 0.0 || f1 >= 0.0).then_some({ let mut r = au_core::selected_region::SelectedRegion::new(s0, s1); r.set_frequencies(f0, f1); crate::spectrogram_view::SpectralSelection::new(0, r) });
                if is_unsaved_session(&path_for_session) {
                    // A recovered untitled project stays untitled; it keeps journaling there.
                    self.project_path = None;
                    self.session_file = Some(path_for_session.clone());
                }
                if source == au_project::aup3::DocSource::Autosave {
                    // Unsaved until the user saves, like Audacity after recovery.
                    self.autosave_sig = (0, None);
                    self.status = "Recovered unsaved changes from autosave — save to keep them".to_owned();
                }
            }
            Err(e) => self.status = au_core::i18n::tr_format("Open failed: %s", &[&e]),
        }
    }

    /// Deletes the untitled-project journal (after a save, or on a clean exit).
    fn discard_session_file(&mut self) {
        self.wait_autosave();
        if let Some(p) = self.session_file.take() {
            for suffix in ["", "-wal", "-shm"] {
                let mut f = p.clone().into_os_string();
                f.push(suffix);
                let _ = std::fs::remove_file(f);
            }
        }
    }

    fn install_state(&mut self, state: au_core::ProjectState, path: PathBuf) {
        self.wait_autosave();
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
        self.wait_autosave();
        let path = match &self.project_path {
            Some(p) if Self::is_aup3(p) => p.clone(),
            Some(_) => return,
            None => match &self.session_file {
                Some(p) => p.clone(),
                None => return,
            },
        };
        if self.history_sig() != self.autosave_sig {
            let _ = au_project::aup3::autosave(&path, &self.current_aup3());
        }
    }

    /// Shows crash reports left by earlier sessions: view, copy, dismiss.
    fn crash_report_dialog(&mut self, ctx: &egui::Context) {
        let Some(report) = self.crash_reports.first().cloned() else { return };
        let mut action = None;
        egui::Window::new(au_core::i18n::tr_ui("Audacity-RS crashed last time"))
            .collapsible(false)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(au_core::i18n::tr_format("A crash report was saved locally (nothing was uploaded):\n%s", &[&report.path.display().to_string()]));
                if let Some(m) = report.message() { ui.label(au_core::i18n::tr_format("Reason: %s", &[&m])); }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    let mut text = report.text.as_str();
                    ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(f32::INFINITY));
                });
                ui.horizontal(|ui| {
                    if ui.button(au_core::i18n::tr_ui("Copy report")).clicked() { action = Some(0); }
                    if ui.button(au_core::i18n::tr_ui("Dismiss")).clicked() { action = Some(1); }
                    if self.crash_reports.len() > 1 { ui.label(au_core::i18n::tr_format("%d more", &[&(self.crash_reports.len() - 1)])); }
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
        egui::Window::new(au_core::i18n::tr_ui("Automatic Crash Recovery"))
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
                    if ui.button(au_core::i18n::tr_ui("Recover")).clicked() { choice = Some(0); }
                    if ui.button(au_core::i18n::tr_ui("Discard changes")).clicked() { choice = Some(1); }
                    if ui.button(au_core::i18n::tr_ui("Cancel")).clicked() { choice = Some(2); }
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
                    // Discarding an untitled project's recovery deletes its temp file.
                    Ok(()) if is_unsaved_session(&path) => { self.session_file = Some(path); self.discard_session_file(); }
                    Ok(()) => self.open_aup3(path, au_project::aup3::DocSource::Saved),
                    Err(e) => self.status = au_core::i18n::tr_format("Discard failed: %s", &[&e]),
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
        if let Some(path) = self.file_dialog(crate::settings::DirOp::Export)
            .add_filter(filter, &[extension])
            .save_file().inspect(|p| self.remember_dir(crate::settings::DirOp::Export, p))
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
                Err(error) => au_core::i18n::tr_format("Export failed: %s", &[&format!("{error:?}")]),
            };
        }
    }

    /// Export Audio / Export Multiple / Metadata Editor windows (Stage 8).
    /// Audacity's Spectrogram Settings (Spectrograms preferences page, algorithm part).
    fn show_spectrogram_settings(&mut self, ctx: &egui::Context) {
        if !self.spectrogram_settings_open {
            return;
        }
        let mut open = true;
        let st = &mut self.spectrogram_settings;
        egui::Window::new(au_core::i18n::tr_ui("Spectrogram Settings")).open(&mut open).show(ctx, |ui| {
            const ALGS: [&str; 3] = ["Frequencies", "Reassignment", "Pitch (EAC)"];
            egui::ComboBox::from_label(au_core::i18n::tr_ui("Algorithm")).selected_text(ALGS[usize::from(st.algorithm.min(2))]).show_ui(ui, |ui| {
                for (i, a) in ALGS.iter().enumerate() {
                    ui.selectable_value(&mut st.algorithm, i as u8, *a);
                }
            });
            const WINS: [&str; 10] = ["Rectangular", "Bartlett", "Hamming", "Hann", "Blackman", "Blackman-Harris", "Welch", "Gaussian(a=2.5)", "Gaussian(a=3.5)", "Gaussian(a=4.5)"];
            egui::ComboBox::from_label(au_core::i18n::tr_ui("Window type")).selected_text(WINS[usize::from(st.window_type.min(9))]).show_ui(ui, |ui| {
                for (i, w) in WINS.iter().enumerate() {
                    ui.selectable_value(&mut st.window_type, i as u8, *w);
                }
            });
            egui::ComboBox::from_label(au_core::i18n::tr_ui("Window size")).selected_text(st.window_size.to_string()).show_ui(ui, |ui| {
                for k in 3..=15 {
                    ui.selectable_value(&mut st.window_size, 1usize << k, (1usize << k).to_string());
                }
            });
            if st.algorithm != 2 {
                egui::ComboBox::from_label(au_core::i18n::tr_ui("Zero padding factor")).selected_text(st.zero_padding.to_string()).show_ui(ui, |ui| {
                    for z in [1u8, 2, 4, 8, 16] {
                        ui.selectable_value(&mut st.zero_padding, z, z.to_string());
                    }
                });
            }
            ui.add(egui::Slider::new(&mut st.gain_db, 0.0..=100.0).text("Gain (dB)"));
            ui.add(egui::Slider::new(&mut st.range_db, 1.0..=300.0).text("Range (dB)"));
            ui.add(egui::Slider::new(&mut st.frequency_gain, 0..=60).text("Frequency gain (dB/dec)"));
        });
        self.spectrogram_settings_open = open;
    }

    fn show_export_dialogs(&mut self, ctx: &egui::Context) {
        if let Some(ed) = self.metadata_editor.as_mut() {
            if let Some(result) = ed.show(ctx) {
                if let Some(tags) = result {
                    self.aup3_extras.tags = tags;
                    self.status = "Metadata updated".to_owned();
                }
                self.metadata_editor = None;
            }
        }
        if let Some(dlg) = self.export_audio_dialog.as_mut() {
            if let Some((path, fmt, tags, selection_only)) = dlg.show(ctx) {
                self.export_audio_dialog = None;
                self.start_export_audio(path, fmt, tags, selection_only);
            }
        }
        if let Some(dlg) = self.export_multi_dialog.as_mut() {
            if let Some(req) = dlg.show(ctx) {
                self.export_multi_dialog = None;
                let fmt = crate::export_glue::FormatChooser::from_name(&req.format).format;
                let (state, extras) = (self.editor.state().clone(), self.aup3_extras.clone());
                let handle = self.io_runtime.submit(0, move |ctx: &au_runtime::pool::JobCtx| {
                    ctx.cancel.check()?;
                    let files = crate::export_glue::export_multiple_with(&state, &extras, &req, &fmt, &ctx.cancel, &ctx.progress)
                        .map_err(|e| if ctx.cancel.is_cancelled() { au_runtime::JobError::Cancelled } else { au_runtime::JobError::Failed(e) })?;
                    Ok(au_core::i18n::tr_format("Exported %d files", &[&files.len()]))
                });
                let ticket = self.progress.register("Export Multiple", "Exporting files", &handle, false);
                self.io_jobs.push((ticket, IoJob::Export("Export Multiple".into(), handle)));
            }
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
        match au_io::device::Player::start_with_effects_on(
            &self.device_choice(false),
            tracks,
            track_fx,
            master_fx,
            rate,
            self.master_gain,
            start,
            self.play_speed,
            loop_range,
        ) {
            Ok(mut player) => {
                // Playback > Micro-fades.
                player.handle.send(EngineCommand::SetMicrofades(self.settings.micro_fades));
                self.player = Some(player);
                self.status = "Playing".to_owned();
            }
            Err(error) => self.status = au_core::i18n::tr_format("Playback failed: %s", &[&format!("{error:?}")]),
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
            // AudioIO::StopStream: with micro-fades and latency < 150 ms, wait latency + 50 ms
            // so the faded-out buffer reaches the sound card.
            let latency = self.settings.latency_duration_ms;
            if self.settings.micro_fades && latency < 150.0 && player.handle.is_playing() {
                std::thread::sleep(std::time::Duration::from_millis((latency.max(0.0) + 50.0) as u64));
            }
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

    fn input_device_key(&self) -> String {
        match self.input_kind {
            InputKind::Loopback => "loopback".to_owned(),
            _ => au_io::device::input_devices().into_iter().next().map_or_else(|| "default".to_owned(), |n| crate::room_noise::RoomNoise::device_key(&n)),
        }
    }

    fn start_noise_capture(&mut self) {
        match Recorder::start_on(self.input_kind, &self.device_choice(true), Some(self.settings.record_channels.max(1) as u16)) {
            Ok(rec) => {
                let key = self.input_device_key();
                self.room_noise.start_capture(&key, rec.sample_rate.max(1), rec.channels.max(1) as usize);
                self.noise_capture = Some(rec);
                self.status = "Capturing room noise (10 s) - stay quiet".to_owned();
            }
            Err(e) => self.status = au_core::i18n::tr_format("Noise capture failed: %s", &[&format!("{e:?}")]),
        }
    }

    fn process_noise_capture(&mut self) {
        let Some(rec) = self.noise_capture.as_mut() else { return };
        let mut buf = Vec::new();
        rec.poll(&mut buf);
        match self.room_noise.feed(&buf) {
            crate::room_noise::RoomNoiseEvent::None => {
                if !self.room_noise.is_capturing() { self.noise_capture = None; }
            }
            crate::room_noise::RoomNoiseEvent::CaptureFinished(dev) => {
                self.noise_capture = None;
                self.status = au_core::i18n::tr_format("Room noise profile captured for %s", &[&dev]);
            }
            crate::room_noise::RoomNoiseEvent::CaptureFailed(e) => {
                self.noise_capture = None;
                self.status = au_core::i18n::tr_format("Noise capture failed: %s", &[&e]);
            }
        }
    }

    fn open_record_options(&mut self) {
        self.record_opts_window = true;
    }

    fn begin_recording(&mut self) {
        let recorder = match Recorder::start_monitored(
            self.input_kind,
            &self.device_choice(true),
            Some(self.settings.record_channels.max(1) as u16),
            // Recording > Audible input monitoring (software playthrough).
            self.settings.sw_playthrough.then(|| self.device_choice(false)).as_ref(),
        ) {
            Ok(recorder) => recorder,
            Err(error) => {
                self.status = au_core::i18n::tr_format("Recording failed: %s", &[&format!("{error:?}")]);
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

        let key = self.input_device_key();
        self.live_denoised.clear();
        self.live_take_active = self.room_noise.begin_live_take(&key, rate, channels);
        self.recorder = Some(recorder);
        self.sync_record_volume();
        self.record_controller = Some(controller);
        self.record_buffer.clear();
        self.record_opts_window = false;

        // Recording > "Hear other tracks during recording" (/AudioIO/Duplex, Audacity's overdub):
        // other tracks play while recording; punch-and-roll always plays its pre-roll.
        if self.settings.duplex || matches!(self.record_mode, RecordModeUi::PunchAndRoll) {
            let project_rate = self.project_rate();
            let start_secs = au_core::frames_to_secs(start, rate);
            let playback_start = au_core::secs_to_frames(start_secs, project_rate);
            let tracks = crate::transport::mix_tracks(self.editor.state());
            match au_io::device::Player::start_with_effects_on(
                &self.device_choice(false),
                tracks,
                Vec::new(),
                None,
                project_rate,
                self.master_gain,
                playback_start,
                self.play_speed,
                None,
            ) {
                Ok(mut player) => {
                    player.handle.send(EngineCommand::SetMicrofades(self.settings.micro_fades));
                    self.player = Some(player);
                }
                Err(error) => self.status = au_core::i18n::tr_format("Playback failed: %s", &[&format!("{error:?}")]),
            }
        }
        self.status = "Recording".to_owned();
    }

    fn stop_recording(&mut self) {
        let dropped = self.recorder.as_ref().map_or(0, |r| r.dropped_samples());
        let rec_channels = self.recorder.as_ref().map_or(1, |r| r.channels as usize);
        let names_before: Vec<String> = self.editor.state().tracks.iter().map(|t| t.name.clone()).collect();
        self.recorder = None;
        self.record_buffer.clear();
        if let Some(controller) = self.record_controller.take() {
            match controller.commit(&mut self.editor) {
                Ok(track) => {
                    if self.live_take_active {
                        if let Some(buf) = self.room_noise.end_live_take() {
                            let name = au_core::i18n::tr_format("%s (noise suppressed)", &[&self.editor.state().tracks[track].name]);
                            self.editor.add_track(Track::new(&name, buf));
                        }
                    }
                    self.live_take_active = false;
                    // Recording > Name newly recorded track (ProjectAudioManager naming).
                    if track >= names_before.len() {
                        // Recorded tracks use the default sample format (quantized on write).
                        let bits = crate::settings::quantize_needed(self.settings.default_format);
                        self.editor.history.modify_current(|st| {
                            if let Some(t) = st.tracks.get_mut(track) {
                                t.int_bits = bits;
                                let len = t.len();
                                crate::fx_bridge::quantize_track_range(t, 0, len, crate::fx_bridge::hq_dither());
                            }
                        });
                        let names = crate::settings::recording_track_names(
                            &self.settings, &names_before, names_before.len(), rec_channels, local_now());
                        if let Some(name) = names.into_iter().next() {
                            self.editor.history.modify_current(|st| {
                                if let Some(t) = st.tracks.get_mut(track) { t.name = name; }
                            });
                        }
                    }
                    if dropped > 0 && self.settings.dropout_detected {
                        self.dropout_warning = true;
                    }
                    self.selected_track = Some(track);
                    self.selection = None;
                    self.ensure_track_mix();
                    self.status = "Recording committed".to_owned();
                }
                Err(error) => self.status = au_core::i18n::tr_format("Recording commit failed: %s", &[&error]),
            }
        }
        if let Some(mut player) = self.player.take() {
            player.stop();
        }
    }

    /// Pushes the recording-volume slider to the OS mixer, or to the software gain of
    /// the active captures when there is no working mixer (Audacity's emulation).
    fn sync_record_volume(&mut self) {
        let v = self.input_volume.clamp(0.0, 1.0);
        if (self.record_volume.get() - v).abs() > 1e-4 {
            self.record_volume.set(v);
        }
        let g = self.record_volume.software_gain().get();
        for rec in [self.recorder.as_ref(), self.noise_capture.as_ref()].into_iter().flatten() {
            rec.set_input_gain(g);
        }
    }

    fn process_recording(&mut self) {
        if self.recorder.is_some() || self.noise_capture.is_some() {
            self.sync_record_volume();
        }
        let Some(recorder) = self.recorder.as_mut() else {
            return;
        };
        self.record_buffer.clear();
        recorder.poll(&mut self.record_buffer);
        if self.live_take_active {
            // Raw samples still go to the take below (non-destructive); the denoised copy is kept alongside.
            self.room_noise.process_live(&self.record_buffer, &mut self.live_denoised);
        }
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
            Err(e) => self.status = au_core::i18n::tr_format("Plot Spectrum: %s", &[&e]),
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
        if let Some(mut state) = self.plugin_manager.take() {
            let mut open = true;
            egui::Window::new(au_core::i18n::tr_ui("Plugin Manager")).open(&mut open).default_size([760.0, 420.0]).show(ctx, |ui| {
                crate::plugin_manager::plugin_manager_ui(ui, &mut self.plugins, &mut state);
            });
            if open {
                self.plugin_manager = Some(state);
            }
        }
        if self.plugins.poll() {
            self.status = au_core::i18n::tr_format("%d plugins found", &[&self.plugins.plugins.len()]);
        }

        self.analyzer_windows(ctx);
        let names: Vec<String> = self
            .editor
            .state()
            .tracks
            .iter()
            .map(|t| t.name.clone())
            .collect();

        if let Some(mut dlg) = self.fx_dialog.take() {
            let action = dlg.show(ctx, &names);
            let sel = self.selected_track.map(|i| self.selection_for_track(i));
            let aux = dlg
                .aux_track
                .and_then(|i| self.editor.state().tracks.get(i))
                .map(|t| t.render());
            let mut keep = true;

            match action {
                crate::fx_ui::DialogAction::None => {}
                crate::fx_ui::DialogAction::Close => keep = false,
                _ if sel.is_none() => {
                    dlg.report = "Select a track (and a time range) first".into()
                }
                crate::fx_ui::DialogAction::GetProfile => {
                    let sel = sel.unwrap().normalized();
                    let t = &self.editor.state().tracks[sel.track];
                    match t
                        .clips
                        .render_range(sel.start, sel.end)
                        .map_err(|e| e.to_string())
                        .and_then(|b| dlg.effect.learn_profile(&b).map_err(|e| e.to_string()))
                    {
                        Ok(()) => {
                            dlg.has_profile = true;
                            dlg.report =
                                "Noise profile captured. Now select the audio to clean and press Apply."
                                    .into();
                        }
                        Err(e) => dlg.report = e,
                    }
                }
                crate::fx_ui::DialogAction::Preview => {
                    match crate::fx_bridge::preview(
                        self.editor.state(),
                        sel.unwrap(),
                        dlg.effect.as_mut(),
                        self.settings.effects_preview_len.max(0.1),
                    ) {
                        Ok(buf) => {
                            self.player = None;
                            let play_choice = self.device_choice(false);
                            let rate = buf.sample_rate;
                            let mt = au_io::engine::MixTrack {
                                audio: std::sync::Arc::new(buf),
                                offset: 0,
                                params: Default::default(),
                            };
                            match au_io::device::Player::start_with_effects_on(
                                &play_choice,
                                vec![mt],
                                Vec::new(),
                                None,
                                rate,
                                self.master_gain,
                                0,
                                1.0,
                                None,
                            ) {
                                Ok(p) => {
                                    self.player = Some(p);
                                    self.status = "Previewing".into();
                                }
                                Err(e) => {
                                    dlg.report = au_core::i18n::tr_format("Preview playback failed: %s", &[&format!("{e:?}")])
                                }
                            }
                        }
                        Err(e) => dlg.report = e,
                    }
                }
                crate::fx_ui::DialogAction::Apply => {
                    match crate::fx_bridge::apply_builtin(
                        &mut self.editor,
                        sel.unwrap(),
                        dlg.effect.as_mut(),
                        aux.as_ref(),
                    ) {
                        Ok(out) => {
                            dlg.report = out.report;
                            if !out.labels.is_empty() {
                                dlg.report
                                    .push_str(&au_core::i18n::tr_format("\n%d labels added", &[&out.labels.len()]));
                            }
                            self.status = au_core::i18n::tr_format("Applied %s", &[&dlg.effect.name()]);
                            if dlg.effect.kind() != EffectKind::Analyze {
                                keep = false;
                            }
                        }
                        Err(e) => dlg.report = e,
                    }
                }
            }

            if keep && crate::plugin_bridge::is_plugin_id(&dlg.effect.id()) {
                let id = dlg.effect.id().to_string();

                if self
                    .plugin_editor
                    .as_ref()
                    .is_some_and(|(editor_id, _)| editor_id != &id)
                {
                    if let Some((_, editor)) = self.plugin_editor.take() {
                        editor.close();
                    }
                }

                let editor_open = self
                    .plugin_editor
                    .as_ref()
                    .is_some_and(|(editor_id, _)| editor_id == &id);
                let mut open_editor = false;
                let mut close_editor = false;

                egui::Window::new(au_core::i18n::tr_ui("Plugin editor")).show(ctx, |ui| {
                    if editor_open {
                        if ui.button(au_core::i18n::tr_ui("Close editor")).clicked() {
                            close_editor = true;
                        }
                        ui.label(au_core::i18n::tr_ui("Editor process active"));
                    } else {
                        if ui.button(au_core::i18n::tr_ui("Open editor")).clicked() {
                            open_editor = true;
                        }
                        ui.label(au_core::i18n::tr_ui("Editor is not running"));
                    }
                });

                if close_editor {
                    if let Some((_, editor)) = self.plugin_editor.take() {
                        editor.close();
                    }
                } else if open_editor {
                    if let Some(desc) = self.plugins.find(&id).cloned() {
                        let state: Vec<(String, String)> = dlg
                            .effect
                            .text_keys()
                            .into_iter()
                            .filter_map(|key| {
                                dlg.effect
                                    .get_text(&key)
                                    .map(|value| (key.to_string(), value))
                            })
                            .collect();

                        match au_plugin_host::editor::EditorProcess::spawn(&desc, &state) {
                            Ok(editor) => self.plugin_editor = Some((id.clone(), editor)),
                            Err(e) => self.status = e.to_string(),
                        }
                    } else {
                        self.status = au_core::i18n::tr_format("Plugin not found: %s", &[&id]);
                    }
                }

                let mut drop_editor = false;
                if let Some((editor_id, editor)) = self.plugin_editor.as_mut() {
                    if editor_id == &id {
                        for event in editor.poll() {
                            match event {
                                au_plugin_host::editor::EditorEvent::Ready { .. } => {}
                                au_plugin_host::editor::EditorEvent::Param { id, value } => {
                                    dlg.effect.set(&format!("p{id}"), value as f64);
                                }
                                au_plugin_host::editor::EditorEvent::State(pairs) => {
                                    for (key, value) in pairs {
                                        dlg.effect.set_text(&key, &value);
                                    }
                                }
                                au_plugin_host::editor::EditorEvent::Error(error) => {
                                    self.status = au_core::i18n::tr_format("Plugin editor: %s", &[&error]);
                                }
                                au_plugin_host::editor::EditorEvent::Closed => {
                                    drop_editor = true;
                                }
                                au_plugin_host::editor::EditorEvent::Crashed { status } => {
                                    drop_editor = true;
                                    self.status = au_core::i18n::tr_format("Plugin editor crashed (%s); Audacity-RS is unaffected", &[&status]);
                                }
                            }
                        }

                        if editor.is_running() {
                            ctx.request_repaint_after(std::time::Duration::from_millis(100));
                        } else {
                            drop_editor = true;
                        }
                    }
                }

                if drop_editor {
                    if let Some((_, editor)) = self.plugin_editor.take() {
                        editor.close();
                    }
                }
            } else if let Some((_, editor)) = self.plugin_editor.take() {
                editor.close();
            }

            if keep {
                self.fx_dialog = Some(dlg);
            }
        }

        if self.fx_dialog.is_none() {
            if let Some((_, editor)) = self.plugin_editor.take() {
                editor.close();
            }
        }

        if self.rt_window {
            let mut open = true;
            let n = self.editor.state().tracks.len();
            self.aup3_extras.track_effects.resize_with(n, Default::default);
            let mut changed = false;
            let plugins = std::mem::take(&mut self.plugins);

            egui::Window::new(au_core::i18n::tr_ui("Realtime Effects"))
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.rt_target,
                            crate::fx_ui::RtTarget::Master,
                            "Master",
                        );
                        for (i, name) in names.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.rt_target,
                                crate::fx_ui::RtTarget::Track(i),
                                name,
                            );
                        }
                    });
                    ui.separator();

                    let list = match self.rt_target {
                        crate::fx_ui::RtTarget::Master => {
                            &mut self.aup3_extras.master_effects
                        }
                        crate::fx_ui::RtTarget::Track(i) if i < n => {
                            &mut self.aup3_extras.track_effects[i]
                        }
                        _ => &mut self.aup3_extras.master_effects,
                    };
                    changed = crate::fx_ui::realtime_panel(
                        ui,
                        list,
                        &mut self.rt_editing,
                        &plugins,
                    );
                    ui.label(
                        "Effects here are non-destructive: heard on playback, rendered on export, saved in .aup3.",
                    );
                });

            self.plugins = plugins;

            if changed && self.player.is_some() {
                // Restart playback so the engine picks up the new stack.
                let pos = self.player.as_ref().map(|p| p.position()).unwrap_or(0);
                self.player = None;
                self.play();
                if let Some(p) = self.player.as_mut() {
                    p.handle.send(EngineCommand::Seek(pos));
                }
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
        self.add_label_with_text(String::new());
    }

    /// DoAddLabel / LabelTrackView::DoChar: with "Use dialog for the name of a
    /// new label" the "New Label" dialog asks for the name first (Cancel adds
    /// nothing); otherwise the label is added and its text edited in place.
    fn add_label_with_text(&mut self, initial: String) {
        if self.settings.dialog_for_name_new_label {
            let (start, end) = self.selection_secs();
            self.new_label_dialog = Some((start, end, initial));
            return;
        }
        self.add_label_now(initial);
    }

    fn selection_secs(&self) -> (f64, f64) {
        let selection = self.selected_track.map(|index| self.selection_for_track(index));
        let rate = self.selected_rate();
        selection.map_or((0.0, 0.0), |selection| {
            (
                au_core::frames_to_secs(selection.start, rate),
                au_core::frames_to_secs(selection.end, rate),
            )
        })
    }

    fn add_label_now(&mut self, initial: String) {
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
        if !initial.is_empty() {
            let _ = self.editor.edit_label(label_track, label_index, &initial);
        }
        self.label_edit = Some(LabelEdit {
            track: label_track,
            label: label_index,
            text: initial,
        });
        self.status = au_core::i18n::tr_ui("Editing new label");
    }

    fn import_labels(&mut self) {
        if let Some(path) = self.file_dialog(crate::settings::DirOp::Import)
            .add_filter(au_core::i18n::tr_ui("Audacity label track"), &["txt"])
            .pick_file().inspect(|p| self.remember_dir(crate::settings::DirOp::Import, p))
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
                Err(error) => self.status = au_core::i18n::tr_format("Label import failed: %s", &[&error]),
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
        if let Some(path) = self.file_dialog(crate::settings::DirOp::Export)
            .add_filter(au_core::i18n::tr_ui("Audacity label track"), &["txt"])
            .save_file().inspect(|p| self.remember_dir(crate::settings::DirOp::Export, p))
        {
            self.status = match std::fs::write(path, contents) {
                Ok(()) => "Labels exported".to_owned(),
                Err(error) => au_core::i18n::tr_format("Label export failed: %s", &[&error]),
            };
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        // Tracks Behaviors > Type to create a label: a printable key typed while
        // a label track has focus starts a new label (LabelTrackView::DoCaptureKey).
        if self.settings.type_to_create_label
            && self.label_focus.is_some()
            && self.label_edit.is_none()
            && self.new_label_dialog.is_none()
        {
            let typed: String = ctx.input(|input| {
                input
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        egui::Event::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect()
            });
            let (start, end) = self.selection_secs();
            let label_here = self.editor.state().labels.iter().any(|lt| {
                lt.labels.iter().any(|l| l.start == start && l.end == end)
            });
            if !typed.is_empty() && !typed.chars().any(char::is_control) && !label_here {
                self.add_label_with_text(typed);
                return;
            }
        }
        // Pressed keys as Audacity key strings, resolved through the (remappable) key map.
        let pressed: Vec<String> = ctx.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => {
                        egui_key_to_audacity(*key, *modifiers)
                    }
                    _ => None,
                })
                .collect()
        });
        let mut commands: Vec<String> = pressed
            .iter()
            .filter_map(|k| crate::shortcuts::KeyCombo::parse(k))
            .filter_map(|combo| self.keymap.command_for(&combo).map(str::to_owned))
            .collect();
        let mut take = |ids: &[&str]| {
            let hit = commands.iter().position(|c| ids.contains(&c.as_str()));
            hit.map(|i| commands.remove(i)).is_some()
        };
        let shortcuts = (
            take(&["Cut"]),
            take(&["Copy"]),
            take(&["Paste"]),
            take(&["Undo"]),
            take(&["Redo"]),
            take(&["Delete", "DeleteKey", "DeleteKey2"]),
            take(&["DefaultPlayStop", "PlayStop"]),
            take(&["Split"]),
            take(&["Join"]),
            take(&["AddLabel"]),
            take(&["ZoomIn"]),
            take(&["ZoomOut"]),
            take(&["FitInWindow"]),
        );
        // Tracks Behaviors > "Select all audio, if selection required" (/GUI/SelectAllOnNone).
        let (cut, copy, delete) = if self.selection.is_none() && !self.settings.select_all_on_none && (shortcuts.0 || shortcuts.1 || shortcuts.5) {
            let action = if shortcuts.0 { "Cut" } else if shortcuts.1 { "Copy" } else { "Delete" };
            self.status = au_core::i18n::tr_format(
                "Select the audio for %s to use (for example, Ctrl + A to Select All) then try again.",
                &[&au_core::i18n::tr_ui(action)],
            );
            (false, false, false)
        } else {
            (shortcuts.0, shortcuts.1, shortcuts.5)
        };
        let shortcuts = (cut, copy, shortcuts.2, shortcuts.3, shortcuts.4, delete, shortcuts.6, shortcuts.7, shortcuts.8, shortcuts.9, shortcuts.10, shortcuts.11, shortcuts.12);
        let preferences = take(&["Preferences"]);
        if preferences {
            self.prefs_dialog.open_with(&self.prefs);
        }
        // Everything else goes through the scripting layer under its Audacity command id.
        for id in commands {
            self.keyboard_event = true;
            let (message, ok) = self.execute_script(&format!("{id}:"));
            self.keyboard_event = false;
            if !ok {
                self.status = message;
            }
        }

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
                { let behavior = self.edit_behavior(); self.edit_selection(|editor| editor.delete_with(selection, &behavior)) };
            }
        } else if let Some(index) = self.selected_track {
            let selection = self.selection_for_track(index);
            if cut {
                { let behavior = self.edit_behavior(); self.edit_selection(|editor| editor.cut_with(selection, &behavior)) };
            } else if copy {
                let result = self.editor.copy(selection);
                self.report(result);
            } else if paste {
                let at = selection.start;
                { let behavior = self.edit_behavior(); self.edit_selection(|editor| editor.paste_with(index, at, &behavior)) };
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

    /// The waveform view state of a track, created from the Tracks preferences on first use.
    /// Envelope value -> y. In dB view this is Audacity's GetWaveYPos(value, zoomMin, zoomMax,
    /// height, dB=true, outer=true, dBRange, clip=false) (Envelope::DrawPoints); linear view keeps 0..2.
    fn envelope_y(&self, index: usize, value: f64, top: f32, height: f32) -> f32 {
        match self.wave_ui.get(index) {
            Some(w) if w.db => {
                let range = self.settings.env_db_range.max(1) as f32;
                top + crate::wave_paint::get_wave_y_pos(value as f32, w.zoom_min, w.zoom_max,
                    height.round().max(1.0) as i32, true, true, range, false) as f32
            }
            _ => top + ((2.0 - value.clamp(0.0, 2.0)) / 2.0) as f32 * height,
        }
    }

    /// y -> envelope value, inverse of [`Self::envelope_y`] (ValueOfPixel in dB view).
    fn envelope_value(&self, index: usize, y: f32, top: f32, height: f32) -> f64 {
        match self.wave_ui.get(index) {
            Some(w) if w.db => {
                let range = self.settings.env_db_range.max(1) as f64;
                let v = crate::wave_paint::value_of_pixel((y - top).round() as i32,
                    height.round().max(1.0) as i32, false, true, range, w.zoom_min, w.zoom_max);
                (v.abs() as f64).clamp(0.0, 2.0)
            }
            _ => ((1.0 - (y - top) as f64 / height as f64) * 2.0).clamp(0.0, 2.0),
        }
    }

    fn wave_ui(&mut self, index: usize) -> WaveUi {
        let db = matches!(self.settings.default_wave_scale, crate::settings::WaveScale::LinearDb);
        while self.wave_ui.len() <= index {
            self.wave_ui.push(WaveUi { height: WaveUi::DEFAULT_HEIGHT, minimized: false, db, zoom_min: -1.0, zoom_max: 1.0 });
        }
        self.wave_ui[index]
    }

    /// Collapse / expand a track (Audacity's track minimize button); with "Half-wave display
    /// when collapsed" the vertical bounds follow WaveformView::DoSetMinimized.
    fn set_track_minimized(&mut self, index: usize, minimized: bool) {
        let mut w = self.wave_ui(index);
        w.minimized = minimized;
        if let Some((lo, hi)) = crate::wave_paint::collapsed_bounds(self.settings.collapse_to_half_wave, minimized) {
            w.zoom_min = lo;
            w.zoom_max = hi;
        }
        self.wave_ui[index] = w;
    }

    /// View > Track Size > Fit to Height (Viewport::ZoomFitVertically): share the available
    /// height between the non-collapsed tracks, never below the minimum track height.
    fn zoom_fit_vertically(&mut self, available: f32) {
        let count = self.editor.state().tracks.len();
        if count == 0 {
            return;
        }
        let _ = self.wave_ui(count - 1);
        let expanded: Vec<usize> = (0..count).filter(|&i| !self.wave_ui[i].minimized).collect();
        let channels: usize = expanded
            .iter()
            .map(|&i| self.editor.state().tracks[i].clips.n_channels.max(1))
            .sum();
        if channels == 0 {
            return;
        }
        let fixed: f32 = (0..count).filter(|i| !expanded.contains(i)).map(|_| WaveUi::MINIMIZED_HEIGHT).sum();
        let per = ((available - 28.0 - fixed) / channels as f32).max(WaveUi::MIN_HEIGHT);
        for i in expanded {
            let ch = self.editor.state().tracks[i].clips.n_channels.max(1) as f32;
            self.wave_ui[i].height = per * ch;
        }
    }

    fn draw_track(&mut self, ui: &mut egui::Ui, index: usize) {
        let wave = self.wave_ui(index);
        let db_range = self.settings.env_db_range.max(1) as f32;
        let sample_view = self.settings.sample_view;
        let width = ui.available_width().max(1.0);
        let zoom = self.zoom.clone();
        let selection = self.selection.filter(|selection| selection.track == index);
        let playhead = self
            .player
            .as_ref()
            .filter(|_| self.selected_track == Some(index))
            .map(Player::position);
        let drag_preview = self.drag;
        // Interface > Show Clipping / Show RMS in waveform (GUIPrefs).
        let (show_clipping, show_rms) = (self.settings.show_clipping, self.settings.show_rms);
        let wave_ctx = ui.ctx().clone();
        // Track-panel colours from the active Audacity theme (clr* ids).
        let theme_colour = |name: &str| crate::theme::c32(self.theme.colour(name));
        let clr_background = theme_colour("TrackBackground");
        let clr_clip = theme_colour("Unselected");
        let clr_clip_header = theme_colour("ClipAffordanceUnselectedBrush");
        let clr_sample = theme_colour("Sample");
        let [sr, sg, sb, _] = self.theme.colour("Selected");
        let clr_selection = egui::Color32::from_rgba_unmultiplied(sr, sg, sb, 110);

        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, if wave.minimized { WaveUi::MINIMIZED_HEIGHT } else { wave.height }), egui::Sense::click_and_drag());
        let info = {
            let state = self.editor.state();
            state.tracks.get(index).map(|track| {
                (
                    track.name.clone(),
                    track.clips.n_channels.max(1),
                    au_core::frames_to_secs(track.len(), track.sample_rate().max(1)),
                    state.tracks.len(),
                )
            })
        };
        if let Some((name, channels, seconds, count)) = info {
            let selected = self.selected_track == Some(index);
            self.track_accessibility(ui, &response, index, count, &name, channels, seconds, selected);
        }
        let painter = ui.painter_at(rect);
        self.canvas_width = rect.width();
        if let Some(p) = response.hover_pos() {
            self.last_pointer_time = Some(self.time_at_x(p.x, rect));
        }

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
            painter.rect_filled(rect, 3.0, clr_background);
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
                    clr_selection,
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
                    painter.rect_filled(clip_rect, 4.0, clr_clip);
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            clip_rect.left_top(),
                            egui::pos2(
                                clip_rect.right(),
                                (clip_rect.top() + 19.0).min(clip_rect.bottom()),
                            ),
                        ),
                        4.0,
                        clr_clip_header,
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

                    let waveform_cache = &mut self.waveform_cache;
                    let ctx = &wave_ctx;
                    let summaries = waveform_cache.get(clip, ctx);
                    let rendered_len = summaries.as_ref().map_or(0, |summaries| summaries.len);
                    if clip.len() > 0 {
                        let first_x = clip_rect.left().ceil().max(rect.left()) as i32;
                        let last_x = clip_rect.right().floor().min(rect.right()) as i32;
                        let waveform_top = (body.top() + 19.0).min(body.bottom());
                        let waveform_height = (body.bottom() - waveform_top).max(1.0);
                        for channel in 0..clip.buffer.n_channels().min(channels) {
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

                            let Some(summaries) = summaries.as_ref() else {
                                continue;
                            };
                            if rendered_len == 0 {
                                continue;
                            }
                            let raw = summaries.raw(&clip.buffer, channel);
                            let h_px = channel_height.round().max(1.0) as i32;
                            let y_of = |py: i32| channel_top + py as f32;
                            let clip_rate = rendered_len as f64 / clip_len_secs.max(f64::EPSILON);
                            // Zoomed in to >= 1/2 pixel per sample: draw individual samples
                            // (Tracks > Sample display: stem plot / connect dots), as WaveformView.
                            if crate::wave_paint::show_individual_samples(zoom.pixels_per_sec, clip_rate) {
                                if let Some(raw) = raw {
                                    let t_left = zoom.x_to_time((first_x as f32 - rect.left()) as f64) - clip_start;
                                    let t_right = zoom.x_to_time((last_x as f32 - rect.left()) as f64) - clip_start;
                                    let s0 = ((t_left * clip_rate).floor().max(0.0) as usize).min(raw.len());
                                    let s1 = (((t_right * clip_rate).ceil() as usize) + 1).min(raw.len());
                                    let g = crate::wave_paint::individual_samples(
                                        &raw[s0..s1], clip_start + s0 as f64 / clip_rate, clip_rate,
                                        zoom.h_offset, zoom.pixels_per_sec, rect.width() as i32, h_px,
                                        wave.zoom_min, wave.zoom_max, wave.db, db_range);
                                    let pts: Vec<egui::Pos2> = g.xs.iter().zip(&g.ys)
                                        .map(|(&x, &y)| egui::pos2(rect.left() + x as f32, y_of(y))).collect();
                                    let stroke = egui::Stroke::new(1.0, clr_sample);
                                    let stem = matches!(sample_view, crate::settings::SampleView::StemPlot);
                                    if stem {
                                        for p in &pts {
                                            painter.line_segment([egui::pos2(p.x, y_of(g.y_zero)), *p], stroke);
                                        }
                                    } else {
                                        painter.add(egui::Shape::line(pts.clone(), stroke));
                                    }
                                    if crate::wave_paint::show_points(zoom.pixels_per_sec, clip_rate) {
                                        for p in &pts {
                                            painter.circle_filled(*p, 2.5, clr_sample);
                                        }
                                    }
                                }
                                continue;
                            }
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
                                let start = local_start.min(rendered_len);
                                let end = local_end
                                    .max(start.saturating_add(1))
                                    .min(rendered_len);
                                if start >= end {
                                    continue;
                                }
                                let Some((low, high)) =
                                    summaries.min_max(raw, channel, start, end)
                                else {
                                    continue;
                                };
                                let rms = if show_rms {
                                    raw.and_then(|r| r.get(start..end)).map(|samples| {
                                        (samples.iter().map(|v| v * v).sum::<f32>() / samples.len().max(1) as f32).sqrt()
                                    })
                                } else {
                                    None
                                };
                                // TrackArt GetWaveYPos: linear or dB (Interface > Meter dB range).
                                let (h1, h2, r1, r2) = crate::wave_paint::min_max_rms_pixels(
                                    low, high, rms.unwrap_or(0.0), h_px, wave.zoom_min, wave.zoom_max, wave.db, db_range);
                                painter.line_segment(
                                    [egui::pos2(x as f32, y_of(h1)), egui::pos2(x as f32, y_of(h2))],
                                    egui::Stroke::new(1.0, clr_sample),
                                );
                                if let Some(rms) = rms {
                                    let _ = rms;
                                    painter.line_segment(
                                        [egui::pos2(x as f32, y_of(r1.max(h1))), egui::pos2(x as f32, y_of(r2.min(h2)))],
                                        egui::Stroke::new(1.0, clr_sample.gamma_multiply(0.55)),
                                    );
                                }
                                if show_clipping && (high >= 1.0 || low <= -1.0) {
                                    painter.line_segment(
                                        [egui::pos2(x as f32, center - channel_height * 0.5), egui::pos2(x as f32, center + channel_height * 0.5)],
                                        egui::Stroke::new(1.0, egui::Color32::RED),
                                    );
                                }
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
                let value_y = |value: f64| self.envelope_y(index, value, body.top(), body.height());
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

            // Cut lines (Tracks Behaviors > Enable cut lines), drawn like Audacity's.
            if let Some(track) = self.editor.state().tracks.get(index) {
                for line in &track.clips.cut_lines {
                    let x = rect.left() + zoom.time_to_x(au_core::frames_to_secs(line.at, rate)) as f32;
                    painter.line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        egui::Stroke::new(3.0, egui::Color32::from_rgb(200, 0, 0)),
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

        if response.clicked() || response.drag_started() {
            self.label_focus = None;
        }
        // CutlineHandle: left click on a cut line expands it, right click removes it.
        if response.clicked() || response.secondary_clicked() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let rate = self.editor.state().tracks.get(index).map(|t| t.sample_rate()).unwrap_or(1);
                let hit = self.editor.state().tracks.get(index).and_then(|t| {
                    t.clips.cut_lines.iter().map(|l| l.at).find(|&at| {
                        let x = rect.left() + self.zoom.time_to_x(au_core::frames_to_secs(at, rate)) as f32;
                        (x - pointer.x).abs() <= 3.0
                    })
                });
                if let Some(at) = hit {
                    let result = if response.clicked() {
                        self.editor.expand_cut_line(index, at)
                    } else {
                        self.editor.remove_cut_line(index, at)
                    };
                    if let Err(e) = result {
                        self.status = e.to_string();
                    }
                    return;
                }
            }
        }

        if response.drag_started() {
            if let Some(pointer) = response.interact_pointer_pos() {
                self.selected_track = Some(index);
                let time = self.time_at_x(pointer.x, rect);
                match self.tool {
                    Tool::Selection | Tool::Multi => {
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
                                let y = self.envelope_y(index, point_value, body_top, body_height);
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
                        let value = self.envelope_value(index, pointer.y, body_top, body_height);
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
                Tool::Selection | Tool::Multi => {
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
                        let value = self.envelope_value(index, pointer.y, body_top, body_height);
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
                        let y = self.envelope_y(index, value, rect.top() + 23.0, rect.height() - 23.0);
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

fn draw_note_tracks(&mut self, ui: &mut egui::Ui) {
    let note_tracks = self
        .editor
        .state()
        .note_tracks
        .iter()
        .map(|track| {
            (
                track.name.clone(),
                track.notes.clone(),
                track.offset,
                track.visible_channels,
                track.mute,
                track.solo,
                track.velocity,
                track.pitch_range(),
            )
        })
        .collect::<Vec<_>>();
    let zoom = self.zoom.clone();
    let mut mix_changes = Vec::new();
    let mut removals = Vec::new();

    for (track_index, (name, notes, offset, visible_channels, initial_mute, initial_solo, initial_velocity, pitch_range))
        in note_tracks.into_iter().enumerate()
    {
        let (rect, _response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(1.0), 120.0),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, egui::Color32::from_gray(30));
        painter.text(
            rect.left_top() + egui::vec2(7.0, 3.0),
            egui::Align2::LEFT_TOP,
            au_core::i18n::tr_format("%s (%d notes)", &[&name, &notes.len()]),
            egui::FontId::proportional(11.0),
            egui::Color32::LIGHT_GRAY,
        );

        let (mut min_pitch, mut max_pitch) = pitch_range.unwrap_or((48.0, 60.0));
        min_pitch -= 2.0;
        max_pitch += 2.0;
        if max_pitch - min_pitch < 12.0 {
            let center = (min_pitch + max_pitch) * 0.5;
            min_pitch = center - 6.0;
            max_pitch = center + 6.0;
        }
        let pitch_span = max_pitch - min_pitch;
        let pitch_to_y = |pitch: f64| {
            rect.bottom() - (((pitch - min_pitch) / pitch_span) as f32 * rect.height())
        };

        let mut c_pitch = (min_pitch / 12.0).ceil() * 12.0;
        while c_pitch <= max_pitch {
            let y = pitch_to_y(c_pitch);
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                egui::Stroke::new(1.0, egui::Color32::from_gray(55)),
            );
            c_pitch += 12.0;
        }

        for note in &notes {
            let channel = note.channel;
            if !(0..32).contains(&channel) {
                continue;
            }
            let channel_bit = 1_u32 << channel as u32;
            if visible_channels & channel_bit == 0 {
                continue;
            }

            let x0 = rect.left() + zoom.time_to_x(offset + note.start) as f32;
            let x1 = rect.left() + zoom.time_to_x(offset + note.start + note.duration) as f32;
            let note_height = (rect.height() / pitch_span as f32).max(2.0);
            let note_rect = egui::Rect::from_min_size(
                egui::pos2(
                    x0,
                    pitch_to_y(note.pitch) - note_height * 0.5,
                ),
                egui::vec2((x1 - x0).max(1.0), note_height),
            );
            let alpha = 0.4 + 0.6 * (note.loudness.clamp(0.0, 127.0) as f32 / 127.0);
            let hue = ((channel as f32 * 37.0).rem_euclid(360.0)) / 360.0;
            let color: egui::Color32 =
                egui::ecolor::Hsva::new(hue, 0.75, 0.95, alpha).into();
            painter.rect_filled(note_rect, 1.0, color);
        }

        let mut mute = initial_mute;
        let mut solo = initial_solo;
        let mut velocity = initial_velocity;
        let mut remove = false;
        ui.horizontal(|ui| {
            ui.checkbox(&mut mute, au_core::i18n::tr_ui("Mute"));
            ui.checkbox(&mut solo, au_core::i18n::tr_ui("Solo"));
            ui.label(au_core::i18n::tr_ui("Velocity dB"));
            ui.add(egui::DragValue::new(&mut velocity));
            velocity = velocity.clamp(-36.0, 36.0);
            remove = ui.button(au_core::i18n::tr_ui("Remove")).clicked();
        });

        if remove {
            removals.push(track_index);
        } else if mute != initial_mute || solo != initial_solo || velocity != initial_velocity {
            mix_changes.push((track_index, mute, solo, velocity));
        }
        ui.add_space(4.0);
    }

    for (track_index, mute, solo, velocity) in mix_changes {
        match self
            .editor
            .set_note_track_mix(track_index, mute, solo, velocity)
        {
            Ok(()) => self.ensure_track_mix(),
            Err(error) => self.status = au_core::i18n::tr_format("Could not update note track: %s", &[&format!("{error:?}")]),
        }
    }

    removals.sort_unstable_by(|left, right| right.cmp(left));
    for track_index in removals {
        match self.editor.remove_note_track(track_index) {
            Ok(()) => {
                self.ensure_track_mix();
                self.status = "Removed note track".to_owned();
            }
            Err(error) => self.status = au_core::i18n::tr_format("Could not remove note track: %s", &[&format!("{error:?}")]),
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
            if response.clicked() || response.drag_started() {
                self.label_focus = Some(track_index);
            }
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

    fn draw_windows(&mut self, ctx: &egui::Context) {
if self.ffmpeg_window {
    let mut retry_requested = false;
    egui::Window::new(au_core::i18n::tr_ui("FFmpeg"))
        .open(&mut self.ffmpeg_window)
        .show(ctx, |ui| match &self.ffmpeg_status {
            au_io::ffmpeg::FfmpegStatus::Available { version } => {
                ui.label(au_core::i18n::tr_format("FFmpeg %s loaded — M4A/AAC, WMA, AC3 and audio from video files can be imported and exported.", &[&version]));
            }
            au_io::ffmpeg::FfmpegStatus::Missing { message } => {
                ui.label(egui::RichText::new(message).monospace());
                ui.label(au_core::i18n::tr_format("Required: %s", &[&au_io::ffmpeg::SUPPORTED]));
                if ui.button(au_core::i18n::tr_ui("Retry detection")).clicked() {
                    retry_requested = true;
                }
            }
        });
    if retry_requested {
        self.status = "Restart Audacity-RS after installing FFmpeg to load it.".to_owned();
    }
}
        if self.speed_window {
            let mut open = self.speed_window;
            let mut apply = false;
            egui::Window::new(au_core::i18n::tr_ui("Change Clip Speed"))
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(au_core::i18n::tr_ui("Speed:"));
                    ui.add(
                        egui::DragValue::new(&mut self.speed_percent)
                            .speed(1.0)
                            .suffix("%"),
                    );
                    if ui.button(au_core::i18n::tr_ui("Apply")).clicked() {
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
            let mut capture_noise = false;
            egui::Window::new(au_core::i18n::tr_ui("Recording options"))
                .open(&mut open)
                .show(ctx, |ui| {
                    egui::ComboBox::from_label(au_core::i18n::tr_ui("Mode"))
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
                        ui.label(au_core::i18n::tr_ui("Punch uses the current selection on the selected track."));
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
                    egui::ComboBox::from_label(au_core::i18n::tr_ui("Trigger"))
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

                    egui::ComboBox::from_label(au_core::i18n::tr_ui("Input"))
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
                    ui.separator();
                    ui.label(au_core::i18n::tr_ui("Room noise profile"));
                    let key = self.input_device_key();
                    if self.room_noise.ui(ui, &key) && self.noise_capture.is_none() && self.recorder.is_none() {
                        capture_noise = true;
                    }
                    ui.separator();
                    if ui.button(au_core::i18n::tr_ui("Start Recording")).clicked() {
                        start_recording = true;
                    }
                });
            self.record_opts_window = open;
            if capture_noise {
                self.start_noise_capture();
            }
            if !self.room_noise.is_capturing() && self.noise_capture.is_some() {
                self.noise_capture = None;
            }
            if start_recording {
                self.begin_recording();
            }
        }

        self.show_export_dialogs(ctx);
        self.show_spectrogram_settings(ctx);
        self.show_macros(ctx);

        if self.export_options_window {
            let mut open = self.export_options_window;
            egui::Window::new(au_core::i18n::tr_ui("Export options"))
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(au_core::i18n::tr_ui("Bits:"));
                        egui::ComboBox::from_id_salt("export-bits")
                            .selected_text(self.export_bits.to_string())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.export_bits, 16, au_core::i18n::tr_ui("16"));
                                ui.selectable_value(&mut self.export_bits, 24, au_core::i18n::tr_ui("24"));
                                ui.selectable_value(&mut self.export_bits, 32, au_core::i18n::tr_ui("32"));
                            });
                    });
                    egui::ComboBox::from_label(au_core::i18n::tr_ui("Dither"))
                        .selected_text(match self.export_dither {
                            Dither::None => "None",
                            Dither::Rectangle => "Rectangle",
                            Dither::Triangle => "Triangle",
                            Dither::Shaped => "Shaped",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.export_dither, Dither::None, au_core::i18n::tr_ui("None"));
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
                            ui.selectable_value(&mut self.export_dither, Dither::Shaped, au_core::i18n::tr_ui("Shaped"));
                        });
                });
            self.export_options_window = open;
        }

        if let Some((start, end, mut text)) = self.new_label_dialog.take() {
            // LabelTrackView::DialogForLabelName: "New Label" with "Name:" and OK/Cancel.
            let mut done = None;
            egui::Window::new(au_core::i18n::tr_ui("New Label"))
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(au_core::i18n::tr_ui("&Name:"));
                        let r = ui.text_edit_singleline(&mut text);
                        r.request_focus();
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            done = Some(true);
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button(au_core::i18n::tr_ui("OK")).clicked() {
                            done = Some(true);
                        }
                        if ui.button(au_core::i18n::tr_ui("Cancel")).clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            done = Some(false);
                        }
                    });
                });
            match done {
                Some(true) => {
                    let label_track = if self.editor.state().labels.is_empty() {
                        self.editor.add_label_track("Labels")
                    } else {
                        self.label_focus.unwrap_or(0)
                    };
                    let result = self.editor.add_label(label_track, start, end, &text);
                    self.report(result);
                }
                Some(false) => {}
                None => self.new_label_dialog = Some((start, end, text)),
            }
        }

        if self.label_edit.is_some() {
            let mut open = true;
            let mut apply = false;
            egui::Window::new(au_core::i18n::tr_ui("Edit Label"))
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
                        if ui.button(au_core::i18n::tr_ui("Apply")).clicked() {
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
                ui.heading(au_core::i18n::tr_ui("Mixer"));
                ui.separator();
                let mut solo_click: Option<(usize, bool)> = None;
                for (index, name) in track_info.iter().enumerate() {
                    let mut params = self.track_mix[index];
                    let mut commit = false;
                    ui.group(|ui| {
                        ui.label(name);
                        let gain_now = params.gain;
                        let response = ui.add(
                            egui::Slider::new(&mut params.gain, 0.0..=2.0)
                                .text(au_core::i18n::tr_format("Gain (%.1f dB)", &[&(if gain_now > 0.0 { 20.0 * gain_now.log10() } else { f32::NEG_INFINITY })])),
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

                        let mute_changed = ui.checkbox(&mut params.mute, au_core::i18n::tr_ui("Mute")).changed();
                        let solo_changed = ui.checkbox(&mut params.solo, au_core::i18n::tr_ui("Solo")).changed();
                        if solo_changed {
                            solo_click = Some((index, ui.input(|i| i.modifiers.shift)));
                        }
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
                // Tracks Behaviors > Solo Button (Simple / Multi-track / None).
                if let Some((clicked, shift)) = solo_click {
                    let mut solos: Vec<bool> = self.track_mix.iter().map(|p| p.solo).collect();
                    // Undo the checkbox toggle and let the policy decide, like Audacity's OnSolo.
                    solos[clicked] = !solos[clicked];
                    let before = solos.clone();
                    crate::settings::apply_solo_click(self.settings.solo, &mut solos, clicked, shift);
                    for (i, solo) in solos.into_iter().enumerate() {
                        if i == clicked || solo != before[i] {
                            let mut p = self.track_mix[i];
                            p.solo = solo;
                            self.track_mix[i] = p;
                            self.send_track_mix(i, p, true);
                        }
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

    /// Meter bar on a dB scale whose floor is Interface > Meter dB range (/GUI/EnvdBRange,
    /// Audacity's DecibelScaleCutoff used by MeterPanel).
    fn draw_meter_bar(ui: &mut egui::Ui, label: &str, peak: f32, rms: f32, db_range: f32) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(190.0, 18.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, egui::Color32::from_gray(38));
        let db_position = |value: f32| {
            if value <= 0.0 {
                0.0
            } else {
                ((20.0 * value.log10() + db_range) / db_range).clamp(0.0, 1.0)
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
            au_core::i18n::tr_format("%s  P %.1f dB  R %.1f dB", &[&label, &(if peak > 0.0 { (20.0 * peak.log10()).max(-db_range) } else { -db_range }), &(if rms > 0.0 { (20.0 * rms.log10()).max(-db_range) } else { -db_range })]),
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
            ui.label(au_core::i18n::tr_ui("Start"));
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

            ui.label(au_core::i18n::tr_ui("End"));
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

            ui.label(au_core::i18n::tr_ui("Length"));
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
            ui.label(au_core::i18n::tr_format("Snap: %s", &[&snap_mode_name(self.snapper.mode.clone())]));
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
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.discard_session_file();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The panic hook has already written the crash report; flush unsaved
        // edits to the autosave before the panic continues to unwind.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.update_frame(ctx)));
        if let Err(p) = r {
            self.flush_autosave();
            std::panic::resume_unwind(p);
        }
    }
}

impl AudacityApp {
    fn update_frame(&mut self, ctx: &egui::Context) {
        if !self.prefs_applied {
            self.prefs_applied = true;
            self.apply_prefs();
        }
        if !self.theme_applied {
            crate::theme::apply(&self.theme, ctx);
            self.theme_applied = true;
        }
        self.auto_scroll(ctx);
        self.show_dropout_warning(ctx);

if self.script_pipe_enabled && self.script_pipe.is_none() && !self.script_pipe_failed {
    match crate::script_bridge::ScriptPipe::start(ctx) {
        Ok(pipe) => self.script_pipe = Some(pipe),
        Err(error) => {
            self.status = format!("mod-script-pipe could not start: {error}");
            self.script_pipe_failed = true;
        }
    }
}
if !self.script_pipe_enabled {
    self.script_pipe = None;
}
if let Some(pipe) = self.script_pipe.as_mut() {
    for request in pipe.drain(32) {
        let response = match self
            .execute_app_command(request.command())
            .or_else(|| self.execute_edit_command(request.command()))
            .or_else(|| self.execute_diagnostic_command(request.command()))
            .or_else(|| {
                let (name, _) = crate::script_bridge::parse_command(request.command());
                self.execute_macro_command(&name)
            })
        {
            Some((message, ok)) => au_scriptpipe::finished(&message, ok),
            None => {
                let (message, ok) = self.execute_script(request.command());
                au_scriptpipe::finished(&message, ok)
            }
        };
        request.respond(&response);
    }
}
        self.handle_shortcuts(ctx);

        let dt = ctx.input(|input| input.unstable_dt).max(0.0);
        self.record_peak *= (-dt * 2.0).exp();
        if self.recorder.is_some() {
            self.process_recording();
        }
        if self.noise_capture.is_some() {
            self.sync_record_volume();
            self.process_noise_capture();
            ctx.request_repaint();
        }
        self.poll_extra_playback(ctx);
        if self.player.as_ref().is_some_and(Player::is_finished) {
            self.player = None;
        }
        self.extra_dialogs(ctx);
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
            self.menu_titles.clear();
            let mut extra_command: Option<&'static str> = None;
            egui::menu::bar(ui, |ui| {
                ui.menu_button({ let t = au_core::i18n::tr_ui("File"); self.menu_titles.push(t.clone()); t }, |ui| {
                    if ui.button(au_core::i18n::tr_ui("Import audio…")).clicked() {
                        ui.close_menu();
                        self.import_file();
                    }
                    if ui.button(au_core::i18n::tr_ui("Open… (.aup3 / .aurs)")).clicked() {
                        ui.close_menu();
                        self.open_project();
                    }
                    if ui.button(au_core::i18n::tr_ui("Save Project")).clicked() {
                        ui.close_menu();
                        self.save_project();
                    }
                    if ui.button(au_core::i18n::tr_ui("Save Project As…")).clicked() {
                        ui.close_menu();
                        self.save_project_as();
                    }
                    if ui.button(au_core::i18n::tr_ui("Export WAV…")).clicked() {
                        ui.close_menu();
                        self.export(false);
                    }
                    if ui.button(au_core::i18n::tr_ui("Export FLAC…")).clicked() {
                        ui.close_menu();
                        self.export(true);
                    }
ui.separator();

for format in au_io::ffmpeg::ExportFormat::all() {
    let enabled = matches!(
        &self.ffmpeg_status,
        au_io::ffmpeg::FfmpegStatus::Available { .. }
    );
    let response = ui
        .add_enabled(
            enabled,
            egui::Button::new(au_core::i18n::tr_format("Export %s (FFmpeg)…", &[&format.label()])),
        )
        .on_disabled_hover_text("FFmpeg is not installed — see FFmpeg status");
    if response.clicked() {
        ui.close_menu();
        self.export_ffmpeg(format);
    }
}

let has_note_tracks = !self.editor.state().note_tracks.is_empty();
if ui
    .add_enabled(has_note_tracks, egui::Button::new(au_core::i18n::tr_ui("Export MIDI…")))
    .clicked()
{
    ui.close_menu();
    self.export_midi();
}

if ui.button(au_core::i18n::tr_ui("FFmpeg status…")).clicked() {
    ui.close_menu();
    self.ffmpeg_window = true;
}
                    if ui.button(au_core::i18n::tr_ui("Export Audio…")).clicked() {
                        ui.close_menu();
                        let mut dlg = crate::export_glue::ExportAudioDialog::new(self.aup3_extras.tags.clone());
                        apply_export_dither(&mut dlg.chooser.format, &self.prefs);
                        self.export_audio_dialog = Some(dlg);
                    }
                    if ui.button(au_core::i18n::tr_ui("Export Multiple…")).clicked() {
                        ui.close_menu();
                        self.export_multi_dialog = Some(crate::export_ui::ExportMultipleDialog::new(self.aup3_extras.tags.clone()));
                    }
                    if ui.button(au_core::i18n::tr_ui("Metadata Editor…")).clicked() {
                        ui.close_menu();
                        self.metadata_editor = Some(crate::export_ui::MetadataEditor::new(self.aup3_extras.tags.clone()));
                    }
                    if ui.button(au_core::i18n::tr_ui("Export options…")).clicked() {
                        ui.close_menu();
                        self.export_options_window = true;
                    }
                    ui.separator();
                    if ui.button(au_core::i18n::tr_ui("Import Labels…")).clicked() {
                        ui.close_menu();
                        self.import_labels();
                    }
                    if ui.button(au_core::i18n::tr_ui("Export Labels…")).clicked() {
                        ui.close_menu();
                        self.export_labels();
                    }
                });
                ui.menu_button({ let t = au_core::i18n::tr_ui("View"); self.menu_titles.push(t.clone()); t }, |ui| {
                    self.toolbars_menu(ui);
                    if ui.button(au_core::i18n::tr_ui("Spectrogram Settings…")).clicked() {
                        self.spectrogram_settings_open = true;
                        ui.close_menu();
                    }
                    ui.menu_button(au_core::i18n::tr_ui("Theme"), |ui| {
                        if let Some(message) = crate::theme::theme_menu(ui, &mut self.theme, &mut self.custom_theme) {
                            self.status = message;
                        }
                    });

let response = ui
    .checkbox(&mut self.script_pipe_enabled, au_core::i18n::tr_ui("mod-script-pipe"))
    .on_hover_text(
        "Enable the mod-script-pipe server at /tmp/audacity_script_pipe.to.<uid> and /tmp/audacity_script_pipe.from.<uid>",
    );
if response.changed() && self.script_pipe_enabled {
    self.script_pipe_failed = false;
}
                    ui.menu_button(au_core::i18n::tr_ui("Spectrogram scale"), |ui| {
                        use crate::spectrogram_view::FreqScale;
                        let before = self.spectrogram_settings.scale;
                        for (scale, name) in [(FreqScale::Linear, "Linear"), (FreqScale::Logarithmic, "Logarithmic"), (FreqScale::Mel, "Mel")] {
                            ui.radio_value(&mut self.spectrogram_settings.scale, scale, name);
                        }
                        if before != self.spectrogram_settings.scale {
                            self.spectrogram_cache.invalidate_all();
                        }
                    });
                    if ui
                        .selectable_label(self.mixer_visible, au_core::i18n::tr_ui("Mixer"))
                        .clicked()
                    {
                        self.mixer_visible = !self.mixer_visible;
                        ui.close_menu();
                    }
                });
                ui.menu_button({ let t = au_core::i18n::tr_ui("Edit"); self.menu_titles.push(t.clone()); t }, |ui| {
                    if ui.button(au_core::i18n::tr_ui("Cut")).clicked() {
                        ui.close_menu();
                        if let Some(index) = self.selected_track {
                            let selection = self.selection_for_track(index);
                            { let behavior = self.edit_behavior(); self.edit_selection(|editor| editor.cut_with(selection, &behavior)) };
                        }
                    }
                    if ui.button(au_core::i18n::tr_ui("Copy")).clicked() {
                        ui.close_menu();
                        if let Some(index) = self.selected_track {
                            let selection = self.selection_for_track(index);
                            let result = self.editor.copy(selection);
                            self.report(result);
                        }
                    }
                    if ui.button(au_core::i18n::tr_ui("Paste")).clicked() {
                        ui.close_menu();
                        if let Some(index) = self.selected_track {
                            let at = self.selection_for_track(index).start;
                            { let behavior = self.edit_behavior(); self.edit_selection(|editor| editor.paste_with(index, at, &behavior)) };
                        }
                    }
                    if ui.button(au_core::i18n::tr_ui("Delete")).clicked() {
                        ui.close_menu();
                        if let Some(index) = self.selected_track {
                            let selection = self.selection_for_track(index);
                            { let behavior = self.edit_behavior(); self.edit_selection(|editor| editor.delete_with(selection, &behavior)) };
                        }
                    }
                    ui.separator();
                    if ui.button(au_core::i18n::tr_ui("Split  (Ctrl+I)")).clicked() {
                        ui.close_menu();
                        self.split_selection();
                    }
                    if ui.button(au_core::i18n::tr_ui("Join  (Ctrl+J)")).clicked() {
                        ui.close_menu();
                        self.join_selection();
                    }
                    if ui.button(au_core::i18n::tr_ui("Change Clip Speed…")).clicked() {
                        ui.close_menu();
                        self.speed_window = true;
                    }
                    ui.separator();
                    if ui.button(au_core::i18n::tr_ui("Undo")).clicked() {
                        ui.close_menu();
                        let result = self.editor.undo();
                        self.report(result);
                    }
                    if ui.button(au_core::i18n::tr_ui("Redo")).clicked() {
                        ui.close_menu();
                        let result = self.editor.redo();
                        self.report(result);
                    }
                    ui.separator();
                    if ui.button(au_core::i18n::tr_ui("Preferences…")).clicked() {
                        ui.close_menu();
                        self.prefs_dialog.open_with(&self.prefs);
                    }
                });
                ui.menu_button({ let t = au_core::i18n::tr_ui("Tools"); self.menu_titles.push(t.clone()); t }, |ui| {
                    if ui.button(au_core::i18n::tr_ui("Macros…")).clicked() {
                        self.macros.open();
                        ui.close_menu();
                    }
                });
                ui.menu_button({ let t = au_core::i18n::tr_ui("Tracks"); self.menu_titles.push(t.clone()); t }, |ui| {
                    if ui.button(au_core::i18n::tr_ui("Add Label Track")).clicked() {
                        ui.close_menu();
                        self.editor.add_label_track("Labels");
                        self.status = "Label track added".to_owned();
                    }
                    if ui.button(au_core::i18n::tr_ui("Add Label at Selection  (Ctrl+B)")).clicked() {
                        ui.close_menu();
                        self.add_label_at_selection();
                    }
                    if ui.button(au_core::i18n::tr_ui("Import Labels…")).clicked() {
                        ui.close_menu();
                        self.import_labels();
                    }
                    if ui.button(au_core::i18n::tr_ui("Export Labels…")).clicked() {
                        ui.close_menu();
                        self.export_labels();
                    }
                    ui.separator();
                    let mut sync_lock = self.sync_lock;
                    if ui.checkbox(&mut sync_lock, au_core::i18n::tr_ui("Sync-Lock")).changed() {
                        self.sync_lock = sync_lock;
                        self.editor.set_sync_lock(sync_lock);
                    }
                });

                // Interface > Show 'Extra' menus: Audacity's Extra menu, item for item.
                if self.settings.show_extra_menus {
                    ui.menu_button({ let t = au_core::i18n::tr_ui("Extra"); self.menu_titles.push(t.clone()); t }, |ui| {
                        if let Some(id) = show_extra_menu(ui, crate::extra_menu::EXTRA_MENU) {
                            ui.close_menu();
                            extra_command = Some(id);
                        }
                    });
                }

                for (menu, kind) in [
                    ("Generate", EffectKind::Generate),
                    ("Effect", EffectKind::Process),
                    ("Analyze", EffectKind::Analyze),
                ] {
                    ui.menu_button({ let t = au_core::i18n::tr_ui(menu); self.menu_titles.push(t.clone()); t }, |ui| {
                        if kind == EffectKind::Process
                            && ui.button(au_core::i18n::tr_ui("Realtime Effects…")).clicked()
                        {
                            ui.close_menu();
                            self.rt_window = true;
                        }
                        egui::ScrollArea::vertical().max_height(500.0).show(ui, |ui| {
                            // Effects menu layout from Preferences > Effects ("Effects in menus" grouping), MenuHelper.cpp.
                            let nodes = effects_menu_nodes(kind, &self.settings.group_by);
                            if let Some(id) = show_effect_nodes(ui, &nodes) {
                                ui.close_menu();
                                match id.as_str() {
                                        "PlotSpectrum" => self.open_spectrum(),
                                        "Contrast" => self.contrast_win = Some(crate::analyze_ui::ContrastWindow::new()),
                                        "NoiseReduction" => {
                                            self.fx_dialog = crate::fx_ui::EffectDialog::open(&id);
                                            let key = self.input_device_key();
                                            if let (Some(d), Some(fx)) = (self.fx_dialog.as_mut(), self.room_noise.offline_effect(&key)) {
                                                d.effect = Box::new(fx);
                                                d.has_profile = true;
                                                d.report = "Using the captured room noise profile. Use Get Noise Profile to replace it.".into();
                                            }
                                        }
                                        _ => self.fx_dialog = crate::fx_ui::EffectDialog::open(&id),
                                    }
                            }

                                if kind == EffectKind::Process {
                                    ui.separator();
                                    if self.plugins.is_scanning() {
                                        ui.add_enabled(
                                            false,
                                            egui::Button::new(au_core::i18n::tr_ui("Scanning plugins…")),
                                        );
                                    } else {
                                        for d in self.plugins.enabled() {
                                            let id = d.id.clone();
                                            if ui.button(format!("{}…", d.name)).clicked() {
                                                ui.close_menu();
                                                match self.plugins.instantiate(&id) {
                                                    Ok(eff) => {
                                                        self.fx_dialog = Some(
                                                            crate::fx_ui::EffectDialog {
                                                                effect: Box::new(eff),
                                                                preset_name: String::new(),
                                                                aux_track: None,
                                                                report: String::new(),
                                                                has_profile: false,
                                                            },
                                                        );
                                                    }
                                                    Err(e) => self.status = e,
                                                }
                                            }
                                        }

                                        if ui.button(au_core::i18n::tr_ui("Plugin Manager…")).clicked() {
                                            self.plugin_manager = Some(Default::default());
                                            ui.close_menu();
                                        }
                                        if ui.button(au_core::i18n::tr_ui("Rescan plugins")).clicked() {
                                            self.plugins.start_scan();
                                            ui.close_menu();
                                        }
                                    }
                                }
                            });
                    });
                }
            });
            if let Some(id) = extra_command {
                if id == "FullScreenOnOff" {
                    // ExtraMenus.cpp OnFullScreen.
                    let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
                } else {
                    let (message, ok) = self.execute_script(&format!("{id}:"));
                    if !ok {
                        self.status = message;
                    }
                }
            }

            self.toolbar_dock(ui, ctx, true);

            ui.horizontal(|ui| {
                let duration = self.project_duration();
                let visible = ui.available_width() as f64 / self.zoom.pixels_per_sec.max(0.1);
                let max_offset = (duration - visible).max(0.0);
                self.zoom.h_offset = self.zoom.h_offset.clamp(0.0, max_offset);
                ui.label(au_core::i18n::tr_ui("Horizontal scroll:"));
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
            // Tracks > Auto-fit track height: fit vertically after every new undo state
            // (TracksPrefs.cpp Handler on UndoRedoMessage::Pushed).
            let pushes = self.editor.history.push_count();
            if pushes != self.fit_seen_pushes {
                self.fit_seen_pushes = pushes;
                if self.settings.tracks_fit_vertically_zoomed {
                    self.zoom_fit_vertically(ui.available_height());
                }
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                let count = self.editor.state().tracks.len();
                self.track_views.resize(count, Default::default());
                for index in 0..count {
                    let mut selected_view = self.track_views[index];
                    egui::ComboBox::from_id_salt(("track_view", index))
                        .selected_text(match selected_view {
                            crate::spectrogram_view::TrackView::Waveform => "Waveform",
                            crate::spectrogram_view::TrackView::Spectrogram => "Spectrogram",
                            crate::spectrogram_view::TrackView::Multiview => "Multi-view",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut selected_view,
                                crate::spectrogram_view::TrackView::Waveform,
                                "Waveform",
                            );
                            ui.selectable_value(
                                &mut selected_view,
                                crate::spectrogram_view::TrackView::Spectrogram,
                                "Spectrogram",
                            );
                            ui.selectable_value(
                                &mut selected_view,
                                crate::spectrogram_view::TrackView::Multiview,
                                "Multi-view",
                            );
                        });
                    self.track_views[index] = selected_view;
                    let mut w = self.wave_ui(index);
                    ui.horizontal(|ui| {
                        let label = if w.minimized { au_core::i18n::tr_ui("Expand") } else { au_core::i18n::tr_ui("Collapse") };
                        if ui.small_button(label).clicked() {
                            w.minimized = !w.minimized;
                        }
                        ui.checkbox(&mut w.db, au_core::i18n::tr_ui("dB"));
                    });
                    let minimized = w.minimized;
                    self.wave_ui[index].db = w.db;
                    if minimized != self.wave_ui[index].minimized {
                        self.set_track_minimized(index, minimized);
                    }

                    match selected_view {
                        crate::spectrogram_view::TrackView::Waveform => {
                            self.draw_track(ui, index);
                        }
                        crate::spectrogram_view::TrackView::Spectrogram => {
                            // Borrow (not clone) the track: cloning copies all of its audio every frame.
                            let track = &self.editor.state().tracks[index];
                            let changed = crate::spectrogram_view::draw_spectrogram_track(
                                ui,
                                &mut self.spectrogram_cache,
                                index,
                                track,
                                &self.zoom,
                                &self.spectrogram_settings,
                                140.0,
                                &mut self.spectral_selection,
                            );
                            if changed {
                                if let Some(selection) = self.spectral_selection {
                                    self.status = au_core::i18n::tr_format("Spectral selection: %.3f–%.3f s, %.0f–%.0f Hz", &[&selection.t0(), &selection.t1(), &selection.f0(), &selection.f1()]);
                                }
                            }
                        }
                        crate::spectrogram_view::TrackView::Multiview => {
                            self.draw_track(ui, index);
                            let track = &self.editor.state().tracks[index];
                            let changed = crate::spectrogram_view::draw_spectrogram_track(
                                ui,
                                &mut self.spectrogram_cache,
                                index,
                                track,
                                &self.zoom,
                                &self.spectrogram_settings,
                                90.0,
                                &mut self.spectral_selection,
                            );
                            if changed {
                                if let Some(selection) = self.spectral_selection {
                                    self.status = au_core::i18n::tr_format("Spectral selection: %.3f–%.3f s, %.0f–%.0f Hz", &[&selection.t0(), &selection.t1(), &selection.f0(), &selection.f1()]);
                                }
                            }
                        }
                    }
                    ui.add_space(5.0);
                }

                self.draw_note_tracks(ui);
                self.draw_labels(ui);
                if count == 0 && self.editor.state().labels.is_empty() && self.editor.state().note_tracks.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label(au_core::i18n::tr_ui("Open or import audio to begin."));
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
                    ui.label(au_core::i18n::tr_format("Label %d:%d", &[&(track + 1), &(label + 1)]));
                }
                if self.player.is_some() {
                    ui.label(au_core::i18n::tr_ui("Playhead active"));
                }
            });

            self.toolbar_dock(ui, ctx, false);
        });

        self.floating_toolbars(ctx);
        self.draw_windows(ctx);
        self.waveform_cache.end_frame();
        let progress = &mut self.progress;
        self.test_jobs.retain(|(ticket, handle)| {
            if handle.try_result().is_some() {
                progress.finish(*ticket);
                false
            } else {
                true
            }
        });
        self.show_preferences(ctx);
        self.poll_io_jobs();
        if !self.io_jobs.is_empty() { ctx.request_repaint_after(std::time::Duration::from_millis(50)); }
        self.progress.show(ctx);

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

// ---------------------------------------------------------------------------
// au-script host: the full Audacity scripting command layer runs after the
// app's own pipe commands (Import2/Export2/Select/... above), replacing the
// temporary script_bridge interpreter.
// ---------------------------------------------------------------------------
struct AppScriptHost<'a> {
    app: &'a mut AudacityApp,
    work: Option<au_core::ProjectState>,
    sel: au_script::Selection,
}

impl<'a> AppScriptHost<'a> {
    fn new(app: &'a mut AudacityApp) -> Self {
        let state = app.editor.state();
        let n = state.tracks.len() + state.labels.len();
        let mut sel = au_script::Selection { t0: 0.0, t1: 0.0, f0: -1.0, f1: -1.0, tracks: vec![false; n] };
        if let Some(track) = app.selected_track {
            let s = app.selection_for_track(track);
            let rate = f64::from(state.tracks.get(track).map_or(44_100, |t| t.sample_rate()).max(1));
            sel.t0 = s.start as f64 / rate;
            sel.t1 = s.end as f64 / rate;
            if track < n {
                sel.tracks[track] = true;
            }
        }
        if let Some(sp) = app.spectral_selection {
            sel.f0 = sp.f0();
            sel.f1 = sp.f1();
        }
        Self { app, work: None, sel }
    }

    /// Writes the script selection back into the app's selection model.
    fn finish(self) {
        let n_wave = self.app.editor.state().tracks.len();
        if let Some(track) = self.sel.tracks.iter().take(n_wave).position(|s| *s) {
            let rate = self.app.editor.state().tracks[track].sample_rate();
            let len = self.app.editor.state().tracks[track].len();
            let a = au_core::secs_to_frames(self.sel.t0.max(0.0), rate).min(len);
            let b = au_core::secs_to_frames(self.sel.t1.max(0.0), rate).min(len);
            self.app.selected_track = Some(track);
            self.app.selection = Some(Selection { track, start: a.min(b), end: a.max(b) });
            if self.sel.f0 >= 0.0 || self.sel.f1 >= 0.0 {
                let mut region = au_core::selected_region::SelectedRegion::new(self.sel.t0, self.sel.t1);
                region.set_frequencies(self.sel.f0, self.sel.f1);
                self.app.spectral_selection = Some(crate::spectrogram_view::SpectralSelection::new(track, region));
            }
        }
    }

    fn delegate(&mut self, line: &str) -> Option<(String, bool)> {
        self.work = None;
        let r = self
            .app
            .execute_app_command(line)
            .or_else(|| self.app.execute_edit_command(line));
        let n = self.app.editor.state().tracks.len() + self.app.editor.state().labels.len();
        self.sel.tracks.resize(n, false);
        r
    }
}

impl au_script::ScriptHost for AppScriptHost<'_> {
    fn state(&mut self) -> &mut au_core::ProjectState {
        if self.work.is_none() {
            self.work = Some(self.app.editor.state().clone());
        }
        self.work.as_mut().expect("work state")
    }
    fn selection(&mut self) -> &mut au_script::Selection {
        &mut self.sel
    }
    fn push_undo(&mut self, desc: &str) {
        if let Some(work) = self.work.take() {
            self.app.editor.history.push(desc, work);
        }
    }
    fn apply_effect(&mut self, id: &str, params: &[(String, String)]) -> Result<String, String> {
        self.work = None;
        let mut effect = au_effects::registry::create(id).ok_or_else(|| format!("unknown effect {id}"))?;
        au_effects::stack::params_from_strings(effect.as_mut(), params);
        let n_wave = self.app.editor.state().tracks.len();
        let tracks: Vec<usize> = (0..n_wave).filter(|t| self.sel.tracks.get(*t).copied().unwrap_or(false)).collect();
        if tracks.is_empty() {
            return Err("No tracks selected".into());
        }
        let mut report = String::new();
        for track in tracks {
            let rate = self.app.editor.state().tracks[track].sample_rate();
            let len = self.app.editor.state().tracks[track].len();
            let a = au_core::secs_to_frames(self.sel.t0.max(0.0), rate);
            let b = au_core::secs_to_frames(self.sel.t1.max(0.0), rate);
            // Generators fill the whole selected time, also past the end of the track.
            let (a, b) = if effect.kind() == au_effects::core::EffectKind::Generate {
                if b.max(a) > len {
                    let n_ch = self.app.editor.state().tracks[track].clips.n_channels.max(1);
                    let pad = au_core::AudioBuffer::from_channels(rate, vec![vec![0.0; b.max(a) - len]; n_ch]);
                    self.app.editor.history.modify_current(|st| {
                        let _ = st.tracks[track].clips.insert(len, &pad);
                    });
                }
                (a, b)
            } else {
                (a.min(len), b.min(len))
            };
            let out = crate::fx_bridge::apply_builtin(
                &mut self.app.editor,
                Selection { track, start: a.min(b), end: a.max(b) },
                effect.as_mut(),
                None,
            )?;
            report.push_str(&out.report);
        }
        Ok(report)
    }
    fn effect_ids(&self) -> Vec<(String, Vec<(String, String, String)>)> {
        au_effects::registry::all()
            .iter()
            .map(|e| {
                (
                    e.id().to_string(),
                    e.params()
                        .iter()
                        .map(|p| {
                            let ty = if matches!(p.ty, au_effects::core::ParamType::Choice(_)) { "enum" } else { "double" };
                            (p.key.to_string(), ty.to_string(), p.default.to_string())
                        })
                        .collect(),
                )
            })
            .collect()
    }
    fn import(&mut self, path: &str) -> Result<(), String> {
        match self.delegate(&format!("Import2: Filename=\"{path}\"")) {
            Some((_, true)) => Ok(()),
            Some((m, false)) => Err(m),
            None => Err("Import2 unavailable".into()),
        }
    }
    fn export(&mut self, path: &str, _channels: u32) -> Result<(), String> {
        match self.delegate(&format!("Export2: Filename=\"{path}\"")) {
            Some((_, true)) => Ok(()),
            Some((m, false)) => Err(m),
            None => Err("Export2 unavailable".into()),
        }
    }
    fn menu_command(&mut self, id: &str) -> Option<Result<(), String>> {
        self.delegate(id).map(|(m, ok)| if ok { Ok(()) } else { Err(m) })
    }
    fn message(&mut self, text: &str) {
        self.app.status = text.to_string();
    }
}

impl AudacityApp {
    /// Runs one scripting line through au-script against this project.
    fn execute_script(&mut self, line: &str) -> (String, bool) {
        let mut host = AppScriptHost::new(self);
        let r = au_script::execute(&mut host, line);
        host.finish();
        r
    }

    /// Audacity's `Macro_<NameWithoutSpaces>` command (Tools > Apply Macro over the pipe).
    fn execute_macro_command(&mut self, command: &str) -> Option<(String, bool)> {
        let id = command.strip_prefix("Macro_")?;

        let mut mc =
            au_script::macros::MacroCommands::new(au_script::macros::default_macro_dir());
        let Some(name) = mc
            .get_names()
            .into_iter()
            .find(|name| name.replace(' ', "") == id)
        else {
            return Some((
                format!("Your batch command of {command} was not recognized."),
                false,
            ));
        };

        if mc.read_macro(&name).is_err() || mc.commands().is_empty() {
            mc.restore_macro(&name);
        }

        let mut runner = au_script::macros::MacroRunner::new(mc.commands().to_vec());
        let mut host = AppScriptHost::new(self);
        let result = runner.apply(&mut host);
        host.finish();

        match result {
            Ok(()) => Some((String::new(), true)),
            Err(error) => Some((error, false)),
        }
    }
}

/// Tools > Macros… (Audacity's Macros Manager: list, edit, apply to project / files).
#[derive(Default)]
struct MacrosDialog {
    open: bool,
    mc: Option<au_script::macros::MacroCommands>,
    names: Vec<String>,
    current: String,
    text: String,
    new_name: String,
    report: String,
}

impl MacrosDialog {
    fn open(&mut self) {
        let mc = au_script::macros::MacroCommands::new(au_script::macros::default_macro_dir());
        self.names = mc.get_names();
        self.mc = Some(mc);
        self.open = true;
        if let Some(first) = self.names.first().cloned() {
            self.select(&first);
        }
    }
    fn select(&mut self, name: &str) {
        let Some(mc) = self.mc.as_mut() else { return };
        self.current = name.to_string();
        let _ = mc.read_macro(name);
        self.text = mc.commands().iter().map(|l| l.serialize()).collect::<Vec<_>>().join("\n");
    }
    fn lines(&self) -> Vec<au_script::macros::MacroLine> {
        self.text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let (c, p) = l.split_once(':').unwrap_or((l, ""));
                au_script::macros::MacroLine::new(c.trim(), p.trim())
            })
            .collect()
    }
}

impl AudacityApp {
    fn show_macros(&mut self, ctx: &egui::Context) {
        if !self.macros.open {
            return;
        }
        let mut open = true;
        let mut apply: Option<bool> = None; // Some(false)=project, Some(true)=files
        let d = &mut self.macros;
        egui::Window::new(au_core::i18n::tr_ui("Macros")).open(&mut open).default_width(520.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    for n in d.names.clone() {
                        if ui.selectable_label(n == d.current, &n).clicked() {
                            d.select(&n);
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut d.new_name);
                        if ui.button(au_core::i18n::tr_ui("New")).clicked() && !d.new_name.trim().is_empty() {
                            if let Some(mc) = d.mc.as_ref() {
                                mc.add_macro(d.new_name.trim());
                                d.names = mc.get_names();
                            }
                            let n = d.new_name.trim().to_string();
                            d.select(&n);
                        }
                    });
                    let fixed = au_script::macros::MacroCommands::is_fixed(&d.current);
                    if ui.add_enabled(!fixed && !d.current.is_empty(), egui::Button::new(au_core::i18n::tr_ui("Remove"))).clicked() {
                        if let Some(mc) = d.mc.as_ref() {
                            mc.delete_macro(&d.current);
                            d.names = mc.get_names();
                        }
                        d.current.clear();
                        d.text.clear();
                    }
                });
                ui.vertical(|ui| {
                    ui.label(au_core::i18n::tr_format("Macro: %s", &[&d.current]));
                    ui.add(egui::TextEdit::multiline(&mut d.text).code_editor().desired_rows(10));
                    ui.horizontal(|ui| {
                        if ui.button(au_core::i18n::tr_ui("Save")).clicked() {
                            let lines = d.lines();
                            if let Some(mc) = d.mc.as_mut() {
                                mc.set_commands(lines);
                                d.report = match mc.write_macro(&d.current) {
                                    Ok(_) => "Saved".into(),
                                    Err(e) => au_core::i18n::tr_format("Save failed: %s", &[&e]),
                                };
                            }
                        }
                        if ui.button(au_core::i18n::tr_ui("Apply to Project")).clicked() {
                            apply = Some(false);
                        }
                        if ui.button(au_core::i18n::tr_ui("Apply to Files…")).clicked() {
                            apply = Some(true);
                        }
                    });
                    ui.label(&d.report);
                });
            });
        });
        self.macros.open = open;
        if let Some(files) = apply {
            let mut runner = au_script::macros::MacroRunner::new(self.macros.lines());
            if files {
                let picked = self.file_dialog(crate::settings::DirOp::Import).pick_files().inspect(|ps| if let Some(p) = ps.first() { self.remember_dir(crate::settings::DirOp::Import, p) }).unwrap_or_default();
                if picked.is_empty() {
                    return;
                }
                let out_dir = picked[0].parent().map(|p| p.to_path_buf()).unwrap_or_default();
                let mut host = AppScriptHost::new(self);
                let r = runner.apply_to_files(&mut host, &picked, &out_dir, "wav", 2);
                host.finish();
                self.macros.report = match r {
                    Ok(paths) => au_core::i18n::tr_format("Exported %d file(s) to %s", &[&paths.len(), &out_dir.join("macro-output").display().to_string()]),
                    Err(e) => e,
                };
            } else {
                let mut host = AppScriptHost::new(self);
                let r = runner.apply(&mut host);
                host.finish();
                self.macros.report = match r {
                    Ok(()) => au_core::i18n::tr_format("Applied macro %s", &[&self.macros.current]),
                    Err(e) => e,
                };
            }
        }
    }
}

enum IoJob {
    Import(PathBuf, au_runtime::JobHandle<Result<au_core::AudioBuffer, String>>),
    Export(String, au_runtime::JobHandle<String>),
}
enum IoDone {
    Import(Result<Result<au_core::AudioBuffer, String>, au_runtime::JobError>),
    Export(Result<String, au_runtime::JobError>),
}

/// egui key event -> Audacity NormalizedKeyString (e.g. "Ctrl+Shift+Z", "Space", "Return").
pub(crate) fn egui_key_to_audacity(key: egui::Key, m: egui::Modifiers) -> Option<String> {
    use egui::Key as K;
    let name = match key {
        K::Enter => "Return".to_owned(),
        K::ArrowLeft => "Left".to_owned(),
        K::ArrowRight => "Right".to_owned(),
        K::ArrowUp => "Up".to_owned(),
        K::ArrowDown => "Down".to_owned(),
        K::Comma => ",".to_owned(),
        K::Period => ".".to_owned(),
        K::OpenBracket => "[".to_owned(),
        K::CloseBracket => "]".to_owned(),
        K::Slash => "/".to_owned(),
        K::Backslash => "\\".to_owned(),
        K::Minus => "-".to_owned(),
        K::Equals => "=".to_owned(),
        K::Semicolon => ";".to_owned(),
        other => {
            let n = other.name();
            n.strip_prefix("Num").filter(|d| d.len() == 1).unwrap_or(n).to_owned()
        }
    };
    let mut s = String::new();
    if m.command || m.ctrl { s.push_str("Ctrl+"); }
    if m.alt { s.push_str("Alt+"); }
    if m.shift { s.push_str("Shift+"); }
    s.push_str(&name);
    Some(s)
}

/// Where this app keeps its audacity.cfg (AU_PREFS_PATH overrides). It never writes
/// Audacity's own file; on first run Audacity's ~/.config/audacity/audacity.cfg is imported.
fn prefs_path() -> PathBuf {
    if let Some(p) = std::env::var_os("AU_PREFS_PATH") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("audacity-rs").join("audacity.cfg")
}

fn load_prefs() -> au_core::prefs::Prefs {
    let path = prefs_path();
    if let Ok(p) = au_core::prefs::Prefs::load(&path) {
        return p;
    }
    if std::env::var_os("AU_PREFS_PATH").is_none() {
        if let Ok(p) = au_core::prefs::Prefs::load(au_core::prefs::default_cfg_path()) {
            return p;
        }
    }
    au_core::prefs::Prefs::new()
}

/// Quality > High-quality conversion dither (Audacity uses it for export).
fn apply_export_dither(format: &mut au_io::export::ExportFormat, prefs: &au_core::prefs::Prefs) {
    use au_io::export::intfmt::AudDither;
    use au_io::export::ExportFormat as F;
    let kind = match prefs.high_quality_dither_kind().as_str() {
        "None" => AudDither::None,
        "Rectangle" => AudDither::Rectangle,
        "Triangle" => AudDither::Triangle,
        _ => AudDither::Shaped,
    };
    match format {
        F::Pcm(o) => o.dither = kind,
        F::Flac(o) => o.dither = kind,
        F::WavPack(o) => o.dither = kind,
        _ => {}
    }
}

impl AudacityApp {
    /// A file dialog starting in the Directories-preference default for `op` (FileNames::FindDefaultPath).
    fn file_dialog(&self, op: crate::settings::DirOp) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new();
        match crate::settings::find_default_path(&self.prefs, op) {
            Some(dir) if dir.is_dir() => dialog.set_directory(dir),
            _ => dialog,
        }
    }

    /// FileNames::UpdateDefaultPath after a successful pick; saved immediately.
    fn remember_dir(&mut self, op: crate::settings::DirOp, path: &std::path::Path) {
        crate::settings::update_default_path(&mut self.prefs, op, path);
        self.save_prefs();
    }

    /// Preferences > Devices: host + playback/recording device (empty = system default).
    fn device_choice(&self, recording: bool) -> au_io::device::DeviceChoice {
        au_io::device::DeviceChoice {
            host: self.settings.host.clone(),
            device: if recording { self.settings.recording_device.clone() } else { self.settings.playback_device.clone() },
        }
    }

    /// Tracks > Default View Mode for a newly created track.
    fn apply_default_view(&mut self, index: usize) {
        use crate::spectrogram_view::TrackView;
        let view = match self.settings.default_view_mode {
            crate::settings::ViewMode::Waveform => TrackView::Waveform,
            crate::settings::ViewMode::Spectrogram => TrackView::Spectrogram,
            crate::settings::ViewMode::Multiview => TrackView::Multiview,
        };
        if self.track_views.len() <= index {
            self.track_views.resize(index + 1, Default::default());
        }
        self.track_views[index] = view;
    }

    /// Interface > Beep on completion of longer activities.
    fn beep_if_enabled(&self) {
        if self.settings.beep_on_completion {
            use std::io::Write;
            let _ = std::io::stderr().write_all(b"\x07");
        }
    }

    /// Tracks > Auto-scroll if head unpinned: page the view when the play head leaves it.
    fn auto_scroll(&mut self, ctx: &egui::Context) {
        if !self.settings.auto_scroll {
            return;
        }
        let Some(pos) = self.player.as_ref().map(|p| p.position()) else { return };
        let t = pos as f64 / self.project_rate().max(1) as f64;
        let visible = ctx.screen_rect().width() as f64 / self.zoom.pixels_per_sec.max(0.1);
        if t > self.zoom.h_offset + visible || t < self.zoom.h_offset {
            self.zoom.h_offset = t;
        }
    }

    /// Audacity's DropoutDetector warning (/Warnings/DropoutDetected).
    fn show_dropout_warning(&mut self, ctx: &egui::Context) {
        if !self.dropout_warning {
            return;
        }
        let mut close = false;
        egui::Window::new(au_core::i18n::tr_ui("Warning")).collapsible(false).show(ctx, |ui| {
            ui.label(au_core::i18n::tr("Recorded audio was lost at the labeled locations. Possible causes:\n\nOther applications are competing with Audacity for processor time\n\nYou are saving directly to a slow external storage device\n"));
            ui.checkbox(&mut self.dropout_dont_show, au_core::i18n::tr_ui("Don't show this warning again"));
            if ui.button(au_core::i18n::tr_ui("OK")).clicked() {
                close = true;
            }
        });
        if close {
            self.dropout_warning = false;
            if self.dropout_dont_show {
                self.prefs.set("/Warnings/DropoutDetected", au_core::prefs::PrefValue::Bool(false));
                self.save_prefs();
                self.settings.dropout_detected = false;
            }
        }
    }

    /// Pushes typed settings into the running app (called from apply_prefs).
    fn apply_settings(&mut self) {
        let s = self.settings.clone();
        if let Ok(mut g) = crate::fx_ui::REALTIME_GROUP_BY.write() {
            *g = s.realtime_group_by.clone();
        }
        // Recording: punch-and-roll crossfade, sound-activated recording level, latency correction
        // (Audacity's negative correction shifts the take earlier = drop frames at the start).
        self.record_crossfade_ms = s.roll_crossfade_ms;
        self.sound_threshold_db = s.silence_level_db as f32;
        if s.sound_activated_record {
            self.record_trigger = RecordTriggerUi::SoundActivated;
        } else if matches!(self.record_trigger, RecordTriggerUi::SoundActivated) {
            self.record_trigger = RecordTriggerUi::Immediate;
        }
        self.record_latency_ms = -s.latency_correction_ms;
        // Recording > Punch and Roll: pre-roll seconds (/AudioIO/PreRoll).
        self.record_preroll_secs = s.pre_roll;
        // Quality > High-quality conversion dither: used when writing into 16/24-bit tracks.
        crate::fx_bridge::set_hq_dither(&s.hq_dither);
        // Interface > Theme: only when the preference changes (theme.cfg remains the theme menu's store).
        match &self.applied_theme_pref {
            None => self.applied_theme_pref = Some(s.theme.clone()),
            Some(old) if *old != s.theme => {
                if let Some(id) = crate::theme::ThemeId::from_key(&s.theme) {
                    if id != crate::theme::ThemeId::Custom {
                        self.theme = crate::theme::Theme::builtin(id);
                        self.theme_applied = false;
                        let _ = crate::theme::save_choice(&self.theme);
                    }
                }
                self.applied_theme_pref = Some(s.theme.clone());
            }
            Some(_) => {}
        }
    }

    fn save_prefs(&mut self) {
        if let Some(dir) = prefs_path().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = self.prefs.save(prefs_path()) {
            self.status = au_core::i18n::tr_format("Could not save preferences: %s", &[&e]);
        }
    }

    /// Applies preferences to the running app: language, shortcuts, default project rate.
    fn apply_prefs(&mut self) {
        let lang = self.prefs.language();
        let code = match lang.as_str() {
            "" | "English" | "en" => "en".to_owned(),
            other => other.to_owned(),
        };
        if let Err(e) = au_core::i18n::set_language(&code, &au_core::i18n::default_search_dirs()) {
            self.status = e;
        }
        self.keymap = crate::shortcuts::KeyMap::load_from_prefs(&self.prefs);
        self.settings = crate::settings::AppSettings::from_prefs(&self.prefs);
        self.apply_settings();
        // Quality > Default Sample Rate for a still-empty project (Audacity's ProjectRate default).
        let rate = self.prefs.quality_default_rate();
        let empty = au_core::ProjectState { sample_rate: self.editor.state().sample_rate, ..Default::default() };
        if *self.editor.state() == empty && rate != empty.sample_rate {
            self.editor = au_core::Editor::new(au_core::ProjectState { sample_rate: rate, ..Default::default() });
        }
    }

    fn show_preferences(&mut self, ctx: &egui::Context) {
        if !self.prefs_dialog.open {
            return;
        }
        let devices = crate::prefs_dialog::DeviceLists::default();
        let keymap = &mut self.keymap;
        let filter = &mut self.keys_filter;
        let xml_path = &mut self.keys_xml_path;
        let mut keys_status = String::new();
        let result = self.prefs_dialog.show(ctx, &devices, &mut |ui: &mut egui::Ui| {
            keyboard_page(ui, keymap, filter, xml_path, &mut keys_status);
        });
        if !keys_status.is_empty() {
            self.status = keys_status;
        }
        match result {
            Some(Some(mut new_prefs)) => {
                self.keymap.save_to_prefs(&mut new_prefs);
                let changed = crate::prefs_dialog::changed_keys(&self.prefs, &new_prefs).len();
                self.prefs = new_prefs;
                self.status = au_core::i18n::tr_format("%d preference(s) changed", &[&changed]);
                if let Some(dir) = prefs_path().parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = self.prefs.save(prefs_path()) {
                    self.status = au_core::i18n::tr_format("Could not save preferences: %s", &[&e]);
                }
                self.apply_prefs();
            }
            Some(None) => self.keymap = crate::shortcuts::KeyMap::load_from_prefs(&self.prefs),
            None => {}
        }
    }
}

/// Preferences > Keyboard (KeyConfigPrefs): filterable list, set/clear, Import/Export XML, Defaults.
fn keyboard_page(
    ui: &mut egui::Ui,
    keymap: &mut crate::shortcuts::KeyMap,
    filter: &mut String,
    xml_path: &mut String,
    status: &mut String,
) {
    use au_core::i18n::tr_ui;
    ui.horizontal(|ui| {
        ui.label(tr_ui("Searc&h:").replace('&', ""));
        ui.text_edit_singleline(filter);
    });
    let rows: Vec<(String, String)> = keymap
        .iter()
        .map(|(id, k)| (id.to_owned(), k.map(|k| k.to_audacity_string()).unwrap_or_default()))
        .filter(|(id, k)| {
            filter.is_empty()
                || id.to_lowercase().contains(&filter.to_lowercase())
                || k.to_lowercase().contains(&filter.to_lowercase())
        })
        .collect();
    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
        egui::Grid::new("keys_grid").striped(true).show(ui, |ui| {
            for (id, key) in rows {
                ui.label(&id);
                let mut text = key.clone();
                if ui.text_edit_singleline(&mut text).lost_focus() && text != key {
                    if text.trim().is_empty() {
                        keymap.set(&id, None);
                    } else if let Some(combo) = crate::shortcuts::KeyCombo::parse(&text) {
                        let clash = keymap.conflicts(&combo, &id);
                        if !clash.is_empty() {
                            *status = au_core::i18n::tr_format("%s is already used by %s", &[&combo.to_audacity_string(), &clash.join(", ")]);
                        }
                        keymap.set(&id, Some(combo));
                    } else {
                        *status = au_core::i18n::tr_format("Not a valid key: %s", &[&text]);
                    }
                }
                ui.end_row();
            }
        });
    });
    ui.horizontal(|ui| {
        if ui.button(tr_ui("Clear")).clicked() {
            *filter = String::new();
        }
        ui.menu_button(tr_ui("Defaults"), |ui| {
            if ui.button(tr_ui("Standard")).clicked() {
                keymap.reset_standard();
                ui.close_menu();
            }
            if ui.button(tr_ui("Full")).clicked() {
                keymap.reset_full();
                ui.close_menu();
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label(au_core::i18n::tr_ui("XML:"));
        ui.text_edit_singleline(xml_path);
        if ui.button(tr_ui("Import...")).clicked() {
            match std::fs::read_to_string(&*xml_path).map_err(|e| e.to_string()).and_then(|t| keymap.import_xml(&t)) {
                Ok(n) => *status = au_core::i18n::tr_format("Loaded %d keyboard shortcuts", &[&n]),
                Err(e) => *status = au_core::i18n::tr_format("Import failed: %s", &[&e]),
            }
        }
        if ui.button(tr_ui("Export...")).clicked() {
            let ids: Vec<String> = keymap.iter().map(|(id, _)| id.to_owned()).collect();
            let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
            *status = match std::fs::write(&*xml_path, keymap.to_xml(&ids)) {
                Ok(()) => au_core::i18n::tr_format("Exported shortcuts to %s", &[&xml_path]),
                Err(e) => au_core::i18n::tr_format("Export failed: %s", &[&e]),
            };
        }
    });
}

/// Bridges an au_io progress callback (frames done/total) to a pool job's Progress and CancelToken.
fn io_progress<'a>(ctx: &'a au_runtime::pool::JobCtx, last: &'a mut u64) -> impl FnMut(u64, u64) -> bool + 'a {
    move |done, total| {
        if total > 0 && ctx.progress.total() != total {
            ctx.progress.set_total(total);
        }
        if done > *last {
            ctx.progress.add(done - *last);
            *last = done;
        }
        !ctx.cancel.is_cancelled()
    }
}

fn io_job_error(error: au_io::IoError) -> au_runtime::JobError {
    match error {
        au_io::IoError::Cancelled => au_runtime::JobError::Cancelled,
        error => au_runtime::JobError::Failed(error.to_string()),
    }
}

/// Renders an effects menu tree; returns the clicked effect id.
pub(crate) fn show_effect_nodes(ui: &mut egui::Ui, nodes: &[crate::settings::MenuNode]) -> Option<String> {
    let mut clicked = None;
    for node in nodes {
        match node {
            crate::settings::MenuNode::Item { id, label } => {
                if ui.button(format!("{}…", au_core::i18n::tr_ui(label))).clicked() {
                    clicked = Some(id.clone());
                }
            }
            crate::settings::MenuNode::Submenu { label, items } => {
                ui.menu_button(au_core::i18n::tr_ui(label), |ui| {
                    if let Some(id) = show_effect_nodes(ui, items) {
                        clicked = Some(id);
                    }
                });
            }
            crate::settings::MenuNode::Separator => {
                ui.separator();
            }
        }
    }
    clicked
}

/// Local (y, m, d, h, mi, s) for recording track names.
/// Draws the Extra menu's submenus and items; returns the command id clicked.
fn show_extra_menu(ui: &mut egui::Ui, items: &'static [crate::extra_menu::ExtraItem]) -> Option<&'static str> {
    let mut clicked = None;
    let mut i = 0;
    while i < items.len() {
        let it = items[i];
        if it.label == "----" {
            ui.separator();
            i += 1;
        } else if it.id.is_empty() {
            let end = items[i + 1..].iter().position(|x| x.depth <= it.depth).map_or(items.len(), |p| i + 1 + p);
            let children = &items[i + 1..end];
            ui.menu_button(au_core::i18n::tr_ui(it.label), |ui| {
                for c in children {
                    if ui.button(au_core::i18n::tr_ui(c.label)).clicked() {
                        clicked = Some(c.id);
                    }
                }
            });
            i = end;
        } else {
            if ui.button(au_core::i18n::tr_ui(it.label)).clicked() {
                clicked = Some(it.id);
            }
            i += 1;
        }
    }
    clicked
}

/// Directories > Temporary files directory, or our default (Audacity: <data dir>/SessionData).
fn session_dir(pref: &str) -> PathBuf {
    if !pref.trim().is_empty() {
        return PathBuf::from(pref);
    }
    // A private preferences file (tests, portable runs) keeps its session data beside it.
    if let Some(p) = std::env::var_os("AU_PREFS_PATH") {
        if let Some(dir) = std::path::Path::new(&p).parent() {
            return dir.join("SessionData");
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("audacity-rs/SessionData")
}

/// FileNames::CreateUniqueName("New Project", "aup3unsaved").
fn unsaved_project_name(now: (i32, u32, u32, u32, u32, u32)) -> String {
    static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    format!(
        "New Project {:04}-{:02}-{:02} {:02}-{:02}-{:02} N-{n}.aup3unsaved",
        now.0, now.1, now.2, now.3, now.4, now.5
    )
}

fn is_unsaved_session(p: &std::path::Path) -> bool {
    p.extension().is_some_and(|e| e == "aup3unsaved")
}

/// Unsaved-project journals left by a crash, for the recovery dialog on launch.
fn leftover_sessions(pref: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(session_dir(pref)) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "aup3unsaved"))
        .collect();
    v.sort();
    v
}

fn local_now() -> (i32, u32, u32, u32, u32, u32) {
    let out = std::process::Command::new("date").arg("+%Y %m %d %H %M %S").output().ok();
    let v: Vec<u32> = out
        .map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_default();
    if v.len() == 6 {
        (v[0] as i32, v[1], v[2], v[3], v[4], v[5])
    } else {
        (1970, 1, 1, 0, 0, 0)
    }
}

/// Generate / Effect / Analyze menu layout. Analyze starts with Audacity's non-effect commands
/// (Contrast…, Plot Spectrum…) and a separator, then the grouped analyzers (MenuHelper).
fn effects_menu_nodes(kind: EffectKind, group_by: &str) -> Vec<crate::settings::MenuNode> {
    use crate::settings::MenuNode;
    const COMMANDS: [&str; 2] = ["Contrast", "PlotSpectrum"];
    let all = au_effects::registry::all();
    let entries: Vec<crate::settings::EffectEntry> = all
        .iter()
        .filter(|e| e.kind() == kind && !COMMANDS.contains(&e.id().as_ref()))
        .map(|e| {
            let mut entry = crate::settings::entry_for(&e.id(), &e.name());
            entry.effect_type = format!("{:?}", kind);
            entry
        })
        .collect();
    let mut nodes = Vec::new();
    if kind == EffectKind::Analyze {
        for id in COMMANDS {
            if let Some(e) = all.iter().find(|e| e.id() == id) {
                nodes.push(MenuNode::Item { id: id.to_owned(), label: e.name().to_string() });
            }
        }
        nodes.push(MenuNode::Separator);
    }
    nodes.extend(crate::settings::effect_menu(&entries, group_by));
    nodes
}

/// Per-track waveform view state (Audacity's ChannelView height/minimized + WaveformScale).
#[derive(Clone, Copy, Debug, PartialEq)]
struct WaveUi {
    height: f32,
    minimized: bool,
    db: bool,
    zoom_min: f32,
    zoom_max: f32,
}

impl WaveUi {
    const DEFAULT_HEIGHT: f32 = 116.0;
    const MINIMIZED_HEIGHT: f32 = 44.0;
    const MIN_HEIGHT: f32 = 44.0;
}
