//! How often frames go to the terminal.
//!
//! The runner draws at the full rate by default -- 60 fps, or whatever
//! higher rate is configured -- and steps down to 30 only when a terminal
//! cannot keep up: when encoding and writing a frame keeps costing more than
//! the frame budget (Sixel on a large window, say), or when output backs up
//! (a slow SSH link). It steps back up once there is headroom again. The
//! decision is made per session from measured costs, never from the
//! terminal's name, and an explicit rate in the configuration turns the
//! adaptation off.

use std::collections::VecDeque;
use std::time::Duration;

/// The rate frames are throttled to when the full rate cannot be held.
pub const THROTTLED_FPS: u32 = 30;

/// Frames looked at when deciding to step down.
const WINDOW: usize = 10;
/// Of the last [WINDOW] frames, how many must be over budget to step down.
/// Most of them, so one slow frame (a font loading, a GC) never throttles.
const OVER_TO_STEP_DOWN: usize = 6;
/// A frame is over budget above this share of the full-rate interval: the
/// write is not the only thing the frame interval has to hold.
const OVER_BUDGET_SHARE: f64 = 0.9;
/// A frame has headroom below this share of the full-rate interval.
const HEADROOM_SHARE: f64 = 0.5;
/// Consecutive frames with headroom before stepping back up: about two
/// seconds at 30 fps, long enough not to oscillate.
const HEADROOM_TO_STEP_UP: u32 = 60;

/// One frame's measured cost.
#[derive(Debug, Clone, Copy)]
pub struct FrameCost {
    /// Encoding plus writing the frame.
    pub duration: Duration,
    /// Whether output was backing up: bytes still queued for the terminal
    /// when the frame was written, or a write that blocked.
    pub backlog: bool,
}

/// Chooses the frame rate from measured frame costs.
#[derive(Debug, Clone)]
pub struct FramePacer {
    full_fps: u32,
    fixed: bool,
    current: u32,
    recent_over: VecDeque<bool>,
    headroom_streak: u32,
}

impl FramePacer {
    /// Adapts between `full_fps` (the display rate, 60 unless configured)
    /// and [THROTTLED_FPS].
    pub fn adaptive(full_fps: u32) -> Self {
        assert!(full_fps > 0, "a frame rate is at least 1");
        FramePacer {
            full_fps,
            fixed: false,
            current: full_fps,
            recent_over: VecDeque::with_capacity(WINDOW),
            headroom_streak: 0,
        }
    }

    /// Always `fps`: the configuration named a rate.
    pub fn fixed(fps: u32) -> Self {
        FramePacer { fixed: true, ..FramePacer::adaptive(fps) }
    }

    pub fn fps(&self) -> u32 {
        self.current
    }

    /// The time between frames at the current rate.
    pub fn interval(&self) -> Duration {
        Duration::from_nanos(1_000_000_000 / self.current as u64)
    }

