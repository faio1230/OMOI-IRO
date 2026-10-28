//! End-to-end check of the synthetic fixture through the JSONL reader and
//! the decoders. This is a decoder test on synthetic data, not a device or
//! Live measurement.

use omoiiro_runtime::sync::clock::Transport;
use omoiiro_runtime::sync::mtc::SyncHealth;
use omoiiro_runtime::diagnostics::record::read_jsonl;
use omoiiro_runtime::diagnostics::replay::replay;
use omoiiro_probe::synth::write_fixture;

#[test]
fn synthetic_fixture_replays_with_expected_outcome() {
    let dir = std::env::temp_dir().join(format!("omoiiro-probe-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mtc30-play-seek-gap.jsonl");
    let written = write_fixture(&path).unwrap();
    let records = read_jsonl(&path).unwrap();
    assert_eq!(records.len() as u64, written);
    // Records are time-ordered and sequentially numbered.
    for (i, w) in records.windows(2).enumerate() {
        assert!(w[0].t_mono_us <= w[1].t_mono_us, "record {i} out of order");
        assert_eq!(w[1].seq, w[0].seq + 1);
    }

    let s = replay(&records);
    assert!(s.synthetic);
    assert_eq!(s.mtc_messages, 2 + 75 * 8);
    assert_eq!(s.clock_messages, 1 + 144 + 1);

    let m = s.mtc_final.expect("mtc snapshot");
    assert_eq!(m.stats.full_frames, 2);
    assert_eq!(m.stats.sequences, 75);
    assert_eq!(m.stats.dropped, 0);
    assert_eq!(m.stats.discontinuities, 1);
    assert_eq!(m.stats.locks, 3);
    assert_eq!(m.stats.invalid_labels, 0);
    // 1.6 s after the last Quarter Frame: lost and black, position frozen.
    assert_eq!(m.health, SyncHealth::Lost);
    assert!(m.blackout);
    assert!(!m.advancing);
    assert_eq!(m.label.map(|l| l.to_string()), Some("01:00:12:10".to_string()));

    let c = s.clock_final.expect("clock snapshot");
    assert_eq!(c.transport, Transport::Stopped);
    assert_eq!(c.stats.ticks, 144);
    assert_eq!(c.beats, Some(6.0));
    let bpm = c.bpm.expect("bpm");
    assert!((bpm - 120.0).abs() < 0.2, "bpm {bpm}");

    let texts: Vec<&str> = s.events.iter().map(|(_, _, t)| t.as_str()).collect();
    assert!(texts.iter().any(|t| t.starts_with("full_frame 01:00:00:00")));
    assert!(texts.iter().any(|t| t == &"locked 01:00:00:02"));
    assert!(texts.iter().any(|t| t.starts_with("full_frame 01:00:10:00")));
    assert!(texts.iter().any(|t| t == &"locked 01:00:10:02"));
    assert!(texts.iter().any(|t| t.starts_with("discontinuity 01:00:10:28 -> 01:00:11:12 (delta +12 frames)")));
    assert!(texts.iter().any(|t| t == &"locked 01:00:11:14"));
    assert!(texts.iter().any(|t| t == &"start"));
    assert!(texts.iter().any(|t| t == &"stop"));

    std::fs::remove_dir_all(&dir).ok();
}
