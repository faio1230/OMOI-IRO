//! `route-check`: forward Note / CC from one input to the Live control bus.
//!
//! * Only Note Off (8n), Note On (9n) and Control Change (Bn) are forwarded.
//!   Note On with velocity 0 is normalized to Note Off (8n kk 00). Everything
//!   else (aftertouch, SysEx, Real Time, ...) is recorded but not forwarded.
//! * The midir callback takes the monotonic time, copies the bytes and
//!   `try_send`s. Normalization, sending and recording run on the pipeline
//!   thread.
//! * Ports are given as exact names or `#index`. The output must not be the
//!   input (loop) and must not look like a Launchpad.

use crossbeam_channel::{bounded, Receiver, RecvTimeoutError};
pub use omoiiro_midi::backend::{MidiSend, RawIn};
use omoiiro_midi::backend::{Ingress, IngressStats, MidiBackend};
use omoiiro_midi::clock::MonotonicClock;
use omoiiro_midi::midir_backend::MidirBackend;
use omoiiro_midi::ports;
use omoiiro_midi::routing::{route, Routed};
use omoiiro_runtime::diagnostics::record::{hex, midi_payload, JsonlWriter};
use omoiiro_runtime::diagnostics::report::us_to_secs;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const QUEUE_CAPACITY: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RouteStats {
    pub received: u64,
    pub forwarded: u64,
    pub normalized: u64,
    pub ignored: u64,
    pub send_errors: u64,
}

/// Route one received message: record it under `in_source`, forward Note / CC
/// to `out`, record the forwarded bytes as `control_out`.
pub fn handle_input<S: MidiSend + ?Sized>(
    ev: &RawIn,
    in_source: &str,
    out: &mut S,
    w: &mut JsonlWriter,
    stats: &mut RouteStats,
    verbose: bool,
) -> std::io::Result<()> {
    stats.received += 1;
    w.write(ev.t_mono_us, in_source, "midi", midi_payload(&ev.bytes, ev.backend_ts_us))?;
    match route(&ev.bytes) {
        Routed::Forward { bytes, normalized } => {
            let result = out.send_msg(&bytes);
            if normalized {
                stats.normalized += 1;
            }
            let payload = match &result {
                Ok(()) => {
                    stats.forwarded += 1;
                    json!({ "bytes": hex(&bytes), "from": hex(&ev.bytes), "normalized": normalized })
                }
                Err(e) => {
                    stats.send_errors += 1;
                    json!({ "bytes": hex(&bytes), "from": hex(&ev.bytes), "normalized": normalized, "error": e })
                }
            };
            if verbose {
                println!(
                    "  t={:10.6}s {} -> control_out {}{}{}",
                    us_to_secs(ev.t_mono_us),
                    hex(&ev.bytes),
                    hex(&bytes),
                    if normalized { " (note on vel 0 -> note off)" } else { "" },
                    result.as_ref().err().map(|e| format!(" SEND ERROR {e}")).unwrap_or_default(),
                );
            }
            // `kind` is `forward`, not `midi`, so replay never treats it as input.
            w.write(ev.t_mono_us, "control_out", "forward", payload)?;
        }
        Routed::Ignore => stats.ignored += 1,
    }
    Ok(())
}

pub fn stats_json(s: &RouteStats) -> serde_json::Value {
    json!({
        "received": s.received,
        "forwarded": s.forwarded,
        "normalized": s.normalized,
        "ignored": s.ignored,
        "send_errors": s.send_errors,
    })
}

#[derive(clap::Args, Debug)]
pub struct RouteArgs {
    /// Input port: exact name or `#index` (Launchpad X MIDI port, or any test input).
    #[arg(long = "in")]
    pub input: String,
    /// Output port: exact name or `#index`.
    #[arg(long = "out", default_value = omoiiro_midi::ports::names::CONTROL_PORT)]
    pub output: String,
    /// JSONL output path. Default: artifacts/p0/<run-id>.jsonl
    #[arg(long)]
    pub record: Option<PathBuf>,
    /// Run identifier. Default: route-<unix seconds>
    #[arg(long)]
    pub run_id: Option<String>,
    /// Stop after this many seconds. Default: run until Ctrl+C.
    #[arg(long)]
    pub duration: Option<u64>,
}

