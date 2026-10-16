//! Extra-menu commands 68-89 of docs/SPLIT_extra.md: cursor jumps, focused-track
//! pan/gain/menu/mute/solo/delete/move, and the scriptables SetEnvelope, SetProject,
//! Drag (Move Mouse) and CompareAudio. Ported from Audacity 3.7.3 (SelectMenus.cpp,
//! TrackMenus.cpp, TrackUtilities.cpp, ASlider.cpp and src/commands/*Command.cpp).
//! Started from BizyBot's unpushed extra_cmds.rs (items 68-89 only).
use super::AudacityApp;
use super::extra2_logic::{self as xl, MoveChoice};
use eframe::egui;

/// State kept by the commands in this file.
#[derive(Default)]
pub(crate) struct Extra2State {
    /// Open "Change Pan/Gain on Focused Track..." or track-menu dialog.
    pub dialog: Option<Extra2Dialog>,
    /// Viewport commands queued by SetProject (title, position, size).
    pub viewport: Vec<egui::ViewportCommand>,
    /// Pointer events queued by Drag, one batch per frame (egui needs press, motion and
    /// release on separate frames to see a drag), injected through `raw_input_hook`.
    pub injected: std::collections::VecDeque<Vec<egui::Event>>,
    pub track_rects: Vec<egui::Rect>,
    pub ruler_rect: Option<egui::Rect>,
}

pub(crate) enum Extra2Dialog {
    Pan { track: usize, value: f32, orig: f32, text: String },
    Gain { track: usize, db: f32, orig: f32, text: String },
    TrackMenu { track: usize },
    Rename { track: usize, text: String },
}

impl AudacityApp {
    /// The focused track (TrackFocus), falling back to the selected track; over the
    /// pipe focus follows the selection.
    fn focus_index(&self) -> Option<usize> {
        self.focused_track.or(self.selected_track).filter(|&i| i < self.editor.state().tracks.len())
    }

    /// Selected wave tracks in order: the script multi-selection when it includes the
    /// selected track, else just the selected track.
    pub(super) fn selected_tracks(&self) -> Vec<usize> {
        let n = self.editor.state().tracks.len();
        let multi: Vec<usize> = self.extra.track_sel.iter().take(n).enumerate().filter_map(|(i, s)| s.then_some(i)).collect();
        match self.selected_track.filter(|&i| i < n) {
            Some(t) if multi.contains(&t) => multi,
            Some(t) => vec![t],
            None => multi,
        }
    }

    fn set_mix(&mut self, i: usize, f: impl FnOnce(&mut au_io::engine::TrackParams)) {
        let mut p = self.track_mix[i];
        f(&mut p);
        self.track_mix[i] = p;
        self.send_track_mix(i, p, true);
    }

    /// Applies mute/solo vectors computed by TrackUtilities::DoTrackMute/DoTrackSolo.
    fn set_mute_solo(&mut self, mutes: Vec<bool>, solos: Vec<bool>) {
        for (i, (m, s)) in mutes.into_iter().zip(solos).enumerate() {
            if self.track_mix[i].mute != m || self.track_mix[i].solo != s {
                self.set_mix(i, |p| { p.mute = m; p.solo = s; });
            }
        }
    }

