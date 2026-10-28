//! Live observation of MTC and MIDI Clock inputs with JSONL recording.
//!
//! Thread layout:
//! * MIDI inputs are opened through the `MidiBackend` contract (midir).
//!   Each port's ingress is tagged with its role (mtc / clock / sync); the
//!   callback takes the monotonic time, copies the bytes and `try_send`s
//!   into one bounded channel. Nothing else. A full channel increments a
//!   drop counter instead of blocking.
//! * pipeline thread (this function): drains the channel, feeds the
//!   decoders, writes JSONL, prints a status line at a fixed interval.
//! * Link (`--link`, feature `link`): Link's callback threads stamp the same
//!   monotonic clock and `try_send` into a second bounded channel. The
//!   pipeline drains it, captures (never commits) the session state once a
//!   second, and records `source = link` (`kind = event` for changes,
//!   `kind = state` for the 1 s samples).

use omoiiro_midi::backend::{Ingress, IngressStats, MidiBackend, RawIn};
use omoiiro_midi::clock::MonotonicClock;
use omoiiro_midi::midir_backend::MidirBackend;
use omoiiro_midi::ports;
use omoiiro_runtime::diagnostics::record::{midi_payload, JsonlWriter};
use omoiiro_runtime::diagnostics::report::{clock_event_line, clock_json, clock_line, mtc_event_is_notable, mtc_event_line, mtc_json, mtc_line, us_to_secs};
use omoiiro_runtime::link::{link_json, link_line, LinkRaw, LinkTracker};
use omoiiro_runtime::sync::clock::ClockDecoder;
use omoiiro_runtime::sync::mtc::{MtcDecoder, Params};
use crossbeam_channel::{bounded, RecvTimeoutError};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(clap::Args, Debug)]
pub struct MonitorArgs {
    /// MTC input port: exact name, `#index`, or a unique substring.
    #[arg(long)]
    pub mtc_in: Option<String>,
    /// MIDI Clock input port (alternative beat input), same spec forms.
    #[arg(long)]
    pub clock_in: Option<String>,
    /// Observe Ableton Link (needs a build with `--features link`). Observation only.
    #[arg(long)]
    pub link: bool,
    /// With --link: enable Start Stop Sync on this peer so that is_playing is received.
    /// Off by default: with it on, Live 12.4.7 in the same session did not advance its
    /// transport after Play (measured 2026-10-28, S07). Use only for that investigation.
    #[arg(long)]
    pub link_start_stop_sync: bool,
    /// Accepted for older command lines; Start Stop Sync is off unless --link-start-stop-sync.
    #[arg(long, hide = true)]
    pub link_no_start_stop_sync: bool,
    /// Also receive OmoiIroBridge UDP datagrams on this address (see `bridge-clock`).
    #[arg(long, hide = true)]
    pub bridge_udp: Option<String>,
    /// Timecode label of Live's song start (bridge display only), HH:MM:SS:FF at 30 fps.
    #[arg(long, hide = true)]
    pub bridge_origin: Option<String>,
    /// Fixed input offset added to the bridge position, in ms (display only).
    #[arg(long, hide = true, default_value_t = 0.0)]
    pub bridge_offset_ms: f64,
    /// JSONL output path. Default: artifacts/p0/<run-id>.jsonl
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Run identifier written into every record. Default: monitor-<unix seconds>
    #[arg(long)]
    pub run_id: Option<String>,
    /// Stop after this many seconds. Default: run until Ctrl+C.
    #[arg(long)]
    pub duration: Option<u64>,
    /// Status line interval in milliseconds.
    #[arg(long, default_value_t = 250)]
    pub interval_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Mtc,
    Clock,
    /// One port carries both MTC and Clock; recorded once as `sync`.
    Sync,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Source::Mtc => "mtc",
            Source::Clock => "clock",
            Source::Sync => "sync",
        }
    }

    /// Ingress tag of the role.
    fn tag(self) -> u16 {
        match self {
            Source::Mtc => 1,
            Source::Clock => 2,
            Source::Sync => 3,
        }
    }

    fn from_tag(tag: u16) -> Source {
        match tag {
            1 => Source::Mtc,
            2 => Source::Clock,
            _ => Source::Sync,
        }
    }
}

