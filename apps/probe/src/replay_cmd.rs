//! `replay` printing. The replay itself is `omoiiro_runtime::diagnostics::replay`.

use omoiiro_runtime::diagnostics::record::read_jsonl;
use omoiiro_runtime::diagnostics::replay::replay_with;
use omoiiro_runtime::diagnostics::report::{clock_json, clock_line, mtc_json, mtc_line, us_to_secs};
use omoiiro_runtime::sync::bridge::{snapshot_json as bridge_json, status_line as bridge_status_line};
use omoiiro_runtime::sync::mtc::Params;
use serde_json::json;
use std::path::Path;

/// `label - predicted` in frames at 30 fps (only 30 NDF is continued in practice).
fn frames(delta_us: i64) -> f64 {
    delta_us as f64 * 30.0 / 1_000_000.0
}

pub fn run(path: &Path, json: bool, ff_continue: bool) -> Result<(), Box<dyn std::error::Error>> {
    let records = read_jsonl(path)?;
    let params = Params { ff_continue_when_clock_playing: ff_continue, ..Params::default() };
    let s = replay_with(&records, params);
    let freewheel_json = s.ff_continue.then(|| {
        json!({
            "locks": s.freewheel_locks.iter().map(|(t, l, d)| json!({
                "t_us": t, "label": l, "delta_us": d, "delta_frames": frames(*d),
            })).collect::<Vec<_>>(),
        })
    });
    if json {
        let v = json!({
            "file": path.display().to_string(),
            "run_id": s.run_id,
            "synthetic": s.synthetic,
            "records": s.records,
            "mtc_messages": s.mtc_messages,
            "clock_messages": s.clock_messages,
            "other_messages": s.other_messages,
            "first_t_us": s.first_t_us,
            "last_t_us": s.last_t_us,
            "events": s.events.iter().map(|(t, src, text)| json!({"t_us": t, "source": src, "event": text})).collect::<Vec<_>>(),
            "mtc_final": s.mtc_final.as_ref().map(mtc_json),
            "clock_final": s.clock_final.as_ref().map(clock_json),
            "bridge_samples": s.bridge_samples,
            "bridge_final": s.bridge_final.as_ref().map(bridge_json),
            "ff_continue_when_clock_playing": freewheel_json,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    println!("file: {}", path.display());
    println!(
        "run_id: {}  synthetic: {}  records: {}  mtc: {}  clock: {}  other: {}  bridge: {}",
        s.run_id.as_deref().unwrap_or("-"),
        s.synthetic,
        s.records,
        s.mtc_messages,
        s.clock_messages,
        s.other_messages,
        s.bridge_samples
    );
    if s.synthetic {
        println!("NOTE: synthetic fixture. This is a decoder test, not a device or Live measurement.");
    }
    println!("events:");
    for (t, src, text) in &s.events {
        println!("  t={:10.6}s {:<5} {}", us_to_secs(*t), src, text);
    }
    if let Some(m) = &s.mtc_final {
        println!("final (at t={:.6}s): {}", us_to_secs(s.last_t_us.unwrap_or(0)), mtc_line(m));
    }
    if let Some(c) = &s.clock_final {
        println!("final (at t={:.6}s): {}", us_to_secs(s.last_t_us.unwrap_or(0)), clock_line(c));
    }
    if let Some(b) = &s.bridge_final {
        println!("final (at t={:.6}s): {}", us_to_secs(s.last_t_us.unwrap_or(0)), bridge_status_line(b));
    }
    if s.ff_continue {
        println!("ff_continue_when_clock_playing: {} lock(s) after a continued Full Frame (label - predicted)", s.freewheel_locks.len());
        for (t, l, d) in &s.freewheel_locks {
            println!("  t={:10.6}s locked {l}: {:+.2} frames ({d:+} us)", us_to_secs(*t), frames(*d));
        }
    }
    Ok(())
}
