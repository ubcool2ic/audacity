//! Mouse scrubbing (Audacity 3.7.3 Scrubbing.cpp / ScrubState.cpp): scrub ruler zone,
//! Scrub / Seek modes and the "Always scrub un-pinned" rule. Pure logic; the GUI glue
//! lives in gui/scrub_glue.rs. Keyboard scrubbing (U/I) is separate (extra_cmds.rs).

pub const SCRUB_POLL_INTERVAL_MS: u64 = 50;
pub const MAX_SCRUB_SPEED: f64 = 32.0;
pub const MIN_SCRUB_SPEED: f64 = 0.01;
pub const SCRUB_RULER_HEIGHT: f32 = 14.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrubMode {
    Scrub,
    Seek,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrubAction {
    SetSpeed(f64),
    SeekTo(f64),
    Stop,
}

pub struct Scrubber {
    pub mode: ScrubMode,
    pub last_time: f64,
    pub last_ms: u64,
    pub active: bool,
}

impl Scrubber {
    pub fn start(mode: ScrubMode, time: f64, now_ms: u64) -> Self {
        Self { mode, last_time: time, last_ms: now_ms, active: true }
    }

    pub fn poll(&mut self, mouse_time: f64, now_ms: u64, end_time: f64) -> Option<ScrubAction> {
        if !self.active {
            return None;
        }
        let elapsed_ms = now_ms.saturating_sub(self.last_ms);
        if elapsed_ms < SCRUB_POLL_INTERVAL_MS {
            return None;
        }
        let time = mouse_time.clamp(0.0, end_time.max(0.0));
        let action = match self.mode {
            ScrubMode::Scrub => {
                let speed = ((time - self.last_time) / (elapsed_ms as f64 / 1000.0))
                    .clamp(-MAX_SCRUB_SPEED, MAX_SCRUB_SPEED);
                Some(ScrubAction::SetSpeed(if speed.abs() < MIN_SCRUB_SPEED { 0.0 } else { speed }))
            }
            ScrubMode::Seek => ((time - self.last_time).abs() > 1e-9).then_some(ScrubAction::SeekTo(time)),
        };
        self.last_time = time;
        self.last_ms = now_ms;
        action
    }

    pub fn stop(&mut self) -> ScrubAction {
        self.active = false;
        ScrubAction::Stop
    }
}

/// Which mode a ruler drag starts: Seek if checked, else Scrub if checked, else Scrub when the
/// play head is unpinned and /AudioIO/UnpinnedScrubbing ("Always scrub un-pinned") is on.
pub fn should_scrub_on_ruler(scrub_toggled: bool, seek_toggled: bool, pinned_head: bool, always_scrub_unpinned: bool) -> Option<ScrubMode> {
    if seek_toggled {
        Some(ScrubMode::Seek)
    } else if scrub_toggled || (!pinned_head && always_scrub_unpinned) {
        Some(ScrubMode::Scrub)
    } else {
        None
    }
}

pub fn in_scrub_zone(y_in_ruler: f32, ruler_height: f32, scrub_ruler_shown: bool) -> bool {
    scrub_ruler_shown && y_in_ruler >= ruler_height - SCRUB_RULER_HEIGHT && y_in_ruler <= ruler_height
}

/// Transport > Scrubbing > Scrub / Seek are mutually exclusive checks.
pub fn toggle_exclusive(scrub: &mut bool, seek: &mut bool, which: ScrubMode) {
    match which {
        ScrubMode::Scrub => { *scrub = !*scrub; *seek = false; }
        ScrubMode::Seek => { *seek = !*seek; *scrub = false; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_and_stop() {
        let mut s = Scrubber::start(ScrubMode::Scrub, 2.0, 100);
        assert!(s.active);
        assert_eq!(s.stop(), ScrubAction::Stop);
        assert!(!s.active);
        assert_eq!(s.poll(3.0, 200, 10.0), None);
    }

    #[test]
    fn poll_interval_gates_updates() {
        let mut s = Scrubber::start(ScrubMode::Scrub, 1.0, 100);
        assert_eq!(s.poll(2.0, 149, 10.0), None);
        assert_eq!((s.last_time, s.last_ms), (1.0, 100));
        assert_eq!(s.poll(2.0, 150, 10.0), Some(ScrubAction::SetSpeed(20.0)));
    }

    #[test]
    fn scrub_speed_clamps_and_min_speed_is_zero() {
        let mut s = Scrubber::start(ScrubMode::Scrub, 0.0, 0);
        assert_eq!(s.poll(100.0, 50, 200.0), Some(ScrubAction::SetSpeed(MAX_SCRUB_SPEED)));
        let mut s = Scrubber::start(ScrubMode::Scrub, 1.0, 0);
        assert_eq!(s.poll(1.0001, 50, 10.0), Some(ScrubAction::SetSpeed(0.0)));
        let mut s = Scrubber::start(ScrubMode::Scrub, 2.0, 0);
        assert_eq!(s.poll(1.0, 1000, 10.0), Some(ScrubAction::SetSpeed(-1.0)));
    }

    #[test]
    fn seek_clamps_and_ignores_no_ops() {
        let mut s = Scrubber::start(ScrubMode::Seek, 5.0, 0);
        assert_eq!(s.poll(20.0, 50, 10.0), Some(ScrubAction::SeekTo(10.0)));
        assert_eq!(s.poll(-2.0, 100, 10.0), Some(ScrubAction::SeekTo(0.0)));
        assert_eq!(s.poll(1e-10, 150, 10.0), None);
    }

    #[test]
    fn ruler_mode_precedence_and_zone() {
        assert_eq!(should_scrub_on_ruler(true, true, false, true), Some(ScrubMode::Seek));
        assert_eq!(should_scrub_on_ruler(true, false, true, false), Some(ScrubMode::Scrub));
        assert_eq!(should_scrub_on_ruler(false, false, false, true), Some(ScrubMode::Scrub));
        assert_eq!(should_scrub_on_ruler(false, false, true, true), None);
        assert_eq!(should_scrub_on_ruler(false, false, false, false), None);
        assert!(in_scrub_zone(26.0, 40.0, true));
        assert!(!in_scrub_zone(25.9, 40.0, true));
        assert!(!in_scrub_zone(30.0, 40.0, false));
    }

    #[test]
    fn exclusive_toggles() {
        let (mut scrub, mut seek) = (true, false);
        toggle_exclusive(&mut scrub, &mut seek, ScrubMode::Seek);
        assert!(!scrub && seek);
        toggle_exclusive(&mut scrub, &mut seek, ScrubMode::Scrub);
        assert!(scrub && !seek);
        toggle_exclusive(&mut scrub, &mut seek, ScrubMode::Scrub);
        assert!(!scrub && !seek);
    }
}
