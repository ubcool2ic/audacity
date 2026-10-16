use std::collections::VecDeque;
use crate::extra_cmds::Stutter;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrubCmd {
    Seek(usize),
    Speed(f64),
    Pause,
    Play,
}

#[derive(Clone, Debug, Default)]
pub struct ScrubQueue {
    pub pending: VecDeque<Stutter>,
    pub current: Option<(Stutter, u64 /*start ms*/)>,
    pub playing: bool,
    pub last_s1: Option<i64>,
}

impl ScrubQueue {
    pub fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            current: None,
            playing: false,
            last_s1: None,
        }
    }

    pub fn push(&mut self, st: Stutter) {
        while self.pending.len() >= 4 {
            self.pending.pop_front();
        }
        self.pending.push_back(st);
    }

    pub fn next_start(&self, fallback: i64) -> i64 {
        self.pending
            .back()
            .map(|st| st.s1)
            .or_else(|| self.current.as_ref().map(|(st, _)| st.s1))
            .unwrap_or(fallback)
    }

    pub fn last_goal(&self) -> i64 {
        self.pending
            .back()
            .map(|st| st.goal)
            .or_else(|| self.current.as_ref().map(|(st, _)| st.goal))
            .unwrap_or(-1)
    }

    pub fn tick(&mut self, now_ms: u64, rate: f64) -> Vec<ScrubCmd> {
        let mut cmds = Vec::new();
        let rate = if rate > 0.0 { rate } else { 44100.0 };

        let need_next = match &self.current {
            Some((st, start_ms)) => {
                let wall_ms = (((st.duration + st.silence) as f64 / rate) * 1000.0).round() as u64;
                now_ms.saturating_sub(*start_ms) >= wall_ms
            }
            None => true,
        };

        if need_next {
            if let Some(st) = self.pending.pop_front() {
                self.current = Some((st, now_ms));
                if self.last_s1 != Some(st.s0) {
                    cmds.push(ScrubCmd::Seek(st.s0.max(0) as usize));
                }
                self.last_s1 = Some(st.s1);

                if st.duration > 0 && st.s1 != st.s0 {
                    let speed = (st.s1 - st.s0) as f64 / st.duration as f64;
                    cmds.push(ScrubCmd::Speed(speed));
                    if !self.playing {
                        self.playing = true;
                        cmds.push(ScrubCmd::Play);
                    }
                } else {
                    self.playing = false;
                    cmds.push(ScrubCmd::Pause);
                }
            } else if self.current.is_some() {
                self.current = None;
                if self.playing {
                    self.playing = false;
                    cmds.push(ScrubCmd::Pause);
                }
            }
        }

        cmds
    }

    #[allow(dead_code)]
    pub fn is_idle(&self) -> bool {
        self.pending.is_empty() && self.current.is_none()
    }

    pub fn clear(&mut self) {
        self.pending.clear();
        self.current = None;
        self.last_s1 = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chained_stutters_seek_once_then_speed_changes() {
        let mut q = ScrubQueue::new();
        let st1 = Stutter { s0: 0, s1: 100, duration: 100, silence: 0, goal: -1 };
        let st2 = Stutter { s0: 100, s1: 300, duration: 100, silence: 0, goal: -1 };
        q.push(st1);
        q.push(st2);

        let cmds1 = q.tick(0, 1000.0);
        assert_eq!(cmds1, vec![ScrubCmd::Seek(0), ScrubCmd::Speed(1.0), ScrubCmd::Play]);

        let cmds2 = q.tick(100, 1000.0);
        assert_eq!(cmds2, vec![ScrubCmd::Speed(2.0)]);
    }

    #[test]
    fn test_silent_stutter_pauses() {
        let mut q = ScrubQueue::new();
        let st1 = Stutter { s0: 0, s1: 100, duration: 100, silence: 0, goal: -1 };
        let silent = Stutter { s0: 100, s1: 100, duration: 0, silence: 100, goal: -1 };
        q.push(st1);
        q.push(silent);

        let _ = q.tick(0, 1000.0);
        let cmds = q.tick(100, 1000.0);
        assert_eq!(cmds, vec![ScrubCmd::Pause]);
    }

    #[test]
    fn test_timing_respects_silence() {
        let mut q = ScrubQueue::new();
        let st1 = Stutter { s0: 0, s1: 100, duration: 100, silence: 100, goal: -1 };
        let st2 = Stutter { s0: 100, s1: 200, duration: 100, silence: 0, goal: -1 };
        q.push(st1);
        q.push(st2);

        let cmds1 = q.tick(0, 1000.0);
        assert_eq!(cmds1, vec![ScrubCmd::Seek(0), ScrubCmd::Speed(1.0), ScrubCmd::Play]);

        let cmds_mid = q.tick(100, 1000.0);
        assert!(cmds_mid.is_empty());

        let cmds2 = q.tick(200, 1000.0);
        assert_eq!(cmds2, vec![ScrubCmd::Speed(1.0)]);
    }

    #[test]
    fn test_queue_cap() {
        let mut q = ScrubQueue::new();
        for i in 0..6 {
            q.push(Stutter { s0: i * 10, s1: (i + 1) * 10, duration: 10, silence: 0, goal: i });
        }
        assert_eq!(q.pending.len(), 4);
        assert_eq!(q.pending.front().unwrap().goal, 2);
        assert_eq!(q.pending.back().unwrap().goal, 5);
    }
}
