use super::AudacityApp;
use super::extra_logic::{self as xl, SelOp, SeekCtx, MoveDir};
use au_core::Selection;
use eframe::egui;

pub(crate) struct ExtraState {
    pub last_label: i32,
    pub last_seek: Option<std::time::Instant>,
    pub track_sel: Vec<bool>,
    pub panel_width: f32,
    pub frame_focus_tracks: bool,
    pub dialog: Option<ExtraDialog>,
}

pub(crate) enum ExtraDialog {
    Pan {
        track: usize,
        value: f32,
        orig: f32,
    },
    Gain {
        track: usize,
        db: f32,
        orig: f32,
    },
    TrackMenu {
        track: usize,
    },
}

impl Default for ExtraState {
    fn default() -> Self {
        Self {
            last_label: -1,
            last_seek: None,
            track_sel: Vec::new(),
            panel_width: 0.0,
            frame_focus_tracks: false,
            dialog: None,
        }
    }
}

impl AudacityApp {
    fn play_pos_secs(&self) -> Option<f64> {
        self.player
            .as_ref()
            .map(|player| player.position() as f64 / self.project_rate().max(1) as f64)
    }

    fn end_secs(&self) -> f64 {
        self.editor
            .state()
            .tracks
            .iter()
            .map(|track| track.len() as f64 / track.sample_rate().max(1) as f64)
            .fold(0.0, f64::max)
    }

    fn set_time_selection(&mut self, t0: f64, t1: f64) {
        let (t0, t1) = (t0.min(t1).max(0.0), t0.max(t1).max(0.0));
        let n = self.editor.state().tracks.len();
        if n == 0 {
            return;
        }

        let track = self
            .selected_track
            .or(self.focused_track)
            .unwrap_or(0)
            .min(n - 1);
        let rate = self.editor.state().tracks[track].sample_rate().max(1) as f64;
        self.selected_track = Some(track);
        self.selection = Some(Selection {
            track,
            start: (t0 * rate).round() as usize,
            end: (t1 * rate).round() as usize,
        });
    }

    fn seek_ctx(&self) -> SeekCtx {
        let (t0, t1) = self.selection_secs();
        let snap = match self.snapper.mode {
            au_core::timeline::SnapMode::Grid { secs } => Some(secs),
            _ => None,
        };
        SeekCtx {
            t0,
            t1,
            end: self.end_secs(),
            pixels_per_sec: self.zoom.pixels_per_sec.max(f64::EPSILON),
            h_offset: self.zoom.h_offset,
            snap,
        }
    }

    fn sync_track_sel(&mut self) {
        let n = self.editor.state().tracks.len();
        self.extra.track_sel.resize(n, false);
        if let Some(i) = self.selected_track.filter(|&i| i < n) {
            self.extra.track_sel[i] = true;
        }
    }

    fn step_mult(&mut self) -> f64 {
        let now = std::time::Instant::now();
        let elapsed = self
            .extra
            .last_seek
            .map(|last| now.duration_since(last).as_millis());
        self.extra.last_seek = Some(now);
        xl::fast_multiplier(elapsed)
    }

    fn scroll_into_view(&mut self, t: f64) {
        let pps = self.zoom.pixels_per_sec;
        if pps <= 0.0 {
            return;
        }
        let visible = self.extra.panel_width.max(1000.0) as f64 / pps;
        if t < self.zoom.h_offset || t > self.zoom.h_offset + visible {
            self.zoom.h_offset = (t - visible / 2.0).max(0.0);
        }
    }

    fn apply_track_sel(&mut self) {
        let (t0, t1) = self.selection_secs();
        let focused_selected = self
            .focused_track
            .filter(|&i| i < self.extra.track_sel.len())
            .is_some_and(|i| self.extra.track_sel[i]);

        if focused_selected {
            self.selected_track = self.focused_track;
        } else if !self
            .selected_track
            .filter(|&i| i < self.extra.track_sel.len())
            .is_some_and(|i| self.extra.track_sel[i])
        {
            self.selected_track = self.extra.track_sel.iter().position(|&selected| selected);
            if self.selected_track.is_none() {
                self.selection = None;
            }
        }

        if self.selected_track.is_some() {
            self.set_time_selection(t0, t1);
        }
    }

