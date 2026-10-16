use super::AudacityApp;
use super::extra_nav_logic::{self as xl, SelOp, SeekCtx};
use au_core::Selection;

pub(crate) struct ExtraState {
    pub last_label: i32,
    pub last_seek: Option<std::time::Instant>,
    pub track_sel: Vec<bool>,
    pub label_sel: Vec<bool>,
    pub panel_width: f32,
    pub frame_focus_tracks: bool,
}


impl Default for ExtraState {
    fn default() -> Self {
        Self {
            last_label: -1,
            last_seek: None,
            track_sel: Vec::new(),
            label_sel: Vec::new(),
            panel_width: 0.0,
            frame_focus_tracks: false,
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
            start: (t0 * rate).round() as i64,
            end: (t1 * rate).round() as i64,
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
            // Audacity: max(tracks end, ViewInfo::GetScreenEndTime()).
            end: self.end_secs().max(self.zoom.h_offset + self.extra.panel_width.max(1000.0) as f64 / self.zoom.pixels_per_sec.max(f64::EPSILON)),
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


    pub(super) fn execute_extra_command(
        &mut self,
        command: &str,
        _params: &[(String, String)],
    ) -> Option<(String, bool)> {
        let ok = || Some((String::new(), true));

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
                // Audacity 3.7.3 DoPrev/NextTrack(shift): at the first/last track with
                // circular navigation off, nothing changes (no select of the focused track).
                let Some(target) = target.filter(|&t| t != focus) else {
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
            _ => None,
        }
    }
}
