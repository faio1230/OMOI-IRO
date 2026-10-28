//! Offline generator replay and analysis of an `mtc-out` recording
//! (`mtc-out-report`): Quarter Frame timing, receiver relock after Full
//! Frames, label / position differences.
//!
//! Moved from tools/p0-probe/src/mtc_out.rs in M1.

use crate::diagnostics::bridge_report::{dist, Dist};
use crate::diagnostics::record::{midi_bytes, Record};
use crate::sync::bridge::{BridgeDecoder, BridgeHealth, BridgeParams, BridgeSample, BridgeTransport};
use crate::sync::mtc::{FrameRate, MtcDecoder, MtcEvent, Params, SyncHealth};
use crate::sync::mtc_out::{desired_from, GenOut, MtcGen};

// ---------------------------------------------------------- offline replay

/// Drive the decoder and the generator offline from recorded bridge samples
/// (their arrival times), stepping the generator every `step_us`. Returns the
/// Full Frames as `(t_us, reason, label)` and the Quarter Frame count. Used to
/// check generator behaviour against real recordings; not a MIDI measurement.
pub fn simulate(records: &[Record], origin_plus_offset_us: f64, rate: FrameRate, step_us: i64) -> (Vec<(i64, &'static str, String)>, u64, Vec<i64>) {
    let samples: Vec<&Record> = records.iter().filter(|r| r.source == "bridge" && r.kind == "sample").collect();
    let mut dec = BridgeDecoder::new(BridgeParams::default());
    let mut gen = MtcGen::new(rate).expect("supported rate");
    let mut ffs = Vec::new();
    let mut qf_stops = Vec::new();
    let mut ev = Vec::new();
    let mut out = Vec::new();
    let Some(first) = samples.first() else { return (ffs, 0, qf_stops) };
    let end = samples.last().unwrap().t_mono_us + 1_000_000;
    let mut i = 0;
    let mut t = first.t_mono_us;
    let mut last_qf: Option<i64> = None;
    while t <= end {
        while i < samples.len() && samples[i].t_mono_us <= t {
            ev.clear();
            dec.feed(BridgeSample::from_payload(samples[i].t_mono_us, &samples[i].payload), &mut ev);
            i += 1;
        }
        let snap = dec.snapshot(t);
        out.clear();
        gen.step(t as f64, desired_from(&snap, origin_plus_offset_us, t), snap.stats.discontinuities, &mut out);
        for o in &out {
            match o {
                GenOut::FullFrame { label, reason, .. } => ffs.push((t, *reason, label.to_string())),
                GenOut::Qf { .. } => last_qf = Some(t),
            }
        }
        // Record where a Quarter Frame run ended (no QF for > 50 ms).
        if let Some(q) = last_qf {
            if t - q > 50_000 {
                qf_stops.push(q);
                last_qf = None;
            }
        }
        t += step_us;
    }
    (ffs, gen.stats.quarter_frames, qf_stops)
}

// ------------------------------------------------------------------ report

#[derive(Clone, Debug, PartialEq)]
pub struct MtcOutReport {
    pub quarter_frames: usize,
    /// Send time minus scheduled time, ms.
    pub qf_late_ms: Option<Dist>,
    /// Interval between consecutive Quarter Frames inside a run, ms.
    pub qf_interval_ms: Option<Dist>,
    /// `(t_us, reason, label)` of each Full Frame.
    pub full_frames: Vec<(i64, String, String)>,
    /// Per start / jump / resync Full Frame: time to the receiver's next lock, ms (None: no lock seen).
    pub relock_ms: Vec<(i64, String, Option<f64>)>,
    /// Sent piece-0 label minus the source position at send time, ms.
    pub label_minus_source_ms: Option<Dist>,
    /// Received (verify) MTC position minus the bridge label position, at each playing bridge sample, ms.
    pub received_minus_bridge_ms: Option<Dist>,
}

pub fn analyse(records: &[Record]) -> MtcOutReport {
    let origin_plus_offset_us = records
        .iter()
        .find(|r| r.source == "meta" && r.kind == "start")
        .map(|r| r.payload["origin_us"].as_f64().unwrap_or(0.0) + r.payload["input_offset_ms"].as_f64().unwrap_or(0.0) * 1000.0)
        .unwrap_or(0.0);
    let mut late = Vec::new();
    let mut intervals = Vec::new();
    let mut last_qf: Option<i64> = None;
    let mut full_frames = Vec::new();
    let mut label_diff = Vec::new();
    let mut pending: Vec<(i64, String)> = Vec::new();
    let mut relock = Vec::new();
    let mut rx_diff = Vec::new();
    let mut mtc = MtcDecoder::new(Params::default());
    let mut bridge = BridgeDecoder::new(BridgeParams::default());
    let (mut mev, mut bev) = (Vec::new(), Vec::new());
    let mut qfs = 0;
    for r in records {
        match (r.source.as_str(), r.kind.as_str()) {
            ("mtc_out", "send") => {
                let p = &r.payload;
                if p["type"] == "qf" {
                    qfs += 1;
                    if let Some(l) = p["late_us"].as_f64() {
                        late.push(l / 1000.0);
                    }
                    if let Some(prev) = last_qf {
                        intervals.push((r.t_mono_us - prev) as f64 / 1000.0);
                    }
                    last_qf = Some(r.t_mono_us);
                    if let (Some(lu), Some(sp)) = (p["label_us"].as_f64(), p["src_pos_us"].as_f64()) {
                        label_diff.push((lu - sp) / 1000.0);
                    }
                } else {
                    last_qf = None;
                    let reason = p["reason"].as_str().unwrap_or("").to_string();
                    full_frames.push((r.t_mono_us, reason.clone(), p["label"].as_str().unwrap_or("").to_string()));
                    // A new Full Frame supersedes a pending one without a lock.
                    for (t, why) in pending.drain(..) {
                        relock.push((t, why, None));
                    }
                    if matches!(reason.as_str(), "start" | "jump" | "resync") {
                        pending.push((r.t_mono_us, reason));
                    }
                }
            }
            ("mtc", "midi") => {
                if let Some(b) = midi_bytes(r) {
                    mev.clear();
                    mtc.feed(r.t_mono_us, &b, &mut mev);
                    if mev.iter().any(|e| matches!(e, MtcEvent::Locked { .. })) {
                        for (t, why) in pending.drain(..) {
                            relock.push((t, why, Some((r.t_mono_us - t) as f64 / 1000.0)));
                        }
                    }
                }
            }
            ("bridge", "sample") => {
                bev.clear();
                bridge.feed(BridgeSample::from_payload(r.t_mono_us, &r.payload), &mut bev);
                let b = bridge.snapshot(r.t_mono_us);
                let m = mtc.snapshot(r.t_mono_us);
                if b.health == BridgeHealth::Locked && b.transport == BridgeTransport::Playing && m.health == SyncHealth::Locked {
                    if let (Some(ms), Some(p)) = (b.song_ms, m.position_us) {
                        rx_diff.push((p as f64 - (ms * 1000.0 + origin_plus_offset_us)) / 1000.0);
                    }
                }
            }
            _ => {}
        }
    }
    for (t, why) in pending.drain(..) {
        relock.push((t, why, None));
    }
    MtcOutReport {
        quarter_frames: qfs,
        qf_late_ms: dist(&late),
        qf_interval_ms: dist(&intervals),
        full_frames,
        relock_ms: relock,
        label_minus_source_ms: dist(&label_diff),
        received_minus_bridge_ms: dist(&rx_diff),
    }
}
