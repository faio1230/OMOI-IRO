//! Runtime timing diagnostics (T07): fixed-memory histograms for frame
//! jitter, render time, LED send time and callback -> send latency, plus
//! drop counters. Recording is O(1) and allocation-free, so it can run on
//! the render / writer threads.

use serde::Serialize;
use serde_json::{json, Value};

const FINE: i64 = 1_000; // 1 us buckets below 1 ms
const COARSE_STEP: i64 = 10; // 10 us buckets up to 100 ms
const LIMIT: i64 = 100_000;
const BUCKETS: usize = FINE as usize + ((LIMIT - FINE) / COARSE_STEP) as usize;

/// Percentiles of a histogram for live display (microseconds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TimingSummary {
    pub n: u64,
    pub p50_us: i64,
    pub p95_us: i64,
    pub p99_us: i64,
    pub max_us: i64,
}

/// Histogram of microsecond values (exact below 1 ms, 10 us steps up to
/// 100 ms, then one overflow bucket; the maximum is kept exactly).
#[derive(Clone)]
pub struct Histogram {
    counts: Box<[u32]>,
    over: u64,
    n: u64,
    sum: i128,
    max: i64,
}

impl Default for Histogram {
    fn default() -> Self {
        Histogram { counts: vec![0u32; BUCKETS].into_boxed_slice(), over: 0, n: 0, sum: 0, max: 0 }
    }
}

impl std::fmt::Debug for Histogram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Histogram(n={}, p95={:?}, max={})", self.n, self.percentile(95.0), self.max)
    }
}

fn bucket(us: i64) -> Option<usize> {
    if us < FINE {
        Some(us as usize)
    } else if us < LIMIT {
        Some(FINE as usize + ((us - FINE) / COARSE_STEP) as usize)
    } else {
        None
    }
}

/// Upper bound of a bucket (conservative percentile).
fn bucket_upper(i: usize) -> i64 {
    if (i as i64) < FINE {
        i as i64
    } else {
        FINE + (i as i64 - FINE + 1) * COARSE_STEP - 1
    }
}

impl Histogram {
    pub fn record(&mut self, us: i64) {
        let us = us.max(0);
        self.n += 1;
        self.sum += us as i128;
        self.max = self.max.max(us);
        match bucket(us) {
            Some(i) => self.counts[i] = self.counts[i].saturating_add(1),
            None => self.over += 1,
        }
    }

    pub fn count(&self) -> u64 {
        self.n
    }

    pub fn max(&self) -> i64 {
        self.max
    }

    pub fn mean(&self) -> Option<f64> {
        (self.n > 0).then(|| self.sum as f64 / self.n as f64)
    }

    /// `p`-th percentile (nearest rank, bucket upper bound, capped at max).
    pub fn percentile(&self, p: f64) -> Option<i64> {
        if self.n == 0 {
            return None;
        }
        let rank = ((p / 100.0) * self.n as f64).ceil().max(1.0) as u64;
        let mut seen = 0u64;
        for (i, c) in self.counts.iter().enumerate() {
            seen += *c as u64;
            if seen >= rank {
                return Some(bucket_upper(i).min(self.max));
            }
        }
        Some(self.max)
    }

    pub fn summary(&self) -> TimingSummary {
        TimingSummary {
            n: self.n,
            p50_us: self.percentile(50.0).unwrap_or(0),
            p95_us: self.percentile(95.0).unwrap_or(0),
            p99_us: self.percentile(99.0).unwrap_or(0),
            max_us: if self.n > 0 { self.max } else { 0 },
        }
    }

    pub fn summary_json(&self) -> Value {
        json!({
            "n": self.n,
            "mean_us": self.mean().map(|m| (m * 10.0).round() / 10.0),
            "p50_us": self.percentile(50.0),
            "p95_us": self.percentile(95.0),
            "p99_us": self.percentile(99.0),
            "max_us": if self.n > 0 { Some(self.max) } else { None },
        })
    }

    pub fn merge(&mut self, other: &Histogram) {
        for (a, b) in self.counts.iter_mut().zip(other.counts.iter()) {
            *a = a.saturating_add(*b);
        }
        self.over += other.over;
        self.n += other.n;
        self.sum += other.sum;
        self.max = self.max.max(other.max);
    }
}

/// Render-loop measurements.
#[derive(Clone, Debug, Default)]
pub struct RenderPerf {
    /// `now - due` of each rendered frame.
    pub jitter: Histogram,
    /// Time spent evaluating a frame (render + quantize).
    pub render: Histogram,
    pub frames: u64,
    pub skipped_frames: u64,
}

/// LED writer measurements.
#[derive(Clone, Debug, Default)]
pub struct SendPerf {
    /// Time spent in `send` for one frame.
    pub send: Histogram,
    /// Render start -> LED send done.
    pub frame_latency: Histogram,
    pub frames: u64,
    pub errors: u64,
}

/// Operation-route measurements.
#[derive(Clone, Debug, Default)]
pub struct ControlPerf {
    /// Ingress callback stamp -> control send done.
    pub latency: Histogram,
    pub sent: u64,
    pub errors: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_exact_below_1ms_and_bounded_above() {
        let mut h = Histogram::default();
        for v in 1..=100 {
            h.record(v);
        }
        assert_eq!(h.percentile(50.0), Some(50));
        assert_eq!(h.percentile(95.0), Some(95));
        assert_eq!(h.percentile(100.0), Some(100));
        assert_eq!(h.mean(), Some(50.5));

        let mut h = Histogram::default();
        h.record(4_321);
        assert_eq!(h.percentile(95.0), Some(4_321), "capped at the exact max");
        h.record(4_329);
        h.record(250_000); // overflow bucket
        assert_eq!(h.percentile(50.0), Some(4_329));
        assert_eq!(h.percentile(99.0), Some(250_000));
        assert_eq!(h.max(), 250_000);
        let mut m = Histogram::default();
        m.merge(&h);
        assert_eq!(m.count(), 3);
        assert_eq!(Histogram::default().percentile(95.0), None);
    }
}
