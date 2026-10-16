//! Extra-menu commands 1–45 of docs/SPLIT_extra.md (Transport, Tools, Mixer, Edit,
//! Play-at-Speed, Device, Selection): app glue around the pure ports in `extra_cmds.rs`.
use super::*;
use crate::extra_cmds as xc;

/// Small dialogs opened by the "…" Extra commands.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ExtraDialog {
    OutputGain,
    InputGain,
    PlaySpeed,
    /// Choose from a list and write the preference (device/host/channels).
    Choose { title: String, key: String, items: Vec<String>, current: String },
}

/// Bounded or cut-preview playback started by an Extra command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PlayStop {
    /// Stop once the play head reaches this project frame (forwards) or falls to it (backwards).
    pub frame: usize,
    pub backwards: bool,
}

/// Keyboard scrub in progress: which direction and the selection when it started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct KeyScrub {
    pub backwards: bool,
    pub sel: (f64, f64),
}

const TOOL_ORDER: [Tool; xc::NUM_TOOLS] = [Tool::Selection, Tool::Envelope, Tool::Draw, Tool::Multi];

impl AudacityApp {
    fn cut_preview_lens(&self) -> (f64, f64) {
        let before = self.prefs.read_string_or("/AudioIO/CutPreviewBeforeLen", "2").parse().unwrap_or(2.0);
        let after = self.prefs.read_string_or("/AudioIO/CutPreviewAfterLen", "1").parse().unwrap_or(1.0);
        (before, after)
    }

    /// Selection in seconds on the selected (or first) track; a point when nothing is selected.
    pub(super) fn sel_secs(&self) -> (f64, f64) {
        if self.selection.is_some() {
            self.selection_secs()
        } else {
            let t = self.cursor_secs();
            (t, t)
        }
    }

    fn cursor_secs(&self) -> f64 {
        self.selection.map_or(0.0, |s| au_core::frames_to_secs(s.start, self.selected_rate()))
    }

    pub(super) fn set_sel_secs(&mut self, t0: f64, t1: f64) {
        let track = self.selected_track.or(self.selection.map(|s| s.track)).unwrap_or(0);
        self.selected_track = Some(track);
        let rate = self.selected_rate();
        let (a, b) = (t0.min(t1).max(0.0), t0.max(t1).max(0.0));
        self.selection = Some(Selection {
            track,
            start: au_core::secs_to_frames(a, rate),
            end: au_core::secs_to_frames(b, rate),
        });
    }

    /// max(track end, screen end), as SelectMenus' seek limits.
    fn seek_end(&self) -> f64 {
        let screen_end = self.zoom.x_to_time(self.canvas_width.max(1.0) as f64);
        self.project_duration().max(screen_end)
    }

    /// Plays `[t0, t1)` of the mix (clamped to the project) at `speed`; with `cut` the
    /// range `cut.0..cut.1` is removed from every track first (Audacity's cut preview).
    pub(super) fn play_region(&mut self, t0: f64, t1: f64, speed: f64, looped: bool, cut: Option<(f64, f64)>) {
        if self.recorder.is_some() {
            return;
        }
        self.stop_playback();
        let rate = self.project_rate();
        let mut tracks = crate::transport::mix_tracks(self.editor.state());
        if let Some((c0, c1)) = cut {
            let (f0, f1) = (au_core::secs_to_frames(c0, rate), au_core::secs_to_frames(c1, rate));
            for t in &mut tracks {
                let audio = t.audio.as_ref();
                let (a0, a1) = (f0.saturating_sub(t.offset).min(audio.len()), f1.saturating_sub(t.offset).min(audio.len()));
                if a1 > a0 {
                    let channels = audio.channels.iter().map(|c| [&c[..a0], &c[a1..]].concat()).collect();
                    t.audio = std::sync::Arc::new(AudioBuffer::from_channels(audio.sample_rate, channels));
                }
            }
        }
        let backwards = speed < 0.0;
        let start = au_core::secs_to_frames(if backwards { t1 } else { t0 }.max(0.0), rate);
        let end = au_core::secs_to_frames(if backwards { t0 } else { t1 }.max(0.0), rate);
        let loop_range = looped.then_some((start.min(end), start.max(end))).filter(|(a, b)| b > a);
        let (track_fx, master_fx) = crate::fx_bridge::stacks_for(&self.aup3_extras, tracks.len());
        match au_io::device::Player::start_with_effects_on(
            &self.device_choice(false), tracks, track_fx, master_fx, rate, self.master_gain, start, speed, loop_range,
        ) {
            Ok(mut player) => {
                player.handle.send(EngineCommand::SetMicrofades(self.settings.micro_fades));
                self.player = Some(player);
                self.play_stop = (!looped).then_some(PlayStop { frame: end, backwards });
                self.status = "Playing".to_owned();
            }
            Err(error) => self.status = au_core::i18n::tr_format("Playback failed: %s", &[&format!("{error:?}")]),
        }
    }