    /// Records a frame and returns the new rate when it changed.
    pub fn record(&mut self, cost: FrameCost) -> Option<u32> {
        if self.fixed || self.full_fps <= THROTTLED_FPS {
            return None;
        }
        // The budget is always the full rate's: the question is whether the
        // full rate could be held, not whether the throttled one is.
        let budget = 1.0 / self.full_fps as f64;
        let seconds = cost.duration.as_secs_f64();
        let over = cost.backlog || seconds > budget * OVER_BUDGET_SHARE;
        let headroom = !cost.backlog && seconds < budget * HEADROOM_SHARE;

        if self.current == self.full_fps {
            if self.recent_over.len() == WINDOW {
                self.recent_over.pop_front();
            }
            self.recent_over.push_back(over);
            if self.recent_over.iter().filter(|&&slow| slow).count() >= OVER_TO_STEP_DOWN {
                self.current = THROTTLED_FPS;
                self.recent_over.clear();
                self.headroom_streak = 0;
                return Some(self.current);
            }
            None
        } else {
            self.headroom_streak = if headroom { self.headroom_streak + 1 } else { 0 };
            if self.headroom_streak >= HEADROOM_TO_STEP_UP {
                self.current = self.full_fps;
                self.headroom_streak = 0;
                self.recent_over.clear();
                return Some(self.current);
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64, backlog: bool) -> FrameCost {
        FrameCost { duration: Duration::from_millis(value), backlog }
    }

    #[test]
    fn it_starts_at_the_full_rate() {
        assert_eq!(FramePacer::adaptive(60).fps(), 60);
        assert_eq!(FramePacer::adaptive(120).fps(), 120);
        assert_eq!(FramePacer::adaptive(60).interval(), Duration::from_nanos(16_666_666));
    }

    #[test]
    fn cheap_frames_stay_at_the_full_rate() {
        let mut pacer = FramePacer::adaptive(60);
        for _ in 0..500 {
            assert_eq!(pacer.record(ms(4, false)), None);
        }
        assert_eq!(pacer.fps(), 60);
    }

    #[test]
    fn one_slow_frame_does_not_throttle() {
        let mut pacer = FramePacer::adaptive(60);
        for index in 0..100 {
            let cost = if index % 10 == 0 { ms(40, false) } else { ms(4, false) };
            assert_eq!(pacer.record(cost), None);
        }
    }

    #[test]
    fn frames_that_keep_missing_the_budget_step_down_to_30() {
        let mut pacer = FramePacer::adaptive(60);
        let mut changes = Vec::new();
        for _ in 0..10 {
            if let Some(fps) = pacer.record(ms(25, false)) {
                changes.push(fps);
            }
        }
        assert_eq!(changes, vec![30]);
        assert_eq!(pacer.fps(), 30);
        assert_eq!(pacer.interval(), Duration::from_nanos(33_333_333));
    }

    #[test]
    fn it_steps_down_on_the_sixth_slow_frame_not_before() {
        let mut pacer = FramePacer::adaptive(60);
        for _ in 0..5 {
            assert_eq!(pacer.record(ms(20, false)), None);
        }
        assert_eq!(pacer.record(ms(20, false)), Some(30));
    }

    #[test]
    fn output_backing_up_steps_down_even_when_encoding_is_cheap() {
        // A slow SSH link: the encoder is fast, the bytes pile up.
        let mut pacer = FramePacer::adaptive(60);
        let mut stepped = None;
        for _ in 0..10 {
            stepped = stepped.or(pacer.record(ms(2, true)));
        }
        assert_eq!(stepped, Some(30));
    }

    #[test]
    fn it_steps_back_up_after_sustained_headroom_and_not_sooner() {
        let mut pacer = FramePacer::adaptive(60);
        for _ in 0..10 {
            pacer.record(ms(30, false));
        }
        assert_eq!(pacer.fps(), 30);
        for _ in 0..59 {
            assert_eq!(pacer.record(ms(3, false)), None);
        }
        assert_eq!(pacer.record(ms(3, false)), Some(60));
    }

    #[test]
    fn a_slow_frame_restarts_the_headroom_count() {
        let mut pacer = FramePacer::adaptive(60);
        for _ in 0..10 {
            pacer.record(ms(30, false));
        }
        for _ in 0..50 {
            pacer.record(ms(3, false));
        }
        pacer.record(ms(12, false)); // under budget but no headroom
        for _ in 0..59 {
            assert_eq!(pacer.record(ms(3, false)), None);
        }
        assert_eq!(pacer.record(ms(3, false)), Some(60));
    }

    #[test]
    fn after_stepping_up_it_needs_fresh_evidence_to_step_down_again() {
        let mut pacer = FramePacer::adaptive(60);
        for _ in 0..10 {
            pacer.record(ms(30, false));
        }
        for _ in 0..60 {
            pacer.record(ms(3, false));
        }
        assert_eq!(pacer.fps(), 60);
        // The slow frames from before the step up must not count.
        for _ in 0..5 {
            assert_eq!(pacer.record(ms(30, false)), None);
        }
    }

    #[test]
    fn a_fixed_rate_never_adapts() {
        let mut pacer = FramePacer::fixed(24);
        for _ in 0..100 {
            assert_eq!(pacer.record(ms(200, true)), None);
        }
        assert_eq!(pacer.fps(), 24);
    }

    #[test]
    fn the_budget_follows_the_full_rate() {
        // 10 ms is fine at 60 fps (16.7 ms budget) but over budget at 120.
        let mut sixty = FramePacer::adaptive(60);
        let mut one_twenty = FramePacer::adaptive(120);
        let mut stepped = None;
        for _ in 0..10 {
            assert_eq!(sixty.record(ms(10, false)), None);
            stepped = stepped.or(one_twenty.record(ms(10, false)));
        }
        assert_eq!(stepped, Some(30));
    }

    #[test]
    fn a_full_rate_at_or_below_30_has_nothing_to_step_down_to() {
        let mut pacer = FramePacer::adaptive(30);
        for _ in 0..50 {
            assert_eq!(pacer.record(ms(100, true)), None);
        }
        assert_eq!(pacer.fps(), 30);
    }
}
