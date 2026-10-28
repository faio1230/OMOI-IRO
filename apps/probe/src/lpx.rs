//! `lpx-check`: Launchpad X identity, mode, one low-brightness RGB pad, pad
//! input, and restore (runbook C, steps 1-7).
//!
//! Order and guarantees:
//! 1. Ports are exact names or `#index`. A missing port is an error.
//! 2. Device Inquiry (F0 7E 7F 06 01 F7). No valid Launchpad X reply within
//!    the timeout: stop with an error, having sent nothing else.
//! 3. Mode query (F0 00 20 29 02 0C 0E F7). No reply: stop, nothing else sent.
//! 4. Programmer Mode (... 0E 01 F7), confirmed by a second mode query, then
//!    one pad in low-brightness RGB (... 03 03 <pad> r g b F7).
//! 5. Pad press / release are received (Note On velocity 0 is a release) and
//!    optionally forwarded to the Live control bus through `route`.
//! 6. On normal end and on Ctrl+C alike: the pad is turned off, Live Mode is
//!    selected (... 0E 00 F7) and confirmed by a mode query. Failures are
//!    reported, not hidden.
//!
//! The SysEx builders and reply parsers are pure and unit tested. The
//! session runs against any `MidiSend` and a channel of received messages,
//! so the whole sequence is tested with a simulated device. Those tests are
//! not a Launchpad X measurement.

use crate::route::{self, handle_input, MidiSend, RawIn, RouteStats};
use omoiiro_devices::launchpad_x::*;
use omoiiro_midi::clock::MonotonicClock;
use omoiiro_midi::ports;
use omoiiro_runtime::diagnostics::record::{hex, midi_payload, JsonlWriter};
use omoiiro_runtime::diagnostics::report::us_to_secs;
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug)]
pub struct CheckConfig {
    pub pad: u8,
    pub rgb: (u8, u8, u8),
    pub reply_timeout_us: i64,
    pub allow_unknown_family: bool,
}

