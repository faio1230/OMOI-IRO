//! A running follow-mode session (M5, U01): the start / stop boundary used by
//! the CLI (`omoiiro-probe follow`) and the desktop shell.
//!
//! `FollowSession::start` opens the bridge UDP receiver, the beat input (MIDI
//! Clock or Link, one source, no fallback), the optional operation route and
//! the LED sink, then runs:
//!
//! * the sync thread (this module): feeds the `Transport` from the bridge and
//!   the beat input, writes the optional JSONL record, queues readable event
//!   lines for the UI;
//! * the render, LED and control threads (`follow`).
//!
//! Callers talk to it only through methods: status, overrides, show
//! replacement (validated on the caller's thread), fault reset and `stop`.
//! Nothing is sent to Live or Link except the operation route's Note / CC.

use crate::connections::OutputBinding;
use crate::control::ControlRoute;
use crate::diagnostics::record::{midi_payload, JsonlWriter};
use crate::diagnostics::report::us_to_secs;
use crate::follow::{spawn_control, spawn_led, spawn_render, summary_json, FollowShared, FollowStatus, LedLoop, LedSink, SharedBackend};
use crate::link::LinkRaw;
use crate::output::mtc::{spawn_sender, MtcSource, Sent};
use crate::output::reconnect::ReconnectingOutput;
use crate::sync::bridge::BridgeSnapshot;
use crate::sync::mtc_out::{GenStats, MtcGen};
use crate::scheduler::RenderRate;
use crate::sync::bridge::{event_line, BridgeSample};
use crate::sync::bridge_ingress::{datagram_payload, spawn_receiver, RawDatagram};
use crate::sync::clock::ClockEvent;
use crate::sync::transport::{snapshot_json, BeatSourceKind, Transport, TransportConfig, TransportSnapshot};
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
use omoiiro_core::model::Show;
use omoiiro_core::validation::Issue;
use omoiiro_devices::launchpad_x::LaunchpadXAdapter;
use omoiiro_devices::simulated::VirtualLpx;
use omoiiro_midi::backend::{Ingress, IngressStats, RawIn};
use omoiiro_midi::clock::MonotonicClock;
use omoiiro_midi::ports;
use omoiiro_midi::routing::RouteRules;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// Where LED frames go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedOut {
    /// In-process virtual Launchpad X.
    Virtual,
    /// A MIDI output (exact name or `#index`). A Launchpad-looking port is
    /// refused unless `allow_device` (device work is M6).
    Port { spec: String, allow_device: bool },
}

/// A21: where the show time comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeSource {
    /// Follow Live (bridge + the show's beat source).
    #[default]
    Live,
    /// A local play / pause / seek clock (no Live, no bridge, no beat input).
    Local,
}

/// A21: the local clock as shown in the status.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct LocalTransport {
    pub playing: bool,
    pub position_us: i64,
    pub length_us: Option<i64>,
    pub bpm: f64,
}

#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub show: Show,
    /// A21: `Local` opens no bridge, clock input, Link or MTC output.
    pub time_source: TimeSource,
    pub bind: String,
    /// MIDI Clock input (used when the show's beat source is midi_clock).
    pub clock_in: Option<String>,
    /// Observe Link (used when the show's beat source is ableton_link; feature `link`).
    pub link: bool,
    pub led_out: LedOut,
    /// Operation route: (input spec, output spec).
    pub control: Option<(String, String)>,
    /// MTC output port (exact name or `#index`); used only when the show
    /// enables `sync.mtc_output`.
    pub timecode_out: Option<String>,
    /// JSONL record: (path, run id).
    pub record: Option<(PathBuf, String)>,
    /// Extra fields for the record's `meta/start`.
    pub meta: Value,
    /// Terminal safety limits (brightness ceiling, flashing limit).
    pub safety: omoiiro_core::safety::SafetyLimits,
    /// Identify the LED device (Device Inquiry, Programmer Mode) before
    /// writing frames. Replies are read from the control input, so the
    /// control input must be the same device.
    pub identify_led: bool,
}

/// Status for display (cheap to take, any thread).
#[derive(Clone, Debug)]
pub struct SessionStatus {
    pub uptime_us: i64,
    pub follow: FollowStatus,
    pub transport: TransportSnapshot,
    pub blackout: bool,
    pub brightness: f32,
    pub control_fault: Option<String>,
    pub dropped_bridge: u64,
    pub dropped_clock: u64,
    pub dropped_control: u64,
    pub time_source: TimeSource,
    /// A21: the local clock (`time_source: local` only).
    pub local: Option<LocalTransport>,
}

pub struct FollowSession {
    time_source: TimeSource,
    shared: Arc<FollowShared>,
    clock: MonotonicClock,
    started_us: i64,
    sync: Option<JoinHandle<(Result<(), String>, Option<JsonlWriter>)>>,
    render: Option<JoinHandle<crate::follow::RenderLoop>>,
    led: Option<JoinHandle<LedLoop<LaunchpadXAdapter>>>,
    control: Option<JoinHandle<ControlRoute>>,
    mtc: Option<(JoinHandle<GenStats>, Arc<AtomicBool>, Receiver<Sent>, Arc<AtomicU64>)>,
    sync_stop: Arc<AtomicBool>,
    events: Receiver<String>,
    bridge_dropped: Arc<AtomicU64>,
    clock_stats: Arc<IngressStats>,
    control_stats: Arc<IngressStats>,
    pub revision: u64,
    pub notes: Vec<String>,
}