    pub(super) fn execute_extra2_command(&mut self, command: &str, params: &[(String, String)]) -> Option<(String, bool)> {
        let ok = || Some((String::new(), true));
        let param = |name: &str| params.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str());
        match command {
            // Tracks > Remove Tracks (TrackMenus.cpp OnRemoveTracks): delete the selected wave tracks.
            "RemoveTracks" => {
                let mut sel = self.selected_tracks();
                sel.sort_unstable();
                for i in sel.into_iter().rev() {
                    self.focused_track = Some(i);
                    let _ = self.execute_extra2_command("TrackClose", &[]);
                }
                ok()
            }
            // 68-71 SelectMenus DoCursorMove: seek while playing, else MoveWhenAudioInactive
            // in seconds (snapping only when a range collapses).
            "CursorShortJumpLeft" | "CursorShortJumpRight" | "CursorLongJumpLeft" | "CursorLongJumpRight" => {
                let period = crate::settings::seek_seconds(&self.settings, command.contains("Long"));
                let step = if command.ends_with("Left") { -period } else { period };
                if self.player.is_some() {
                    let rate = self.project_rate() as f64;
                    let pos = self.player.as_ref().map(|p| p.position()).unwrap_or(0) as f64;
                    self.seek_transport((pos + step * rate).max(0.0) as usize);
                } else {
                    let (t0, t1) = self.sel_secs();
                    let rate = self.selected_rate();
                    let snapping = !matches!(self.snapper.mode, au_core::timeline::SnapMode::Off);
                    let snap = |t: f64| self.snap_time(t, rate, None);
                    let t = xl::cursor_jump(t0, t1, step, if snapping { Some(&snap) } else { None });
                    self.set_sel_secs(t, t);
                    let visible = self.canvas_width.max(1.0) as f64 / self.zoom.pixels_per_sec.max(f64::EPSILON);
                    if t < self.zoom.h_offset || t > self.zoom.h_offset + visible {
                        self.zoom.h_offset = (t - visible / 2.0).max(0.0);
                    }
                }
                ok()
            }
            "TrackPan" | "TrackPanLeft" | "TrackPanRight" | "TrackGain" | "TrackGainInc" | "TrackGainDec"
            | "TrackMenu" | "TrackMute" | "TrackSolo" | "TrackClose" | "TrackMoveUp" | "TrackMoveDown"
            | "TrackMoveTop" | "TrackMoveBottom" => {
                let Some(i) = self.focus_index() else { return ok() };
                self.ensure_track_mix();
                let simple = self.settings.solo == crate::settings::SoloPolicy::Simple;
                match command {
                    // 73/74, 76/77: LWSlider Increase/Decrease(1) on the PAN (0.1) / DB (1 dB) slider.
                    "TrackPanLeft" | "TrackPanRight" => {
                        let s = if command == "TrackPanLeft" { -1.0 } else { 1.0 };
                        self.set_mix(i, |p| p.pan = xl::pan_step(p.pan, s));
                    }
                    "TrackGainInc" | "TrackGainDec" => {
                        let s = if command == "TrackGainInc" { 1.0 } else { -1.0 };
                        self.set_mix(i, |p| p.gain = xl::gain_step(p.gain, s));
                    }
                    // 72/75/78: the slider pop-ups and the track drop-down menu.
                    "TrackPan" => {
                        let v = self.track_mix[i].pan;
                        let text = format!("{}", (v * 100.0).round() as i32);
                        self.extra2.dialog = Some(Extra2Dialog::Pan { track: i, value: v, orig: v, text });
                    }
                    "TrackGain" => {
                        let g = self.track_mix[i].gain;
                        let db = if g <= 0.0 { -36.0 } else { (20.0 * g.log10()).clamp(-36.0, 36.0) };
                        let text = format!("{db:.1}");
                        self.extra2.dialog = Some(Extra2Dialog::Gain { track: i, db, orig: db, text });
                    }
                    "TrackMenu" => self.extra2.dialog = Some(Extra2Dialog::TrackMenu { track: i }),
                    // 79/80: TrackUtilities::DoTrackMute / DoTrackSolo (not exclusive).
                    "TrackMute" | "TrackSolo" => {
                        if self.settings.solo == crate::settings::SoloPolicy::None && command == "TrackSolo" {
                            return ok();
                        }
                        let mut mutes: Vec<bool> = self.track_mix.iter().map(|p| p.mute).collect();
                        let mut solos: Vec<bool> = self.track_mix.iter().map(|p| p.solo).collect();
                        if command == "TrackMute" {
                            xl::track_mute(&mut mutes, &mut solos, i, simple);
                        } else {
                            xl::track_solo(&mut mutes, &mut solos, i, simple, false);
                        }
                        self.set_mute_solo(mutes, solos);
                    }
                    // 81: OnTrackClose / DoRemoveTrack (focus goes to the next, else previous track).
                    "TrackClose" => {
                        if self.player.is_some() {
                            self.status = au_core::i18n::tr_ui("Can't delete track with active audio");
                            return ok();
                        }
                        let n = self.editor.state().tracks.len();
                        let name = self.editor.state().tracks[i].name.clone();
                        let mut state = self.editor.state().clone();
                        state.tracks.remove(i);
                        self.editor.replace_state(&au_core::i18n::tr_format("Removed track '%s'.", &[&name]), state);
                        if i < self.extra.track_sel.len() { self.extra.track_sel.remove(i); }
                        if i < self.track_mix.len() { self.track_mix.remove(i); }
                        if i < self.track_views.len() { self.track_views.remove(i); }
                        if i < self.wave_ui.len() { self.wave_ui.remove(i); }
                        let fix = |t: usize| if t > i { Some(t - 1) } else if t == i { None } else { Some(t) };
                        self.selected_track = self.selected_track.and_then(fix);
                        if let Some(s) = self.selection {
                            self.selection = fix(s.track).map(|track| au_core::Selection { track, ..s });
                        }
                        self.focused_track = xl::focus_after_remove(n, i);
                    }
                    // 82-85: TrackUtilities::DoMoveTrack.
                    _ => {
                        let choice = match command {
                            "TrackMoveUp" => MoveChoice::Up,
                            "TrackMoveDown" => MoveChoice::Down,
                            "TrackMoveTop" => MoveChoice::Top,
                            _ => MoveChoice::Bottom,
                        };
                        let Some(to) = xl::move_track(self.editor.state().tracks.len(), i, choice) else { return ok() };
                        let mut state = self.editor.state().clone();
                        let name = state.tracks[i].name.clone();
                        xl::apply_move(&mut state.tracks, i, to);
                        let desc = match choice {
                            MoveChoice::Up => "Moved '%s' Up",
                            MoveChoice::Down => "Moved '%s' Down",
                            MoveChoice::Top => "Moved '%s' to Top",
                            MoveChoice::Bottom => "Moved '%s' to Bottom",
                        };
                        self.editor.replace_state(&au_core::i18n::tr_format(desc, &[&name]), state);
                        xl::apply_move(&mut self.track_mix, i, to);
                        xl::apply_move(&mut self.track_views, i, to);
                        xl::apply_move(&mut self.wave_ui, i, to);
                        xl::apply_move(&mut self.extra.track_sel, i, to);
                        self.selected_track = self.selected_track.map(|t| xl::moved_index(t, i, to));
                        if let Some(s) = &mut self.selection { s.track = xl::moved_index(s.track, i, to); }
                        self.focused_track = self.focused_track.map(|t| xl::moved_index(t, i, to));
                    }
                }
                ok()
            }
            // 89: CompareAudioCommand::Apply.
            "CompareAudio" => {
                let (t0, t1) = self.sel_secs();
                if t0 >= t1 {
                    return Some(("There is no selection!".into(), false));
                }
                let sel = self.selected_tracks();
                let (a, b) = match sel.as_slice() {
                    [] => return Some(("No tracks selected! Select two tracks to compare.".into(), false)),
                    [_] => return Some(("Only one track selected! Select two tracks to compare.".into(), false)),
                    [a, b, ..] => (*a, *b),
                };
                let tracks = &self.editor.state().tracks;
                let (ta, tb) = (tracks[a].render(), tracks[b].render());
                if ta.channels.len() != tb.channels.len() {
                    return Some(("Selected tracks must have the same number of channels!".into(), false));
                }
                // Define(errorThreshold, "Threshold", 0.0f, 0.0f, 0.01f): out-of-range values are
                // rejected by parameter validation (Audacity shows an error and does not run).
                let raw = param("Threshold").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                if !(0.0..=f64::from(0.01f32)).contains(&raw) {
                    return Some((format!("Threshold={raw} is out of range 0 to 0.01"), false));
                }
                let threshold = raw as f32;
                let rate = tracks[a].sample_rate().max(1);
                let s0 = au_core::secs_to_frames(t0, rate);
                let s1 = au_core::secs_to_frames(t1, rate);
                let count = xl::compare_audio(&ta.channels, &tb.channels, s0, s1, threshold);
                let mut msg = String::new();
                if sel.len() > 2 {
                    msg.push_str("More than two tracks selected - only the first two will be compared.");
                }
                msg.push_str(&format!("Comparing tracks '{}' and '{}'.", tracks[a].name, tracks[b].name));
                // Audacity 3.7.3 concatenates the Status lines without separators on the pipe.
                msg.push_str(&xl::compare_audio_message(count, rate, threshold).replace('\n', ""));
                Some((msg, true))
            }
            // 86: SetEnvelopeCommand::ApplyInner on each selected track. The Rust model keeps
            // one envelope per track (the clip model is off-limits), so "clip containing Time"
            // becomes "track has audio at Time".
            "SetEnvelope" => {
                let time = param("Time").and_then(|v| v.parse::<f64>().ok());
                let value = param("Value").and_then(|v| v.parse::<f64>().ok());
                let delete = param("Delete").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
                let mut state = self.editor.state().clone();
                let mut changed = false;
                for i in self.selected_tracks() {
                    let track = &mut state.tracks[i];
                    let rate = track.sample_rate().max(1) as f64;
                    let found = time.is_none_or(|t| track.clips.clips.iter().any(|c| {
                        let s = c.start as f64 / rate;
                        s <= t && t <= s + c.buffer.len() as f64 * c.stretch_ratio / rate
                    }));
                    if !found { continue; }
                    if delete { track.clips.envelope.points.clear(); changed = true; }
                    if let (Some(t), Some(v)) = (time, value) {
                        track.clips.envelope.insert(t, v.clamp(0.0, 2.0));
                        changed = true;
                    }
                }
                if changed {
                    self.editor.replace_state("Edited Envelope", state);
                }
                ok()
            }
            // 87: SetProjectCommand::Apply: window title, project rate, position and size.
            "SetProject" => {
                if let Some(name) = param("Name") {
                    self.extra2.viewport.push(egui::ViewportCommand::Title(name.to_owned()));
                }
                if let Some(rate) = param("Rate").and_then(|v| v.parse::<f64>().ok()) {
                    if (1.0..=1_000_000.0).contains(&rate) {
                        let mut state = self.editor.state().clone();
                        state.sample_rate = rate as u32;
                        self.editor.replace_state("Set Project", state);
                    }
                }
                let num = |k: &str, d: f32| param(k).and_then(|v| v.parse::<f32>().ok()).unwrap_or(d);
                // X, Y, Width and Height share one bHasSizing flag that each OptionalY().Define()
                // overwrites in turn, so only Height decides whether the frame is moved/resized.
                if param("Height").is_some() {
                    self.extra2.viewport.push(egui::ViewportCommand::OuterPosition(egui::pos2(num("X", 10.0).clamp(0.0, 2000.0), num("Y", 10.0).clamp(0.0, 2000.0))));
                    self.extra2.viewport.push(egui::ViewportCommand::InnerSize(egui::vec2(num("Width", 1000.0).clamp(200.0, 4000.0), num("Height", 900.0).clamp(200.0, 4000.0))));
                }
                ok()
            }
            // 88: DragCommand::Apply: move the mouse to From, and with To press, drag and release.
            // Coordinates are window points (Audacity's Id/Window targets are wx widgets).
            "Drag" => {
                let num = |k: &str| param(k).and_then(|v| v.parse::<f32>().ok());
                let window: String = match param("Window") {
                    Some(w) => w.trim_matches(|c| c == '"' || c == '\'').to_owned(),
                    None => match param("Id") {
                        Some(id) if id == "1003" => "Track Panel".to_owned(),
                        Some(id) if id == "-2012" => "Timeline".to_owned(),
                        // No Window/Id: coordinates are relative to the project frame itself.
                        _ => String::new(),
                    },
                };
                let from = self.drag_point(&window, num("FromX").unwrap_or(200.0), num("FromY").unwrap_or(10.0));
                let to_opt = param("ToX").and_then(|v| v.parse::<f32>().ok()).map(|tx| {
                    self.drag_point(&window, tx, num("ToY").unwrap_or(10.0))
                });
                let ev = &mut self.extra2.injected;
                // DragCommand::Apply sends each motion event twice (enter + move).
                ev.push_back(vec![egui::Event::PointerMoved(from)]);
                ev.push_back(vec![egui::Event::PointerMoved(from)]);
                if let Some(to) = to_opt {
                    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
                    ev.push_back(vec![button(from, true)]);
                    ev.push_back(vec![egui::Event::PointerMoved(to)]);
                    ev.push_back(vec![egui::Event::PointerMoved(to)]);
                    ev.push_back(vec![button(to, false)]);
                }
                ok()
            }
            _ => None,
        }
    }

    fn drag_point(&self, window: &str, x: f32, y: f32) -> egui::Pos2 {
        if window == "Timeline" {
            if let Some(r) = self.extra2.ruler_rect {
                let t = self.zoom.h_offset + (x as f64 - 182.0) / self.zoom.pixels_per_sec;
                let xx = r.left() + self.zoom.time_to_x(t) as f32;
                let yy = r.top() + y.clamp(0.0, (r.height() - 1.0).max(0.0));
                return egui::pos2(xx, yy);
            }
            return egui::pos2(x, y);
        }
        if window == "Track Panel" {
            let n = self.extra2.track_rects.len();
            if n == 0 {
                return egui::pos2(x, y);
            }
            // Audacity default row heights: mono 150 px, stereo 300 px.
            let row_h_for = |idx: usize| -> f32 {
                match self.editor.state().tracks.get(idx) {
                    Some(t) if t.clips.n_channels >= 2 => 300.0,
                    _ => 150.0,
                }
            };
            let mut found = n - 1;
            let mut found_top = 0.0f32;
            let mut acc = 0.0f32;
            for idx in 0..n {
                let rh = row_h_for(idx);
                if y < acc + rh {
                    found = idx;
                    found_top = acc;
                    break;
                }
                acc += rh;
                if idx == n - 1 {
                    found = idx;
                    found_top = acc - rh;
                }
            }
            let row_top = found_top;
            let row_h = row_h_for(found);
            let tr = match self.extra2.track_rects.get(found) {
                Some(tr) if tr.is_finite() && tr.width() > 0.0 && tr.height() > 0.0 => *tr,
                _ => return egui::pos2(x, y),
            };
            let t = self.zoom.h_offset + (x as f64 - 182.0) / self.zoom.pixels_per_sec;
            let xx = tr.left() + self.zoom.time_to_x(t) as f32;
            let dy = y - row_top;
            let yy = if dy < 23.0 {
                tr.top() + 4.0 * (dy / 23.0)
            } else {
                tr.top() + 4.0 + (dy - 23.0) / (row_h - 25.0) * (tr.height() - 4.0)
            };
            let yy = yy.clamp(tr.top(), (tr.bottom() - 1.0).max(tr.top()));
            return egui::pos2(xx, yy);
        }
        egui::pos2(x, y)
    }
    /// Feeds queued Drag pointer events into egui (called from `raw_input_hook`).
    pub(super) fn inject_extra2_input(&mut self, raw: &mut egui::RawInput) {
        if let Some(mut batch) = self.extra2.injected.pop_front() {
            raw.events.append(&mut batch);
        }
    }

    pub(super) fn show_extra2_dialogs(&mut self, ctx: &egui::Context) {
        for cmd in self.extra2.viewport.drain(..) {
            ctx.send_viewport_cmd(cmd);
        }
        if !self.extra2.injected.is_empty() {
            ctx.request_repaint();
        }
        let Some(mut dialog) = self.extra2.dialog.take() else { return };
        let tr = au_core::i18n::tr_ui;
        // ASlider pop-up: live preview, OK commits one undo step, Cancel restores.
        let (track, value, text, orig, is_pan) = match &mut dialog {
            Extra2Dialog::Pan { track, value, orig, text } => (*track, value, text, *orig, true),
            Extra2Dialog::Gain { track, db, orig, text } => (*track, db, text, *orig, false),
            Extra2Dialog::Rename { track, text } => {
                let track = *track;
                let mut commit = false;
                let mut cancel = false;
                egui::Window::new(tr("Set Track Name")).collapsible(false).resizable(false).show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(tr("Name:"));
                        ui.add(egui::TextEdit::singleline(&mut *text).desired_width(200.0));
                    });
                    ui.horizontal(|ui| {
                        commit = ui.button(tr("OK")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter));
                        cancel = ui.button(tr("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
                    });
                });
                if commit {
                    let new_name = text.clone();
                    if let Some(t) = self.editor.state().tracks.get(track) {
                        let old = t.name.clone();
                        let mut state = self.editor.state().clone();
                        if let Some(tt) = state.tracks.get_mut(track) { tt.name = new_name.clone(); }
                        self.editor.replace_state(&format!("Renamed '{old}' to '{new_name}'"), state);
                    }
                } else if !cancel {
                    self.extra2.dialog = Some(dialog);
                }
                return;
            }
            Extra2Dialog::TrackMenu { track } => {
                let track = *track;
                let n = self.editor.state().tracks.len();
                if track >= n { return; }
                let tname = self.editor.state().tracks[track].name.clone();
                let is_first = track == 0;
                let is_last = track + 1 >= n;
                let nch = self.editor.state().tracks[track].clips.n_channels;
                let is_stereo = nch == 2;
                let can_stereo = nch == 1 && track + 1 < n && self.editor.state().tracks[track + 1].clips.n_channels == 1;
                let cur_view = self.track_views.get(track).cloned().unwrap_or(crate::spectrogram_view::TrackView::Waveform);
                let is_mv = cur_view == crate::spectrogram_view::TrackView::Multiview;
                let cur_bits = self.editor.state().tracks[track].int_bits;
                let cur_rate = self.editor.state().tracks[track].sample_rate();
                enum Act { None, Rename, Move(&'static str), View(crate::spectrogram_view::TrackView), MakeStereo, Swap, Split(bool), Format(Option<u32>, &'static str), Rate(u32) }
                let mut act = Act::None;
                let mut keep = true;
                egui::Window::new(tname).collapsible(false).resizable(false).show(ctx, |ui| {
                    if ui.button(tr("Rename Track...")).clicked() { act = Act::Rename; keep = false; }
                    ui.separator();
                    if ui.add_enabled(!is_first, egui::Button::new(tr("Move Track Up"))).clicked() { act = Act::Move("TrackMoveUp"); keep = false; }
                    if ui.add_enabled(!is_last, egui::Button::new(tr("Move Track Down"))).clicked() { act = Act::Move("TrackMoveDown"); keep = false; }
                    if ui.add_enabled(!is_first, egui::Button::new(tr("Move Track to Top"))).clicked() { act = Act::Move("TrackMoveTop"); keep = false; }
                    if ui.add_enabled(!is_last, egui::Button::new(tr("Move Track to Bottom"))).clicked() { act = Act::Move("TrackMoveBottom"); keep = false; }
                    ui.separator();
                    let mut mv = is_mv;
                    if ui.checkbox(&mut mv, tr("Multi-view")).changed() {
                        act = Act::View(if mv { crate::spectrogram_view::TrackView::Multiview } else { crate::spectrogram_view::TrackView::Waveform });
                        keep = false;
                    }
                    let mut v = cur_view;
                    ui.radio_value(&mut v, crate::spectrogram_view::TrackView::Waveform, tr("Waveform"));
                    ui.radio_value(&mut v, crate::spectrogram_view::TrackView::Spectrogram, tr("Spectrogram"));
                    if v != cur_view { act = Act::View(v); keep = false; }
                    ui.separator();
                    if ui.add_enabled(can_stereo, egui::Button::new(tr("Make Stereo Track"))).clicked() { act = Act::MakeStereo; keep = false; }
                    if ui.add_enabled(is_stereo, egui::Button::new(tr("Swap Stereo Channels"))).clicked() { act = Act::Swap; keep = false; }
                    if ui.add_enabled(is_stereo, egui::Button::new(tr("Split Stereo Track"))).clicked() { act = Act::Split(false); keep = false; }
                    if ui.add_enabled(is_stereo, egui::Button::new(tr("Split Stereo to Mono"))).clicked() { act = Act::Split(true); keep = false; }
                    ui.separator();
                    let mut b = cur_bits;
                    ui.radio_value(&mut b, Some(16), tr("16-bit PCM"));
                    ui.radio_value(&mut b, Some(24), tr("24-bit PCM"));
                    ui.radio_value(&mut b, None, tr("32-bit float"));
                    if b != cur_bits {
                        let label: &'static str = match b { Some(16) => "16-bit PCM", Some(24) => "24-bit PCM", _ => "32-bit float" };
                        act = Act::Format(b, label);
                        keep = false;
                    }
                    ui.separator();
                    for r in [8000u32, 11025, 16000, 22050, 44100, 48000, 88200, 96000, 176400, 192000, 352800, 384000] {
                        if ui.selectable_label(cur_rate == r, format!("{r} Hz")).clicked() { act = Act::Rate(r); keep = false; }
                    }
                    ui.separator();
                    if ui.button(tr("Close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) { keep = false; }
                });
                match act {
                    Act::None => {}
                    Act::Rename => {
                        let txt = self.editor.state().tracks[track].name.clone();
                        self.extra2.dialog = Some(Extra2Dialog::Rename { track, text: txt });
                        return;
                    }
                    Act::Move(cmd) => {
                        self.focused_track = Some(track);
                        let _ = self.execute_extra2_command(cmd, &[]);
                    }
                    Act::View(vv) => {
                        if track < self.track_views.len() { self.track_views[track] = vv; }
                    }
                    Act::MakeStereo => {
                        let mut state = self.editor.state().clone();
                        let l = state.tracks[track].clone();
                        let r = state.tracks[track + 1].clone();
                        match au_core::track_ops::make_stereo(&l, &r) {
                            Some(st) => {
                                let nm = state.tracks[track].name.clone();
                                state.tracks[track] = st;
                                state.tracks.remove(track + 1);
                                self.editor.replace_state(&format!("Made '{nm}' a stereo track"), state);
                                if track + 1 < self.extra.track_sel.len() { self.extra.track_sel.remove(track + 1); }
                                if track + 1 < self.track_mix.len() { self.track_mix.remove(track + 1); }
                                if track + 1 < self.track_views.len() { self.track_views.remove(track + 1); }
                                if track + 1 < self.wave_ui.len() { self.wave_ui.remove(track + 1); }
                                self.ensure_track_mix();
                            }
                            None => { self.status = "Unable to make a stereo track (clips differ)".to_string(); }
                        }
                    }
                    Act::Swap => {
                        let mut state = self.editor.state().clone();
                        let nm = state.tracks[track].name.clone();
                        if let Some(t) = state.tracks.get_mut(track) { au_core::track_ops::swap_channels(t); }
                        self.editor.replace_state(&format!("Swapped channels in '{nm}'"), state);
                    }
                    Act::Split(to_mono) => {
                        let mut state = self.editor.state().clone();
                        let t = state.tracks[track].clone();
                        let nm = t.name.clone();
                        let parts = au_core::track_ops::split_channels(&t, to_mono);
                        state.tracks.remove(track);
                        for (k, p) in parts.iter().enumerate() { state.tracks.insert(track + k, p.clone()); }
                        let extra = parts.len().saturating_sub(1);
                        for k in 0..extra {
                            if track < self.extra.track_sel.len() { let c = self.extra.track_sel[track]; self.extra.track_sel.insert(track + k + 1, c); }
                            if track < self.track_mix.len() { let c = self.track_mix[track]; self.track_mix.insert(track + k + 1, c); }
                            if track < self.track_views.len() { let c = self.track_views[track].clone(); self.track_views.insert(track + k + 1, c); }
                            if track < self.wave_ui.len() { let c = self.wave_ui[track].clone(); self.wave_ui.insert(track + k + 1, c); }
                        }
                        let undo = if to_mono { format!("Split stereo track '{nm}' to mono") } else { format!("Split stereo track '{nm}'") };
                        self.editor.replace_state(&undo, state);
                        self.ensure_track_mix();
                        // Split Stereo Track pans the halves hard left/right; the mixer reads track_mix.
                        for (k, p) in parts.iter().enumerate() {
                            if let Some(m) = self.track_mix.get_mut(track + k) { m.pan = p.pan; }
                        }
                    }
                    Act::Format(bits, label) => {
                        let mut state = self.editor.state().clone();
                        let nm = state.tracks[track].name.clone();
                        if let Some(t) = state.tracks.get_mut(track) { au_core::track_ops::set_format(t, bits); }
                        self.editor.replace_state(&format!("Changed '{nm}' to {label}"), state);
                    }
                    Act::Rate(r) => {
                        let mut state = self.editor.state().clone();
                        let nm = state.tracks[track].name.clone();
                        if let Some(t) = state.tracks.get_mut(track) { au_core::track_ops::set_rate(t, r); }
                        self.editor.replace_state(&format!("Changed '{nm}' to {r} Hz"), state);
                    }
                }
                if keep { self.extra2.dialog = Some(dialog); }
                return;
            }
        };
        let to_gain = |v: f32| if is_pan { v } else { 10f32.powf(v / 20.0) };
        let fmt = |v: f32| if is_pan { format!("{}", (v * 100.0).round() as i32) } else { format!("{v:.1}") };
        let (mut commit, mut cancel, mut changed) = (false, false, false);
        egui::Window::new(tr(if is_pan { "Pan" } else { "Volume" })).collapsible(false).resizable(false).show(ctx, |ui| {
            let out = egui::TextEdit::singleline(&mut *text).desired_width(60.0).show(ui);
            // SliderDialog::TransferDataToWindow: the text box has focus with all text selected.
            if ui.memory(|m| m.focused().is_none()) {
                out.response.request_focus();
            }
            let done_id = out.response.id.with("select_all_done");
            if out.response.has_focus() && !ui.data(|d| d.get_temp::<bool>(done_id).unwrap_or(false)) {
                ui.data_mut(|d| d.insert_temp(done_id, true));
                let mut st = out.state.clone();
                let n = text.chars().count();
                st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                st.store(ui.ctx(), out.response.id);
            }
            if out.response.changed() {
                if is_pan {
                    if let Ok(n) = (*text).trim().parse::<f32>() {
                        *value = (n / 100.0).clamp(-1.0, 1.0);
                        changed = true;
                    }
                } else if let Ok(n) = (*text).trim().parse::<f32>() {
                    *value = n.clamp(-36.0, 36.0);
                    changed = true;
                }
            }
            let slider = if is_pan {
                egui::Slider::new(value, -1.0..=1.0).text(tr("Pan"))
            } else {
                egui::Slider::new(value, -36.0..=36.0)
            };
            if ui.add(slider).changed() {
                changed = true;
                *text = fmt(*value);
            }
            ui.horizontal(|ui| {
                commit = ui.button(tr("OK")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter));
                cancel = ui.button(tr("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
            if commit || cancel {
                ui.data_mut(|d| d.remove::<bool>(done_id)); // next open selects the text again
            }
        });
        if track >= self.track_mix.len() {
            return;
        }
        let set = |p: &mut au_io::engine::TrackParams, v: f32| if is_pan { p.pan = v } else { p.gain = to_gain(v) };
        let mut p = self.track_mix[track];
        if cancel {
            set(&mut p, orig);
        } else if changed || commit {
            set(&mut p, *value);
        }
        self.track_mix[track] = p;
        if cancel || changed {
            self.send_track_mix(track, p, false);
        }
        if commit {
            self.send_track_mix(track, p, true);
        }
        if !(commit || cancel) {
            self.extra2.dialog = Some(dialog);
        }
    }
}
