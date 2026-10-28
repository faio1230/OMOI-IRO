//! `transport`: the Runtime `Transport` (bridge + MIDI Clock or Link) printed
//! once a second. The step before M4's follow mode.
//!
//! Threads: the bridge UDP receiver and the midir callback (or Link
//! callbacks) only stamp the session monotonic clock, copy and `try_send`;
//! this thread feeds the transport, writes JSONL and prints.

use crossbeam_channel::{bounded, RecvTimeoutError};
use serde_json::json;
use omoiiro_core::time::FrameRate;
use omoiiro_midi::backend::{Ingress, IngressStats, MidiBackend, RawIn};
use omoiiro_midi::clock::MonotonicClock;
use omoiiro_midi::midir_backend::MidirBackend;
use omoiiro_runtime::diagnostics::record::{midi_payload, JsonlWriter};
use omoiiro_runtime::diagnostics::report::{clock_event_line, us_to_secs};
use omoiiro_runtime::link::LinkRaw;
use omoiiro_runtime::sync::bridge::{event_line, BridgeSample};
use omoiiro_runtime::sync::bridge_ingress::{datagram_payload, spawn_receiver, RawDatagram};
use omoiiro_runtime::sync::transport::{origin_label, snapshot_json, status_line, BeatSourceKind, Transport, TransportConfig};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(clap::Args, Debug)]
pub struct TransportArgs {
    /// Show file: sync settings (beat source, offset, origin, loss, stop behavior) come from it.
    #[arg(long)]
    pub show: Option<PathBuf>,
    /// UDP address for OmoiIroBridge samples.
    #[arg(long, default_value = omoiiro_runtime::sync::bridge_ingress::DEFAULT_BIND)]
    pub bind: String,
    /// MIDI Clock input (beat source midi_clock): exact name, `#index` or unique substring.
    #[arg(long)]
    pub clock_in: Option<String>,
    /// Observe Ableton Link as the beat source (build with `--features link`). Start Stop Sync stays off.
    #[arg(long)]
    pub link: bool,
    /// Without --show: label of the song start.
    #[arg(long, default_value = "01:00:00:00")]
    pub origin: String,
    /// Without --show: input offset in ms.
    #[arg(long, default_value_t = 0.0)]
    pub input_offset_ms: f64,
    /// JSONL output path (optional).
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Run identifier. Default: transport-<unix seconds>
    #[arg(long)]
    pub run_id: Option<String>,
    /// Stop after this many seconds. Default: run until Ctrl+C.
    #[arg(long)]
    pub duration: Option<u64>,
}

