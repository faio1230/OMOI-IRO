//! `follow`: headless follow mode. DJSCBridge-compatible samples ->
//! `Transport` -> Core render at the show's fps -> LED writer (in-process
//! virtual Launchpad X by default, or a MIDI output port), with an optional
//! operation route (control input -> Live control bus) beside it.
//!
//! The session itself (threads, start / stop) is
//! `omoiiro_runtime::session::FollowSession`, shared with the desktop shell.
//! This command prints the status once a second and reads stdin commands:
//! `blackout on|off`, `brightness <0..1>`, `reload`, `reset`, `reset-input`, `quit`.
//! With `--out` the record also holds every pad press and release with its
//! show time (`make-recording` turns them into a recorded clip).
//!
//! A real Launchpad X is not driven before M6: ports that look like a
//! Launchpad are refused unless `--allow-device` is given.

use omoiiro_midi::backend::MidiBackend;
use omoiiro_midi::midir_backend::MidirBackend;
use omoiiro_midi::ports;
use omoiiro_runtime::diagnostics::report::us_to_secs;
use omoiiro_runtime::follow::SharedBackend;
use omoiiro_runtime::session::{FollowSession, LedOut, SessionConfig};
use omoiiro_runtime::sync::transport::status_line;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `--led-out` value for the in-process virtual Launchpad X.
pub const VIRTUAL_LED: &str = "virtual";

#[derive(clap::Args, Debug)]
pub struct FollowArgs {
    /// Show file (validated; sync and render settings come from it).
    #[arg(long)]
    pub show: PathBuf,
    /// UDP address for OmoiIroBridge samples.
    #[arg(long, default_value = omoiiro_runtime::sync::bridge_ingress::DEFAULT_BIND)]
    pub bind: String,
    /// MIDI Clock input when the show's beat source is midi_clock (exact name, `#index` or unique substring).
    #[arg(long)]
    pub clock_in: Option<String>,
    /// Observe Ableton Link when the show's beat source is ableton_link (build with `--features link`).
    #[arg(long)]
    pub link: bool,
    /// LED output: `virtual` (in-process virtual Launchpad X) or an output port (exact name or `#index`).
    #[arg(long, default_value = VIRTUAL_LED)]
    pub led_out: String,
    /// Allow a port that looks like a Launchpad as --led-out (device work is M6).
    #[arg(long)]
    pub allow_device: bool,
    /// Operation input (exact name or `#index`); enables the route to --control-out.
    #[arg(long)]
    pub control_in: Option<String>,
    /// Live control bus (exact name or `#index`).
    #[arg(long, default_value = ports::names::CONTROL_PORT)]
    pub control_out: String,
    /// MTC output port (exact name or `#index`), sent when the show enables `sync.mtc_output`.
    #[arg(long)]
    pub timecode_out: Option<String>,
    /// Print the virtual Launchpad X grid with the status line.
    #[arg(long)]
    pub view: bool,
    /// JSONL output path (optional): 1 s state records and the end summary.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Run identifier. Default: follow-<unix seconds>
    #[arg(long)]
    pub run_id: Option<String>,
    /// Stop after this many seconds. Default: until Ctrl+C or `quit`.
    #[arg(long)]
    pub duration: Option<u64>,
}

fn spawn_stdin(tx: crossbeam_channel::Sender<String>) {
    std::thread::Builder::new()
        .name("follow-stdin".into())
        .spawn(move || {
            let mut line = String::new();
            while std::io::stdin().read_line(&mut line).map(|n| n > 0).unwrap_or(false) {
                if tx.send(line.trim().to_string()).is_err() {
                    break;
                }
                line.clear();
            }
        })
        .expect("spawn stdin reader");
}

