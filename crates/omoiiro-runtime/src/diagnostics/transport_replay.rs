//! Offline transport replay: feed a recording's bridge samples and MIDI Clock
//! messages (same process, same monotonic clock) into a `Transport` and
//! sample its snapshot on a fixed grid. Used by the regression tests; not a
//! new measurement.

use crate::diagnostics::record::{midi_bytes, Record};
use crate::sync::bridge::BridgeSample;
use crate::sync::transport::{Transport, TransportConfig, TransportSnapshot};

/// Snapshots every `step_us` from the first to the last input (+ `tail_us`).
pub fn replay_transport(records: &[Record], cfg: TransportConfig, step_us: i64, tail_us: i64) -> Vec<TransportSnapshot> {
    let mut t = Transport::new(cfg);
    let inputs: Vec<&Record> = records
        .iter()
        .filter(|r| (r.source == "bridge" && r.kind == "sample") || (matches!(r.source.as_str(), "clock" | "sync") && r.kind == "midi"))
        .collect();
    let mut out = Vec::new();
    let (Some(first), Some(last)) = (inputs.first(), inputs.last()) else { return out };
    let end = last.t_mono_us + tail_us;
    let (mut bev, mut cev) = (Vec::new(), Vec::new());
    let mut i = 0;
    let mut now = first.t_mono_us;
    while now <= end {
        while i < inputs.len() && inputs[i].t_mono_us <= now {
            let r = inputs[i];
            if r.source == "bridge" {
                bev.clear();
                t.feed_bridge(BridgeSample::from_payload(r.t_mono_us, &r.payload), &mut bev);
            } else if let Some(b) = midi_bytes(r) {
                cev.clear();
                t.feed_clock(r.t_mono_us, &b, &mut cev);
            }
            i += 1;
        }
        out.push(t.snapshot(now));
        now += step_us;
    }
    out
}
