// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

use au_core::{AudioBuffer, Editor, Selection, Track};
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

struct AudacityApp {
    editor: Editor,
    selected_track: Option<usize>,
    selection: Option<Selection>,
    player: Option<Player>,
    recorder: Option<Recorder>,
    status: String,
    project_path: Option<PathBuf>,
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
                    .map_or(0, |track| track.buffer.len());
                Selection {
                    track,
                    start: 0,
                    end: len,
                }
            })
            .normalized()
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
                    self.project_path = Some(path);
                    self.status = "Project opened".to_owned();
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
        let filter = if flac { "FLAC audio" } else { "WAV audio" };
        let extension = if flac { "flac" } else { "wav" };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(filter, &[extension])
            .save_file()
        {
            let result = if flac {
                au_io::export_flac(&path, &track.buffer, 16)
            } else {
                au_io::export_wav(&path, &track.buffer, 16)
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
        match Player::play(Arc::new(track.buffer.clone()), start) {
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
        { let result = self.editor
                .apply(selection, description, effect); self.report(result); }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (cut, copy, paste, undo, redo, delete, space) = ctx.input(|input| {
            (
                input.modifiers.ctrl && input.key_pressed(egui::Key::X),
                input.modifiers.ctrl && input.key_pressed(egui::Key::C),
                input.modifiers.ctrl && input.key_pressed(egui::Key::V),
                input.modifiers.ctrl
                    && !input.modifiers.shift
                    && input.key_pressed(egui::Key::Z),
                (input.modifiers.ctrl
                    && input.modifiers.shift
                    && input.key_pressed(egui::Key::Z))
                    || (input.modifiers.ctrl && input.key_pressed(egui::Key::Y)),
                input.key_pressed(egui::Key::Delete),
                input.key_pressed(egui::Key::Space),
            )
        });

        if let Some(index) = self.selected_track {
            let selection = self.selection_for_track(index);
            if cut {
                self.edit_selection(|editor| editor.cut(selection));
            } else if copy {
                { let result = self.editor.copy(selection); self.report(result); }
            } else if delete {
                self.edit_selection(|editor| editor.delete(selection));
            } else if paste {
                let at = selection.start;
                self.edit_selection(|editor| editor.paste(index, at));
            }
        }
        if undo {
            { let result = self.editor.undo(); self.report(result); }
        } else if redo {
            { let result = self.editor.redo(); self.report(result); }
        }
        if space {
            if self.player.is_some() {
                self.stop_playback();
            } else {
                self.play();
            }
        }
    }

    fn draw_track(&mut self, ui: &mut egui::Ui, index: usize) {
        let track = &self.editor.state().tracks[index];
        let buffer = &track.buffer;
        let frames = buffer.len();
        let channels = buffer.n_channels().max(1);
        let height = 112.0_f32.max(channels as f32 * 42.0);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, egui::Color32::from_gray(34));
        painter.text(
            rect.left_top() + egui::vec2(8.0, 6.0),
            egui::Align2::LEFT_TOP,
            format!("{}  ·  {} ch  ·  {:.2}s", track.name, channels, buffer.duration_secs()),
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );

        if frames > 0 {
            let waveform_top = rect.top() + 25.0;
            let waveform_height = rect.height() - 30.0;
            let columns = rect.width().max(1.0) as usize;
            for channel in 0..buffer.n_channels() {
                let samples = &buffer.channels[channel];
                let channel_top = waveform_top + channel as f32 * waveform_height / channels as f32;
                let channel_height = waveform_height / channels as f32;
                let center = channel_top + channel_height * 0.5;
                painter.line_segment(
                    [
                        egui::pos2(rect.left(), center),
                        egui::pos2(rect.right(), center),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::from_gray(75)),
                );
                for column in 0..columns {
                    let start = column * frames / columns;
                    let end = ((column + 1) * frames / columns).max(start + 1).min(frames);
                    if start >= end {
                        continue;
                    }
                    let mut low = 1.0_f32;
                    let mut high = -1.0_f32;
                    for &sample in &samples[start..end.min(samples.len())] {
                        low = low.min(sample);
                        high = high.max(sample);
                    }
                    let x = rect.left() + column as f32;
                    let scale = channel_height * 0.44;
                    painter.line_segment(
                        [
                            egui::pos2(x, center - high * scale),
                            egui::pos2(x, center - low * scale),
                        ],
                        egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 200, 135)),
                    );
                }
            }
        }

        if let Some(selection) = self.selection.filter(|selection| selection.track == index) {
            if frames > 0 {
                let left = rect.left() + rect.width() * selection.start.min(frames) as f32 / frames as f32;
                let right = rect.left() + rect.width() * selection.end.min(frames) as f32 / frames as f32;
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(left, rect.top()),
                        egui::pos2(right.max(left + 1.0), rect.bottom()),
                    ),
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(60, 130, 230, 50),
                );
            }
        }

        if let Some(player) = self.player.as_ref() {
            if self.selected_track == Some(index) && frames > 0 {
                let x = rect.left() + rect.width() * player.position().min(frames) as f32 / frames as f32;
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(2.0, egui::Color32::YELLOW),
                );
            }
        }

        if response.clicked() {
            self.selected_track = Some(index);
            self.selection = Some(Selection {
                track: index,
                start: 0,
                end: 0,
            });
        }
        if response.drag_started() || response.dragged() {
            if let Some(pointer) = response.interact_pointer_pos() {
                self.selected_track = Some(index);
                let fraction = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                let frame = (fraction * frames as f32) as usize;
                let start = if response.drag_started() {
                    frame
                } else {
                    self.selection
                        .filter(|selection| selection.track == index)
                        .map_or(frame, |selection| selection.start)
                };
                self.selection = Some(Selection {
                    track: index,
                    start,
                    end: frame,
                });
            }
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
                { let result = self.editor.copy(self.selection_for_track(index)); self.report(result); }
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
            { let result = self.editor.undo(); self.report(result); }
        }
        if ui.button("Redo").clicked() {
            { let result = self.editor.redo(); self.report(result); }
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
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let count = self.editor.state().tracks.len();
                for index in 0..count {
                    self.draw_track(ui, index);
                    ui.add_space(6.0);
                }
                if count == 0 {
                    ui.centered_and_justified(|ui| {
                        ui.label("Open or import audio to begin.");
                    });
                }
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            let selection_text = self
                .selection
                .filter(|selection| {
                    self.editor.state().tracks.get(selection.track).is_some()
                })
                .map(|selection| {
                    let rate = self.editor.state().sample_rate;
                    let (start, end) = if rate == 0 {
                        (0.0, 0.0)
                    } else {
                        (
                            au_core::frames_to_secs(selection.start, rate),
                            au_core::frames_to_secs(selection.end, rate),
                        )
                    };
                    format!("Selection: {start:.3}s – {end:.3}s")
                })
                .unwrap_or_else(|| "No selection".to_owned());
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.separator();
                ui.label(selection_text);
                if self.player.is_some() {
                    ui.separator();
                    ui.label("Playhead active");
                }
            });
        });

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