pub fn run(args: FollowArgs) -> Result<(), Box<dyn std::error::Error>> {
    let show = omoiiro_store::load_path(&args.show)?;
    let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let run_id = args.run_id.clone().unwrap_or_else(|| format!("follow-{unix}"));
    let led_out = if args.led_out == VIRTUAL_LED {
        LedOut::Virtual
    } else {
        LedOut::Port { spec: args.led_out.clone(), allow_device: args.allow_device }
    };
    let cfg = SessionConfig {
        show,
        bind: args.bind.clone(),
        clock_in: args.clock_in.clone(),
        link: args.link,
        led_out,
        control: args.control_in.clone().map(|i| (i, args.control_out.clone())),
        timecode_out: args.timecode_out.clone(),
        safety: Default::default(),
        // A real device (allowed only with --allow-device) is identified first.
        identify_led: args.allow_device,
        record: args.out.clone().map(|p| (p, run_id)),
        meta: json!({
            "command": "follow", "unix_s": unix, "show": args.show.display().to_string(),
            "led_out": args.led_out, "control_in": args.control_in,
            "control_out": args.control_in.as_ref().map(|_| &args.control_out),
        }),
    };
    let backend: SharedBackend = Arc::new(Mutex::new(Box::new(MidirBackend::new("omoiiro-follow")) as Box<dyn MidiBackend>));
    let mut session = FollowSession::start(cfg, backend)?;
    for n in &session.notes {
        eprintln!("note: {n}");
    }

    println!("show        : {} (revision {})", args.show.display(), session.revision);
    println!("bridge UDP  : {}", args.bind);
    println!("LED out     : {}", args.led_out);
    if let Some(c) = &args.control_in {
        println!("control     : {c} -> {}", args.control_out);
    }
    println!("commands    : blackout on|off, brightness <0..1>, reload, reset, reset-input, quit");

    let (cmd_tx, cmd_rx) = crossbeam_channel::bounded::<String>(16);
    spawn_stdin(cmd_tx);
    let stop_flag = Arc::new(AtomicBool::new(false));
    {
        let s = stop_flag.clone();
        ctrlc::set_handler(move || s.store(true, Ordering::SeqCst))?;
    }
    let deadline = args.duration.map(|d| d as i64 * 1_000_000);
    let mut next_print = 1_000_000;
    loop {
        std::thread::sleep(Duration::from_millis(20));
        for e in session.drain_events() {
            println!("  {e}");
        }
        for cmd in cmd_rx.try_iter() {
            let mut parts = cmd.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some("blackout"), Some(v)) => {
                    session.set_blackout(v == "on");
                    println!("blackout {}", if session.status().blackout { "on" } else { "off" });
                }
                (Some("brightness"), Some(v)) => match v.parse::<f32>() {
                    Ok(b) => {
                        session.set_brightness(b);
                        println!("brightness {}", session.status().brightness);
                    }
                    Err(_) => println!("brightness needs a number 0..1"),
                },
                // Parse and validate here, never on the render thread.
                (Some("reload"), None) => match omoiiro_store::load_path(&args.show).map_err(|e| e.to_string()).and_then(|s| session.submit_show(&s).map_err(|i| format!("{i:?}"))) {
                    Ok(r) => println!("reloaded: revision {r} (adopted at the next frame)"),
                    Err(e) => println!("reload rejected, keeping the current show: {e}"),
                },
                (Some("reset"), None) => {
                    session.reset_control_fault();
                    println!("control route fault reset requested");
                }
                (Some("reset-input"), None) => {
                    session.reset_input();
                    println!("pad input history cleared");
                }
                (Some("quit"), None) => stop_flag.store(true, Ordering::SeqCst),
                _ => println!("unknown command {cmd:?}"),
            }
        }
        let st = session.status();
        if st.uptime_us >= next_print {
            let f = &st.follow;
            println!(
                "[{:9.3}s] frame {} rev {} {}gain {:.2} | LED {}{} lit {}{} | {}",
                us_to_secs(st.uptime_us),
                f.frame_index,
                f.revision,
                f.dark.map(|d| format!("dark={d} ")).unwrap_or_default(),
                f.gain,
                f.led_state.unwrap_or("-"),
                f.led_generation.map(|g| format!(" gen {g}")).unwrap_or_default(),
                f.led_lit,
                st.control_fault.as_ref().map(|c| format!(" | CONTROL FAULT: {c}")).unwrap_or_default(),
                status_line(&st.transport),
            );
            if args.view {
                if let Some(v) = &f.virtual_view {
                    print!("{v}");
                }
            }
            next_print = st.uptime_us + 1_000_000;
        }
        if stop_flag.load(Ordering::SeqCst) || deadline.is_some_and(|d| st.uptime_us >= d) {
            break;
        }
    }
    let end = session.stop()?;
    println!("{}", serde_json::to_string_pretty(&end)?);
    Ok(())
}
