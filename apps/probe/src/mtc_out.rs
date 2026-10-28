//! `mtc-out`: MTC output driven by OmoiIroBridge samples (docs/10-live-bridge.md
//! "MTC出力"), and `mtc-out-report` for the loopback self-check.
//!
//! Threads:
//! * UDP receiver: stamps arrival on the session monotonic clock, copies,
//!   `try_send`s (as in `bridge-clock`).
//! * Sender (dedicated): `omoiiro_runtime::output::mtc::spawn_sender`, driven by
//!   the Runtime `Transport` (bridge view + origin + input offset), the same
//!   path the application uses.
//! * Optional verify input (`--verify-in`, e.g. the same loopMIDI bus): the
//!   midir callback stamps, copies, `try_send`s. Used only for the record and
//!   the report, never as the application's clock.
//! * Pipeline (this thread): feeds the transport's bridge decoder, the verify MTC
//!   decoder, writes JSONL, prints a status line once a second.

use omoiiro_midi::backend::{make_callback, RawIn};
use omoiiro_midi::clock::MonotonicClock;
use omoiiro_midi::ports;
use omoiiro_runtime::diagnostics::bridge_report::{dist, Dist};
use omoiiro_runtime::diagnostics::mtc_out_report::analyse;
use omoiiro_runtime::diagnostics::record::{midi_payload, read_jsonl, JsonlWriter};
use omoiiro_runtime::diagnostics::report::{mtc_event_is_notable, mtc_event_line, mtc_line, us_to_secs};
use omoiiro_runtime::output::mtc::{sent_payload, spawn_sender, Sent};
use omoiiro_runtime::sync::bridge::{event_line, status_line, BridgeSample};
use omoiiro_runtime::sync::bridge_ingress::{datagram_payload, spawn_receiver, RawDatagram};
use omoiiro_runtime::sync::mtc::{FrameRate, MtcDecoder, Params};
use omoiiro_runtime::sync::mtc_out::{GenOut, MtcGen};
use omoiiro_runtime::sync::transport::{origin_label, Transport, TransportConfig};
use crossbeam_channel::{bounded, RecvTimeoutError};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(clap::Args, Debug)]
pub struct MtcOutArgs {
    /// UDP address to receive OmoiIroBridge samples on.
    #[arg(long, default_value = omoiiro_runtime::sync::bridge_ingress::DEFAULT_BIND)]
    pub bind: String,
    /// MTC output port: exact name or `#index`.
    #[arg(long = "out-port", default_value = omoiiro_midi::ports::names::TIMECODE_PORT)]
    pub out_port: String,
    /// Frame rate: 30 (NDF), 25 or 24.
    #[arg(long, default_value_t = 30)]
    pub fps: u32,
    /// Label of Live's song start (show `sync.origin`), HH:MM:SS:FF.
    #[arg(long, default_value = "01:00:00:00")]
    pub origin: String,
    /// Fixed input offset added to the bridge position, in ms.
    #[arg(long, default_value_t = 0.0)]
    pub input_offset_ms: f64,
    /// Also receive this input (exact name or `#index`), e.g. the same loopMIDI bus, for the self-check record.
    #[arg(long)]
    pub verify_in: Option<String>,
    /// JSONL output path. Default: artifacts/p0/<run-id>.jsonl
    #[arg(long)]
    pub record: Option<PathBuf>,
    /// Run identifier. Default: mtcout-<unix seconds>
    #[arg(long)]
    pub run_id: Option<String>,
    /// Stop after this many seconds. Default: run until Ctrl+C.
    #[arg(long)]
    pub duration: Option<u64>,
}

fn rate_of(fps: u32) -> Result<FrameRate, String> {
    match fps {
        24 => Ok(FrameRate::Fps24),
        25 => Ok(FrameRate::Fps25),
        30 => Ok(FrameRate::Fps30),
        other => Err(format!("--fps {other}: use 30, 25 or 24 (29.97 DF is out of scope)")),
    }
}

/// Parse `HH:MM:SS:FF` at `rate` into microseconds.
pub fn parse_origin_us(s: &str, rate: FrameRate) -> Result<f64, String> {
    let p: Vec<&str> = s.split(':').collect();
    if p.len() != 4 {
        return Err(format!("origin {s:?} must be HH:MM:SS:FF"));
    }
    let n = |i: usize| p[i].parse::<u8>().map_err(|e| format!("origin {s:?}: {e}"));
    let l = omoiiro_runtime::sync::mtc::Label::new(n(0)?, n(1)?, n(2)?, n(3)?, rate);
    l.validate().map_err(|e| format!("origin {s:?}: {e:?}"))?;
    Ok(l.to_us().ok_or("unsupported rate")? as f64)
}

