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
    /// Pointer events queued by Drag, injected through `raw_input_hook` on the next frame.
    pub injected: Vec<egui::Event>,
}

pub(crate) enum Extra2Dialog {
    Pan { track: usize, value: f32, orig: f32 },
    Gain { track: usize, db: f32, orig: f32 },
    TrackMenu { track: usize },
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
                        self.extra2.dialog = Some(Extra2Dialog::Pan { track: i, value: v, orig: v });
                    }
                    "TrackGain" => {
                        let g = self.track_mix[i].gain;
                        let db = if g <= 0.0 { -36.0 } else { (20.0 * g.log10()).clamp(-36.0, 36.0) };
                        self.extra2.dialog = Some(Extra2Dialog::Gain { track: i, db, orig: db });
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
                let threshold = param("Threshold").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
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
                if ["X", "Y", "Width", "Height"].iter().any(|k| param(k).is_some()) {
                    self.extra2.viewport.push(egui::ViewportCommand::OuterPosition(egui::pos2(num("X", 10.0).clamp(0.0, 2000.0), num("Y", 10.0).clamp(0.0, 2000.0))));
                    self.extra2.viewport.push(egui::ViewportCommand::InnerSize(egui::vec2(num("Width", 1000.0).clamp(200.0, 4000.0), num("Height", 900.0).clamp(200.0, 4000.0))));
                }
                ok()
            }
            // 88: DragCommand::Apply: move the mouse to From, and with To press, drag and release.
            // Coordinates are window points (Audacity's Id/Window targets are wx widgets).
            "Drag" => {
                let num = |k: &str| param(k).and_then(|v| v.parse::<f32>().ok());
                let from = egui::pos2(num("FromX").unwrap_or(200.0), num("FromY").unwrap_or(10.0));
                let ev = &mut self.extra2.injected;
                ev.push(egui::Event::PointerMoved(from));
                if num("ToX").is_some() || num("ToY").is_some() {
                    let to = egui::pos2(num("ToX").unwrap_or(400.0), num("ToY").unwrap_or(10.0));
                    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
                    ev.push(button(from, true));
                    ev.push(egui::Event::PointerMoved(to));
                    ev.push(button(to, false));
                }
                ok()
            }
            _ => None,
        }
    }

    /// Feeds queued Drag pointer events into egui (called from `raw_input_hook`).
    pub(super) fn inject_extra2_input(&mut self, raw: &mut egui::RawInput) {
        raw.events.append(&mut self.extra2.injected);
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
        let (track, value, orig, is_pan) = match &mut dialog {
            Extra2Dialog::Pan { track, value, orig } => (*track, value, *orig, true),
            Extra2Dialog::Gain { track, db, orig } => (*track, db, *orig, false),
            Extra2Dialog::TrackMenu { track } => {
                let track = *track;
                let mut action: Option<&'static str> = None;
                egui::Window::new(tr("Track")).collapsible(false).resizable(false).show(ctx, |ui| {
                    for (label, command) in [
                        ("Mute/Unmute", "TrackMute"), ("Solo/Unsolo", "TrackSolo"),
                        ("Change Pan...", "TrackPan"), ("Change Gain...", "TrackGain"),
                        ("Move Track Up", "TrackMoveUp"), ("Move Track Down", "TrackMoveDown"),
                        ("Move Track to Top", "TrackMoveTop"), ("Move Track to Bottom", "TrackMoveBottom"),
                        ("Delete Track", "TrackClose"),
                    ] {
                        if ui.button(tr(label)).clicked() { action = Some(command); }
                    }
                    if ui.button(tr("Close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) { action = Some(""); }
                });
                match action {
                    Some(command) => {
                        self.focused_track = Some(track);
                        if !command.is_empty() { let _ = self.execute_extra2_command(command, &[]); }
                    }
                    None => self.extra2.dialog = Some(dialog),
                }
                return;
            }
        };
        let to_gain = |v: f32| if is_pan { v } else { 10f32.powf(v / 20.0) };
        let (mut commit, mut cancel, mut changed) = (false, false, false);
        egui::Window::new(tr(if is_pan { "Pan" } else { "Gain" })).collapsible(false).resizable(false).show(ctx, |ui| {
            let slider = if is_pan {
                egui::Slider::new(value, -1.0..=1.0).step_by(0.1).text(tr("Pan"))
            } else {
                egui::Slider::new(value, -36.0..=36.0).step_by(0.1).text("dB")
            };
            changed = ui.add(slider).changed();
            ui.horizontal(|ui| {
                commit = ui.button(tr("OK")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter));
                cancel = ui.button(tr("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
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
