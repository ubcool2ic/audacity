//! Transport > Scrubbing: Scrub / Seek checks, the scrub ruler, and ruler-drag scrubbing
//! (Audacity 3.7.3 Scrubbing.cpp). Logic in crate::scrub; keyboard scrub is in extra_glue.rs.
use super::AudacityApp;
use crate::scrub::{self, ScrubMode, Scrubber};
use crate::scrub_queue::ScrubQueue;
use au_io::engine::EngineCommand;
use eframe::egui;
use std::time::Instant;

#[derive(Default)]
pub(crate) struct ScrubUi {
    pub scrub: bool,
    pub seek: bool,
    pub ruler_shown: bool,
    pub active: Option<Scrubber>,
    pub queue: Option<(crate::scrub_queue::ScrubQueue, std::time::Instant)>,
    epoch: Option<std::time::Instant>,
}

impl ScrubUi {
    fn now_ms(&mut self) -> u64 {
        self.epoch.get_or_insert_with(std::time::Instant::now).elapsed().as_millis() as u64
    }
}

impl AudacityApp {
    /// Pipe/menu commands "Scrub", "Seek", "ToggleScrubRuler".
    pub(super) fn scrub_command(&mut self, name: &str) -> Option<(String, bool)> {
        let s = &mut self.scrub_ui;
        match name {
            "Scrub" => scrub::toggle_exclusive(&mut s.scrub, &mut s.seek, ScrubMode::Scrub),
            "Seek" => scrub::toggle_exclusive(&mut s.scrub, &mut s.seek, ScrubMode::Seek),
            "ToggleScrubRuler" => s.ruler_shown = !s.ruler_shown,
            _ => return None,
        }
        Some((String::new(), true))
    }

    /// Height of the timeline including the scrub bar when shown.
    pub(super) fn ruler_height(&self) -> f32 {
        28.0 + if self.scrub_ui.ruler_shown { scrub::SCRUB_RULER_HEIGHT } else { 0.0 }
    }

    pub(super) fn paint_scrub_bar(&self, painter: &egui::Painter, rect: egui::Rect) {
        if !self.scrub_ui.ruler_shown {
            return;
        }
        let bar = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - scrub::SCRUB_RULER_HEIGHT), rect.right_bottom());
        let active = self.scrub_ui.active.is_some();
        painter.rect_filled(bar, 0.0, egui::Color32::from_gray(if active { 60 } else { 40 }));
    }

    /// Ruler pointer handling: a drag in the scrub zone (or anywhere on the ruler while
    /// Scrub/Seek is checked, or unpinned with "Always scrub un-pinned") scrubs or seeks.
    pub(super) fn ruler_scrub(&mut self, response: &egui::Response, rect: egui::Rect) {
        let end = self.project_duration();
        let time_at = |app: &Self, x: f32| app.zoom.x_to_time((x - rect.left()).max(0.0) as f64);
        if response.drag_started() || response.clicked() {
            let Some(pos) = response.interact_pointer_pos() else { return };
            let zone = scrub::in_scrub_zone(pos.y - rect.top(), rect.height(), self.scrub_ui.ruler_shown);
            let mode = scrub::should_scrub_on_ruler(self.scrub_ui.scrub || zone, self.scrub_ui.seek, self.settings.pinned_head, self.settings.unpinned_scrubbing);
            if let Some(mode) = mode {
                let t = time_at(self, pos.x).clamp(0.0, end);
                let speed = if mode == ScrubMode::Seek { 1.0 } else { 0.0 };
                self.extra_play_from(t, end, speed);
                self.scrub_ui.queue = Some((ScrubQueue::new(), Instant::now()));
                let now = self.scrub_ui.now_ms();
                self.scrub_ui.active = Some(Scrubber::start(mode, t, now));
            }
        }
        if self.scrub_ui.active.is_some() {
            if response.drag_stopped() || response.clicked() && !response.dragged() && self.player.is_none() {
                if let Some(s) = self.scrub_ui.active.as_mut() {
                    s.stop();
                }
                self.scrub_ui.active = None;
                self.scrub_ui.queue = None;
                self.stop_playback();
                return;
            }
            let Some(pos) = response.interact_pointer_pos().or(response.hover_pos()) else { return };
            let t = time_at(self, pos.x);
            let now = self.scrub_ui.now_ms();
            let rate = self.project_rate() as f64;
            let end_samples = (end * rate).round() as i64;
            let should_poll = self.scrub_ui.active.as_mut().map_or(false, |s| s.poll(t, now, end).is_some());
            let is_seek = self.scrub_ui.active.as_ref().map_or(false, |s| s.mode == ScrubMode::Seek);
            let player_pos = self.player.as_ref().map_or((t * rate).round() as i64, |p| p.position() as i64);

            let mut cmds = Vec::new();
            if let Some((queue, started_at)) = self.scrub_ui.queue.as_mut() {
                if should_poll {
                    let duration = ((rate * scrub::SCRUB_POLL_INTERVAL_MS as f64) / 1000.0).round() as i64;
                    let s0 = queue.next_start(player_pos);
                    let s1 = ((t * rate).round() as i64).clamp(0, end_samples);
                    let entry = if is_seek {
                        crate::extra_cmds::scrub_entry_init(
                            queue.last_goal(),
                            s0,
                            s1,
                            duration,
                            1.0,
                            crate::scrub::MAX_SCRUB_SPEED,
                            true,
                            (0.2 * rate).round() as i64,
                            0,
                            end_samples,
                        )
                    } else {
                        crate::extra_cmds::scrub_entry_init(
                            queue.last_goal(),
                            s0,
                            s1,
                            duration,
                            0.0,
                            crate::scrub::MAX_SCRUB_SPEED,
                            false,
                            duration,
                            0,
                            end_samples,
                        )
                    };
                    if let Some(entry) = entry {
                        queue.push(entry);
                    }
                }
                let elapsed_ms = started_at.elapsed().as_millis() as u64;
                cmds = queue.tick(elapsed_ms, rate);
            }
            if let Some(player) = self.player.as_mut() {
                for cmd in cmds {
                    match cmd {
                        crate::scrub_queue::ScrubCmd::Seek(sample) => {
                            let engine_frame = (sample as f64 * player.rate_ratio).round() as usize;
                            let _ = player.handle.send(EngineCommand::Seek(engine_frame));
                        }
                        crate::scrub_queue::ScrubCmd::Speed(speed) => {
                            let _ = player.handle.send(EngineCommand::SetSpeed(speed));
                        }
                        crate::scrub_queue::ScrubCmd::Pause => {
                            let _ = player.handle.send(EngineCommand::Pause);
                        }
                        crate::scrub_queue::ScrubCmd::Play => {
                            let _ = player.handle.send(EngineCommand::Play);
                        }
                    }
                }
            }
            response.ctx.request_repaint();
        }
    }

    fn extra_play_from(&mut self, t: f64, end: f64, speed: f64) {
        self.play_region(t, end.max(t), speed, false, None);
        // Scrubbing is open-ended: do not stop at the region end while the mouse is held.
        self.play_stop = None;
    }
}
