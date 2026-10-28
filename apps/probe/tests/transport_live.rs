//! Runtime `Transport` driven offline by real recordings (B4 / O12 / S07,
//! Live 12.4.7, 2026-10-28). `*.bridge.jsonl` keep only the bridge samples and
//! meta records; the O12 recordings also carry MIDI Clock from the same
//! process. Re-analysis of recorded input, not a new measurement.

use omoiiro_core::model::StopBehavior;
use omoiiro_core::time::{FrameRate, Timecode};
use omoiiro_runtime::diagnostics::mtc_out_report::analyse;
use omoiiro_runtime::diagnostics::record::{read_jsonl, Record};
use omoiiro_runtime::diagnostics::transport_replay::replay_transport;
use omoiiro_runtime::sync::bridge::BridgeHealth;
use omoiiro_runtime::sync::transport::{BeatHealth, BeatSourceKind, OutputState, TransportConfig, TransportSnapshot, TransportState};
use std::path::Path;

fn records(name: &str) -> Vec<Record> {
    read_jsonl(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/live").join(name)).unwrap()
}

fn cfg(beat: Option<BeatSourceKind>) -> TransportConfig {
    TransportConfig::defaults(Timecode::parse("01:00:00:00", FrameRate::Fps30).unwrap(), 0, beat)
}

const STEP: i64 = 10_000;

fn first(snaps: &[TransportSnapshot], f: impl Fn(&TransportSnapshot) -> bool) -> Option<usize> {
    snaps.iter().position(f)
}

#[test]
fn p1_c4_tempo_automation_follows_continuously() {
    let s = replay_transport(&records("win-p1-c4.bridge.jsonl"), cfg(None), STEP, 0);
    let start = first(&s, |x| x.output == OutputState::Follow).unwrap();
    let stop = start + first(&s[start..], |x| x.output != OutputState::Follow).unwrap();
    assert!(stop - start > 2_500, "followed {} steps", stop - start); // ~28 s
    // Show time advances by about one step per step while following (no jumps).
    for w in s[start + 2..stop].windows(2) {
        let d = w[1].show_time_us.unwrap() - w[0].show_time_us.unwrap();
        assert!((d - STEP).abs() < 25_000, "jump {d} at {}", w[1].now_us);
    }
    assert!(s[start..stop].iter().all(|x| x.output_gain == 1.0));
    // After Stop: held position (stop_behavior hold).
    assert_eq!(s[stop + 5].output, OutputState::Hold);
    assert_eq!(s[stop + 5].transport, TransportState::Stopped);
}

#[test]
fn p1_c6b_script_stop_holds_then_blacks_out_then_recovers_with_fade() {
    let s = replay_transport(&records("win-p1-c6b.bridge.jsonl"), cfg(None), STEP, 0);
    // The script stopped at 10.118 s: lost at once, held, black 1 s later.
    let lost = first(&s, |x| x.time_health == BridgeHealth::Lost).unwrap();
    assert!((10_110_000..10_130_000).contains(&s[lost].now_us), "{}", s[lost].now_us);
    assert_eq!(s[lost].output, OutputState::Hold);
    let black = first(&s, |x| x.now_us > s[lost].now_us && x.output == OutputState::Black).unwrap();
    let dark_after = s[black].now_us - 10_118_000;
    assert!((1_000_000..1_020_000).contains(&dark_after), "{dark_after}");
    // Script restart at 17.9 s: follow again with a 100 ms fade.
    let back = first(&s, |x| x.now_us > s[black].now_us && x.output == OutputState::Follow).unwrap();
    // The fade starts when the output relights (first Hold while acquiring), so
    // the first Follow sample is part-way up the 100 ms ramp.
    assert!(s[back].output_gain < 0.5, "{}", s[back].output_gain);
    assert_eq!(s[back + 12].output_gain, 1.0);
}

#[test]
fn p1_c1_stop_behavior_hold_or_blackout() {
    let r = records("win-p1-c1.bridge.jsonl");
    for (behavior, expect) in [(StopBehavior::Hold, OutputState::Hold), (StopBehavior::Blackout, OutputState::Black)] {
        let mut c = cfg(None);
        c.stop_behavior = behavior;
        let s = replay_transport(&r, c, STEP, 0);
        // Between the two plays (stopped at 21.8 s, played again at 27.0 s).
        let mid = s.iter().find(|x| (24_000_000..24_010_000).contains(&x.now_us)).unwrap();
        assert_eq!((mid.transport, mid.output), (TransportState::Stopped, expect), "{behavior:?}");
        // No recovery fade after a normal Stop.
        let replay = s.iter().find(|x| x.now_us > 27_200_000 && x.output == OutputState::Follow).unwrap();
        assert_eq!(replay.output_gain, 1.0, "{behavior:?}");
    }
}

#[test]
fn o12_bridge_03_clock_beat_follows_tempo_automation() {
    let s = replay_transport(&records("win-o12-bridge-03.jsonl"), cfg(Some(BeatSourceKind::MidiClock)), STEP, 0);
    let playing: Vec<&TransportSnapshot> = s.iter().filter(|x| x.output == OutputState::Follow).collect();
    assert!(playing.len() > 3_000, "{}", playing.len());
    let locked = playing.iter().filter(|x| x.beat_health == BeatHealth::Locked).count();
    assert!(locked as f64 > 0.95 * playing.len() as f64, "{locked}/{}", playing.len());
    // The clock tempo reaches 140 and 90 (automation), and beats increase.
    let tempos: Vec<f64> = playing.iter().filter_map(|x| x.beat.map(|b| b.tempo_bpm)).collect();
    assert!(tempos.iter().any(|t| (t - 140.0).abs() < 2.0));
    assert!(tempos.iter().any(|t| (t - 90.0).abs() < 2.0));
    let beats: Vec<f64> = playing.iter().filter_map(|x| x.beat.map(|b| b.beat)).collect();
    // Two plays (the second restarts from the song start); the first reaches 30+ beats.
    let max_beat = beats.iter().cloned().fold(f64::MIN, f64::max);
    assert!(max_beat > beats[0] + 30.0, "{max_beat}");
    // The time side never lost the bridge while playing.
    assert!(playing.iter().all(|x| x.time_health != BridgeHealth::Lost));
}

#[test]
fn o12_bridge_02_recording_keeps_following() {
    let s = replay_transport(&records("win-o12-bridge-02.jsonl"), cfg(Some(BeatSourceKind::MidiClock)), STEP, 0);
    // While the bridge says playing, the output never goes black (timer samples carry the position).
    let playing: Vec<&TransportSnapshot> = s.iter().filter(|x| x.transport == TransportState::Playing).collect();
    assert!(playing.len() > 2_000, "{}", playing.len());
    assert!(playing.iter().all(|x| x.output != OutputState::Black));
}

#[test]
fn s07_sss_freeze_is_unknown_stalled_and_held() {
    let s = replay_transport(&records("win-s07-sss-link-04.bridge.jsonl"), cfg(None), STEP, 0);
    let st = s.iter().find(|x| x.stalled).expect("stalled");
    assert_eq!((st.transport, st.output), (TransportState::Unknown, OutputState::Hold));
}

/// The B4 C2 numbers (Live 12.4.7, recorded 2026-10-28) must stay the same.
#[test]
fn p1_c2b_mtc_out_numbers_are_unchanged() {
    let r = analyse(&records("win-p1-c2b.jsonl"));
    let relock: Vec<(String, f64)> = r.relock_ms.iter().filter_map(|(_, why, v)| v.map(|v| (why.clone(), (v * 10.0).round() / 10.0))).collect();
    assert_eq!(relock, vec![("start".to_string(), 150.2), ("jump".to_string(), 132.7), ("jump".to_string(), 145.5)]);
    let rx = r.received_minus_bridge_ms.unwrap();
    assert!((rx.p95 - 3.88).abs() < 0.01, "{rx:?}");
    let late = r.qf_late_ms.unwrap();
    assert!(late.p95 < 1.0, "{late:?}");
    assert_eq!(r.quarter_frames, 2220);
}