pub fn run(args: RouteArgs) -> Result<(), Box<dyn std::error::Error>> {
    let mut backend = MidirBackend::new("omoiiro-probe-route");
    let inputs = backend.list_inputs()?;
    let outputs = backend.list_outputs()?;
    let in_name = inputs[ports::resolve_exact(&inputs, &args.input)?].clone();
    let out_name = outputs[ports::resolve_exact(&outputs, &args.output)?].clone();
    if in_name == out_name {
        return Err(format!("input and output are the same port {in_name:?}; this would loop").into());
    }
    if ports::looks_like_launchpad(&out_name) {
        return Err(format!("refusing to forward into a device-looking port {out_name:?}").into());
    }

    let clock = MonotonicClock::new();
    let (tx, rx) = bounded::<RawIn>(QUEUE_CAPACITY);
    let in_stats = Arc::new(IngressStats::default());
    let mut out_conn = backend.open_output(&args.output).map_err(|e| format!("open output: {e}"))?;
    let in_conn = backend
        .open_input(&args.input, true, Ingress::new(clock.clone(), tx, in_stats.clone()))
        .map_err(|e| format!("open input: {e}"))?;
    println!("input  : {in_name}");
    println!("output : {out_name}");

    let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let run_id = args.run_id.clone().unwrap_or_else(|| format!("route-{unix}"));
    let path = args.record.clone().unwrap_or_else(|| PathBuf::from("artifacts/p0").join(format!("{run_id}.jsonl")));
    let mut w = JsonlWriter::create(&path, &run_id)?;
    println!("record : {}", path.display());
    w.write(
        clock.now_us(),
        "meta",
        "start",
        json!({
            "command": "route-check",
            "unix_s": unix,
            "backend": ports::backend_name(),
            "input": in_name,
            "output": out_name,
            "queue_capacity": QUEUE_CAPACITY,
        }),
    )?;

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))?;
    }
    println!("forwarding Note / CC (Ctrl+C to stop)...");
    let deadline = args.duration.map(|d| d as i64 * 1_000_000);
    let stats = pump(&rx, &clock, "route_in", &mut out_conn, &mut w, &stop, deadline)?;

    let q_drop = in_stats.dropped();
    let end = json!({ "route": stats_json(&stats), "queue_dropped": q_drop });
    w.write(clock.now_us(), "meta", "end", end.clone())?;
    w.flush()?;
    drop(in_conn);
    println!("{}", serde_json::to_string_pretty(&end)?);
    Ok(())
}

/// Drain `rx` until `stop`, the deadline, or the sender is gone.
pub fn pump<S: MidiSend>(
    rx: &Receiver<RawIn>,
    clock: &MonotonicClock,
    in_source: &str,
    out: &mut S,
    w: &mut JsonlWriter,
    stop: &AtomicBool,
    deadline_us: Option<i64>,
) -> std::io::Result<RouteStats> {
    let mut stats = RouteStats::default();
    let mut next_flush = clock.now_us() + 1_000_000;
    loop {
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(ev) => handle_input(&ev, in_source, out, w, &mut stats, true)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let now = clock.now_us();
        if now >= next_flush {
            w.flush()?;
            next_flush = now + 1_000_000;
        }
        if stop.load(Ordering::SeqCst) || deadline_us.is_some_and(|d| now >= d) {
            break;
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omoiiro_runtime::diagnostics::record::read_jsonl;

    #[derive(Default)]
    struct Sink(Vec<Vec<u8>>);

    impl MidiSend for Sink {
        fn send_msg(&mut self, bytes: &[u8]) -> Result<(), String> {
            self.0.push(bytes.to_vec());
            Ok(())
        }
    }

    #[test]
    fn handle_input_forwards_and_records() {
        let dir = std::env::temp_dir().join(format!("omoiiro-probe-route-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("route.jsonl");
        let mut w = JsonlWriter::create(&path, "route-test").unwrap();
        let mut sink = Sink::default();
        let mut stats = RouteStats::default();
        for (t, b) in [(1, vec![0x90, 11, 90]), (2, vec![0xA0, 11, 30]), (3, vec![0x90, 11, 0]), (4, vec![0xB0, 91, 127])] {
            let ev = RawIn::new(t, b);
            handle_input(&ev, "route_in", &mut sink, &mut w, &mut stats, false).unwrap();
        }
        w.flush().unwrap();
        assert_eq!(sink.0, vec![vec![0x90, 11, 90], vec![0x80, 11, 0], vec![0xB0, 91, 127]]);
        assert_eq!(stats, RouteStats { received: 4, forwarded: 3, normalized: 1, ignored: 1, send_errors: 0 });
        let recs = read_jsonl(&path).unwrap();
        let outs: Vec<_> = recs.iter().filter(|r| r.source == "control_out").collect();
        assert_eq!(outs.len(), 3);
        assert_eq!(outs[1].payload["bytes"], "80 0B 00");
        assert_eq!(outs[1].payload["from"], "90 0B 00");
        assert_eq!(outs[1].payload["normalized"], true);
        assert_eq!(recs.iter().filter(|r| r.source == "route_in").count(), 4);
        std::fs::remove_dir_all(&dir).ok();
    }
}