    fn track_command_index(&self) -> Option<usize> {
        self.focused_track.or(self.selected_track).filter(|&i| i < self.editor.state().tracks.len())
    }

    pub(super) fn execute_extra_command(
        &mut self,
        command: &str,
        params: &[(String, String)],
    ) -> Option<(String, bool)> {
        let ok = || Some((String::new(), true));
        let param = |name: &str| {
            params
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        };

        match command {
            "SelSetExtRight" => {
                let ctx = self.seek_ctx();
                let (t0, t1) = xl::boundary_move(
                    ctx.t0,
                    ctx.t1,
                    ctx.end,
                    ctx.pixels_per_sec,
                    self.zoom.h_offset,
                    if self.player.is_some() { 1.0 } else { self.step_mult() },
                    self.play_pos_secs(),
                );
                self.set_time_selection(t0, t1);
                ok()
            }
            "SelCntrLeft" | "SelCntrRight" | "CursorLeft" | "CursorRight" => {
                let is_contract = command.starts_with("SelCntr");
                let direction = match command {
                    "SelCntrLeft" => 1.0,
                    "SelCntrRight" | "CursorRight" => 1.0,
                    "CursorLeft" => -1.0,
                    _ => 1.0,
                };
                let direction = if command == "SelCntrRight" { -1.0 } else { direction };
                if self.player.is_some() {
                    let current = self.play_pos_secs().unwrap_or(0.0);
                    let seconds = if is_contract {
                        direction
                    } else {
                        direction
                    } * self.settings.seek_short_period;
                    self.seek_transport(
                        ((current + seconds).max(0.0) * self.project_rate() as f64).round() as usize,
                    );
                } else {
                    let mult = self.step_mult();
                    let ctx = self.seek_ctx();
                    let op = if is_contract {
                        SelOp::Contract
                    } else {
                        SelOp::CursorMove
                    };
                    let (t0, t1) = xl::seek_inactive_pixels(&ctx, direction * mult, op);
                    let target = if direction < 0.0 { t0 } else { t1 };
                    self.set_time_selection(t0, t1);
                    self.scroll_into_view(target);
                }
                ok()
            }
            "CursorShortJumpLeft" | "CursorShortJumpRight" | "CursorLongJumpLeft"
            | "CursorLongJumpRight" => {
                let left = command.ends_with("Left");
                let period = if command.contains("Short") {
                    self.settings.seek_short_period
                } else {
                    self.settings.seek_long_period
                };
                let direction = if left { -1.0 } else { 1.0 };
                if self.player.is_some() {
                    let current = self.play_pos_secs().unwrap_or(0.0);
                    self.seek_transport(
                        ((current + direction * period).max(0.0) * self.project_rate() as f64)
                            .round() as usize,
                    );
                } else {
                    let (t0, t1) = self.selection_secs();
                    let step = direction * period;
                    let (t0, t1) =
                        xl::seek_inactive_seconds(t0, t1, step);
                    let t0 = t0.max(0.0);
                    let t1 = t1.max(0.0);
                    self.set_time_selection(t0, t1);
                    self.scroll_into_view(if left { t0 } else { t1 });
                }
                ok()
            }
            "MoveToPrevLabel" | "MoveToNextLabel" => {
                let label_track = self.label_focus.unwrap_or(0);
                let Some(track) = self.editor.state().labels.get(label_track).cloned() else {
                    self.status = au_core::i18n::tr_ui("no label track");
                    return ok();
                };
                let starts: Vec<f64> = track.labels.iter().map(|label| label.start).collect();
                let (t0, _) = self.selection_secs();
                let i = if command == "MoveToNextLabel" {
                    xl::find_next_label(&starts, t0, &mut self.extra.last_label)
                } else {
                    xl::find_prev_label(&starts, t0, &mut self.extra.last_label)
                };
                if i < 0 {
                    self.status = au_core::i18n::tr_ui("no labels in label track");
                } else if let Some(label) = track.labels.get(i as usize) {
                    let (start, end, text) = (label.start, label.end, label.text.clone());
                    self.set_time_selection(start, end);
                    if self.player.is_some() {
                        self.seek_transport(
                            (start * self.project_rate() as f64).round() as usize,
                        );
                    }
                    self.status = au_core::i18n::tr_format(
                        "%s %d of %d",
                        &[
                            &text,
                            &(i + 1).to_string(),
                            &track.labels.len().to_string(),
                        ],
                    );
                    self.scroll_into_view(start);
                }
                ok()
            }
            "ExportMp3" | "ExportWav" | "ExportOgg" | "ExportFLAC" | "ExportSel" => {
                let mut dlg =
                    crate::export_glue::ExportAudioDialog::new(self.aup3_extras.tags.clone());
                let selection_only = command == "ExportSel";
                if selection_only {
                    dlg.chooser = crate::export_glue::FormatChooser::default();
                } else {
                    let name = match command {
                        "ExportMp3" => "MP3",
                        "ExportWav" => "WAV",
                        "ExportOgg" => "Ogg Vorbis",
                        _ => "FLAC",
                    };
                    dlg.chooser = crate::export_glue::FormatChooser::from_name(name);
                    super::apply_export_dither(&mut dlg.chooser.format, &self.prefs);
                }
                dlg.selection_only = selection_only;
                self.export_audio_dialog = Some(dlg);
                ok()
            }
            "PrevFrame" | "NextFrame" => {
                self.extra.frame_focus_tracks = !self.extra.frame_focus_tracks;
                if self.extra.frame_focus_tracks
                    && self.focused_track.is_none()
                    && !self.editor.state().tracks.is_empty()
                {
                    self.focused_track = Some(0);
                }
                self.status = au_core::i18n::tr_ui(if self.extra.frame_focus_tracks {
                    "Tracks"
                } else {
                    "Toolbars"
                });
                ok()
            }
            "PrevTrack" | "NextTrack" | "FirstTrack" | "LastTrack" => {
                let n = self.editor.state().tracks.len();
                self.focused_track = match command {
                    "PrevTrack" => xl::prev_track(
                        self.focused_track,
                        n,
                        self.settings.circular_track_navigation,
                    ),
                    "NextTrack" => xl::next_track(
                        self.focused_track,
                        n,
                        self.settings.circular_track_navigation,
                    ),
                    "FirstTrack" => (n > 0).then_some(0),
                    _ => n.checked_sub(1),
                };
                ok()
            }
            "ShiftUp" | "ShiftDown" => {
                self.sync_track_sel();
                let Some(focus) = self.focused_track else {
                    return ok();
                };
                let n = self.extra.track_sel.len();
                let target = if command == "ShiftUp" {
                    xl::prev_track(Some(focus), n, false)
                } else {
                    xl::next_track(Some(focus), n, false)
                };
                let Some(target) = target else {
                    return ok();
                };
                xl::shift_nav(&mut self.extra.track_sel, focus, target);
                self.focused_track = Some(target);
                self.apply_track_sel();
                ok()
            }
            "Toggle" | "ToggleAlt" => {
                self.sync_track_sel();
                let Some(focus) = self.focused_track else {
                    return ok();
                };
                if let Some(selected) = self.extra.track_sel.get_mut(focus) {
                    *selected = !*selected;
                }
                self.apply_track_sel();
                ok()
            }
            "TrackPanLeft" | "TrackPanRight" | "TrackGainInc" | "TrackGainDec"
            | "TrackPan" | "TrackGain" | "TrackMenu" | "TrackMute" | "TrackSolo"
            | "TrackClose" | "TrackMoveUp" | "TrackMoveDown" | "TrackMoveTop"
            | "TrackMoveBottom" => {
                let Some(i) = self.track_command_index() else {
                    return ok();
                };
                self.ensure_track_mix();
                match command {
                    "TrackPanLeft" | "TrackPanRight" => {
                        let mut p = self.track_mix[i];
                        p.pan = xl::pan_step(
                            p.pan,
                            if command == "TrackPanLeft" { -1.0 } else { 1.0 },
                        );
                        self.track_mix[i] = p;
                        self.send_track_mix(i, p, true);
                    }
                    "TrackGainInc" | "TrackGainDec" => {
                        let mut p = self.track_mix[i];
                        p.gain = xl::gain_step(
                            p.gain,
                            if command == "TrackGainInc" { 1.0 } else { -1.0 },
                        );
                        self.track_mix[i] = p;
                        self.send_track_mix(i, p, true);
                    }
                    "TrackPan" => {
                        let value = self.track_mix[i].pan;
                        self.extra.dialog = Some(ExtraDialog::Pan {
                            track: i,
                            value,
                            orig: value,
                        });
                    }
                    "TrackGain" => {
                        let gain = self.track_mix[i].gain;
                        let db = if gain <= 0.0 {
                            -36.0
                        } else {
                            (20.0 * gain.log10()).clamp(-36.0, 36.0)
                        };
                        self.extra.dialog = Some(ExtraDialog::Gain {
                            track: i,
                            db,
                            orig: db,
                        });
                    }
                    "TrackMenu" => {
                        self.extra.dialog = Some(ExtraDialog::TrackMenu { track: i });
                    }
                    "TrackMute" => {
                        let mut p = self.track_mix[i];
                        p.mute = !p.mute;
                        self.track_mix[i] = p;
                        self.send_track_mix(i, p, true);
                    }
                    "TrackSolo" => {
                        let mut solos: Vec<bool> =
                            self.track_mix.iter().map(|p| p.solo).collect();
                        crate::settings::apply_solo_click(
                            self.settings.solo,
                            &mut solos,
                            i,
                            false,
                        );
                        for (index, solo) in solos.into_iter().enumerate() {
                            if self.track_mix[index].solo != solo {
                                let mut p = self.track_mix[index];
                                p.solo = solo;
                                self.track_mix[index] = p;
                                self.send_track_mix(index, p, true);
                            }
                        }
                    }
                    "TrackClose" => {
                        if self.player.is_some() {
                            self.status =
                                au_core::i18n::tr_ui("Can't delete track with active audio");
                            return ok();
                        }
                        if let Err(error) = self.editor.remove_track(i) {
                            return Some((error.to_string(), false));
                        }
                        if i < self.track_mix.len() {
                            self.track_mix.remove(i);
                        }
                        if i < self.track_views.len() {
                            self.track_views.remove(i);
                        }
                        if i < self.wave_ui.len() {
                            self.wave_ui.remove(i);
                        }
                        if i < self.extra.track_sel.len() {
                            self.extra.track_sel.remove(i);
                        }
                        self.selected_track = self.selected_track.and_then(|index| {
                            if index == i {
                                None
                            } else {
                                Some(if index > i { index - 1 } else { index })
                            }
                        });
                        if let Some(selection) = &mut self.selection {
                            if selection.track == i {
                                self.selection = None;
                            } else if selection.track > i {
                                selection.track -= 1;
                            }
                        }
                        let n_after = self.editor.state().tracks.len();
                        self.focused_track =
                            (n_after > 0).then_some(i.min(n_after.saturating_sub(1)));
                    }
                    _ => {
                        let dir = match command {
                            "TrackMoveUp" => MoveDir::Up,
                            "TrackMoveDown" => MoveDir::Down,
                            "TrackMoveTop" => MoveDir::Top,
                            _ => MoveDir::Bottom,
                        };
                        let Some(to) =
                            xl::move_target(i, self.editor.state().tracks.len(), dir)
                        else {
                            return ok();
                        };
                        let mut state = self.editor.state().clone();
                        let name = state.tracks[i].name.clone();
                        xl::move_index(&mut state.tracks, i, to);
                        let direction = match dir {
                            MoveDir::Up => "up",
                            MoveDir::Down => "down",
                            MoveDir::Top => "to Top",
                            MoveDir::Bottom => "to Bottom",
                        };
                        self.editor.replace_state(
                            &au_core::i18n::tr_format(
                                "Moved '%s' %s",
                                &[&name, &direction],
                            ),
                            state,
                        );
                        xl::move_index(&mut self.track_mix, i, to);
                        xl::move_index(&mut self.track_views, i, to);
                        xl::move_index(&mut self.wave_ui, i, to);
                        xl::move_index(&mut self.extra.track_sel, i, to);
                        self.selected_track =
                            self.selected_track.map(|index| xl::moved_index(index, i, to));
                        if let Some(selection) = &mut self.selection {
                            selection.track = xl::moved_index(selection.track, i, to);
                        }
                        self.focused_track =
                            self.focused_track.map(|index| xl::moved_index(index, i, to));
                    }
                }
                ok()
            }
            "CompareAudio" => {
                self.sync_track_sel();
                let mut selected: Vec<usize> = self
                    .extra
                    .track_sel
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &selected)| selected.then_some(i))
                    .collect();
                if selected.len() < 2 && self.editor.state().tracks.len() == 2 {
                    selected = vec![0, 1];
                }
                if selected.len() != 2 {
                    return Some((
                        au_core::i18n::tr_ui("Please select two tracks."),
                        false,
                    ));
                }
                let tracks = &self.editor.state().tracks;
                let first_name = tracks[selected[0]].name.clone();
                let second_name = tracks[selected[1]].name.clone();
                let a = tracks[selected[0]].render();
                let b = tracks[selected[1]].render();
                let threshold = param("Threshold")
                    .and_then(|value| value.parse::<f32>().ok())
                    .unwrap_or(0.0);
                let (s0, s1) = if let Some(selection) = &self.selection {
                    (selection.start, selection.end)
                } else {
                    (0, tracks[selected[0]].len().min(tracks[selected[1]].len()))
                };
                let count = xl::compare_audio(&a.channels, &b.channels, s0, s1, threshold);
                let rate = tracks[selected[0]].sample_rate().max(1) as f64;
                let mut messages =
                    vec![format!("Comparing tracks '{}' and '{}'.", first_name, second_name)];
                messages.extend(xl::compare_audio_messages(count, rate, threshold));
                // Audacity 3.7.3 concatenates the Status messages without separators on the pipe.
                Some((messages.concat(), true))
            }
            "SetEnvelope" => {
                let Some(time) = param("Time").and_then(|value| value.parse::<f64>().ok()) else {
                    return Some(("Missing or invalid Time parameter".into(), false));
                };
                let index = self.selected_track.unwrap_or(0);
                if index >= self.editor.state().tracks.len() {
                    return Some((String::new(), false));
                }
                let delete = param("Delete")
                    .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
                let value = param("Value").and_then(|value| value.parse::<f64>().ok());
                let mut state = self.editor.state().clone();
                let envelope = &mut state.tracks[index].clips.envelope;
                if delete {
                    envelope.points.clear();
                } else if let Some(value) = value {
                    envelope.insert(time, value);
                }
                self.editor.replace_state("Set Envelope", state);
                ok()
            }
            "SetProject" => {
                if let Some(rate) = param("Rate").and_then(|value| value.parse::<u32>().ok()) {
                    let mut state = self.editor.state().clone();
                    state.sample_rate = rate;
                    self.editor.replace_state("Set Project", state);
                }
                if let Some(name) = param("Name") {
                    self.status = name.to_string();
                }
                ok()
            }
            _ => None,
        }
    }

    pub(super) fn show_extra_dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.extra.dialog.take() else {
            return;
        };
        let mut keep_open = true;

        match &mut dialog {
            ExtraDialog::Pan { track, value, orig } => {
                let mut commit = false;
                let mut cancel = false;
                egui::Window::new(au_core::i18n::tr_ui("Pan"))
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        let changed = ui
                            .add(egui::Slider::new(value, -1.0..=1.0).text(au_core::i18n::tr_ui("Pan")))
                            .changed();
                        if changed && *track < self.track_mix.len() {
                            let mut p = self.track_mix[*track];
                            p.pan = *value;
                            self.track_mix[*track] = p;
                            self.send_track_mix(*track, p, false);
                        }
                        ui.horizontal(|ui| {
                            commit = ui.button(au_core::i18n::tr_ui("OK")).clicked();
                            cancel = ui.button(au_core::i18n::tr_ui("Cancel")).clicked();
                        });
                    });
                if commit {
                    if *track < self.track_mix.len() {
                        self.send_track_mix(*track, self.track_mix[*track], true);
                    }
                    keep_open = false;
                } else if cancel {
                    if *track < self.track_mix.len() {
                        let mut p = self.track_mix[*track];
                        p.pan = *orig;
                        self.track_mix[*track] = p;
                        self.send_track_mix(*track, p, false);
                    }
                    keep_open = false;
                }
            }
            ExtraDialog::Gain { track, db, orig } => {
                let mut commit = false;
                let mut cancel = false;
                egui::Window::new(au_core::i18n::tr_ui("Gain"))
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        let changed = ui
                            .add(egui::Slider::new(db, -36.0..=36.0).text("dB"))
                            .changed();
                        if changed && *track < self.track_mix.len() {
                            let mut p = self.track_mix[*track];
                            p.gain = 10.0_f32.powf(*db / 20.0);
                            self.track_mix[*track] = p;
                            self.send_track_mix(*track, p, false);
                        }
                        ui.horizontal(|ui| {
                            commit = ui.button(au_core::i18n::tr_ui("OK")).clicked();
                            cancel = ui.button(au_core::i18n::tr_ui("Cancel")).clicked();
                        });
                    });
                if commit {
                    if *track < self.track_mix.len() {
                        self.send_track_mix(*track, self.track_mix[*track], true);
                    }
                    keep_open = false;
                } else if cancel {
                    if *track < self.track_mix.len() {
                        let mut p = self.track_mix[*track];
                        p.gain = 10.0_f32.powf(*orig / 20.0);
                        self.track_mix[*track] = p;
                        self.send_track_mix(*track, p, false);
                    }
                    keep_open = false;
                }
            }
            ExtraDialog::TrackMenu { track } => {
                let mut action: Option<&'static str> = None;
                egui::Window::new(au_core::i18n::tr_ui("Track"))
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        for (label, command) in [
                            ("Mute/Unmute", "TrackMute"),
                            ("Solo", "TrackSolo"),
                            ("Move Up", "TrackMoveUp"),
                            ("Move Down", "TrackMoveDown"),
                            ("Move to Top", "TrackMoveTop"),
                            ("Move to Bottom", "TrackMoveBottom"),
                            ("Delete Track", "TrackClose"),
                        ] {
                            if ui.button(au_core::i18n::tr_ui(label)).clicked() {
                                action = Some(command);
                            }
                        }
                        if ui.button(au_core::i18n::tr_ui("Close")).clicked() {
                            action = Some("");
                        }
                    });
                if let Some(command) = action {
                    self.focused_track = Some(*track);
                    if !command.is_empty() {
                        let _ = self.execute_extra_command(command, &[]);
                    }
                    keep_open = false;
                }
            }
        }

        if keep_open {
            self.extra.dialog = Some(dialog);
        }
    }
}