impl FollowSession {
    pub fn start(cfg: SessionConfig, backend: SharedBackend) -> Result<FollowSession, String> {
        if cfg.link && !cfg!(feature = "link") {
            return Err("built without the `link` feature".into());
        }
        let tcfg = TransportConfig::from_show(&cfg.show)?;
        let rate = RenderRate { num: cfg.show.render.fps as u64, den: 1 };
        let mut notes = Vec::new();
        let local = cfg.time_source == TimeSource::Local;
        if local {
            notes.push(format!("時刻源：手元の時計（Live なし、{} bpm）", cfg.show.preview.bpm));
        }
        match tcfg.beat_source {
            _ if local => {}
            Some(BeatSourceKind::MidiClock) if cfg.clock_in.is_none() => notes.push("beat source midi_clock without a clock input; Pulse stays transparent".to_string()),
            Some(BeatSourceKind::Link) if !cfg.link => notes.push("beat source ableton_link without Link; Pulse stays transparent".to_string()),
            _ => {}
        }
        let beat_source = tcfg.beat_source;
        let clock = MonotonicClock::new();

        let sink = match &cfg.led_out {
            LedOut::Virtual => LedSink::Virtual(VirtualLpx::new()),
            LedOut::Port { spec, allow_device } => {
                let outs = backend.lock().unwrap().list_outputs().map_err(|e| e.to_string())?;
                if let Ok(i) = ports::resolve_exact(&outs, spec) {
                    if ports::looks_like_launchpad(&outs[i]) && !allow_device {
                        return Err(format!("{:?} looks like a Launchpad; device output is M6", outs[i]));
                    }
                }
                LedSink::Port(OutputBinding::new("led_out", spec))
            }
        };

        let transport = if local {
            Transport::new_local(tcfg.clone(), crate::preview::show_length_us(&cfg.show), cfg.show.preview.bpm)
        } else {
            Transport::new(tcfg.clone())
        };
        let shared = FollowShared::new(transport);
        shared.overrides.set_limits(&cfg.safety);
        let revision = shared.exchange.submit(&cfg.show).map_err(|issues| format!("show rejected: {issues:?}"))?;

        let (bridge_tx, bridge_rx) = bounded::<RawDatagram>(8192);
        let bridge_dropped = Arc::new(AtomicU64::new(0));
        let sync_stop = Arc::new(AtomicBool::new(false));
        // Local: no UDP receiver; the sender is kept so the sync loop keeps running.
        let (bridge_thread, bridge_keep) = if local {
            (None, Some(bridge_tx))
        } else {
            (Some(spawn_receiver(&cfg.bind, clock.clone(), bridge_tx, bridge_dropped.clone(), sync_stop.clone()).map_err(|e| format!("bind UDP {}: {e}", cfg.bind))?), None)
        };

        let (clock_tx, clock_rx) = bounded::<RawIn>(8192);
        let clock_stats = Arc::new(IngressStats::default());
        let clock_conn = match (&cfg.clock_in, beat_source) {
            (Some(spec), Some(BeatSourceKind::MidiClock)) if !local => Some(
                backend
                    .lock()
                    .unwrap()
                    .open_input(spec, false, Ingress::new(clock.clone(), clock_tx.clone(), clock_stats.clone()))
                    .map_err(|e| format!("open clock input: {e}"))?,
            ),
            _ => None,
        };
        drop(clock_tx);
        let use_link = cfg.link && beat_source == Some(BeatSourceKind::Link) && !local;

        let (ctl_tx, ctl_rx) = bounded::<RawIn>(4096);
        let control_stats = Arc::new(IngressStats::default());
        let (sysex_tx, sysex_rx) = bounded::<Vec<u8>>(64);
        let identify = cfg.identify_led && matches!(sink, LedSink::Port(_));
        if identify && cfg.control.is_none() {
            notes.push("LED device identification needs the same device as the operation input; it will fault with \"no reply\"".into());
        }
        let control = cfg.control.as_ref().map(|(input, output)| {
            let mut route = ControlRoute::new(input, output, RouteRules::default(), Ingress::new(clock.clone(), ctl_tx.clone(), control_stats.clone()));
            if identify {
                route.sysex_to = Some(sysex_tx.clone());
            }
            spawn_control(shared.clone(), clock.clone(), route, ctl_rx, backend.clone())
        });
        drop(ctl_tx);

        // MTC output for external gear (show `sync.mtc_output.enabled` + a timecode port).
        let mtc_enabled = cfg.show.sync.mtc_output.as_ref().is_some_and(|m| m.enabled);
        let mtc = match (&cfg.timecode_out, mtc_enabled) {
            (Some(_), true) if local => {
                notes.push("手元の時計では MTC を送らない".into());
                None
            }
            (Some(spec), true) => {
                let gen = MtcGen::new(cfg.show.sync.timecode.frame_rate).map_err(|e| format!("MTC output: {e}"))?;
                // Reconnects by itself; a missing port is retried like the other bindings.
                let out = ReconnectingOutput::new(OutputBinding::new("timecode_out", spec), backend.clone(), clock.clone(), shared.clone());
                notes.push(match out.port_name() {
                    Some(n) => format!("MTC output: {n} ({} fps)", cfg.show.sync.timecode.frame_rate.name()),
                    None => format!("MTC output: {spec:?} is not present yet; it connects when it appears"),
                });
                let stop = Arc::new(AtomicBool::new(false));
                let (tx, rx) = bounded::<Sent>(4096);
                let dropped = Arc::new(AtomicU64::new(0));
                let h = spawn_sender(out, gen, shared.clone(), clock.clone(), tx, dropped.clone(), stop.clone());
                Some((h, stop, rx, dropped))
            }
            (Some(_), false) => {
                notes.push("timecode output is set but the show does not enable mtc_output; no MTC is sent".into());
                None
            }
            _ => None,
        };

        let led_backend = matches!(sink, LedSink::Port(_)).then(|| backend.clone());
        let render = spawn_render(shared.clone(), clock.clone(), rate);
        let mut led_loop = LedLoop::new(LaunchpadXAdapter, sink);
        if identify {
            led_loop = led_loop.with_identify(crate::output::identify::IdentifyConfig::default(), sysex_rx);
        }
        let led = spawn_led(shared.clone(), clock.clone(), led_loop, led_backend);

        let mut writer = match &cfg.record {
            Some((p, run_id)) => Some(JsonlWriter::create(p, run_id).map_err(|e| format!("record {}: {e}", p.display()))?),
            None => None,
        };
        if let Some(w) = writer.as_mut() {
            let mut meta = cfg.meta.clone();
            if !meta.is_object() {
                meta = json!({});
            }
            meta["revision"] = json!(revision);
            meta["fps"] = json!(cfg.show.render.fps);
            meta["bridge_udp"] = json!(cfg.bind);
            meta["time_source"] = json!(cfg.time_source);
            meta["beat_source"] = json!(format!("{:?}", tcfg.beat_source));
            meta["origin"] = json!(tcfg.origin.to_string());
            meta["input_offset_us"] = json!(tcfg.input_offset_us);
            w.write(clock.now_us(), "meta", "start", meta).map_err(|e| e.to_string())?;
            // Pad events go to the record too (make-recording reads them).
            shared.pad_log.lock().unwrap().to_file = Some(Vec::new());
        }

        let (ev_tx, ev_rx) = bounded::<String>(256);
        let sync = {
            let shared = shared.clone();
            let clock = clock.clone();
            let stop = sync_stop.clone();
            std::thread::Builder::new()
                .name("omoiiro-sync".into())
                .spawn(move || {
                    let _clock_conn = clock_conn;
                    let _bridge_keep = bridge_keep;
                    let r = sync_loop(&shared, &clock, &stop, bridge_rx, clock_rx, use_link, writer.as_mut(), &ev_tx);
                    if let Some(h) = bridge_thread {
                        let _ = h.join();
                    }
                    (r, writer)
                })
                .map_err(|e| e.to_string())?
        };

        let started_us = clock.now_us();
        Ok(FollowSession {
            time_source: cfg.time_source,
            shared,
            clock,
            started_us,
            sync: Some(sync),
            render: Some(render),
            led: Some(led),
            control,
            mtc,
            sync_stop,
            events: ev_rx,
            bridge_dropped,
            clock_stats,
            control_stats,
            revision,
            notes,
        })
    }

