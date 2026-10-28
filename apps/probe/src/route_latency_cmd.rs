//! `route-latency`: the operation route (control input callback → Live
//! output send) while a show is followed, with a fake MIDI backend and a
//! synthetic bridge. Run it with a light and a heavy show to see whether
//! the LED rendering drags the forwarding. Synthetic: no device, no Live.

use omoiiro_midi::backend::MidiBackend;
use omoiiro_midi::fake::fake_backend;
use omoiiro_midi::ports::names;
use omoiiro_runtime::follow::SharedBackend;
use omoiiro_runtime::session::{FollowSession, LedOut, SessionConfig};
use serde_json::json;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(clap::Args, Debug)]
pub struct RouteLatencyArgs {
    /// Show to follow.
    #[arg(long)]
    pub show: PathBuf,
    /// Seconds of following.
    #[arg(long, default_value_t = 20)]
    pub seconds: u64,
    /// Pad presses per second sent through the route (each a Note On and a Note Off).
    #[arg(long, default_value_t = 20)]
    pub rate: u32,
}

pub fn run(args: RouteLatencyArgs) -> Result<(), Box<dyn std::error::Error>> {
    let show = omoiiro_store::load_path(&args.show)?;
    let (be, ports) = fake_backend();
    ports.add_input("Pads");
    ports.add_output(names::CONTROL_PORT);
    let backend: SharedBackend = Arc::new(Mutex::new(Box::new(be) as Box<dyn MidiBackend>));
    let port = UdpSocket::bind("127.0.0.1:0")?.local_addr()?.port();
    let cfg = SessionConfig {
        time_source: Default::default(),
        show,
        bind: format!("127.0.0.1:{port}"),
        clock_in: None,
        link: false,
        led_out: LedOut::Virtual,
        control: Some(("Pads".into(), names::CONTROL_PORT.into())),
        timecode_out: None,
        record: None,
        meta: json!({"command": "route-latency"}),
        safety: Default::default(),
        identify_led: false,
    };
    let s = FollowSession::start(cfg, backend)?;
    // Synthetic bridge: playing from 0 at 60 Hz.
    let stop = Arc::new(AtomicBool::new(false));
    let bridge = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let sock = UdpSocket::bind("127.0.0.1:0").expect("bridge socket");
            let t0 = Instant::now();
            let mut seq = 0u64;
            while !stop.load(Ordering::SeqCst) {
                seq += 1;
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                let rec = json!({"v": 2, "seq": seq, "kind": "listener", "event": "song_time", "song_ms": ms, "beats": ms / 500.0, "tempo": 128.0, "is_playing": true, "errors": []});
                let _ = sock.send_to(rec.to_string().as_bytes(), ("127.0.0.1", port));
                std::thread::sleep(Duration::from_micros(16_667));
            }
        })
    };
    std::thread::sleep(Duration::from_secs(1));
    // Presses across the grid (LPX programmer-mode notes 11..88).
    let t0 = Instant::now();
    let gap = Duration::from_secs_f64(1.0 / args.rate.max(1) as f64);
    let mut k = 0u32;
    while t0.elapsed() < Duration::from_secs(args.seconds) {
        let note = 11 + (k % 8) as u8 + 10 * ((k / 8) % 8) as u8;
        ports.inject("Pads", &[0x90, note, 100]);
        std::thread::sleep(gap / 2);
        ports.inject("Pads", &[0x80, note, 0]);
        std::thread::sleep(gap / 2);
        k += 1;
    }
    std::thread::sleep(Duration::from_millis(1500)); // diagnostics are published about once a second
    let st = s.status();
    let d = &st.follow.diag;
    stop.store(true, Ordering::SeqCst);
    let _ = bridge.join();
    s.stop()?;
    let sent = ports.sent(names::CONTROL_PORT).len();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "show": args.show.display().to_string(),
            "presses": k, "messages_forwarded": sent,
            "callback_to_send_us": {"n": d.callback_to_send.n, "p50": d.callback_to_send.p50_us, "p95": d.callback_to_send.p95_us, "p99": d.callback_to_send.p99_us, "max": d.callback_to_send.max_us},
            "render_us": {"p50": d.render.p50_us, "p95": d.render.p95_us, "max": d.render.max_us},
            "frames": d.frames, "skipped_frames": d.skipped_frames,
            "control_errors": d.control_errors, "render_panics": st.follow.render_panics,
        }))?
    );
    Ok(())
}