pub fn run(args: MtcOutArgs) -> Result<(), Box<dyn std::error::Error>> {
    let rate = rate_of(args.fps)?;
    let origin_us = parse_origin_us(&args.origin, rate)?;
    let gen = MtcGen::new(rate)?;
    let core_rate = match args.fps {
        24 => omoiiro_core::time::FrameRate::Fps24,
        25 => omoiiro_core::time::FrameRate::Fps25,
        _ => omoiiro_core::time::FrameRate::Fps30,
    };
    let cfg = TransportConfig::defaults(origin_label(&args.origin, core_rate)?, (args.input_offset_ms * 1000.0).round() as i64, None);

    let (output, out_port, out_name) = ports::open_output_exact("omoiiro-probe-mtc-out", &args.out_port)?;
    if ports::looks_like_launchpad(&out_name) {
        return Err(format!("refusing to send MTC to a device-looking port {out_name:?}").into());
    }
    let conn = output.connect(&out_port, "omoiiro-probe-mtc-out").map_err(|e| format!("open output {out_name:?}: {e}"))?;

    let clock = MonotonicClock::new();
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))?;
    }

    let (verify_tx, verify_rx) = bounded::<RawIn>(8192);
    let verify_dropped = Arc::new(AtomicU64::new(0));
    let mut verify_name = None;
    let _verify_conn = match &args.verify_in {
        Some(spec) => {
            let (input, port, name) = ports::open_input_exact("omoiiro-probe-mtc-verify", spec)?;
            let c = input
                .connect(&port, "omoiiro-probe-mtc-verify", make_callback(clock.clone(), verify_tx.clone(), verify_dropped.clone()), ())
                .map_err(|e| format!("open verify input {name:?}: {e}"))?;
            verify_name = Some(name);
            Some(c)
        }
        None => None,
    };
    let _verify_keepalive = verify_tx;

    let (bridge_tx, bridge_rx) = bounded::<RawDatagram>(8192);
    let bridge_dropped = Arc::new(AtomicU64::new(0));
    let bridge_stop = Arc::new(AtomicBool::new(false));
    let bridge_thread = spawn_receiver(&args.bind, clock.clone(), bridge_tx, bridge_dropped.clone(), bridge_stop.clone())
        .map_err(|e| format!("bind UDP {}: {e}", args.bind))?;

    let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let run_id = args.run_id.clone().unwrap_or_else(|| format!("mtcout-{unix}"));
    let path = args.record.clone().unwrap_or_else(|| PathBuf::from("artifacts/p0").join(format!("{run_id}.jsonl")));
    let mut w = JsonlWriter::create(&path, &run_id)?;
    w.write(
        clock.now_us(),
        "meta",
        "start",
        json!({
            "command": "mtc-out", "unix_s": unix, "backend": ports::backend_name(),
            "bridge_udp": args.bind, "out_port": out_name, "verify_in": verify_name,
            "fps": args.fps, "origin": args.origin, "origin_us": origin_us, "input_offset_ms": args.input_offset_ms,
            "slew": gen.slew,
        }),
    )?;
    println!("bridge UDP : {}", args.bind);
    println!("MTC out    : {out_name} ({} fps, origin {}, offset {} ms)", args.fps, args.origin, args.input_offset_ms);
    if let Some(n) = &verify_name {
        println!("verify in  : {n} (record only; not used as the clock)");
    }
    println!("record     : {}", path.display());

    let dec = Arc::new(Mutex::new(Transport::new(cfg)));
    let (sent_tx, sent_rx) = bounded::<Sent>(16384);
    let sent_dropped = Arc::new(AtomicU64::new(0));
    let sender = spawn_sender(conn, gen, dec.clone(), clock.clone(), sent_tx, sent_dropped.clone(), stop.clone());

    let mut mtc = MtcDecoder::new(Params::default());
    let (mut bev, mut mev) = (Vec::new(), Vec::new());
    let mut late = Vec::new();
    let mut next_print = clock.now_us() + 1_000_000;
    let mut next_flush = clock.now_us() + 1_000_000;
    let deadline = args.duration.map(|d| d as i64 * 1_000_000);
    println!("running (Ctrl+C to stop)...");
    loop {
        match bridge_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(d) => {
                let payload = datagram_payload(&d.bytes);
                w.write(d.t_mono_us, "bridge", "sample", payload.clone())?;
                bev.clear();
                dec.lock().unwrap_or_else(|e| e.into_inner()).feed_bridge(BridgeSample::from_payload(d.t_mono_us, &payload), &mut bev);
                for e in &bev {
                    let line = event_line(e);
                    println!("  t={:10.6}s bridge {line}", us_to_secs(d.t_mono_us));
                    w.write(d.t_mono_us, "bridge", "event", json!({ "event": line }))?;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        for s in sent_rx.try_iter() {
            if let GenOut::FullFrame { label, reason, .. } = &s.out {
                println!("  t={:10.6}s mtc-out full_frame {label} ({reason})", us_to_secs(s.t_us));
            } else {
                late.push(s.late_us / 1000.0);
            }
            w.write(s.t_us, "mtc_out", "send", sent_payload(&s))?;
        }
        for ev in verify_rx.try_iter() {
            w.write(ev.t_mono_us, "mtc", "midi", midi_payload(&ev.bytes, ev.backend_ts_us))?;
            mev.clear();
            mtc.feed(ev.t_mono_us, &ev.bytes, &mut mev);
            for e in &mev {
                if mtc_event_is_notable(e) {
                    w.write(ev.t_mono_us, "mtc", "event", json!({ "event": mtc_event_line(e) }))?;
                }
            }
        }
        let now = clock.now_us();
        if now >= next_print {
            let b = dec.lock().unwrap_or_else(|e| e.into_inner()).bridge_view(now).0;
            let jit = dist(&late).map(|d| format!("late p95 {:.2}ms max {:.2}ms", d.p95, d.max)).unwrap_or_else(|| "late --".into());
            let mut line = format!("[{:9.3}s] | {} | OUT qf={} {jit}", us_to_secs(now), status_line(&b), late.len());
            if verify_name.is_some() {
                line.push_str(&format!(" | RX {}", mtc_line(&mtc.snapshot(now))));
            }
            println!("{line}");
            next_print = now + 1_000_000;
        }
        if now >= next_flush {
            w.flush()?;
            next_flush = now + 1_000_000;
        }
        if stop.load(Ordering::SeqCst) || deadline.is_some_and(|d| now >= d) {
            break;
        }
    }
    stop.store(true, Ordering::SeqCst);
    let stats = sender.join().map_err(|_| "sender thread panicked")?;
    for s in sent_rx.try_iter() {
        if !matches!(s.out, GenOut::FullFrame { .. }) {
            late.push(s.late_us / 1000.0);
        }
        w.write(s.t_us, "mtc_out", "send", sent_payload(&s))?;
    }
    bridge_stop.store(true, Ordering::SeqCst);
    let _ = bridge_thread.join();
    let now = clock.now_us();
    let end = json!({
        "generator": { "quarter_frames": stats.quarter_frames, "full_frames": stats.full_frames, "starts": stats.starts,
            "jumps": stats.jumps, "resyncs": stats.resyncs, "stops": stats.stops },
        "qf_late_ms": dist(&late).map(|d| json!({"median": d.median, "p95": d.p95, "max": d.max, "n": d.n})),
        "bridge": omoiiro_runtime::sync::bridge::snapshot_json(&dec.lock().unwrap_or_else(|e| e.into_inner()).bridge_view(now).0),
        "verify_mtc": verify_name.as_ref().map(|_| omoiiro_runtime::diagnostics::report::mtc_json(&mtc.snapshot(now))),
        "dropped": { "bridge": bridge_dropped.load(Ordering::Relaxed), "sent_records": sent_dropped.load(Ordering::Relaxed),
            "verify": verify_dropped.load(Ordering::Relaxed) },
    });
    w.write(now, "meta", "end", end.clone())?;
    w.flush()?;
    println!("{}", serde_json::to_string_pretty(&end)?);
    Ok(())
}

fn dj(d: &Option<Dist>) -> Value {
    d.map(|d| json!({"median": d.median, "p95": d.p95, "max": d.max, "n": d.n})).unwrap_or(Value::Null)
}

fn ds(d: &Option<Dist>) -> String {
    d.map(|d| format!("median {:.2} p95 {:.2} max {:.2} ms (n={})", d.median, d.p95, d.max, d.n)).unwrap_or_else(|| "--".into())
}

pub fn run_report(path: &Path, json_out: bool) -> Result<(), Box<dyn std::error::Error>> {
    let r = analyse(&read_jsonl(path)?);
    let relock_vals: Vec<f64> = r.relock_ms.iter().filter_map(|(_, _, v)| *v).collect();
    if json_out {
        let v = json!({
            "quarter_frames": r.quarter_frames,
            "qf_late_ms": dj(&r.qf_late_ms),
            "qf_interval_ms": dj(&r.qf_interval_ms),
            "full_frames": r.full_frames.iter().map(|(t, why, l)| json!({"t_s": us_to_secs(*t), "reason": why, "label": l})).collect::<Vec<_>>(),
            "relock_ms": r.relock_ms.iter().map(|(t, why, v)| json!({"t_s": us_to_secs(*t), "reason": why, "ms": v})).collect::<Vec<_>>(),
            "relock_summary_ms": dj(&dist(&relock_vals)),
            "label_minus_source_ms": dj(&r.label_minus_source_ms),
            "received_minus_bridge_ms": dj(&r.received_minus_bridge_ms),
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    println!("file: {}", path.display());
    println!("quarter frames sent: {}", r.quarter_frames);
    println!("QF send lateness (sent - scheduled): {}", ds(&r.qf_late_ms));
    println!("QF interval within runs: {}", ds(&r.qf_interval_ms));
    println!("full frames: {}", r.full_frames.len());
    for (t, why, l) in &r.full_frames {
        println!("  t={:10.6}s {l} ({why})", us_to_secs(*t));
    }
    println!("receiver lock after start / jump / resync Full Frames: {}", ds(&dist(&relock_vals)));
    for (t, why, v) in &r.relock_ms {
        println!("  t={:10.6}s {why}: {}", us_to_secs(*t), v.map(|v| format!("{v:.1} ms")).unwrap_or_else(|| "no lock".into()));
    }
    println!("sent label - source position (piece 0): {}", ds(&r.label_minus_source_ms));
    println!("received MTC - bridge position (playing, both locked): {}", ds(&r.received_minus_bridge_ms));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use omoiiro_runtime::diagnostics::record::{hex, Record};
    use omoiiro_runtime::sync::mtc_out::Desired;

    #[test]
    fn origin_and_rate_parsing() {
        assert_eq!(parse_origin_us("01:00:00:00", FrameRate::Fps30), Ok(3_600_000_000.0));
        assert!(parse_origin_us("01:00:00:30", FrameRate::Fps30).is_err());
        assert!(parse_origin_us("01:00:00:24", FrameRate::Fps25).is_ok());
        assert!(rate_of(29).is_err());
    }

    /// Records as `mtc-out --verify-in` writes them, from an ideal simulation.
    #[test]
    fn report_measures_relock_and_differences() {
        let rec = |seq: u64, t: i64, source: &str, kind: &str, payload: Value| Record { run_id: "t".into(), seq, t_mono_us: t, source: source.into(), kind: kind.into(), payload };
        let origin = 3_600_000_000.0;
        let mut v = vec![rec(0, 0, "meta", "start", json!({"origin_us": origin, "input_offset_ms": 0.0}))];
        let mut gen = MtcGen::new(FrameRate::Fps30).unwrap();
        let mut seq = 1;
        let mut t = 0.0f64;
        let mut bridge_seq = 0;
        let mut next_bridge = 0.0;
        while t < 1_000_000.0 {
            // Bridge sample every 16 ms (song from 0 at t = 0).
            while next_bridge <= t {
                bridge_seq += 1;
                v.push(rec(seq, next_bridge as i64, "bridge", "sample", json!({"v": 2, "seq": bridge_seq, "song_ms": next_bridge / 1000.0, "is_playing": true, "errors": []})));
                seq += 1;
                next_bridge += 16_000.0;
            }
            let mut out = Vec::new();
            let wake = gen.step(t, Desired::Run { pos_us: origin + t }, 0, &mut out);
            for o in out {
                let s = Sent { t_us: t as i64, late_us: 0.0, src_pos_us: Some(origin + t), out: o };
                v.push(rec(seq, t as i64, "mtc_out", "send", sent_payload(&s)));
                seq += 1;
                let bytes = match &s.out {
                    GenOut::FullFrame { bytes, .. } => bytes.to_vec(),
                    GenOut::Qf { data, .. } => vec![0xF1, *data],
                };
                v.push(rec(seq, t as i64 + 300, "mtc", "midi", json!({"bytes": hex(&bytes), "backend_ts_us": 0})));
                seq += 1;
            }
            t = wake.max(t + 1.0);
        }
        let r = analyse(&v);
        assert_eq!(r.full_frames.len(), 1);
        let (_, why, ms) = &r.relock_ms[0];
        assert_eq!(why, "start");
        let ms = ms.expect("locked");
        // Next frame boundary + 2 sequences (~0.17 s).
        assert!(ms > 150.0 && ms < 220.0, "{ms}");
        let ld = r.label_minus_source_ms.unwrap();
        assert!(ld.max <= 34.0, "{ld:?}");
        let rd = r.received_minus_bridge_ms.unwrap();
        assert!(rd.median.abs() < 34.0, "{rd:?}");
        let iv = r.qf_interval_ms.unwrap();
        assert!((iv.median - 8.333).abs() < 0.01, "{iv:?}");
    }
}
