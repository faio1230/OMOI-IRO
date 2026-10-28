//! `bridge-report` printing. The analysis is `omoiiro_runtime::diagnostics::bridge_report`.

use omoiiro_runtime::diagnostics::bridge_report::{analyse, report_json, Dist, Fit, JUMP_TOLERANCE_MS};
use omoiiro_runtime::diagnostics::record::read_jsonl;
use omoiiro_runtime::diagnostics::report::us_to_secs;
use std::path::Path;

fn fmt_dist(d: &Option<Dist>, unit: &str) -> String {
    match d {
        Some(d) => format!("median {:.3}{unit} p95 {:.3}{unit} max {:.3}{unit} (n={})", d.median, d.p95, d.max, d.n),
        None => "--".into(),
    }
}

fn fmt_fit(f: &Option<Fit>) -> String {
    match f {
        Some(f) => format!("slope {:.6} residual {}", f.slope, fmt_dist(&Some(f.residual), "ms")),
        None => "--".into(),
    }
}

pub fn run_report(path: &Path, json_out: bool) -> Result<(), Box<dyn std::error::Error>> {
    let records = read_jsonl(path)?;
    let r = analyse(&records);
    if json_out {
        println!("{}", serde_json::to_string_pretty(&report_json(&r))?);
        return Ok(());
    }
    println!("file: {}", path.display());
    println!(
        "samples: {} (listener {}, timer {}), parse errors {}",
        r.samples, r.listener_samples, r.timer_samples, r.parse_errors
    );
    println!("listener song_time arrival interval while playing: {}", fmt_dist(&r.listener_interval_ms, "ms"));
    println!(
        "listener song_time per second while playing: {}",
        r.listener_per_second.map(|v| format!("{v:.1}")).unwrap_or_else(|| "--".into())
    );
    println!("timer arrival interval: {}", fmt_dist(&r.timer_interval_ms, "ms"));
    println!("playing segments (split at jumps): {}", r.segments.len());
    for (i, s) in r.segments.iter().enumerate() {
        println!(
            "  #{i} t={:.3}..{:.3}s n={}\n      vs arrival: {}\n      vs Live perf: {}",
            us_to_secs(s.start_us),
            us_to_secs(s.end_us),
            s.samples,
            fmt_fit(&s.vs_arrival),
            fmt_fit(&s.vs_perf)
        );
    }
    println!("jumps (|step - elapsed| > {JUMP_TOLERANCE_MS} ms while playing, or a move while stopped): {}", r.jumps.len());
    for j in &r.jumps {
        println!(
            "  t={:10.6}s {:.0} -> {:.0} ms (elapsed {:.1} ms, deviation {:+.1} ms) tempo {} -> {}",
            us_to_secs(j.t_mono_us),
            j.from_ms,
            j.to_ms,
            j.elapsed_ms,
            j.deviation_ms,
            j.tempo_before.map(|t| format!("{t:.2}")).unwrap_or_else(|| "--".into()),
            j.tempo_after.map(|t| format!("{t:.2}")).unwrap_or_else(|| "--".into()),
        );
    }
    let o = &r.offsets;
    println!(
        "smpte - MTC position (locked only): median {} ms, |dev from median| {}",
        o.mtc_median_raw_ms.map(|v| format!("{v:.1}")).unwrap_or_else(|| "--".into()),
        fmt_dist(&o.mtc_ms, "ms")
    );
    println!(
        "beats - Clock beats (playing only): median {} beats, |dev from median| {}",
        o.clock_median_raw_beats.map(|v| format!("{v:.3}")).unwrap_or_else(|| "--".into()),
        fmt_dist(&o.clock_beats, "")
    );
    Ok(())
}