    /// Called every frame: ends bounded plays and keyboard scrubs.
    pub(super) fn poll_extra_playback(&mut self, ctx: &egui::Context) {
        if let (Some(stop), Some(player)) = (self.play_stop, self.player.as_ref()) {
            let pos = player.position();
            if (!stop.backwards && pos >= stop.frame) || (stop.backwards && pos <= stop.frame) {
                self.play_stop = None;
                self.stop_playback();
            } else {
                ctx.request_repaint();
            }
        }
        if let Some(scrub) = self.key_scrub {
            let key = if scrub.backwards { egui::Key::U } else { egui::Key::I };
            if !ctx.input(|i| i.key_down(key)) {
                self.end_keyboard_scrub(scrub);
            } else {
                ctx.request_repaint();
            }
        }
        if self.player.is_none() {
            self.play_stop = None;
        }
    }

    /// Scrubber::DoKeyboardScrub key-down.
    fn start_keyboard_scrub(&mut self, backwards: bool) {
        if let Some(scrub) = self.key_scrub.as_mut() {
            scrub.backwards = backwards;
            return;
        }
        if self.player.is_some() || self.recorder.is_some() {
            return;
        }
        let end = self.project_duration();
        let sel = self.sel_secs();
        if !xc::keyboard_scrub_can_start(sel.0, end, backwards) {
            return;
        }
        let default_zoom = au_core::timeline::ZoomInfo::default().pixels_per_sec;
        let speed = xc::keyboard_scrub_speed(default_zoom, self.zoom.pixels_per_sec);
        if backwards {
            self.play_region(0.0, sel.0, -speed, false, None);
        } else {
            self.play_region(sel.0, end, speed, false, None);
        }
        self.key_scrub = Some(KeyScrub { backwards, sel });
    }

    /// Key-up: the cursor moves to where the scrub stopped if the selection did not change.
    fn end_keyboard_scrub(&mut self, scrub: KeyScrub) {
        self.key_scrub = None;
        let time = self.player.as_ref().map(|p| au_core::frames_to_secs(p.position(), self.project_rate()));
        if let Some(time) = time {
            if self.sel_secs() == scrub.sel {
                let t = time.clamp(0.0, self.project_duration());
                self.set_sel_secs(t, t);
            }
        }
        self.stop_playback();
    }

