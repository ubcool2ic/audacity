// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

use au_core::{
    timeline::{LabelTrack, SnapMode, Snapper, WaveClips, ZoomInfo},
    AudioBuffer, Editor, Selection, Track,
};
use au_effects::{Amplify, FadeIn, FadeOut, Normalize, Reverse};
use au_io::device::{Player, Recorder};
use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;

// egui/eframe desktop UI (feature `gui`).

pub fn run() -> eframe::Result<()> {
    eframe::run_native(
        "Audacity-RS",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(AudacityApp::default()))),
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
}

struct AudacityApp {
    editor: Editor,
    selected_track: Option<usize>,
    selection: Option<Selection>,
    player: Option<Player>,
    recorder: Option<Recorder>,
    status: String,
    project_path: Option<PathBuf>,
    tool: Tool,
    zoom: ZoomInfo,
    snapper: Snapper,
    sync_lock: bool,
    drag: Option<PointerDrag>,
    selected_label: Option<(usize, usize)>,
    label_edit: Option<LabelEdit>,
    speed_window: bool,
    speed_percent: f64,
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
            status: "Ready".to_owned(),
            project_path: None,
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
            selected_label: None,
            label_edit: None,
            speed_window: false,
            speed_percent: 100.0,
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
                    self.status = "Imported audio".to_owned();
                }
                Err(error) => self.status = format!("Import failed: {error:?}"),
            }
        }
    }

    fn save_project(&mut self) {
        let path = self.project_path.clone().or_else(|| {
            rfd::FileDialog::new()
                .add_filter("Audacity-RS project", &["aurs"])
                .save_file()
        });
        if let Some(path) = path {
            match au_project::save_project(&path, self.editor.state()) {
                Ok(()) => {
                    self.project_path = Some(path);
                    self.status = "Project saved".to_owned();
                }
                Err(error) => self.status = format!("Save failed: {error:?}"),
            }
        }
    }

    fn open_project(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Audacity-RS project", &["aurs"])
            .pick_file()
        {
            match au_project::load_project(&path) {
                Ok(state) => {
                    self.editor = Editor::new(state);
                    self.selected_track = if self.editor.state().tracks.is_empty() {
                        None
                    } else {
                        Some(0)
                    };
                    self.selection = None;
                    self.selected_label = None;
                    self.project_path = Some(path);
                    self.sync_lock = self.editor.state().sync_lock;
                    self.status = "Project opened".to_owned();
                    self.fit_zoom(1000.0);
                }
                Err(error) => self.status = format!("Open failed: {error:?}"),
            }
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
        let buffer = track.render();
        let filter = if flac { "FLAC audio" } else { "WAV audio" };
        let extension = if flac { "flac" } else { "wav" };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(filter, &[extension])
            .save_file()
        {
            let result = if flac {
                au_io::export_flac(&path, &buffer, 16)
            } else {
                au_io::export_wav(&path, &buffer, 16)
            };
            self.status = match result {
                Ok(()) => "Audio exported".to_owned(),
                Err(error) => format!("Export failed: {error:?}"),
            };
        }
    }

    fn play(&mut self) {
        let Some(index) = self.selected_track else {
            self.status = "Select a track to play".to_owned();
            return;
        };
        let Some(track) = self.editor.state().tracks.get(index) else {
            self.status = "Selected track not found".to_owned();
            return;
        };
        let start = self.selection_for_track(index).start;
        match Player::play(Arc::new(track.render()), start) {
            Ok(player) => {
                self.player = Some(player);
                self.status = "Playing".to_owned();
            }
            Err(error) => self.status = format!("Playback failed: {error:?}"),
        }
    }

    fn stop_playback(&mut self) {
        if let Some(player) = self.player.take() {
            player.stop();
        }
        self.status = "Stopped".to_owned();
    }

    fn start_recording(&mut self) {
        match Recorder::start(2) {
            Ok(recorder) => {
                self.recorder = Some(recorder);
                self.status = "Recording".to_owned();
            }
            Err(error) => self.status = format!("Recording failed: {error:?}"),
        }
    }

    fn stop_recording(&mut self) {
        if let Some(recorder) = self.recorder.take() {
            let buffer = recorder.stop();
            self.editor.add_track(Track::new("Recording", buffer));
            self.selected_track = Some(self.editor.state().tracks.len() - 1);
            self.status = "Recording added as a track".to_owned();
        }
    }

    fn edit_selection(&mut self, operation: impl FnOnce(&mut Editor) -> au_core::Result<()>) {
        let result = operation(&mut self.editor);
        self.report(result);
    }

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
            if self.player.is_some() {
                self.stop_playback();
            } else {
                self.play();
            }
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

        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(width, 116.0),
            egui::Sense::click_and_drag(),
        );
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
                    painter.rect_filled(
                        clip_rect,
                        4.0,
                        egui::Color32::from_rgb(43, 72, 59),
                    );
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            clip_rect.left_top(),
                            egui::pos2(clip_rect.right(), (clip_rect.top() + 19.0).min(clip_rect.bottom())),
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
                                let local_start =
                                    (local_time_start.max(0.0) / clip_len_secs.max(f64::EPSILON)
                                        * rendered_len as f64)
                                        .floor() as usize;
                                let local_end =
                                    (local_time_end.max(0.0) / clip_len_secs.max(f64::EPSILON)
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
                            egui::pos2(
                                rect.left() + zoom.time_to_x(t0) as f32,
                                value_y(v0),
                            ),
                            egui::pos2(
                                rect.left() + zoom.time_to_x(t1) as f32,
                                value_y(v1),
                            ),
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
                }
            }
        }

        if response.dragged() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let time = self.time_at_x(pointer.x, rect);
                match self.drag {
                    Some(PointerDrag::Selection {
                        track,
                        start,
                        ..
                    }) if track == index => {
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
                        let snapped = self.snap_time(requested, rate, Some(&self.editor.state().tracks[index].clips));
                        self.drag = Some(PointerDrag::Clip {
                            track,
                            clip,
                            original_start,
                            pointer_start,
                            current_start: snapped.max(0.0),
                        });
                    }
                    Some(PointerDrag::Envelope {
                        track,
                        point,
                        ..
                    }) if track == index => {
                        let body_top = rect.top() + 23.0;
                        let body_height = (rect.bottom() - body_top).max(1.0);
                        let value = (1.0
                            - (pointer.y - body_top) as f64 / body_height as f64)
                            * 2.0;
                        self.drag = Some(PointerDrag::Envelope {
                            track,
                            point,
                            time,
                            value: value.clamp(0.0, 2.0),
                        });
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
                        let value = (1.0
                            - (pointer.y - body_top) as f64 / body_height as f64)
                            * 2.0;
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
                Tool::TimeShift => {}
            }
        }

        if self.tool == Tool::Envelope
            && (response.secondary_clicked()
                || (response.clicked()
                    && ui.input(|input| input.modifiers.shift)))
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
                labels.iter().enumerate().find_map(|(label_index, (start, end, _))| {
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

    fn finish_pointer_drag(&mut self, ctx: &egui::Context) {
        let released = ctx.input(|input| {
            input
                .pointer
                .button_released(egui::PointerButton::Primary)
        });
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
                let end = self
                    .snap_time(current_end, rate, None)
                    .max(start);
                let result = self.editor.set_label_times(track, label, start, end);
                self.report(result);
            }
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
        if ui.button("Stop").clicked() {
            self.stop_playback();
        }
        if ui.button("Record").clicked() {
            self.start_recording();
        }
        if ui.button("Stop Record").clicked() {
            self.stop_recording();
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

        if self.label_edit.is_some() {
            let mut open = true;
            let mut apply = false;
            egui::Window::new("Edit Label")
                .open(&mut open)
                .show(ctx, |ui| {
                    if let Some(edit) = self.label_edit.as_mut() {
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut edit.text)
                                .hint_text("Label text"),
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
}

impl eframe::App for AudacityApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ctx);
        if self.player.as_ref().is_some_and(Player::is_finished) {
            self.player = None;
        }

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Import audio…").clicked() {
                        ui.close_menu();
                        self.import_file();
                    }
                    if ui.button("Open project…").clicked() {
                        ui.close_menu();
                        self.open_project();
                    }
                    if ui.button("Save project…").clicked() {
                        ui.close_menu();
                        self.save_project();
                    }
                    if ui.button("Export WAV…").clicked() {
                        ui.close_menu();
                        self.export(false);
                    }
                    if ui.button("Export FLAC…").clicked() {
                        ui.close_menu();
                        self.export(true);
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
                ui.menu_button("Effect", |ui| {
                    if ui.button("Amplify +3 dB").clicked() {
                        ui.close_menu();
                        self.apply_effect("Amplify +3 dB", |buffer| {
                            Amplify::new(3.0).process(buffer);
                        });
                    }
                    if ui.button("Amplify -3 dB").clicked() {
                        ui.close_menu();
                        self.apply_effect("Amplify -3 dB", |buffer| {
                            Amplify::new(-3.0).process(buffer);
                        });
                    }
                    if ui.button("Normalize -1 dB").clicked() {
                        ui.close_menu();
                        self.apply_effect("Normalize", |buffer| {
                            Normalize::new(-1.0, false).process(buffer);
                        });
                    }
                    if ui.button("Fade In").clicked() {
                        ui.close_menu();
                        self.apply_effect("Fade In", |buffer| FadeIn.process(buffer));
                    }
                    if ui.button("Fade Out").clicked() {
                        ui.close_menu();
                        self.apply_effect("Fade Out", |buffer| FadeOut.process(buffer));
                    }
                    if ui.button("Reverse").clicked() {
                        ui.close_menu();
                        self.apply_effect("Reverse", |buffer| Reverse.process(buffer));
                    }
                });
                self.toolbar(ui);
            });

            ui.horizontal(|ui| {
                ui.label("Tool:");
                ui.selectable_value(&mut self.tool, Tool::Selection, "Selection");
                ui.selectable_value(&mut self.tool, Tool::Envelope, "Envelope");
                ui.selectable_value(&mut self.tool, Tool::TimeShift, "Time Shift");
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
                        ui.selectable_value(
                            &mut self.snapper.mode,
                            SnapMode::Samples,
                            "Samples",
                        );
                        ui.selectable_value(
                            &mut self.snapper.mode,
                            SnapMode::ZeroCrossing,
                            "Zero crossings",
                        );
                        ui.selectable_value(
                            &mut self.snapper.mode,
                            SnapMode::Labels,
                            "Labels",
                        );
                    });
                if ui.checkbox(&mut self.sync_lock, "Sync-Lock").changed() {
                    self.editor.set_sync_lock(self.sync_lock);
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
            let selection_text = self
                .selection
                .filter(|selection| {
                    self.editor.state().tracks.get(selection.track).is_some()
                })
                .map(|selection| {
                    let rate = self
                        .editor
                        .state()
                        .tracks
                        .get(selection.track)
                        .map_or(self.editor.state().sample_rate.max(1), Track::sample_rate);
                    let start = au_core::frames_to_secs(selection.start, rate);
                    let end = au_core::frames_to_secs(selection.end, rate);
                    format!("Selection: {start:.3}s – {end:.3}s")
                })
                .unwrap_or_else(|| "No selection".to_owned());
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.separator();
                ui.label(selection_text);
                if let Some((track, label)) = self.selected_label {
                    ui.separator();
                    ui.label(format!("Label {}:{}", track + 1, label + 1));
                }
                if self.player.is_some() {
                    ui.separator();
                    ui.label("Playhead active");
                }
            });
        });

        self.draw_windows(ctx);

        if self.player.is_some() {
            ctx.request_repaint();
        }
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
