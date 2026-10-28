//! Replay a JSONL recording through the decoders. Pure with respect to MIDI
//! I/O; the recorded monotonic times drive the decoders.

use crate::diagnostics::record::{midi_bytes, Record};
use crate::diagnostics::report::{clock_event_line, mtc_event_is_notable, mtc_event_line};
use crate::sync::bridge::{event_line as bridge_event_line, BridgeDecoder, BridgeParams, BridgeSample, BridgeSnapshot};
use crate::sync::clock::{ClockDecoder, Transport};
use crate::sync::mtc::{MtcDecoder, MtcEvent, MtcSnapshot, Params};

pub struct ReplaySummary {
    pub records: usize,
    pub synthetic: bool,
    pub run_id: Option<String>,
    pub first_t_us: Option<i64>,
    pub last_t_us: Option<i64>,
    pub mtc_messages: u64,
    pub clock_messages: u64,
    pub other_messages: u64,
    pub mtc_final: Option<MtcSnapshot>,
    pub clock_final: Option<crate::sync::clock::ClockSnapshot>,
    /// Notable events as `(t_us, source, text)`.
    pub events: Vec<(i64, &'static str, String)>,
    /// `ff_continue_when_clock_playing` was on.
    pub ff_continue: bool,
    /// Per lock after a continued Full Frame: `(t_us, label, label - predicted in us)`.
    pub freewheel_locks: Vec<(i64, String, i64)>,
    pub bridge_samples: u64,
    pub bridge_final: Option<BridgeSnapshot>,
}

pub fn replay(records: &[Record]) -> ReplaySummary {
    replay_with(records, Params::default())
}

pub fn replay_with(records: &[Record], params: Params) -> ReplaySummary {
    let mut mtc = MtcDecoder::new(params);
    let mut clock = ClockDecoder::new();
    let mut summary = ReplaySummary {
        records: records.len(),
        synthetic: records.iter().any(|r| r.kind == "synthetic"),
        run_id: records.first().map(|r| r.run_id.clone()),
        first_t_us: records.first().map(|r| r.t_mono_us),
        last_t_us: records.last().map(|r| r.t_mono_us),
        mtc_messages: 0,
        clock_messages: 0,
        other_messages: 0,
        mtc_final: None,
        clock_final: None,
        events: Vec::new(),
        ff_continue: params.ff_continue_when_clock_playing,
        freewheel_locks: Vec::new(),
        bridge_samples: 0,
        bridge_final: None,
    };
    let mut bridge = BridgeDecoder::new(BridgeParams::default());
    let mut bridge_events = Vec::new();
    let mut mtc_events = Vec::new();
    let mut clock_events = Vec::new();
    for r in records {
        if r.source == "bridge" && r.kind == "sample" {
            summary.bridge_samples += 1;
            bridge_events.clear();
            bridge.feed(BridgeSample::from_payload(r.t_mono_us, &r.payload), &mut bridge_events);
            for e in &bridge_events {
                summary.events.push((r.t_mono_us, "bridge", bridge_event_line(e)));
            }
            continue;
        }
        let Some(bytes) = midi_bytes(r) else {
            continue;
        };
        // Stall detection is time based: check it at every recorded message,
        // whichever source it came from.
        clock_events.clear();
        clock.poll(r.t_mono_us, &mut clock_events);
        for e in &clock_events {
            summary.events.push((r.t_mono_us, "clock", clock_event_line(e)));
        }
        // `sync` records come from one port that carries both MTC and Clock.
        let to_mtc = matches!(r.source.as_str(), "mtc" | "sync");
        let to_clock = matches!(r.source.as_str(), "clock" | "sync");
        if !to_mtc && !to_clock {
            summary.other_messages += 1;
            continue;
        }
        if to_mtc {
            summary.mtc_messages += 1;
            mtc.set_clock_playing(r.t_mono_us, clock.snapshot(r.t_mono_us).transport == Transport::Playing);
            mtc_events.clear();
            mtc.feed(r.t_mono_us, &bytes, &mut mtc_events);
            for e in &mtc_events {
                if let MtcEvent::FreewheelLocked { label, delta_us, .. } = e {
                    summary.freewheel_locks.push((r.t_mono_us, label.to_string(), *delta_us));
                }
                if mtc_event_is_notable(e) {
                    summary.events.push((r.t_mono_us, "mtc", mtc_event_line(e)));
                }
            }
        }
        if to_clock {
            summary.clock_messages += 1;
            clock_events.clear();
            clock.feed(r.t_mono_us, &bytes, &mut clock_events);
            for e in &clock_events {
                summary.events.push((r.t_mono_us, "clock", clock_event_line(e)));
            }
        }
    }
    let now = summary.last_t_us.unwrap_or(0);
    clock_events.clear();
    clock.poll(now, &mut clock_events);
    for e in &clock_events {
        summary.events.push((now, "clock", clock_event_line(e)));
    }
    if summary.mtc_messages > 0 {
        summary.mtc_final = Some(mtc.snapshot(now));
    }
    if summary.clock_messages > 0 {
        summary.clock_final = Some(clock.snapshot(now));
    }
    if summary.bridge_samples > 0 {
        summary.bridge_final = Some(bridge.snapshot(now));
    }
    summary
}