pub fn run(args: MonitorArgs) -> Result<(), Box<dyn std::error::Error>> {
    if args.mtc_in.is_none() && args.clock_in.is_none() && !args.link && args.bridge_udp.is_none() {
        return Err("nothing to observe: give --mtc-in, --clock-in and/or --link".into());
    }
    if args.link && !cfg!(feature = "link") {
        return Err("this binary was built without the `link` feature; rebuild with `cargo build --features link` (needs cmake and libclang, see README)".into());
    }

    let clock = MonotonicClock::new();
    let (tx, rx) = bounded::<RawIn>(8192);
    let stats = Arc::new(IngressStats::default());
    let ingress = Ingress::new(clock.clone(), tx.clone(), stats.clone());
    let mut backend = MidirBackend::new("omoiiro-probe-monitor");
    let mut connections = Vec::new();

    // Resolve both specs first so that one physical port is opened once.
    let inputs = backend.list_inputs()?;
    let mtc_name = match &args.mtc_in {
        Some(spec) => Some(inputs[ports::resolve(&inputs, spec)?].clone()),
        None => None,
    };
    let clock_name = match &args.clock_in {
        Some(spec) => Some(inputs[ports::resolve(&inputs, spec)?].clone()),
        None => None,
    };
    let shared = mtc_name.is_some() && mtc_name == clock_name;

    if let Some(spec) = &args.mtc_in {
        let source = if shared { Source::Sync } else { Source::Mtc };
        let conn = backend.open_input(spec, false, ingress.clone().tagged(source.tag())).map_err(|e| format!("open MTC input: {e}"))?;
        println!("MTC input   : {}{}", conn.name, if shared { " (shared with Clock, recorded as `sync`)" } else { "" });
        connections.push(conn);
    }
    if let Some(spec) = &args.clock_in {
        if shared {
            println!("Clock input : {} (same port as MTC)", clock_name.as_deref().unwrap_or_default());
        } else {
            let conn = backend.open_input(spec, false, ingress.clone().tagged(Source::Clock.tag())).map_err(|e| format!("open Clock input: {e}"))?;
            println!("Clock input : {}", conn.name);
            connections.push(conn);
        }
    }
    // Keep one sender so that a run with only --link / bridge does not see a
    // disconnected channel and stop at once.
    let _tx_keepalive = tx;

    // Link: second channel, same monotonic clock.
    let (link_tx, link_rx) = bounded::<(i64, LinkRaw)>(1024);
    let link_dropped = Arc::new(AtomicU64::new(0));
    #[cfg(feature = "link")]
    let mut link_obs = if args.link {
        let o = omoiiro_runtime::link::observer::LinkObserver::start(&clock, link_tx.clone(), link_dropped.clone(), args.link_start_stop_sync && !args.link_no_start_stop_sync);
        println!(
            "Link        : joined (observation only; start/stop sync {}). Enable Link in Live before starting this.",
            if o.start_stop_sync() { "on" } else { "off" }
        );
        Some(o)
    } else {
        None
    };
    drop(link_tx);
    let mut link = LinkTracker::new();

    // Bridge: UDP receiver thread, same monotonic clock.
    let (bridge_tx, bridge_rx) = bounded::<omoiiro_runtime::sync::bridge_ingress::RawDatagram>(8192);
    let bridge_dropped = Arc::new(AtomicU64::new(0));
    let bridge_stop = Arc::new(AtomicBool::new(false));
    let bridge_thread = match &args.bridge_udp {
        Some(bind) => {
            let h = omoiiro_runtime::sync::bridge_ingress::spawn_receiver(bind, clock.clone(), bridge_tx.clone(), bridge_dropped.clone(), bridge_stop.clone())
                .map_err(|e| format!("bind UDP {bind}: {e}"))?;
            println!("Bridge UDP  : {bind}");
            Some(h)
        }
        None => None,
    };
    drop(bridge_tx);
    let mut bridge = omoiiro_runtime::sync::bridge::BridgeDecoder::new(omoiiro_runtime::sync::bridge::BridgeParams::default());
    let mut bridge_events = Vec::new();
    let bridge_origin_ms = match &args.bridge_origin {
        Some(o) => crate::gen::parse_label(o)?.to_us().map(|us| us as f64 / 1000.0).unwrap_or(0.0),
        None => 0.0,
    };

    let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let run_id = args.run_id.clone().unwrap_or_else(|| format!("monitor-{unix}"));
    let out_path = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("artifacts/p0").join(format!("{run_id}.jsonl")));
    let mut w = JsonlWriter::create(&out_path, &run_id)?;
    println!("record      : {}", out_path.display());

    let params = Params::default();
    let clk_stall_us = omoiiro_runtime::sync::clock::DEFAULT_STALL_US;
    w.write(
        clock.now_us(),
        "meta",
        "start",
        json!({
            "unix_s": unix,
            "backend": ports::backend_name(),
            "mtc_in": mtc_name,
            "clock_in": clock_name,
            "mtc_params": {
                "stale_us": params.stale_us,
                "holdover_us": params.holdover_us,
                "blackout_us": params.blackout_us,
                "piece_timeout_us": params.piece_timeout_us,
                "ff_jump_threshold_frames": params.ff_jump_threshold_frames,
            },
            "queue_capacity": 8192,
            "clock_stall_us": clk_stall_us,
            "bridge_udp": args.bridge_udp,
            "bridge_origin": args.bridge_origin,
            "bridge_offset_ms": args.bridge_offset_ms,
            "link": args.link.then(|| json!({
                "start_stop_sync": args.link_start_stop_sync && !args.link_no_start_stop_sync,
                "quantum": omoiiro_runtime::link::QUANTUM,
                "initial_bpm": link_initial_bpm(),
                "sample_interval_us": 1_000_000,
            })),
        }),
    )?;

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))?;
    }

    let mut mtc = MtcDecoder::new(params);
    let mut clk = ClockDecoder::new();
    let mut mtc_events = Vec::new();
    let mut clk_events = Vec::new();
    let interval_us = (args.interval_ms.max(50) * 1000) as i64;
    let deadline_us = args.duration.map(|d| d as i64 * 1_000_000);
    let mut next_print = clock.now_us() + interval_us;
    let mut next_flush = clock.now_us() + 1_000_000;
    #[cfg(feature = "link")]
    let mut next_link_sample = clock.now_us();
    println!("observing (Ctrl+C to stop)...");

    loop {
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(ev) => {
                let source = Source::from_tag(ev.tag);
                w.write(ev.t_mono_us, source.name(), "midi", midi_payload(&ev.bytes, ev.backend_ts_us))?;
                if matches!(source, Source::Mtc | Source::Sync) {
                    mtc_events.clear();
                    mtc.feed(ev.t_mono_us, &ev.bytes, &mut mtc_events);
                    for e in &mtc_events {
                        if mtc_event_is_notable(e) {
                            let line = mtc_event_line(e);
                            println!("  t={:10.6}s mtc   {line}", us_to_secs(ev.t_mono_us));
                            w.write(ev.t_mono_us, "mtc", "event", json!({ "event": line }))?;
                        }
                    }
                }
                if matches!(source, Source::Clock | Source::Sync) {
                    clk_events.clear();
                    clk.feed(ev.t_mono_us, &ev.bytes, &mut clk_events);
                    for e in &clk_events {
                        let line = clock_event_line(e);
                        println!("  t={:10.6}s clock {line}", us_to_secs(ev.t_mono_us));
                        w.write(ev.t_mono_us, "clock", "event", json!({ "event": line }))?;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        for d in bridge_rx.try_iter() {
            let payload = omoiiro_runtime::sync::bridge_ingress::datagram_payload(&d.bytes);
            w.write(d.t_mono_us, "bridge", "sample", payload.clone())?;
            bridge_events.clear();
            bridge.feed(omoiiro_runtime::sync::bridge::BridgeSample::from_payload(d.t_mono_us, &payload), &mut bridge_events);
            for e in &bridge_events {
                let line = omoiiro_runtime::sync::bridge::event_line(e);
                println!("  t={:10.6}s bridge {line}", us_to_secs(d.t_mono_us));
                w.write(d.t_mono_us, "bridge", "event", json!({ "event": line }))?;
            }
        }
        for (t, raw) in link_rx.try_iter() {
            w.write(t, "link", "notify", link_raw_json(raw))?;
            for line in link.feed(raw) {
                println!("  t={:10.6}s link  {line}", us_to_secs(t));
                w.write(t, "link", "event", json!({ "event": line }))?;
            }
        }
        #[cfg(feature = "link")]
        if let Some(o) = link_obs.as_mut() {
            let t = clock.now_us();
            if t >= next_link_sample {
                let sample = o.sample();
                w.write(t, "link", "state", omoiiro_runtime::link::sample_json(&sample))?;
                for line in link.sample(sample) {
                    println!("  t={:10.6}s link  {line}", us_to_secs(t));
                    w.write(t, "link", "event", json!({ "event": line }))?;
                }
                next_link_sample = t + 1_000_000;
            }
        }
        let now = clock.now_us();
        if args.clock_in.is_some() {
            clk_events.clear();
            clk.poll(now, &mut clk_events);
            for e in &clk_events {
                let line = clock_event_line(e);
                println!("  t={:10.6}s clock {line}", us_to_secs(now));
                w.write(now, "clock", "event", json!({ "event": line }))?;
            }
        }
        if now >= next_print {
            let mut parts = vec![format!("[{:9.3}s]", us_to_secs(now))];
            if args.mtc_in.is_some() {
                parts.push(mtc_line(&mtc.snapshot(now)));
            }
            if args.clock_in.is_some() {
                parts.push(clock_line(&clk.snapshot(now)));
            }
            if args.link {
                parts.push(link_line(&link.snapshot()));
            }
            if args.bridge_udp.is_some() {
                parts.push(bridge_line(&bridge.snapshot(now), bridge_origin_ms + args.bridge_offset_ms));
            }
            parts.push(format!("q_drop={}", stats.dropped()));
            println!("{}", parts.join(" | "));
            next_print += interval_us;
            if next_print < now {
                next_print = now + interval_us;
            }
        }
        if now >= next_flush {
            w.flush()?;
            next_flush = now + 1_000_000;
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if let Some(d) = deadline_us {
            if now >= d {
                break;
            }
        }
    }

    let now = clock.now_us();
    let q_drop = stats.dropped();
    let mut end = json!({ "queue_dropped": q_drop, "records_before_end": w.written() });
    if stats.oversize() > 0 {
        end["queue_oversize"] = json!(stats.oversize());
    }
    if args.mtc_in.is_some() {
        end["mtc"] = mtc_json(&mtc.snapshot(now));
    }
    if args.clock_in.is_some() {
        end["clock"] = clock_json(&clk.snapshot(now));
    }
    if args.bridge_udp.is_some() {
        end["bridge"] = omoiiro_runtime::sync::bridge::snapshot_json(&bridge.snapshot(now));
        end["bridge_queue_dropped"] = json!(bridge_dropped.load(Ordering::Relaxed));
    }
    if args.link {
        end["link"] = link_json(&link.snapshot());
        end["link_queue_dropped"] = json!(link_dropped.load(Ordering::Relaxed));
    }
    w.write(now, "meta", "end", end.clone())?;
    w.flush()?;
    drop(connections);
    bridge_stop.store(true, Ordering::SeqCst);
    if let Some(h) = bridge_thread {
        let _ = h.join();
    }
    #[cfg(feature = "link")]
    drop(link_obs.take());
    println!("stopped at t={:.3}s; queue dropped={q_drop}; records={}", us_to_secs(now), w.written());
    println!("{}", serde_json::to_string_pretty(&end)?);
    Ok(())
}

fn link_raw_json(raw: LinkRaw) -> serde_json::Value {
    match raw {
        LinkRaw::Peers(n) => json!({ "callback": "num_peers", "value": n }),
        LinkRaw::Tempo(bpm) => json!({ "callback": "tempo", "value": bpm }),
        LinkRaw::StartStop(p) => json!({ "callback": "start_stop", "value": p }),
    }
}

fn link_initial_bpm() -> Option<f64> {
    #[cfg(feature = "link")]
    return Some(omoiiro_runtime::link::observer::INITIAL_BPM);
    #[cfg(not(feature = "link"))]
    None
}

/// Bridge status plus the timecode label (`origin + song + offset`, 30 fps).
fn bridge_line(snap: &omoiiro_runtime::sync::bridge::BridgeSnapshot, origin_plus_offset_ms: f64) -> String {
    let label = snap
        .song_ms
        .map(|ms| {
            let frames = ((ms + origin_plus_offset_ms) * 30.0 / 1000.0).floor().max(0.0) as u64;
            omoiiro_runtime::sync::mtc::Label::from_frame_index(frames, omoiiro_runtime::sync::mtc::FrameRate::Fps30).to_string()
        })
        .unwrap_or_else(|| "--:--:--:--".into());
    format!("{} tc={label}", omoiiro_runtime::sync::bridge::status_line(snap))
}