    pub fn shared(&self) -> &Arc<FollowShared> {
        &self.shared
    }

    pub fn now_us(&self) -> i64 {
        self.clock.now_us()
    }

    /// Readable event lines (bridge lock / jump / stop ...), oldest first.
    pub fn drain_events(&self) -> Vec<String> {
        self.events.try_iter().collect()
    }

    pub fn status(&self) -> SessionStatus {
        let now = self.clock.now_us();
        let transport = self.shared.transport.lock().unwrap().snapshot(now);
        let follow = self.shared.status.lock().unwrap().clone();
        SessionStatus {
            uptime_us: now - self.started_us,
            control_fault: follow.control_fault.clone(),
            follow,
            transport,
            blackout: self.shared.overrides.blackout(),
            brightness: self.shared.overrides.brightness(),
            dropped_bridge: self.bridge_dropped.load(Ordering::Relaxed),
            dropped_clock: self.clock_stats.dropped(),
            dropped_control: self.control_stats.dropped(),
            time_source: self.time_source,
            local: self.local_transport(),
        }
    }

    pub fn set_blackout(&self, on: bool) {
        crate::control::apply_ui(crate::control::Action::SetBlackout(on), &self.shared.overrides);
    }

    /// A pad press from the UI (preview grid click), as if it came from the
    /// operation input: only input-driven effects (Ripple) see it; nothing
    /// is sent to Live.
    pub fn press_pad(&self, x: u8, y: u8, pressed: bool) {
        let (x, y, t_us) = (x.min(7), y.min(7), self.clock.now_us());
        let ev = if pressed { omoiiro_core::render::PadEvent::Press { x, y, velocity: 127, t_us } } else { omoiiro_core::render::PadEvent::Release { x, y, t_us } };
        let _ = self.shared.pad_tx.try_send(ev);
    }