pub fn run(args: TransportArgs) -> Result<(), Box<dyn std::error::Error>> {
    if args.link && !cfg!(feature = "link") {
        return Err("this binary was built without the `link` feature (see BUILD.md)".into());
    }
    let cfg = match &args.show {
        Some(p) => TransportConfig::from_show(&omoiiro_store::load_path(p)?)?,
        None => {
            let beat = if args.link {
                Some(BeatSourceKind::Link)
            } else if args.clock_in.is_some() {
                Some(BeatSourceKind::MidiClock)
            } else {
                None
            };
            TransportConfig::defaults(origin_label(&args.origin, FrameRate::Fps30)?, (args.input_offset_ms * 1000.0).round() as i64, beat)
        }
    };
    // The show selects one beat source; the matching input must be given. No fallback.
    match cfg.beat_source {
        Some(BeatSourceKind::MidiClock) if args.clock_in.is_none() => {
            eprintln!("note: the show's beat source is midi_clock but no --clock-in was given; the beat stays unknown");
        }
        Some(BeatSourceKind::Link) if !args.link => {
            eprintln!("note: the show's beat source is ableton_link but no --link was given; the beat stays unknown");
        }
        _ => {}
    }
    if args.show.is_none() && args.link && args.clock_in.is_some() {
        return Err("give one beat source: --clock-in or --link".into());
    }
    let beat_source = cfg.beat_source;

    let clock = MonotonicClock::new();
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))?;
    }

    let (bridge_tx, bridge_rx) = bounded::<RawDatagram>(8192);
    let bridge_dropped = Arc::new(AtomicU64::new(0));
    let bridge_stop = Arc::new(AtomicBool::new(false));
    let bridge_thread = spawn_receiver(&args.bind, clock.clone(), bridge_tx, bridge_dropped.clone(), bridge_stop.clone())
        .map_err(|e| format!("bind UDP {}: {e}", args.bind))?;

    let (clock_tx, clock_rx) = bounded::<RawIn>(8192);
    let clock_stats = Arc::new(IngressStats::default());
    let mut backend = MidirBackend::new("omoiiro-probe-transport");
    let mut clock_name = None;
    let _clock_conn = match (&args.clock_in, beat_source) {
        (Some(spec), Some(BeatSourceKind::MidiClock)) => {
            let c = backend
                .open_input(spec, false, Ingress::new(clock.clone(), clock_tx.clone(), clock_stats.clone()))
                .map_err(|e| format!("open clock input: {e}"))?;
            clock_name = Some(c.name.clone());
            Some(c)
        }
        _ => None,
    };
    let _clock_keepalive = clock_tx;

    let (link_tx, link_rx) = bounded::<(i64, LinkRaw)>(1024);
    #[cfg(feature = "link")]
    let mut link_obs = (args.link && beat_source == Some(BeatSourceKind::Link))
        .then(|| omoiiro_runtime::link::observer::LinkObserver::start(&clock, link_tx.clone(), Arc::new(AtomicU64::new(0)), false));
    drop(link_tx);

    let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let run_id = args.run_id.clone().unwrap_or_else(|| format!("transport-{unix}"));
    let mut w = match &args.out {
        Some(p) => Some(JsonlWriter::create(p, &run_id)?),
        None => None,
    };
    if let Some(w) = w.as_mut() {
        w.write(
            clock.now_us(),
            "meta",
            "start",
            json!({
                "command": "transport", "unix_s": unix, "show": args.show.as_ref().map(|p| p.display().to_string()),
                "bridge_udp": args.bind, "clock_in": clock_name, "link": args.link,
                "beat_source": format!("{:?}", cfg.beat_source), "input_offset_us": cfg.input_offset_us,
                "origin": cfg.origin.to_string(), "holdover_us": cfg.holdover_us, "blackout_us": cfg.blackout_us,
                "stop_behavior": format!("{:?}", cfg.stop_behavior), "recovery_fade_us": cfg.recovery_fade_us,
            }),
        )?;
    }
    println!("bridge UDP  : {}", args.bind);
    println!("beat source : {:?}{}", cfg.beat_source, clock_name.as_ref().map(|n| format!(" ({n})")).unwrap_or_default());
    println!("offset {} us, origin {}, holdover {} ms, blackout {} ms, stop {:?}, recovery {} ms",
        cfg.input_offset_us, cfg.origin, cfg.holdover_us / 1000, cfg.blackout_us / 1000, cfg.stop_behavior, cfg.recovery_fade_us / 1000);

    let mut transport = Transport::new(cfg);
    let (mut bev, mut cev) = (Vec::new(), Vec::new());
    let mut next_print = clock.now_us() + 1_000_000;
    #[cfg(feature = "link")]
    let mut next_link_sample = clock.now_us();
    let deadline = args.duration.map(|d| d as i64 * 1_000_000);
    println!("running (Ctrl+C to stop)...");
    loop {
        match bridge_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(d) => {
                let payload = datagram_payload(&d.bytes);
                if let Some(w) = w.as_mut() {
                    w.write(d.t_mono_us, "bridge", "sample", payload.clone())?;
                }
                bev.clear();
                transport.feed_bridge(BridgeSample::from_payload(d.t_mono_us, &payload), &mut bev);
                for e in &bev {
                    println!("  t={:10.6}s bridge {}", us_to_secs(d.t_mono_us), event_line(e));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        for ev in clock_rx.try_iter() {
            if let Some(w) = w.as_mut() {
                w.write(ev.t_mono_us, "clock", "midi", midi_payload(&ev.bytes, ev.backend_ts_us))?;
            }
            cev.clear();
            transport.feed_clock(ev.t_mono_us, &ev.bytes, &mut cev);
            for e in &cev {
                println!("  t={:10.6}s clock  {}", us_to_secs(ev.t_mono_us), clock_event_line(e));
            }
        }
        for (t, raw) in link_rx.try_iter() {
            for line in transport.feed_link_raw(t, raw) {
                println!("  t={:10.6}s link   {line}", us_to_secs(t));
            }
        }
        #[cfg(feature = "link")]
        if let Some(o) = link_obs.as_mut() {
            let t = clock.now_us();
            if t >= next_link_sample {
                for line in transport.feed_link_sample(t, o.sample()) {
                    println!("  t={:10.6}s link   {line}", us_to_secs(t));
                }
                next_link_sample = t + 100_000;
            }
        }
        let now = clock.now_us();
        cev.clear();
        transport.poll_clock(now, &mut cev);
        if now >= next_print {
            let snap = transport.snapshot(now);
            println!("[{:9.3}s] | {}", us_to_secs(now), status_line(&snap));
            if let Some(w) = w.as_mut() {
                w.write(now, "transport", "state", snapshot_json(&snap))?;
                w.flush()?;
            }
            next_print = now + 1_000_000;
        }
        if stop.load(Ordering::SeqCst) || deadline.is_some_and(|d| now >= d) {
            break;
        }
    }
    bridge_stop.store(true, Ordering::SeqCst);
    let _ = bridge_thread.join();
    let now = clock.now_us();
    let end = json!({
        "transport": snapshot_json(&transport.snapshot(now)),
        "dropped": { "bridge": bridge_dropped.load(Ordering::Relaxed), "clock": clock_stats.dropped() },
    });
    if let Some(w) = w.as_mut() {
        w.write(now, "meta", "end", end.clone())?;
        w.flush()?;
    }
    println!("{}", serde_json::to_string_pretty(&end)?);
    Ok(())
}
