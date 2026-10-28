//! Replay of measured Live 12.4.7 + LiveMTC 1.1.0 recordings (Windows,
//! 2026-10-28). The files in `fixtures/live/` are byte copies of
//! `artifacts/p0/<run-id>.jsonl`. These tests check the decoders against
//! recorded input; they are not a new Live or device measurement.

use omoiiro_runtime::sync::clock::Transport;
use omoiiro_runtime::diagnostics::record::read_jsonl;
use omoiiro_runtime::diagnostics::replay::{replay, ReplaySummary};
use std::path::Path;

fn load(name: &str) -> ReplaySummary {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/live").join(name);
    let records = read_jsonl(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let s = replay(&records);
    assert!(!s.synthetic, "{name} must be a measured record");
    s
}

fn events<'a>(s: &'a ReplaySummary, prefix: &str) -> Vec<(i64, &'a str)> {
    s.events
        .iter()
        .filter(|(_, _, t)| t.starts_with(prefix))
        .map(|(t, _, text)| (*t, text.as_str()))
        .collect()
}

#[test]
fn s04_c4_tempo_changes_are_full_frame_discontinuities() {
    let s = load("win-s04-c4-arr.jsonl");
    let m = s.mtc_final.expect("mtc");
    assert_eq!(m.stats.full_frames, 4);
    assert_eq!(m.stats.ff_discontinuities, 3);
    assert_eq!(m.stats.discontinuities, 0);
    assert_eq!(m.stats.locks, 3);
    assert_eq!(m.stats.dropped, 0);
    let ff = events(&s, "discontinuity(full_frame)");
    let texts: Vec<&str> = ff.iter().map(|(_, t)| *t).collect();
    assert_eq!(
        texts,
        vec![
            // Play: LiveMTC sends the origin first, then the real position.
            "discontinuity(full_frame) 01:00:00:00 -> 01:00:32:00 (delta +960 frames, from acquiring)",
            // 120 -> 128 bpm
            "discontinuity(full_frame) 01:00:38:01 -> 01:00:35:19 (delta -73 frames, from locked)",
            // 128 -> 90 bpm
            "discontinuity(full_frame) 01:00:41:27 -> 01:00:59:14 (delta +526 frames, from locked)",
        ]
    );
}

#[test]
fn s03_c3_loop_wraps_are_full_frame_discontinuities() {
    let s = load("win-s03-c3-arr.jsonl");
    let m = s.mtc_final.expect("mtc");
    assert_eq!(m.stats.full_frames, 6);
    // 1 position change while stopped (loop set-up), 1 on Play, 3 loop wraps.
    assert_eq!(m.stats.ff_discontinuities, 5);
    assert_eq!(m.stats.discontinuities, 0);
    assert_eq!(m.stats.locks, 4);
    let wraps: Vec<&str> = events(&s, "discontinuity(full_frame) 01:00:30:01 -> 01:00:22:00")
        .iter()
        .map(|(_, t)| *t)
        .collect();
    assert_eq!(wraps.len(), 3, "{wraps:?}");
    for w in &wraps {
        assert!(w.contains("(delta -24"), "{w}");
    }
}