impl Default for CheckConfig {
    fn default() -> Self {
        CheckConfig { pad: 11, rgb: (10, 0, 0), reply_timeout_us: 1_000_000, allow_unknown_family: false }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cleanup {
    pub led_off_sent: bool,
    pub live_mode_sent: bool,
    /// Mode reported after restoring; `None` if the device did not answer.
    pub confirmed_mode: Option<u8>,
    pub errors: Vec<String>,
}

impl Cleanup {
    pub fn ok(&self) -> bool {
        self.led_off_sent && self.live_mode_sent && self.confirmed_mode == Some(0) && self.errors.is_empty()
    }
}

#[derive(Clone, Debug, Default)]
pub struct CheckReport {
    pub identity: Option<Identity>,
    pub initial_mode: Option<u8>,
    pub programmer_confirmed: bool,
    pub led_sent: bool,
    pub presses: u64,
    pub releases: u64,
    pub other_inputs: u64,
    pub route: Option<RouteStats>,
    /// Set once the device state was changed; cleanup always follows.
    pub cleanup: Option<Cleanup>,
    pub stopped_by: &'static str,
    /// Error that ended the check early, if any.
    pub error: Option<String>,
}

/// A Launchpad X conversation: sends through `out`, receives from `rx`,
/// records both directions.
pub struct Session<S: MidiSend> {
    pub out: S,
    rx: Receiver<RawIn>,
    clock: MonotonicClock,
    w: JsonlWriter,
}

impl<S: MidiSend> Session<S> {
    pub fn new(out: S, rx: Receiver<RawIn>, clock: MonotonicClock, w: JsonlWriter) -> Self {
        Session { out, rx, clock, w }
    }

    pub fn into_writer(self) -> JsonlWriter {
        self.w
    }

    pub fn writer(&mut self) -> &mut JsonlWriter {
        &mut self.w
    }

    fn send(&mut self, what: &str, bytes: &[u8]) -> Result<(), String> {
        let t = self.clock.now_us();
        let result = self.out.send_msg(bytes);
        let mut payload = json!({ "bytes": hex(bytes), "what": what });
        if let Err(e) = &result {
            payload["error"] = json!(e);
        }
        // `kind` is `send`, not `midi`, so replay never feeds it to a decoder.
        self.w.write(t, "lpx_out", "send", payload).map_err(|e| e.to_string())?;
        result.map_err(|e| format!("send {what}: {e}"))
    }

    fn record_in(&mut self, ev: &RawIn) -> Result<(), String> {
        self.w.write(ev.t_mono_us, "lpx_in", "midi", midi_payload(&ev.bytes, ev.backend_ts_us)).map_err(|e| e.to_string())
    }

    /// Send `request` and wait for the first message that `parse` accepts.
    /// Other messages that arrive meanwhile are recorded and skipped.
    fn request<T>(&mut self, what: &str, request: &[u8], timeout_us: i64, parse: fn(&[u8]) -> Option<T>) -> Result<Option<T>, String> {
        self.send(what, request)?;
        let deadline = self.clock.now_us() + timeout_us;
        loop {
            let left = deadline - self.clock.now_us();
            if left <= 0 {
                return Ok(None);
            }
            match self.rx.recv_timeout(Duration::from_micros(left as u64)) {
                Ok(ev) => {
                    self.record_in(&ev)?;
                    if let Some(v) = parse(&ev.bytes) {
                        return Ok(Some(v));
                    }
                }
                Err(RecvTimeoutError::Timeout) => return Ok(None),
                Err(RecvTimeoutError::Disconnected) => return Err("input closed".into()),
            }
        }
    }

    fn restore(&mut self, cfg: &CheckConfig) -> Cleanup {
        let mut c = Cleanup::default();
        match led_rgb(cfg.pad, 0, 0, 0).and_then(|m| self.send("led off", &m)) {
            Ok(()) => c.led_off_sent = true,
            Err(e) => c.errors.push(e),
        }
        match self.send("live mode", &mode_select(Mode::Live)) {
            Ok(()) => c.live_mode_sent = true,
            Err(e) => c.errors.push(e),
        }
        match self.request("mode query (after restore)", &mode_query(), cfg.reply_timeout_us, parse_mode) {
            Ok(m) => c.confirmed_mode = m,
            Err(e) => c.errors.push(e),
        }
        c
    }
}

/// Run runbook C steps 2-6. `stop` ends the input phase (Ctrl+C); the
/// cleanup runs the same way for a stop, a deadline or an error after the
/// mode was changed. `forward`, when given, receives normalized Note / CC.
pub fn run_check<S: MidiSend>(
    s: &mut Session<S>,
    cfg: &CheckConfig,
    stop: &AtomicBool,
    input_us: Option<i64>,
    forward: Option<&mut dyn MidiSend>,
) -> CheckReport {
    let mut r = CheckReport { stopped_by: "error", ..Default::default() };

    // Step 2: identity. Nothing else is sent unless a Launchpad X answers.
    let identity = match s.request("device inquiry", &DEVICE_INQUIRY, cfg.reply_timeout_us, parse_identity) {
        Ok(Some(id)) => id,
        Ok(None) => {
            r.error = Some("no Device Inquiry reply; is the Launchpad X connected and is this its MIDI port? Mode and LEDs were not touched".into());
            return r;
        }
        Err(e) => {
            r.error = Some(e);
            return r;
        }
    };
    r.identity = Some(identity.clone());
    match identity.kind {
        IdentityKind::LaunchpadX => {}
        IdentityKind::OtherNovation if cfg.allow_unknown_family => {}
        kind => {
            r.error = Some(format!(
                "device answered as {} (family {}), not a Launchpad X application; mode and LEDs were not touched{}",
                kind.name(),
                hex(&identity.family),
                if kind == IdentityKind::OtherNovation { " (use --allow-unknown-family to continue anyway)" } else { "" }
            ));
            return r;
        }
    }

    // Step 3: current mode.
    match s.request("mode query", &mode_query(), cfg.reply_timeout_us, parse_mode) {
        Ok(Some(m)) => r.initial_mode = Some(m),
        Ok(None) => {
            r.error = Some("no reply to the mode query; mode and LEDs were not touched".into());
            return r;
        }
        Err(e) => {
            r.error = Some(e);
            return r;
        }
    }

    // From here the device state changes, so cleanup always runs.
    let result = programmer_phase(s, cfg, stop, input_us, forward, &mut r);
    if let Err(e) = result {
        r.error = Some(e);
        r.stopped_by = "error";
    }
    r.cleanup = Some(s.restore(cfg));
    r
}

fn programmer_phase<S: MidiSend>(
    s: &mut Session<S>,
    cfg: &CheckConfig,
    stop: &AtomicBool,
    input_us: Option<i64>,
    mut forward: Option<&mut dyn MidiSend>,
    r: &mut CheckReport,
) -> Result<(), String> {
    // Step 4.
    s.send("programmer mode", &mode_select(Mode::Programmer))?;
    match s.request("mode query (after programmer)", &mode_query(), cfg.reply_timeout_us, parse_mode)? {
        Some(1) => r.programmer_confirmed = true,
        Some(m) => return Err(format!("device reports mode {m} ({}) after selecting Programmer Mode", mode_name(m))),
        None => return Err("no mode reply after selecting Programmer Mode".into()),
    }
    let (red, green, blue) = cfg.rgb;
    s.send("led rgb", &led_rgb(cfg.pad, red, green, blue)?)?;
    r.led_sent = true;
    println!("programmer mode confirmed, pad {} lit; press pads, Ctrl+C (or --duration) ends and restores Live Mode", cfg.pad);

    // Step 5.
    let mut stats = RouteStats::default();
    let deadline = input_us.map(|d| s.clock.now_us() + d);
    loop {
        if stop.load(Ordering::SeqCst) {
            r.stopped_by = "ctrl_c";
            break;
        }
        if deadline.is_some_and(|d| s.clock.now_us() >= d) {
            r.stopped_by = "duration";
            break;
        }
        let ev = match s.rx.recv_timeout(Duration::from_millis(20)) {
            Ok(ev) => ev,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return Err("input closed (device unplugged?)".into()),
        };
        match parse_pad(&ev.bytes) {
            Some(PadEvent::Press { note, index, value }) => {
                r.presses += 1;
                println!("  t={:10.6}s press   {} {index} value {value}", us_to_secs(ev.t_mono_us), if note { "pad" } else { "cc " });
            }
            Some(PadEvent::Release { note, index }) => {
                r.releases += 1;
                println!("  t={:10.6}s release {} {index}", us_to_secs(ev.t_mono_us), if note { "pad" } else { "cc " });
            }
            None => r.other_inputs += 1,
        }
        match forward.as_deref_mut() {
            Some(out) => handle_input(&ev, "lpx_in", out, &mut s.w, &mut stats, true).map_err(|e| e.to_string())?,
            None => s.record_in(&ev)?,
        }
    }
    if forward.is_some() {
        r.route = Some(stats);
    }
    Ok(())
}

pub fn report_json(r: &CheckReport) -> serde_json::Value {
    json!({
        "identity": r.identity.as_ref().map(|i| json!({
            "kind": i.kind.name(),
            "device_id": i.device_id,
            "manufacturer": hex(&i.manufacturer),
            "family": hex(&i.family),
            "model": hex(&i.model),
            "version_bytes": hex(&i.version),
            "version": i.version_text(),
        })),
        "initial_mode": r.initial_mode.map(|m| json!({ "code": m, "name": mode_name(m) })),
        "programmer_confirmed": r.programmer_confirmed,
        "led_sent": r.led_sent,
        "presses": r.presses,
        "releases": r.releases,
        "other_inputs": r.other_inputs,
        "route": r.route.as_ref().map(route::stats_json),
        "cleanup": r.cleanup.as_ref().map(|c| json!({
            "ok": c.ok(),
            "led_off_sent": c.led_off_sent,
            "live_mode_sent": c.live_mode_sent,
            "confirmed_mode": c.confirmed_mode,
            "errors": c.errors,
        })),
        "stopped_by": r.stopped_by,
        "error": r.error,
    })
}

#[derive(clap::Args, Debug)]
pub struct LpxArgs {
    /// Launchpad X MIDI input port: exact name or `#index`.
    #[arg(long = "in")]
    pub input: String,
    /// Launchpad X MIDI output port: exact name or `#index`.
    #[arg(long = "out")]
    pub output: String,
    /// Programmer Mode pad index for the LED test (11 = bottom left).
    #[arg(long, default_value_t = 11)]
    pub pad: u8,
    /// Low-brightness colour R,G,B (0..127 each).
    #[arg(long, default_value = "10,0,0")]
    pub rgb: String,
    /// Seconds to receive pad input before restoring. Default: until Ctrl+C.
    #[arg(long)]
    pub duration: Option<u64>,
    /// Also forward normalized Note / CC to this port (exact name or `#index`), e.g. "OMOI-IRO Control".
    #[arg(long)]
    pub forward: Option<String>,
    /// Continue when a Novation device answers with an unexpected family code.
    #[arg(long)]
    pub allow_unknown_family: bool,
    /// Reply timeout for Device Inquiry and mode queries, in ms.
    #[arg(long, default_value_t = 1000)]
    pub reply_timeout_ms: u64,
    /// JSONL output path. Default: artifacts/p0/<run-id>.jsonl
    #[arg(long)]
    pub record: Option<PathBuf>,
    /// Run identifier. Default: lpx-<unix seconds>
    #[arg(long)]
    pub run_id: Option<String>,
}

fn parse_rgb(s: &str) -> Result<(u8, u8, u8), String> {
    let v: Vec<u8> = s
        .split(',')
        .map(|p| p.trim().parse::<u8>().map_err(|e| format!("rgb {s:?}: {e}")))
        .collect::<Result<_, _>>()?;
    match v.as_slice() {
        [r, g, b] if *r <= 127 && *g <= 127 && *b <= 127 => Ok((*r, *g, *b)),
        _ => Err(format!("rgb {s:?} must be three values 0..127")),
    }
}

pub fn run(args: LpxArgs) -> Result<(), Box<dyn std::error::Error>> {
    let rgb = parse_rgb(&args.rgb)?;
    led_rgb(args.pad, rgb.0, rgb.1, rgb.2)?; // validate before touching any port
    let cfg = CheckConfig {
        pad: args.pad,
        rgb,
        reply_timeout_us: args.reply_timeout_ms as i64 * 1000,
        allow_unknown_family: args.allow_unknown_family,
    };

    // Step 1: exact ports. Any error here stops before anything is sent.
    let (input, in_port, in_name) = ports::open_input_exact("omoiiro-probe-lpx-in", &args.input)?;
    let (output, out_port, out_name) = ports::open_output_exact("omoiiro-probe-lpx-out", &args.output)?;
    let forward_open = match &args.forward {
        Some(spec) => {
            let (o, p, n) = ports::open_output_exact("omoiiro-probe-lpx-forward", spec)?;
            if ports::looks_like_launchpad(&n) || n == out_name {
                return Err(format!("refusing to forward into {n:?}").into());
            }
            Some((o, p, n))
        }
        None => None,
    };

    let clock = MonotonicClock::new();
    let (tx, rx) = bounded::<RawIn>(route::QUEUE_CAPACITY);
    let dropped = Arc::new(AtomicU64::new(0));
    let out_conn = output.connect(&out_port, "omoiiro-probe-lpx-out").map_err(|e| format!("open output {out_name:?}: {e}"))?;
    let mut forward_conn = match forward_open {
        Some((o, p, n)) => {
            let c = o.connect(&p, "omoiiro-probe-lpx-forward").map_err(|e| format!("open forward output {n:?}: {e}"))?;
            println!("forward : {n}");
            Some((c, n))
        }
        None => None,
    };
    let in_conn = input
        .connect(&in_port, "omoiiro-probe-lpx-in", omoiiro_midi::backend::make_callback(clock.clone(), tx, dropped.clone()), ())
        .map_err(|e| format!("open input {in_name:?}: {e}"))?;
    println!("input   : {in_name}");
    println!("output  : {out_name}");

    let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let run_id = args.run_id.clone().unwrap_or_else(|| format!("lpx-{unix}"));
    let path = args.record.clone().unwrap_or_else(|| PathBuf::from("artifacts/p0").join(format!("{run_id}.jsonl")));
    let mut w = JsonlWriter::create(&path, &run_id)?;
    println!("record  : {}", path.display());
    w.write(
        clock.now_us(),
        "meta",
        "start",
        json!({
            "command": "lpx-check",
            "unix_s": unix,
            "backend": ports::backend_name(),
            "input": in_name,
            "output": out_name,
            "forward": forward_conn.as_ref().map(|(_, n)| n.clone()),
            "pad": cfg.pad,
            "rgb": [rgb.0, rgb.1, rgb.2],
            "reply_timeout_us": cfg.reply_timeout_us,
        }),
    )?;

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))?;
    }

    let mut session = Session::new(out_conn, rx, clock.clone(), w);
    let forward: Option<&mut dyn MidiSend> = forward_conn.as_mut().map(|(c, _)| c as &mut dyn MidiSend);
    let report = run_check(&mut session, &cfg, &stop, args.duration.map(|d| d as i64 * 1_000_000), forward);

    let mut summary = report_json(&report);
    summary["queue_dropped"] = json!(dropped.load(Ordering::Relaxed));
    let mut w = session.into_writer();
    w.write(clock.now_us(), "meta", "end", summary.clone())?;
    w.flush()?;
    drop(in_conn);
    println!("{}", serde_json::to_string_pretty(&summary)?);
    println!("step 7 (manual): check that Live's Control Surface list has no Launchpad X assigned.");

    if let Some(c) = &report.cleanup {
        if !c.ok() {
            eprintln!("WARNING: restore incomplete: {c:?}. Unplug/replug the Launchpad X or select Live mode manually.");
        }
    }
    match (&report.error, report.cleanup.as_ref().map(|c| c.ok())) {
        (Some(e), _) => Err(e.clone().into()),
        (None, Some(false)) => Err("restore to Live Mode was not confirmed".into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omoiiro_runtime::diagnostics::record::read_jsonl;
    use crossbeam_channel::Sender;

    #[test]
    fn rgb_argument_parsing() {
        assert_eq!(parse_rgb("10,0,0"), Ok((10, 0, 0)));
        assert!(parse_rgb("10,0").is_err());
        assert!(parse_rgb("200,0,0").is_err());
    }

    fn lpx_reply(version: [u8; 4]) -> Vec<u8> {
        let mut v = vec![0xF0, 0x7E, 0x00, 0x06, 0x02, 0x00, 0x20, 0x29, 0x03, 0x01, 0x00, 0x00];
        v.extend_from_slice(&version);
        v.push(0xF7);
        v
    }

    /// Simulated device: answers Device Inquiry and mode queries, tracks the
    /// mode, and logs everything sent to it.
    struct FakeLpx {
        tx: Sender<RawIn>,
        clock: MonotonicClock,
        answer_inquiry: bool,
        answer_mode: bool,
        family: [u8; 2],
        mode: u8,
        sent: Vec<Vec<u8>>,
        /// Inputs to emit right after the LED message (pad presses).
        after_led: Vec<Vec<u8>>,
        stop_after_led: Option<Arc<AtomicBool>>,
        /// Stop answering mode queries once the LED is lit.
        mute_after_led: bool,
    }

    impl FakeLpx {
        fn new(tx: Sender<RawIn>, clock: MonotonicClock) -> Self {
            FakeLpx {
                tx,
                clock,
                answer_inquiry: true,
                answer_mode: true,
                family: [0x03, 0x01],
                mode: 0,
                sent: Vec::new(),
                after_led: Vec::new(),
                stop_after_led: None,
                mute_after_led: false,
            }
        }

        fn emit(&self, bytes: Vec<u8>) {
            self.tx.send(RawIn::new(self.clock.now_us(), bytes)).unwrap();
        }
    }

    impl MidiSend for FakeLpx {
        fn send_msg(&mut self, bytes: &[u8]) -> Result<(), String> {
            self.sent.push(bytes.to_vec());
            if bytes == DEVICE_INQUIRY && self.answer_inquiry {
                let mut r = lpx_reply([0, 4, 0, 7]);
                r[8] = self.family[0];
                r[9] = self.family[1];
                self.emit(r);
            } else if bytes == mode_query().as_slice() && self.answer_mode {
                self.emit(mode_select(if self.mode == 1 { Mode::Programmer } else { Mode::Live }));
            } else if let Some(m) = parse_mode(bytes) {
                self.mode = m;
            } else if bytes.len() == 13 && bytes[6] == 0x03 && bytes[9..12] != [0, 0, 0] {
                if self.mute_after_led {
                    self.answer_mode = false;
                }
                for b in self.after_led.clone() {
                    self.emit(b);
                }
                if let Some(s) = &self.stop_after_led {
                    s.store(true, Ordering::SeqCst);
                }
            }
            Ok(())
        }
    }

    fn session(dev_cfg: impl FnOnce(&mut FakeLpx)) -> (Session<FakeLpx>, std::path::PathBuf) {
        let clock = MonotonicClock::new();
        let (tx, rx) = bounded(64);
        let mut dev = FakeLpx::new(tx, clock.clone());
        dev_cfg(&mut dev);
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("omoiiro-probe-lpx-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lpx.jsonl");
        let w = JsonlWriter::create(&path, "lpx-test").unwrap();
        (Session::new(dev, rx, clock, w), path)
    }

    fn fast() -> CheckConfig {
        CheckConfig { reply_timeout_us: 50_000, ..Default::default() }
    }

    #[test]
    fn no_device_reply_stops_without_mode_change_or_led() {
        let (mut s, _) = session(|d| d.answer_inquiry = false);
        let stop = AtomicBool::new(false);
        let r = run_check(&mut s, &fast(), &stop, Some(0), None);
        assert!(r.error.unwrap().contains("no Device Inquiry reply"));
        assert!(r.cleanup.is_none());
        assert_eq!(s.out.sent, vec![DEVICE_INQUIRY.to_vec()]);
    }

    #[test]
    fn non_launchpad_x_reply_stops_without_mode_change() {
        let (mut s, _) = session(|d| d.family = [0x13, 0x01]);
        let stop = AtomicBool::new(false);
        let r = run_check(&mut s, &fast(), &stop, Some(0), None);
        assert!(r.error.unwrap().contains("other_novation"));
        assert_eq!(s.out.sent, vec![DEVICE_INQUIRY.to_vec()]);
    }

    #[test]
    fn no_mode_reply_stops_after_the_query() {
        let (mut s, _) = session(|d| d.answer_mode = false);
        let stop = AtomicBool::new(false);
        let r = run_check(&mut s, &fast(), &stop, Some(0), None);
        assert!(r.error.unwrap().contains("mode query"));
        assert!(r.cleanup.is_none());
        assert_eq!(s.out.sent, vec![DEVICE_INQUIRY.to_vec(), mode_query()]);
    }

    #[test]
    fn normal_end_runs_the_full_sequence_and_restores_live_mode() {
        let (mut s, path) = session(|d| d.after_led = vec![vec![0x90, 11, 70], vec![0xA0, 11, 20], vec![0x90, 11, 0]]);
        let stop = AtomicBool::new(false);
        let r = run_check(&mut s, &fast(), &stop, Some(100_000), None);
        assert_eq!(r.error, None);
        assert_eq!(r.stopped_by, "duration");
        assert_eq!(r.identity.as_ref().unwrap().version_text(), "0.4.0.7");
        assert_eq!(r.initial_mode, Some(0));
        assert!(r.programmer_confirmed);
        assert!(r.led_sent);
        assert_eq!((r.presses, r.releases, r.other_inputs), (1, 1, 1));
        let c = r.cleanup.clone().unwrap();
        assert!(c.ok(), "{c:?}");
        assert_eq!(
            s.out.sent,
            vec![
                DEVICE_INQUIRY.to_vec(),
                mode_query(),
                mode_select(Mode::Programmer),
                mode_query(),
                led_rgb(11, 10, 0, 0).unwrap(),
                led_rgb(11, 0, 0, 0).unwrap(),
                mode_select(Mode::Live),
                mode_query(),
            ]
        );
        assert_eq!(s.out.mode, 0);
        s.writer().flush().unwrap();
        let recs = read_jsonl(&path).unwrap();
        assert_eq!(recs.iter().filter(|r| r.source == "lpx_out").count(), 8);
        // 4 replies + 3 pad inputs
        assert_eq!(recs.iter().filter(|r| r.source == "lpx_in").count(), 7);
    }

    #[test]
    fn ctrl_c_also_turns_the_led_off_and_restores_live_mode() {
        let flag = Arc::new(AtomicBool::new(false));
        let f2 = flag.clone();
        let (mut s, _) = session(move |d| {
            d.after_led = vec![vec![0x90, 11, 70]];
            d.stop_after_led = Some(f2);
        });
        // No duration: only the Ctrl+C flag can end the input phase.
        let r = run_check(&mut s, &fast(), &flag, None, None);
        assert_eq!(r.stopped_by, "ctrl_c");
        assert!(r.cleanup.unwrap().ok());
        let tail = &s.out.sent[s.out.sent.len() - 3..];
        assert_eq!(tail, &[led_rgb(11, 0, 0, 0).unwrap(), mode_select(Mode::Live), mode_query()]);
        assert_eq!(s.out.mode, 0);
    }

    #[test]
    fn unconfirmed_restore_is_reported() {
        let flag = Arc::new(AtomicBool::new(false));
        let f2 = flag.clone();
        let (mut s, _) = session(move |d| {
            d.stop_after_led = Some(f2);
            d.mute_after_led = true;
        });
        let r = run_check(&mut s, &fast(), &flag, None, None);
        let c = r.cleanup.unwrap();
        assert!(c.led_off_sent && c.live_mode_sent);
        assert_eq!(c.confirmed_mode, None);
        assert!(!c.ok());
    }

    #[test]
    fn forwarding_sends_normalized_note_and_cc_only() {
        #[derive(Default)]
        struct Sink(Vec<Vec<u8>>);
        impl MidiSend for Sink {
            fn send_msg(&mut self, b: &[u8]) -> Result<(), String> {
                self.0.push(b.to_vec());
                Ok(())
            }
        }
        let (mut s, _) = session(|d| {
            d.after_led = vec![vec![0x90, 11, 70], vec![0xA0, 11, 20], vec![0x90, 11, 0], vec![0xB0, 91, 127]]
        });
        let mut sink = Sink::default();
        let stop = AtomicBool::new(false);
        let r = run_check(&mut s, &fast(), &stop, Some(100_000), Some(&mut sink));
        assert_eq!(r.error, None);
        assert_eq!(sink.0, vec![vec![0x90, 11, 70], vec![0x80, 11, 0], vec![0xB0, 91, 127]]);
        let st = r.route.unwrap();
        assert_eq!((st.forwarded, st.normalized, st.ignored), (3, 1, 1));
    }
}
