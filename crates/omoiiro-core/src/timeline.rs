//! Active clip search, local time and fades (task C05).
//!
//! Clips are half-open intervals `[start, start + duration)`. A clip's local
//! time is `show_time - start`; a continuous layer's local time is the show
//! time itself and it is active while the show time is non-negative. When
//! clips overlap inside a layer they are composited in array order.

use crate::model::Clip;
use crate::time::ShowTimeUs;

/// One clip prepared for search. `order` is its index in the layer's array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipSpan {
    pub start: u64,
    pub end: u64,
    pub fade_in: u64,
    pub fade_out: u64,
    pub order: usize,
}

impl ClipSpan {
    pub fn from_clip(c: &Clip, order: usize) -> ClipSpan {
        ClipSpan { start: c.start_us, end: c.end_us(), fade_in: c.fade_in_us, fade_out: c.fade_out_us, order }
    }

    pub fn contains(&self, t: ShowTimeUs) -> bool {
        t >= 0 && (t as u64) >= self.start && (t as u64) < self.end
    }

    /// Local time of an active clip.
    pub fn local_us(&self, t: ShowTimeUs) -> u64 {
        t as u64 - self.start
    }

    /// Fade factor 0..=1 (fade in from the start, fade out to the end).
    pub fn fade(&self, t: ShowTimeUs) -> f64 {
        let local = self.local_us(t);
        let remaining = self.end - t as u64;
        let fin = if self.fade_in > 0 { (local as f64 / self.fade_in as f64).min(1.0) } else { 1.0 };
        let fout = if self.fade_out > 0 { (remaining as f64 / self.fade_out as f64).min(1.0) } else { 1.0 };
        fin * fout
    }
}

/// Index over a layer's clips: sorted by start, with a running maximum of
/// ends, so a lookup scans only clips that can still be active.
#[derive(Clone, Debug, Default)]
pub struct ClipIndex {
    spans: Vec<ClipSpan>,
    max_end: Vec<u64>,
}

impl ClipIndex {
    pub fn new(clips: &[Clip]) -> ClipIndex {
        let mut spans: Vec<ClipSpan> = clips.iter().enumerate().map(|(i, c)| ClipSpan::from_clip(c, i)).collect();
        spans.sort_by_key(|s| (s.start, s.order));
        let mut max_end = Vec::with_capacity(spans.len());
        let mut m = 0;
        for s in &spans {
            m = m.max(s.end);
            max_end.push(m);
        }
        ClipIndex { spans, max_end }
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Write the clips active at `t` into `out` in array order. `out` must
    /// have capacity for `len()` entries so this never allocates.
    pub fn active(&self, t: ShowTimeUs, out: &mut Vec<ClipSpan>) {
        out.clear();
        if t < 0 {
            return;
        }
        let tu = t as u64;
        let upper = self.spans.partition_point(|s| s.start <= tu);
        let mut i = upper;
        while i > 0 {
            i -= 1;
            if self.max_end[i] <= tu {
                break;
            }
            let s = self.spans[i];
            if s.end > tu {
                out.push(s);
            }
        }
        out.sort_unstable_by_key(|s| s.order);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: &str, start: u64, dur: u64, fi: u64, fo: u64) -> Clip {
        Clip { id: id.into(), preset_id: "p".into(), start_us: start, duration_us: dur, fade_in_us: fi, fade_out_us: fo }
    }

    fn orders(ix: &ClipIndex, t: i64) -> Vec<usize> {
        let mut v = Vec::with_capacity(ix.len());
        ix.active(t, &mut v);
        v.iter().map(|s| s.order).collect()
    }

    #[test]
    fn half_open_intervals_and_array_order() {
        // Array order differs from start order; one long clip spans the others.
        let ix = ClipIndex::new(&[clip("b", 1_000, 1_000, 0, 0), clip("long", 0, 10_000, 0, 0), clip("a", 500, 1_000, 0, 0)]);
        assert_eq!(orders(&ix, -1), Vec::<usize>::new());
        assert_eq!(orders(&ix, 0), vec![1]);
        assert_eq!(orders(&ix, 999), vec![1, 2]);
        assert_eq!(orders(&ix, 1_000), vec![0, 1, 2]);
        assert_eq!(orders(&ix, 1_500), vec![0, 1]); // "a" ended at 1500 (exclusive)
        assert_eq!(orders(&ix, 2_000), vec![1]);
        assert_eq!(orders(&ix, 10_000), Vec::<usize>::new());
    }

    #[test]
    fn fades() {
        let s = ClipSpan::from_clip(&clip("c", 1_000_000, 10_000_000, 1_000_000, 2_000_000), 0);
        assert_eq!(s.fade(1_000_000), 0.0);
        assert_eq!(s.fade(1_500_000), 0.5);
        assert_eq!(s.fade(5_000_000), 1.0);
        assert_eq!(s.fade(10_000_000), 0.5);
        assert!((s.fade(10_999_999) - 0.0000005).abs() < 1e-9);
        let no_fade = ClipSpan::from_clip(&clip("c", 0, 10, 0, 0), 0);
        assert_eq!(no_fade.fade(0), 1.0);
    }
}