#[test]
fn s03_c6c7_mtc_off_is_a_qf_discontinuity_and_clock_sync_off_stalls() {
    let s = load("win-s03-c6c7-arr.jsonl");
    let m = s.mtc_final.expect("mtc");
    // C6: MTC device off/on resumes Quarter Frames without a Full Frame.
    assert_eq!(m.stats.discontinuities, 1);
    // Play: origin Full Frame then the real position.
    assert_eq!(m.stats.ff_discontinuities, 1);
    assert_eq!(m.stats.locks, 2);
    assert!(s
        .events
        .iter()
        .any(|(_, _, t)| t == "discontinuity 01:00:49:10 -> 01:00:54:24 (delta +162 frames)"));

    // C7: Clock Sync off stops the ticks with no Stop message.
    let stalled = events(&s, "stalled");
    assert_eq!(stalled.len(), 1, "{stalled:?}");
    let resumed = events(&s, "resumed");
    assert_eq!(resumed.len(), 1, "{resumed:?}");
    let (t_stall, _) = stalled[0];
    let (t_resume, _) = resumed[0];
    // Last tick before the gap at 24.506 s; stall reported within 520 ms.
    assert!((25_006_000..25_030_000).contains(&t_stall), "stall at {t_stall}");
    assert!((31_240_000..31_250_000).contains(&t_resume), "resume at {t_resume}");
    // No Stop / Start / Continue between the stall and the resume.
    assert!(!s.events.iter().any(|(t, src, text)| *src == "clock"
        && *t > t_stall
        && *t < t_resume
        && matches!(text.as_str(), "stop" | "start" | "continue")));

    let c = s.clock_final.expect("clock");
    assert_eq!(c.stats.stalls, 1);
    // The run ends with a real Stop, so the final state is stopped.
    assert_eq!(c.transport, Transport::Stopped);
}

/// O13 trial (`ff_continue_when_clock_playing`, default off). Records the
/// lock error after each continued Full Frame; acceptance is decided by the
/// design side, so this only pins the measured values.
#[test]
fn s03_c3_ff_continue_trial_reports_lock_error_per_wrap() {
    use omoiiro_runtime::sync::mtc::Params;
    use omoiiro_runtime::diagnostics::replay::replay_with;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/live/win-s03-c3-arr.jsonl");
    let records = read_jsonl(&path).unwrap();
    let params = Params { ff_continue_when_clock_playing: true, ..Params::default() };
    let s = replay_with(&records, params);
    // Play + 3 loop wraps. The two Full Frames while stopped are not continued.
    let deltas: Vec<i64> = s.freewheel_locks.iter().map(|(_, _, d)| *d).collect();
    assert_eq!(deltas, vec![46_618, 62_619, 49_634, 49_034]);
    assert!(s.freewheel_locks.iter().all(|(_, l, _)| l == "01:00:22:18"));
    // Counters are unchanged by the option.
    let m = s.mtc_final.unwrap();
    assert_eq!((m.stats.discontinuities, m.stats.ff_discontinuities, m.stats.locks), (0, 5, 4));
    // Default off: no continued locks.
    assert!(load("win-s03-c3-arr.jsonl").freewheel_locks.is_empty());
}

// ---- Bridge decoder on the DJSCBridgeProbe recordings (Live 12.4.7,
// 2026-10-28, v1 records). Replay of recorded input, not a new measurement.

fn bridge_discs(s: &ReplaySummary) -> Vec<&str> {
    s.events
        .iter()
        .filter(|(_, src, t)| *src == "bridge" && t.starts_with("discontinuity("))
        .map(|(_, _, t)| t.as_str())
        .collect()
}

#[test]
fn o12_bridge_03_tempo_automation_is_continuous_while_playing() {
    let s = load("win-o12-bridge-03.jsonl");
    let b = s.bridge_final.as_ref().expect("bridge");
    assert_eq!(b.stats.playing_discontinuities, 0, "{:?}", bridge_discs(&s));
    // Only the return to the start while stopped.
    assert_eq!(bridge_discs(&s), vec!["discontinuity(stopped) 31509 -> 0 ms (elapsed 50.5 ms, deviation -31509.0 ms)"]);
    assert_eq!(b.stats.stalls, 0);
    assert_eq!(b.stats.faults, 0);
}

#[test]
fn o12_bridge_01_manual_tempo_changes_are_discontinuities() {
    let s = load("win-o12-bridge-01.jsonl");
    let b = s.bridge_final.as_ref().expect("bridge");
    assert_eq!(b.stats.playing_discontinuities, 2);
    let d = bridge_discs(&s);
    assert!(d[0].starts_with("discontinuity(playing) 6256 -> 5379 ms"), "{d:?}");
    assert!(d[1].starts_with("discontinuity(playing) 11595 -> 18068 ms"), "{d:?}");
}