    /// Forget every press (the input history and the instance window).
    pub fn reset_input(&self) {
        let _ = self.shared.pad_tx.try_send(omoiiro_core::render::PadEvent::Reset);
    }

    /// Start recording pad events from the current show time (refused while
    /// the show time is unknown). Returns that show time.
    pub fn start_recording(&self) -> Result<i64, String> {
        let t = self.shared.status.lock().unwrap().show_time_us.ok_or("曲の時刻が分からない（Live の再生とブリッジを確認）")?;
        self.shared.pad_log.lock().unwrap().recording = Some((t, Vec::new()));
        Ok(t)
    }

    /// Stop recording: start, stop show time, and events (None if not recording).
    pub fn stop_recording(&self) -> Option<(i64, i64, Vec<(i64, omoiiro_core::render::PadEvent)>)> {
        let stop = self.shared.status.lock().unwrap().show_time_us;
        let (start, events) = self.shared.pad_log.lock().unwrap().recording.take()?;
        // If sync was lost or the transport rewound, retain at least every
        // recorded event. Never make the capture shorter than its contents.
        let last = events.iter().map(|e| e.0).max().unwrap_or(start);
        Some((start, stop.unwrap_or(last).max(last).max(start), events))
    }

    /// Latest bridge meters and their age (µs), if any arrived (0.1.3).
    pub fn audio(&self) -> Option<(i64, omoiiro_core::render::AudioInput)> {
        let now = self.clock.now_us();
        self.shared.audio.lock().unwrap().as_ref().map(|(at, a)| (now - at, a.clone()))
    }

    /// Show time the running recording began at.
    pub fn recording_since(&self) -> Option<i64> {
        self.shared.pad_log.lock().unwrap().recording.as_ref().map(|r| r.0)
    }

    pub fn set_brightness(&self, b: f32) {
        self.shared.overrides.set_brightness(b);
    }

    /// Validate on this thread; adopted by the render thread at the next frame.
    pub fn submit_show(&mut self, show: &Show) -> Result<u64, Vec<Issue>> {
        let r = self.shared.exchange.submit(show)?;
        self.revision = r;
        // A21: the local clock wraps at the edited length.
        if let Some(l) = self.shared.transport.lock().unwrap().local_mut() {
            l.clock.set_length(crate::preview::show_length_us(show));
        }
        Ok(r)
    }

    pub fn time_source(&self) -> TimeSource {
        self.time_source
    }

    pub fn local_transport(&self) -> Option<LocalTransport> {
        let now = self.clock.now_us();
        self.shared.transport.lock().unwrap().local().map(|l| LocalTransport { playing: l.clock.playing(), position_us: l.clock.position(now), length_us: l.clock.length_us(), bpm: l.bpm })
    }

    /// A21: change the local clock; refused while following Live.
    fn with_local(&self, f: impl FnOnce(&mut crate::sync::transport::LocalTime, i64) -> Result<(), String>) -> Result<LocalTransport, String> {
        {
            let now = self.clock.now_us();
            let mut t = self.shared.transport.lock().unwrap();
            let l = t.local_mut().ok_or("Live に追従中：再生位置とテンポは Live が決める（手元の時計で開始すると操作できる）")?;
            f(l, now)?;
        }
        Ok(self.local_transport().expect("local"))
    }

    pub fn local_play(&self, playing: bool) -> Result<LocalTransport, String> {
        self.with_local(|l, now| {
            if playing {
                l.clock.play(now)
            } else {
                l.clock.pause(now)
            }
            Ok(())
        })
    }

    pub fn local_seek(&self, position_us: i64) -> Result<LocalTransport, String> {
        self.with_local(|l, now| {
            l.clock.seek(now, position_us);
            Ok(())
        })
    }

    pub fn local_set_bpm(&self, bpm: f64) -> Result<LocalTransport, String> {
        self.with_local(|l, _| {
            if !bpm.is_finite() || !(20.0..=999.0).contains(&bpm) {
                return Err(format!("tempo must be 20..999 bpm, got {bpm}"));
            }
            l.bpm = bpm;
            Ok(())
        })
    }

    /// Newest frame the render thread evaluated since the last call (what the
    /// LED writer is given; for the UI preview only).
    pub fn take_preview_frame(&self) -> Option<crate::follow::FrameMsg> {
        self.shared.preview.try_take()
    }

    /// Close and reopen every port binding (LED, operation route, timecode
    /// output) as new connection generations. Sounding notes are released first.
    pub fn reconnect_all(&self) {
        self.shared.reconnect_request.fetch_add(1, Ordering::SeqCst);
    }

    pub fn reset_control_fault(&self) {
        self.shared.control_reset.store(true, Ordering::SeqCst);
    }

