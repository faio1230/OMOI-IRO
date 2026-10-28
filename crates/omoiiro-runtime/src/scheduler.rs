//! Render scheduler (T05): frame `k` is due at `start + k / fps` on the
//! session monotonic clock (exact rational, no accumulated rounding). When
//! frames are late, the missed ones are skipped and the next render is for
//! the current position; nothing is rendered twice to catch up.
//!
//! 30 fps is the provisional rate (docs/07); the rate is a parameter.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderRate {
    pub num: u64,
    pub den: u64,
}

impl RenderRate {
    pub const FPS30: RenderRate = RenderRate { num: 30, den: 1 };

    pub fn period_us(self) -> f64 {
        self.den as f64 * 1e6 / self.num as f64
    }
}

/// One due frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    pub index: u64,
    pub due_us: i64,
    /// `now - due` (>= 0): scheduling jitter of this frame.
    pub late_us: i64,
    /// Frames skipped right before this one.
    pub skipped: u64,
}

#[derive(Clone, Debug)]
pub struct FrameScheduler {
    start_us: i64,
    rate: RenderRate,
    next: u64,
    pub ticks: u64,
    pub skipped: u64,
}

impl FrameScheduler {
    pub fn new(start_us: i64, rate: RenderRate) -> FrameScheduler {
        assert!(rate.num > 0 && rate.den > 0);
        FrameScheduler { start_us, rate, next: 0, ticks: 0, skipped: 0 }
    }

    pub fn rate(&self) -> RenderRate {
        self.rate
    }

    pub fn due_us(&self, index: u64) -> i64 {
        self.start_us + (index as u128 * self.rate.den as u128 * 1_000_000 / self.rate.num as u128) as i64
    }

    /// Index of the frame whose slot contains `now`.
    fn index_at(&self, now_us: i64) -> u64 {
        let el = (now_us - self.start_us).max(0) as u128;
        (el * self.rate.num as u128 / (self.rate.den as u128 * 1_000_000)) as u64
    }

    /// The frame to render now, if one is due.
    pub fn poll(&mut self, now_us: i64) -> Option<Tick> {
        if now_us < self.due_us(self.next) {
            return None;
        }
        let current = self.index_at(now_us).max(self.next);
        let skipped = current - self.next;
        self.next = current + 1;
        self.ticks += 1;
        self.skipped += skipped;
        let due_us = self.due_us(current);
        Some(Tick { index: current, due_us, late_us: now_us - due_us, skipped })
    }

    /// Microseconds until the next frame is due (0 if due now).
    pub fn until_next_us(&self, now_us: i64) -> i64 {
        (self.due_us(self.next) - now_us).max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thirty_fps_due_times_are_exact() {
        let s = FrameScheduler::new(1000, RenderRate::FPS30);
        assert_eq!(s.due_us(0), 1000);
        assert_eq!(s.due_us(1), 1000 + 33_333);
        assert_eq!(s.due_us(3), 1000 + 100_000);
        assert_eq!(s.due_us(30 * 3600), 1000 + 3_600_000_000);
    }

    #[test]
    fn late_frames_jump_to_the_current_time() {
        let mut s = FrameScheduler::new(0, RenderRate::FPS30);
        assert_eq!(s.poll(0), Some(Tick { index: 0, due_us: 0, late_us: 0, skipped: 0 }));
        assert_eq!(s.poll(10_000), None);
        assert_eq!(s.until_next_us(10_000), 23_333);
        assert_eq!(s.poll(34_000), Some(Tick { index: 1, due_us: 33_333, late_us: 667, skipped: 0 }));
        // A 200 ms stall: frames 2..6 are skipped, frame 7 (233.3 ms) is rendered once.
        assert_eq!(s.poll(240_000), Some(Tick { index: 7, due_us: 233_333, late_us: 6_667, skipped: 5 }));
        assert_eq!(s.poll(250_000), None);
        assert_eq!((s.ticks, s.skipped), (3, 5));
    }
}