#[test]
fn o12_bridge_02_recording_keeps_position_through_timer_samples() {
    let s = load("win-o12-bridge-02.jsonl");
    let b = s.bridge_final.as_ref().expect("bridge");
    // Listener samples stop while recording; timer samples keep it consistent.
    assert_eq!(b.stats.stalls, 0);
    assert_eq!(b.stats.faults, 0);
    // Live reports Play, then moves to the start position 6 / 15 ms later:
    // phase start (within 50 ms of Play), not playing.
    assert_eq!(b.stats.playing_discontinuities, 0);
    assert_eq!(b.stats.start_discontinuities, 2);
    let d = bridge_discs(&s);
    assert!(d.iter().any(|t| t.starts_with("discontinuity(start) 1378 -> 5 ms")), "{d:?}");
    assert!(d.iter().any(|t| t.starts_with("discontinuity(start) 2212 -> 6 ms")), "{d:?}");
}

// ---- mtc-out generator driven offline by real DJSCBridge (now OmoiIroBridge) recordings
// (B4 / S07, Live 12.4.7, 2026-10-28). `fixtures/live/*.bridge.jsonl` keep
// only the bridge samples and meta records of `artifacts/p0/<run-id>.jsonl`.
// Offline simulation, not a MIDI measurement.

use omoiiro_runtime::sync::mtc::FrameRate;
use omoiiro_runtime::diagnostics::mtc_out_report::simulate;

fn sim(name: &str) -> (Vec<(i64, &'static str, String)>, u64, Vec<i64>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/live").join(name);
    simulate(&read_jsonl(&path).unwrap(), 3_600_000_000.0, FrameRate::Fps30, 1000)
}

fn reasons(ffs: &[(i64, &'static str, String)]) -> Vec<&'static str> {
    ffs.iter().map(|(_, r, _)| *r).collect()
}

#[test]
fn p1_c1_play_sends_a_single_start_full_frame() {
    let (ffs, _, _) = sim("win-p1-c1.bridge.jsonl");
    // The live run sent start (old position) + jump on the second Play; with
    // the 50 ms settle there is one start at the new position.
    assert_eq!(reasons(&ffs), vec!["hold", "start", "stop", "start", "stop"]);
    assert_eq!(ffs[3].2, "01:00:00:00");
}

#[test]
fn p1_c6b_meta_stop_stops_quarter_frames_at_once() {
    let (ffs, _, ends) = sim("win-p1-c6b.bridge.jsonl");
    assert_eq!(reasons(&ffs), vec!["hold", "start", "start", "stop"]);
    // meta stop arrived at 10.118 s; the live run (before the change) kept QF until 10.363 s.
    assert!((10_090_000..10_125_000).contains(&ends[0]), "{ends:?}");
}

#[test]
fn s07_sss_wait_holds_then_starts_without_resync_or_playing_discontinuity() {
    let (ffs, _, _) = sim("win-s07-sss-link-03.bridge.jsonl");
    assert_eq!(reasons(&ffs), vec!["hold", "hold", "start", "stop"]);
    let s = load("win-s07-sss-link-03.bridge.jsonl");
    let b = s.bridge_final.as_ref().unwrap();
    assert_eq!(b.stats.playing_discontinuities, 0, "{:?}", bridge_discs(&s));
    assert_eq!(b.stats.stalls, 1);
}

#[test]
fn s07_sss_freeze_is_stalled_not_discontinuities() {
    let s = load("win-s07-sss-link-04.bridge.jsonl");
    let b = s.bridge_final.as_ref().unwrap();
    assert_eq!(b.stats.playing_discontinuities, 0, "{:?}", bridge_discs(&s));
    assert_eq!(b.stats.stalls, 1);
    // The generator may start for a few Quarter Frames before the first static
    // pair arrives, then holds.
    let (ffs, qf, _) = sim("win-s07-sss-link-04.bridge.jsonl");
    assert!(qf <= 8, "qf {qf}");
    assert!(reasons(&ffs).contains(&"stop"));
}