    /// Stop every thread (LEDs off, Note Offs for sounding notes) and return
    /// the run summary (T07).
    pub fn stop(mut self) -> Result<Value, String> {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.sync_stop.store(true, Ordering::SeqCst);
        let (sync, writer) = self.sync.take().unwrap().join().map_err(|_| "sync thread panicked")?;
        let render = self.render.take().unwrap().join().map_err(|_| "render thread panicked")?;
        let led = self.led.take().unwrap().join().map_err(|_| "LED thread panicked")?;
        let control = match self.control.take() {
            Some(h) => Some(h.join().map_err(|_| "control thread panicked")?),
            None => None,
        };
        let mtc = match self.mtc.take() {
            Some((h, stop, rx, dropped)) => {
                stop.store(true, Ordering::SeqCst);
                let stats = h.join().map_err(|_| "MTC thread panicked")?;
                let sent = rx.try_iter().count() as u64;
                Some(json!({
                    "quarter_frames": stats.quarter_frames, "full_frames": stats.full_frames, "starts": stats.starts,
                    "jumps": stats.jumps, "resyncs": stats.resyncs, "stops": stats.stops,
                    "records_dropped": dropped.load(Ordering::Relaxed), "records_left": sent,
                }))
            }
            None => None,
        };
        let mut end = summary_json(&render, &led, control.as_ref(), self.shared.mailbox.overwritten());
        end["dropped"] = json!({
            "bridge": self.bridge_dropped.load(Ordering::Relaxed),
            "clock": self.clock_stats.dropped(),
            "control": self.control_stats.dropped(),
            "control_oversize": self.control_stats.oversize(),
        });
        end["mtc_out"] = json!(mtc);
        end["transport"] = snapshot_json(&self.shared.transport.lock().unwrap().snapshot(self.clock.now_us()));
        if let Err(e) = sync {
            end["sync_error"] = json!(e);
        }
        if let Some(mut w) = writer {
            w.write(self.clock.now_us(), "meta", "end", end.clone()).map_err(|e| e.to_string())?;
            w.flush().map_err(|e| e.to_string())?;
        }
        Ok(end)
    }
}

/// The MTC sender reads the session's transport.
impl MtcSource for FollowShared {
    fn view(&self, now_us: i64) -> (BridgeSnapshot, u64, f64) {
        let (s, epoch, origin) = self.transport.view(now_us);
        // A reconnect of the timecode output counts as a discontinuity, so
        // the generator sends a Full Frame on the new connection.
        (s, epoch + self.mtc_epoch_bump.load(Ordering::SeqCst), origin)
    }
}

