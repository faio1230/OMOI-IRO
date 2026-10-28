//! Session monotonic clock. Every input connection's events are stamped with
//! this clock inside the callback, so events from different connections share
//! one time axis. Backend timestamps are recorded next to it but never used
//! for cross-connection comparison.
//!
//! `MonotonicClock::fake()` returns a clock driven by a `FakeTime` handle so
//! Runtime tests (fake backend, scheduler) run without real time.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Debug)]
enum Source {
    Real(Instant),
    Fake(Arc<AtomicI64>),
}

#[derive(Clone, Debug)]
pub struct MonotonicClock {
    source: Source,
}

impl MonotonicClock {
    pub fn new() -> Self {
        Self { source: Source::Real(Instant::now()) }
    }

    /// A clock that only moves when the returned handle moves it (starts at 0).
    pub fn fake() -> (MonotonicClock, FakeTime) {
        let t = Arc::new(AtomicI64::new(0));
        (MonotonicClock { source: Source::Fake(t.clone()) }, FakeTime(t))
    }

    pub fn is_fake(&self) -> bool {
        matches!(self.source, Source::Fake(_))
    }

    /// Microseconds since this clock was created. Never goes backwards.
    pub fn now_us(&self) -> i64 {
        match &self.source {
            Source::Real(origin) => origin.elapsed().as_micros() as i64,
            Source::Fake(t) => t.load(Ordering::SeqCst),
        }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle that drives a fake `MonotonicClock`. Time only moves forward.
#[derive(Clone, Debug)]
pub struct FakeTime(Arc<AtomicI64>);

impl FakeTime {
    pub fn now_us(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }

    pub fn advance_us(&self, us: i64) -> i64 {
        assert!(us >= 0, "fake time never goes backwards");
        self.0.fetch_add(us, Ordering::SeqCst) + us
    }

    pub fn set_us(&self, us: i64) {
        let prev = self.0.swap(us, Ordering::SeqCst);
        assert!(us >= prev, "fake time never goes backwards ({prev} -> {us})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_clock_moves_only_with_its_handle() {
        let (c, t) = MonotonicClock::fake();
        assert!(c.is_fake());
        assert_eq!(c.now_us(), 0);
        t.advance_us(1500);
        assert_eq!(c.clone().now_us(), 1500);
        t.set_us(2000);
        assert_eq!(c.now_us(), 2000);
    }
}