    fn snap_fn(&self) -> impl Fn(f64) -> f64 + '_ {
        let rate = self.selected_rate();
        move |t| self.snap_time(t, rate, None)
    }

    fn grid_secs(&self) -> Option<f64> {
        match self.snapper.mode {
            au_core::timeline::SnapMode::Grid { secs } if secs > 0.0 => Some(secs),
            _ => None,
        }
    }

    /// SelectMenus SeekLeftOrRight with audio inactive (holding the key moves 4× faster).
    fn seek_selection(&mut self, direction: f64, op: xc::SelOp) {
        let now = std::time::Instant::now();
        let fast = self.last_seek.is_some_and(|t| now.duration_since(t).as_millis() < xc::FAST_REPEAT_MS as u128);
        self.last_seek = Some(now);
        let (t0, t1) = self.sel_secs();
        let step = xc::seek_step(direction, fast);
        let snap_on = !matches!(self.snapper.mode, au_core::timeline::SnapMode::Off);
        let grid = self.grid_secs();
        let prior = self.snap_prior;
        let single = move |t: f64, forward: bool| match grid {
            Some(g) => {
                let k = (t / g).floor();
                if forward { (k + 1.0) * g } else if (t / g - k).abs() < 1e-9 { (k - 1.0) * g } else { k * g }
            }
            None => t,
        };
        let (pps, end, h) = (self.zoom.pixels_per_sec, self.seek_end(), self.zoom.h_offset);
        let (a, b) = {
            let base = self.snap_fn();
            let snap = |t: f64| if prior { grid.map_or(base(t), |g| (t / g).floor() * g) } else { base(t) };
            xc::seek_inactive(h, t0, t1, step, op, pps, end, &snap, snap_on, &single)
        };
        self.set_sel_secs(a, b);
    }

    /// Handles the Extra commands of our half; `None` when `name` is not one of them.
    pub(super) fn extra_command(&mut self, name: &str) -> Option<(String, bool)> {
        let (t0, t1) = self.sel_secs();
        let (before, after) = self.cut_preview_lens();
        let pointer = self.last_pointer_time.unwrap_or(t0);
        let play = |cmd| xc::play_region(cmd, pointer, t0, t1, before, after);
        let ok = |m: &str| Some((m.to_owned(), true));
        match name {
            "PlayOneSec" | "PlayToSelection" | "PlayBeforeSelectionStart" | "PlayAfterSelectionStart"
            | "PlayBeforeSelectionEnd" | "PlayAfterSelectionEnd" | "PlayBeforeAndAfterSelectionStart"
            | "PlayBeforeAndAfterSelectionEnd" => {
                let cmd = match name {
                    "PlayOneSec" => xc::PlayCmd::OneSec,
                    "PlayToSelection" => xc::PlayCmd::ToSelection,
                    "PlayBeforeSelectionStart" => xc::PlayCmd::BeforeSelStart,
                    "PlayAfterSelectionStart" => xc::PlayCmd::AfterSelStart,
                    "PlayBeforeSelectionEnd" => xc::PlayCmd::BeforeSelEnd,
                    "PlayAfterSelectionEnd" => xc::PlayCmd::AfterSelEnd,
                    "PlayBeforeAndAfterSelectionStart" => xc::PlayCmd::BeforeAndAfterSelStart,
                    _ => xc::PlayCmd::BeforeAndAfterSelEnd,
                };
                let (a, b) = play(cmd);
                self.play_region(a, b, 1.0, false, None);
                Some((format!("Playing {a:.6}-{b:.6}"), true))
            }
            "PlayCutPreview" | "PlayAtSpeedCutPreview" => {
                let speed = if name == "PlayCutPreview" { 1.0 } else { self.play_speed };
                let [(a, _), (_, b)] = xc::cut_preview_segments(t0, t1, before, after);
                // After removing t0..t1 the second segment follows the first directly.
                self.play_region(a, b - (t1 - t0), speed, false, Some((t0, t1)));
                ok("Playing cut preview")
            }
            "KeyboardScrubBackwards" | "KeyboardScrubForwards" => {
                let backwards = name == "KeyboardScrubBackwards";
                self.start_keyboard_scrub(backwards);
                if !self.keyboard_event {
                    // Menu/pipe: simulated key-down then key-up, as Audacity.
                    if let Some(scrub) = self.key_scrub {
                        self.end_keyboard_scrub(scrub);
                    }
                }
                ok("Scrub")
            }
            "SelectTool" | "EnvelopeTool" | "DrawTool" | "MultiTool" | "PrevTool" | "NextTool" => {
                let cur = TOOL_ORDER.iter().position(|t| *t == self.tool).unwrap_or(0);
                self.tool = match name {
                    "SelectTool" => Tool::Selection,
                    "EnvelopeTool" => Tool::Envelope,
                    "DrawTool" => Tool::Draw,
                    "MultiTool" => Tool::Multi,
                    "PrevTool" => TOOL_ORDER[xc::prev_tool(cur)],
                    _ => TOOL_ORDER[xc::next_tool(cur)],
                };
                Some((format!("Tool {}", name), true))
            }
            "OutputGain" => { self.extra_dialog = Some(ExtraDialog::OutputGain); ok("") }
            "InputGain" => { self.extra_dialog = Some(ExtraDialog::InputGain); ok("") }
            "SetPlaySpeed" => { self.extra_dialog = Some(ExtraDialog::PlaySpeed); ok("") }
            "OutputGainInc" | "OutputGainDec" => {
                let adj = if name.ends_with("Inc") { 1 } else { -1 };
                self.master_gain = xc::volume_adjust(self.master_gain.clamp(0.0, 1.0), adj);
                if let Some(p) = self.player.as_mut() {
                    p.handle.send(EngineCommand::SetMasterGain(self.master_gain));
                }
                Some((format!("Playback volume {:.2}", self.master_gain), true))
            }
            "InputGainInc" | "InputGainDec" => {
                let adj = if name.ends_with("Inc") { 1 } else { -1 };
                self.input_volume = xc::volume_adjust(self.input_volume, adj);
                Some((format!("Recording volume {:.2}", self.input_volume), true))
            }
            "DeleteKey" | "DeleteKey2" => {
                // EditMenus OnDelete: selectedRegion.collapseToT0().
                let r = self.execute_script("Delete:");
                if r.1 { self.set_sel_secs(t0, t0); }
                Some(r)
            }
            "ClipLeft" | "ClipRight" => Some(self.clip_left_or_right(name == "ClipRight")),
            "PlayAtSpeed" | "PlayAtSpeedLooped" => {
                let (a, b) = if t1 > t0 { (t0, t1) } else { (t0, self.project_duration()) };
                self.play_region(a, b, self.play_speed, name == "PlayAtSpeedLooped", None);
                ok("Playing at speed")
            }
            "PlaySpeedInc" | "PlaySpeedDec" => {
                let adj = if name.ends_with("Inc") { 0.1 } else { -0.1 };
                let s = xc::play_speed_adjust(self.play_speed as f32, adj);
                self.set_play_speed(f64::from(s));
                Some((format!("Playback speed {:.3}x", self.play_speed), true))
            }
            "InputDevice" | "OutputDevice" | "AudioHost" | "InputChannels" => {
                let (title, key, items, current) = match name {
                    "InputDevice" => ("Select Recording Device", "/AudioIO/RecordingDevice", au_io::device::input_devices(), self.settings.recording_device.clone()),
                    "OutputDevice" => ("Select Playback Device", "/AudioIO/PlaybackDevice", au_io::device::output_devices(), self.settings.playback_device.clone()),
                    "AudioHost" => ("Select Audio Host", "/AudioIO/Host", au_io::device::hosts(), self.settings.host.clone()),
                    _ => ("Select Recording Channels", "/AudioIO/RecordChannels", vec!["1 (Mono) Recording Channel".into(), "2 (Stereo) Recording Channels".into()], self.settings.record_channels.to_string()),
                };
                self.extra_dialog = Some(ExtraDialog::Choose { title: title.into(), key: key.into(), items, current });
                ok("")
            }
            "SnapToOff" | "SnapToNearest" | "SnapToPrior" => {
                use au_core::timeline::SnapMode;
                if name == "SnapToOff" {
                    if self.snapper.mode != SnapMode::Off {
                        self.snap_restore = self.snapper.mode.clone();
                    }
                    self.snapper.mode = SnapMode::Off;
                } else {
                    if self.snapper.mode == SnapMode::Off {
                        self.snapper.mode = self.snap_restore.clone();
                    }
                    self.snap_prior = name == "SnapToPrior";
                }
                // ProjectSnap's SnapModeSetting.
                let v = match name { "SnapToOff" => "OFF", "SnapToNearest" => "NEAREST", _ => "PRIOR" };
                self.prefs.set("/Snap/Mode", au_core::prefs::PrefValue::Str(v.into()));
                Some((format!("Snap {v}"), true))
            }
            "SelStart" => { let (a, b) = xc::sel_to_start(t0, t1); self.set_sel_secs(a, b); self.zoom.h_offset = 0.0; ok("") }
            "SelEnd" => { let (a, b) = xc::sel_to_end(t0, t1, self.project_duration()); self.set_sel_secs(a, b); ok("") }
            "SelExtLeft" => { self.seek_selection(-1.0, xc::SelOp::Extend); ok("") }
            "SelExtRight" => { self.seek_selection(1.0, xc::SelOp::Extend); ok("") }
            "SelSetExtLeft" => {
                let now = std::time::Instant::now();
                let fast = self.last_seek.is_some_and(|t| now.duration_since(t).as_millis() < xc::FAST_REPEAT_MS as u128);
                self.last_seek = Some(now);
                let (a, b) = xc::boundary_move(self.zoom.h_offset, t0, t1, -1, fast, self.zoom.pixels_per_sec, self.seek_end());
                self.set_sel_secs(a, b);
                ok("")
            }
            _ => None,
        }
    }

    /// ClipMenus DoClipLeftOrRight: shift the clip under the cursor of the focused track by
    /// one pixel (rounded to a sample); the selection moves with it.
    fn clip_left_or_right(&mut self, right: bool) -> (String, bool) {
        let Some(track) = self.selected_track else { return ("clip not moved".into(), true) };
        let (t0, t1) = self.sel_secs();
        let rate = self.selected_rate();
        let Some(tr) = self.editor.state().tracks.get(track) else { return ("clip not moved".into(), true) };
        let f0 = au_core::secs_to_frames(t0, rate);
        let Some((idx, start)) = tr.clips.clips.iter().enumerate().find(|(_, c)| c.start <= f0 && f0 < c.end()).map(|(i, _)| (i, tr.clips.clip_start_frames(i))) else {
            return ("clip not moved".into(), true);
        };
        // ClipMenus: desiredT0 = viewInfo.OffsetTimeByPixels(t0, ±1), so horizontal scroll counts.
        let desired = xc::offset_time_by_pixels(t0, if right { 1 } else { -1 }, self.zoom.pixels_per_sec, self.zoom.h_offset) - t0;
        // TrackShifter::HintOffsetLarger: whole samples, at least one.
        let frames = ((desired.abs() * f64::from(rate)).ceil() as i64).max(1) * if right { 1 } else { -1 };
        // Clips may start before time 0, as in Audacity.
        let new_start = start + frames;
        let moved = new_start - start;
        if moved == 0 || self.editor.move_clip(track, idx, new_start).is_err() {
            return ("clip not moved".into(), true);
        }
        let d = moved as f64 / f64::from(rate);
        self.set_sel_secs((t0 + d).max(0.0), (t1 + d).max(0.0));
        (if right { "Moved clips to the right" } else { "Moved clips to the left" }.into(), true)
    }

    /// Dialogs for OutputGain / InputGain / SetPlaySpeed / device choosers.
    pub(super) fn extra_dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.extra_dialog.clone() else { return };
        let mut open = true;
        let mut close = false;
        let title = match &dialog {
            ExtraDialog::OutputGain => au_core::i18n::tr_ui("Playback Volume"),
            ExtraDialog::InputGain => au_core::i18n::tr_ui("Recording Volume"),
            ExtraDialog::PlaySpeed => au_core::i18n::tr_ui("Playback Speed"),
            ExtraDialog::Choose { title, .. } => au_core::i18n::tr_ui(title),
        };
        egui::Window::new(title).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            match &dialog {
                ExtraDialog::OutputGain => {
                    let mut v = self.master_gain.clamp(0.0, 1.0);
                    if ui.add(egui::Slider::new(&mut v, 0.0..=1.0)).changed() {
                        self.master_gain = v;
                        if let Some(p) = self.player.as_mut() {
                            p.handle.send(EngineCommand::SetMasterGain(v));
                        }
                    }
                }
                ExtraDialog::InputGain => {
                    ui.add(egui::Slider::new(&mut self.input_volume, 0.0..=1.0));
                }
                ExtraDialog::PlaySpeed => {
                    let mut s = self.play_speed;
                    if ui.add(egui::Slider::new(&mut s, 0.01..=3.0).suffix("x")).changed() {
                        self.set_play_speed(s);
                    }
                }
                ExtraDialog::Choose { key, items, current, .. } => {
                    let mut pick = None;
                    for (i, item) in items.iter().enumerate() {
                        let value = if key == "/AudioIO/RecordChannels" { (i + 1).to_string() } else { item.clone() };
                        if ui.selectable_label(*current == value, item).clicked() {
                            pick = Some(value);
                        }
                    }
                    if let Some(v) = pick {
                        self.prefs.set(key, au_core::prefs::PrefValue::Str(v));
                        self.save_prefs();
                        self.apply_prefs();
                        close = true;
                    }
                }
            }
            if ui.button(au_core::i18n::tr_ui("OK")).clicked() {
                close = true;
            }
        });
        if !open || close {
            self.extra_dialog = None;
        }
    }
}