impl Drop for FollowSession {
    fn drop(&mut self) {
        // A session dropped without `stop` still stops its threads.
        self.shared.stop.store(true, Ordering::SeqCst);
        self.sync_stop.store(true, Ordering::SeqCst);
        if let Some((_, stop, _, _)) = &self.mtc {
            stop.store(true, Ordering::SeqCst);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_loop(
    shared: &FollowShared,
    clock: &MonotonicClock,
    stop: &AtomicBool,
    bridge_rx: Receiver<RawDatagram>,
    clock_rx: Receiver<RawIn>,
    use_link: bool,
    mut w: Option<&mut JsonlWriter>,
    events: &Sender<String>,
) -> Result<(), String> {
    let (link_tx, link_rx) = bounded::<(i64, LinkRaw)>(1024);
    #[cfg(feature = "link")]
    let mut link_obs = use_link.then(|| crate::link::observer::LinkObserver::start(clock, link_tx.clone(), Arc::new(AtomicU64::new(0)), false));
    #[cfg(not(feature = "link"))]
    let _ = use_link;
    drop(link_tx);
    #[cfg(feature = "link")]
    let mut next_link_sample = clock.now_us();

    let mut bev = Vec::new();
    let mut cev = Vec::new();
    let mut next_state = clock.now_us() + 1_000_000;
    let io = |e: std::io::Error| e.to_string();
    while !stop.load(Ordering::SeqCst) {
        match bridge_rx.recv_timeout(Duration::from_millis(5)) {
            Ok(d) => {
                let payload = datagram_payload(&d.bytes);
                bev.clear();
                // Meter records (v3) are their own stream: not timing samples.
                if let Some(a) = omoiiro_core::render::AudioInput::from_meter_record(&payload) {
                    *shared.audio.lock().unwrap() = Some((d.t_mono_us, a));
                } else {
                    shared.transport.lock().unwrap().feed_bridge(BridgeSample::from_payload(d.t_mono_us, &payload), &mut bev);
                }
                for e in &bev {
                    let _ = events.try_send(format!("t={:.3}s bridge {}", us_to_secs(d.t_mono_us), event_line(e)));
                }
                if let Some(w) = w.as_deref_mut() {
                    w.write(d.t_mono_us, "bridge", "sample", payload).map_err(io)?;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        for ev in clock_rx.try_iter() {
            cev.clear();
            shared.transport.lock().unwrap().feed_clock(ev.t_mono_us, &ev.bytes, &mut cev);
            // Only the seed goes to the event log (ticks and transport would flood it).
            for e in cev.iter().filter(|e| matches!(e, ClockEvent::Seeded { .. })) {
                let _ = events.try_send(format!("t={:.3}s clock {}", us_to_secs(ev.t_mono_us), crate::diagnostics::report::clock_event_line(e)));
            }
            if let Some(w) = w.as_deref_mut() {
                w.write(ev.t_mono_us, "clock", "midi", midi_payload(&ev.bytes, ev.backend_ts_us)).map_err(io)?;
            }
        }
        for (t, raw) in link_rx.try_iter() {
            for line in shared.transport.lock().unwrap().feed_link_raw(t, raw) {
                let _ = events.try_send(format!("t={:.3}s link {line}", us_to_secs(t)));
            }
        }
        #[cfg(feature = "link")]
        if let Some(o) = link_obs.as_mut() {
            let t = clock.now_us();
            if t >= next_link_sample {
                shared.transport.lock().unwrap().feed_link_sample(t, o.sample());
                next_link_sample = t + 100_000;
            }
        }
        let now = clock.now_us();
        cev.clear();
        shared.transport.lock().unwrap().poll_clock(now, &mut cev);
        if now >= next_state {
            if let Some(w) = w.as_deref_mut() {
                let pads = shared.pad_log.lock().unwrap().to_file.as_mut().map(std::mem::take).unwrap_or_default();
                for (t, show_us, ev) in pads {
                    use omoiiro_core::render::PadEvent;
                    let (kind, payload) = match ev {
                        PadEvent::Press { x, y, velocity, .. } => ("press", json!({"show_us": show_us, "x": x, "y": y, "velocity": velocity})),
                        PadEvent::Release { x, y, .. } => ("release", json!({"show_us": show_us, "x": x, "y": y})),
                        PadEvent::ReleaseAll { .. } => ("release_all", json!({"show_us": show_us})),
                        PadEvent::Reset => ("reset", json!({"show_us": show_us})),
                    };
                    w.write(t, "pad", kind, payload).map_err(io)?;
                }
                let snap = shared.transport.lock().unwrap().snapshot(now);
                let st = shared.status.lock().unwrap().clone();
                w.write(now, "follow", "state", json!({
                    "transport": snapshot_json(&snap), "frame": st.frame_index, "revision": st.revision,
                    "dark": st.dark, "gain": st.gain, "led_state": st.led_state, "led_lit": st.led_lit,
                    "blackout": shared.overrides.blackout(), "brightness": shared.overrides.brightness(),
                    "control_fault": st.control_fault,
                }))
                .map_err(io)?;
                w.flush().map_err(io)?;
            }
            next_state = now + 1_000_000;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use omoiiro_midi::fake::fake_backend;
    use std::sync::Mutex;

    #[test]
    fn start_status_stop_with_a_fake_backend() {
        let show = omoiiro_store::load_str(include_str!("../../../examples/demo.show.json")).unwrap();
        let (be, ports) = fake_backend();
        ports.add_input("Pads");
        ports.add_output(ports::names::CONTROL_PORT);
        ports.add_output(ports::names::TIMECODE_PORT);
        let backend: SharedBackend = Arc::new(Mutex::new(Box::new(be)));
        let dir = std::env::temp_dir().join(format!("omoiiro-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rec = dir.join("session.jsonl");
        let cfg = SessionConfig {
            time_source: TimeSource::Live,
            show: show.clone(),
            bind: "127.0.0.1:0".into(),
            clock_in: None,
            link: false,
            led_out: LedOut::Virtual,
            control: Some(("Pads".into(), ports::names::CONTROL_PORT.into())),
            timecode_out: Some(ports::names::TIMECODE_PORT.into()),
            record: Some((rec.clone(), "session-test".into())),
            meta: json!({"command": "test"}),
            safety: Default::default(),
            identify_led: false,
        };
        let mut s = FollowSession::start(cfg, backend).unwrap();
        assert_eq!(s.revision, 1);
        assert!(s.local_play(true).is_err() && s.local_seek(0).is_err() && s.status().local.is_none(), "Live decides the position");
        std::thread::sleep(Duration::from_millis(400));
        ports.inject("Pads", &[0x90, 11, 100]);
        s.set_blackout(true);
        assert_eq!(s.submit_show(&show), Ok(2));
        std::thread::sleep(Duration::from_millis(400));
        let st = s.status();
        assert!(st.follow.frame_index >= 10, "frames {}", st.follow.frame_index);
        assert!(st.blackout);
        assert_eq!(st.follow.revision, 2);
        assert_eq!(st.follow.control_out, Some("ready"));
        assert_eq!(ports.sent(ports::names::CONTROL_PORT), vec![vec![0x90, 11, 100]]);
        let end = s.stop().unwrap();
        assert!(end["render"]["frames"].as_u64().unwrap() >= 10);
        // demo enables mtc_output; with no bridge samples the generator stays off.
        assert!(end["mtc_out"].is_object());
        assert!(ports.sent(ports::names::TIMECODE_PORT).is_empty());
        // Shutdown released the sounding note.
        assert_eq!(ports.sent(ports::names::CONTROL_PORT).last().unwrap(), &vec![0x80, 11, 0]);
        let recs = crate::diagnostics::record::read_jsonl(&rec).unwrap();
        assert_eq!(recs.first().unwrap().kind, "start");
        assert_eq!(recs.last().unwrap().kind, "end");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A21: the local clock drives the same render and LED path without Live.
    #[test]
    fn local_time_source_plays_seeks_and_stops_dark() {
        use omoiiro_devices::adapter::DeviceAdapter;
        let show = omoiiro_store::load_str(include_str!("../../../examples/demo.show.json")).unwrap();
        let (be, ports) = fake_backend();
        ports.add_output("LED Out");
        ports.add_output(ports::names::TIMECODE_PORT);
        let backend: SharedBackend = Arc::new(Mutex::new(Box::new(be)));
        let cfg = SessionConfig {
            time_source: TimeSource::Local,
            show: show.clone(),
            bind: "127.0.0.1:0".into(),
            clock_in: Some("not used".into()),
            link: false,
            led_out: LedOut::Port { spec: "LED Out".into(), allow_device: false },
            control: None,
            timecode_out: Some(ports::names::TIMECODE_PORT.into()),
            record: None,
            meta: json!({}),
            safety: Default::default(),
            identify_led: false,
        };
        let s = FollowSession::start(cfg, backend).unwrap();
        assert!(s.notes.iter().any(|n| n.contains("手元の時計")), "{:?}", s.notes);
        std::thread::sleep(Duration::from_millis(300));
        let st = s.status();
        assert_eq!(st.time_source, TimeSource::Local);
        let l = st.local.unwrap();
        assert!(!l.playing && l.position_us == 0 && l.bpm == show.preview.bpm);
        assert_eq!((st.transport.show_time_us, st.transport.output), (Some(0), crate::sync::transport::OutputState::Hold), "stopped: shown, not black");
        assert!(st.transport.beat.is_some());

        // Play: the show time advances and frames go to the LED port.
        let p = s.local_play(true).unwrap();
        assert!(p.playing);
        std::thread::sleep(Duration::from_millis(400));
        let st = s.status();
        assert!(st.transport.show_time_us.unwrap() >= 300_000, "{:?}", st.transport.show_time_us);
        assert_eq!(st.transport.output, crate::sync::transport::OutputState::Follow);
        assert!(st.follow.frame_index >= 8);
        assert!(!ports.sent("LED Out").is_empty(), "LED frames were sent");
        assert_eq!(st.follow.show_time_us.map(|t| t > 0), Some(true), "the render loop follows the local clock");

        // Seek while paused: the time is exactly there.
        s.local_play(false).unwrap();
        let l = s.local_seek(10_000_000).unwrap();
        assert_eq!(l.position_us, 10_000_000);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(s.status().transport.show_time_us, Some(10_000_000));
        assert_eq!(s.local_set_bpm(90.0).unwrap().bpm, 90.0);
        assert!(s.local_set_bpm(0.0).is_err());

        // No MTC from the local clock; stop turns the LEDs off.
        let end = s.stop().unwrap();
        assert!(end["mtc_out"].is_null());
        assert!(ports.sent(ports::names::TIMECODE_PORT).is_empty());
        let sent = ports.sent("LED Out");
        let off = LaunchpadXAdapter.shutdown();
        assert!(!off.is_empty() && sent.ends_with(&off), "stop sends the LED shutdown last");
    }

    /// A21 (Mac report): a new show (no clips, no length) started locally,
    /// then edited, played and edited again: the time advances and an edit
    /// keeps the position.
    #[test]
    fn local_clock_of_a_new_show_advances_and_survives_edits() {
        use omoiiro_core::edit::{apply, EditCommand, Snap};
        let mut show = omoiiro_core::edit::new_show("x");
        let (be, _ports) = fake_backend();
        let backend: SharedBackend = Arc::new(Mutex::new(Box::new(be)));
        let cfg = SessionConfig {
            time_source: TimeSource::Local,
            show: show.clone(),
            bind: "127.0.0.1:0".into(),
            clock_in: None,
            link: false,
            led_out: LedOut::Virtual,
            control: None,
            timecode_out: None,
            record: None,
            meta: json!({}),
            safety: Default::default(),
            identify_led: false,
        };
        let mut s = FollowSession::start(cfg, backend).unwrap();
        assert_eq!(s.local_transport().unwrap().length_us, None);
        // Edit: a preset, a layer and a 44 s clip.
        let (x, o) = apply(&show, &EditCommand::AddPreset { name: "白".into(), effect_type: "builtin.solid".into() }, Snap::None).unwrap();
        let pid = o.preset_id.unwrap();
        let (x, o) = apply(&x, &EditCommand::AddLayer { name: "a".into(), index: None }, Snap::None).unwrap();
        let (x, _) = apply(&x, &EditCommand::AddClip { layer_id: o.layer_id.unwrap(), preset_id: pid, start_us: 0, duration_us: 44_160_000 }, Snap::None).unwrap();
        show = x;
        s.submit_show(&show).unwrap();
        assert_eq!(s.local_transport().unwrap().length_us, Some(44_160_000));
        s.local_play(true).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let t1 = s.status().transport.show_time_us.unwrap();
        assert!(t1 >= 400_000, "advancing: {t1}");
        let f = s.status().follow.show_time_us.unwrap();
        assert!(f >= 300_000, "the render loop follows: {f}");
        // Another edit keeps playing from where it was.
        let (x, _) = apply(&show, &EditCommand::AddLayer { name: "b".into(), index: None }, Snap::None).unwrap();
        s.submit_show(&x).unwrap();
        let l = s.local_transport().unwrap();
        assert!(l.playing && l.position_us >= t1, "{l:?}");
        std::thread::sleep(Duration::from_millis(200));
        assert!(s.status().follow.show_time_us.unwrap() > t1);
        s.stop().unwrap();
    }

    /// M6: a Launchpad-looking LED output starts only with the device permission.
    #[test]
    fn launchpad_led_output_needs_the_permission() {
        let show = omoiiro_store::load_str(include_str!("../../../examples/demo.show.json")).unwrap();
        let start = |allow: bool| {
            let (be, ports) = fake_backend();
            ports.add_output("LPX MIDI");
            let backend: SharedBackend = Arc::new(Mutex::new(Box::new(be)));
            let cfg = SessionConfig {
                time_source: TimeSource::Live,
                show: show.clone(),
                bind: "127.0.0.1:0".into(),
                clock_in: None,
                link: false,
                led_out: LedOut::Port { spec: "LPX MIDI".into(), allow_device: allow },
                control: None,
                timecode_out: None,
                record: None,
                meta: json!({}),
                safety: Default::default(),
                identify_led: allow,
            };
            (FollowSession::start(cfg, backend), ports)
        };
        let (refused, _) = start(false);
        assert!(refused.err().unwrap().contains("looks like a Launchpad"));
        let (s, ports) = start(true);
        let s = s.unwrap();
        std::thread::sleep(Duration::from_millis(300));
        // Identified first: only the Device Inquiry went out, no LED frame.
        assert_eq!(ports.sent("LPX MIDI"), vec![omoiiro_devices::launchpad_x::DEVICE_INQUIRY.to_vec()]);
        assert_eq!(s.status().follow.led_state, Some("identifying"));
        s.stop().unwrap();
    }

    /// U06: a heavy GUI (threads hammering status and preview, CPU spinners)
    /// does not stop rendering or the route to Live. Fake backend, no Live.
    #[test]
    fn heavy_gui_does_not_stop_render_or_forwarding() {
        let show = omoiiro_store::load_str(include_str!("../../../examples/demo.show.json")).unwrap();
        let (be, ports) = fake_backend();
        ports.add_input("Pads");
        ports.add_output(ports::names::CONTROL_PORT);
        let backend: SharedBackend = Arc::new(Mutex::new(Box::new(be)));
        let cfg = SessionConfig {
            time_source: TimeSource::Live,
            show,
            bind: "127.0.0.1:0".into(),
            clock_in: None,
            link: false,
            led_out: LedOut::Virtual,
            control: Some(("Pads".into(), ports::names::CONTROL_PORT.into())),
            timecode_out: None,
            record: None,
            meta: json!({}),
            safety: Default::default(),
            identify_led: false,
        };
        let s = FollowSession::start(cfg, backend).unwrap();
        std::thread::sleep(Duration::from_millis(300)); // bindings connect
        let frames0 = s.status().follow.frame_index;
        let t0 = std::time::Instant::now();
        let gui_stop = AtomicBool::new(false);
        let polls = AtomicU64::new(0);
        std::thread::scope(|sc| {
            for _ in 0..4 {
                sc.spawn(|| {
                    while !gui_stop.load(Ordering::Relaxed) {
                        let st = s.status();
                        let _ = s.take_preview_frame();
                        let _text: String = st.follow.virtual_view.clone().unwrap_or_default().repeat(50);
                        polls.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
            for _ in 0..2 {
                sc.spawn(|| {
                    let mut x = 0u64;
                    while !gui_stop.load(Ordering::Relaxed) {
                        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                        std::hint::black_box(x);
                    }
                });
            }
            for k in 0..50u8 {
                ports.inject("Pads", &[0x90, 20 + k, 100]);
                std::thread::sleep(Duration::from_millis(5));
                ports.inject("Pads", &[0x80, 20 + k, 0]);
                std::thread::sleep(Duration::from_millis(5));
            }
            std::thread::sleep(Duration::from_millis(1500 - (t0.elapsed().as_millis() as u64).min(1500)));
            gui_stop.store(true, Ordering::Relaxed);
        });
        let elapsed = t0.elapsed().as_secs_f64();
        let frames = s.status().follow.frame_index - frames0;
        let expected = elapsed * 30.0;
        assert!(polls.load(Ordering::Relaxed) > 1000, "the GUI threads were busy");
        assert!(frames as f64 >= expected * 0.8, "render kept its rate: {frames} frames in {elapsed:.2} s");
        let sent = ports.sent(ports::names::CONTROL_PORT);
        assert_eq!(sent.len(), 100, "every Note On / Off reached Live's bus");
        let end = s.stop().unwrap();
        let p99 = end["control"]["callback_to_send"]["p99_us"].as_i64().unwrap();
        assert!(p99 < 50_000, "callback -> send p99 {p99} us");
        eprintln!("GUI load: {frames} frames in {elapsed:.2} s, polls {}, callback->send {}", polls.load(Ordering::Relaxed), end["control"]["callback_to_send"]);
    }
}
