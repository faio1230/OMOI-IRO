//! Headless follow mode (M4): bridge -> `Transport` -> Core render -> LED
//! writer (virtual Launchpad X or a port), with the operation route beside it.
//!
//! Each stage is a step function driven by a clock value, so tests run them
//! deterministically on a fake clock; `spawn_*` run the same steps on
//! threads with the real clock:
//!
//! * `RenderLoop` (T05): the scheduler decides which frame is due; late
//!   frames are skipped and the current position is rendered. A validated
//!   show is adopted at the frame boundary (T06). The evaluated frame goes
//!   to a capacity-1 mailbox; an unsent frame is overwritten, never queued.
//! * `LedLoop` (I05, I06): takes the newest frame, keeps the port binding
//!   connected and writes the diff against the last successful send.
//! * Control route (I03, I06, I07): `control::ControlRoute` on its own
//!   thread, so a slow LED send never delays a Note Off to Live.
//!
//! Nothing here sends tempo or transport anywhere.

use crate::connections::{BindingEvent, BindingState, OutputBinding};
use crate::control::ControlRoute;
use crate::diagnostics::perf::{RenderPerf, SendPerf, TimingSummary};
use crate::engine::{DarkReason, Engine, Evaluated, Overrides, ShowExchange};
use crate::output::led::LedWriter;
use crate::output::mailbox::Mailbox;
use crate::scheduler::{FrameScheduler, RenderRate};
use crate::sync::transport::{OutputState, Transport};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use omoiiro_core::render::PadEvent;
use omoiiro_midi::routing::RouteOutcome;
use serde_json::{json, Value};
use omoiiro_devices::adapter::DeviceAdapter;
use omoiiro_devices::simulated::VirtualLpx;
use omoiiro_midi::backend::{MidiBackend, MidiSend, RawIn};
use omoiiro_midi::clock::MonotonicClock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// A rendered frame on its way to the LED writer.
#[derive(Clone, Copy, Debug)]
pub struct FrameMsg {
    pub index: u64,
    pub render_start_us: i64,
    pub eval: Evaluated,
}

/// State shared by the follow-mode threads.
pub struct FollowShared {
    pub transport: Mutex<Transport>,
    pub exchange: ShowExchange,
    pub overrides: Overrides,
    pub mailbox: Mailbox<FrameMsg>,
    /// Latest evaluated frame for the UI preview (capacity 1, never blocks render).
    pub preview: Mailbox<FrameMsg>,
    pub stop: AtomicBool,
    /// Set by a command to clear a control-route fault (overflow, send failure).
    pub control_reset: AtomicBool,
    /// Bumped by a "reconnect all" command: every port binding closes and
    /// reopens (a new connection generation) at its next upkeep.
    pub reconnect_request: AtomicU64,
    /// Added to the bridge discontinuity epoch for the MTC generator: bumped
    /// when the timecode output reconnects so receivers get a Full Frame.
    pub mtc_epoch_bump: AtomicU64,
    pub status: Mutex<FollowStatus>,
    /// Pad presses for input-driven effects (Ripple): copied by the control
    /// thread after forwarding to Live, drained by the render thread.
    pub pad_tx: Sender<PadEvent>,
    pub pad_rx: Receiver<PadEvent>,
    /// Pad events as the render thread took them, with their show time (0.1.3).
    pub pad_log: Mutex<PadLog>,
    /// Latest bridge meter record and when it arrived (monotonic µs; 0.1.3).
    pub audio: Mutex<Option<(i64, omoiiro_core::render::AudioInput)>>,
}

/// Raw pad events kept for a recording or the JSONL record, at most
/// `PadLog::LIMIT` each (presses and releases).
#[derive(Debug, Default)]
pub struct PadLog {
    /// Recording in progress: the show time it began and the events since.
    pub recording: Option<(i64, Vec<(i64, PadEvent)>)>,
    /// For the JSONL record (drained by the sync thread): (monotonic, show time, event).
    pub to_file: Option<Vec<(i64, Option<i64>, PadEvent)>>,
}

impl PadLog {
    pub const LIMIT: usize = 4096;
}

impl FollowShared {
    pub fn new(transport: Transport) -> Arc<FollowShared> {
        let (pad_tx, pad_rx) = crossbeam_channel::bounded(64);
        Arc::new(FollowShared {
            transport: Mutex::new(transport),
            exchange: ShowExchange::new(),
            overrides: Overrides::default(),
            mailbox: Mailbox::new(),
            preview: Mailbox::new(),
            stop: AtomicBool::new(false),
            control_reset: AtomicBool::new(false),
            reconnect_request: AtomicU64::new(0),
            mtc_epoch_bump: AtomicU64::new(0),
            status: Mutex::new(FollowStatus::default()),
            pad_tx,
            pad_rx,
            pad_log: Mutex::new(PadLog::default()),
            audio: Mutex::new(None),
        })
    }
}

/// Latest status for printing (updated by the threads, read by the CLI).
#[derive(Clone, Debug, Default)]
pub struct FollowStatus {
    pub frame_index: u64,
    pub show_time_us: Option<i64>,
    pub output: Option<&'static str>,
    pub gain: f32,
    pub dark: Option<&'static str>,
    pub revision: u64,
    pub led_state: Option<&'static str>,
    pub led_generation: Option<u64>,
    pub led_lit: usize,
    pub virtual_view: Option<String>,
    /// Virtual LPX colors (RGB 0..127, index = `LedId::index`).
    pub virtual_rgb7: Option<Vec<[u8; 3]>>,
    /// Operation route: binding states and the fault cause, if any.
    pub control_in: Option<&'static str>,
    pub control_out: Option<&'static str>,
    pub control_fault: Option<String>,
    pub control_pending_note_offs: usize,
    pub control_in_error: Option<String>,
    pub control_out_error: Option<String>,
    pub led_error: Option<String>,
    /// Timecode output (MTC) binding, when enabled.
    pub timecode_out: Option<&'static str>,
    pub timecode_generation: Option<u64>,
    pub timecode_error: Option<String>,
    /// Live diagnostics (T07), refreshed about once a second by each thread.
    pub diag: LiveDiag,
    /// Flashing warnings at the current tempo and layers muted by the limit.
    pub safety_warnings: Vec<omoiiro_core::safety::FlashWarning>,
    pub muted_layers: Vec<String>,
    /// Frames whose rendering panicked (the previous frame was held) and the last message.
    pub render_panics: u64,
    pub render_fault: Option<String>,
}

/// Running totals and timing percentiles since the session started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct LiveDiag {
    pub frames: u64,
    pub skipped_frames: u64,
    pub jumps: u64,
    pub jitter: TimingSummary,
    pub render: TimingSummary,
    pub led_frames: u64,
    pub led_errors: u64,
    pub led_send: TimingSummary,
    pub render_to_sent: TimingSummary,
    pub mailbox_overwritten: u64,
    pub control_sent: u64,
    pub control_errors: u64,
    pub callback_to_send: TimingSummary,
    /// MTC messages sent (Quarter Frames and Full Frames).
    pub mtc_sent: u64,
    pub mtc_send_errors: u64,
    /// Pad presses that reached the renderer (Ripple).
    pub ripple_spawned: u64,
}

pub struct RenderLoop {
    pub scheduler: FrameScheduler,
    pub engine: Engine,
    pub perf: RenderPerf,
    last_show_time: Option<i64>,
    /// Frames whose show time moved backwards or by more than 1 s (seek / loop).
    pub jumps: u64,
    /// Pad presses taken into the renderer's input (releases not counted).
    pub pad_hits: u64,
}

impl RenderLoop {
    pub fn new(start_us: i64, rate: RenderRate) -> RenderLoop {
        RenderLoop { scheduler: FrameScheduler::new(start_us, rate), engine: Engine::new(), perf: RenderPerf::default(), last_show_time: None, jumps: 0, pad_hits: 0 }
    }

    /// Render if a frame is due at `now()`. Returns the posted frame.
    pub fn step(&mut self, shared: &FollowShared, now: &dyn Fn() -> i64) -> Option<FrameMsg> {
        let t0 = now();
        let tick = self.scheduler.poll(t0)?;
        self.perf.jitter.record(tick.late_us);
        self.perf.skipped_frames += tick.skipped;
        self.engine.adopt(&shared.exchange);
        let snap = shared.transport.lock().unwrap().snapshot(tick.due_us.max(t0));
        // Each event is stamped with the show time it arrived at (triggered clips accept it inside them).
        let mut log = None;
        for ev in shared.pad_rx.try_iter() {
            self.engine.pads.apply(ev, snap.show_time_us);
            self.pad_hits += matches!(ev, PadEvent::Press { .. }) as u64;
            // Locked only when events arrived (rare next to frames).
            let g = log.get_or_insert_with(|| shared.pad_log.lock().unwrap());
            if let (Some((_, evs)), Some(s)) = (g.recording.as_mut(), snap.show_time_us) {
                if evs.len() < PadLog::LIMIT {
                    evs.push((s, ev));
                }
            }
            if let Some(f) = g.to_file.as_mut() {
                if f.len() < PadLog::LIMIT {
                    f.push((t0, snap.show_time_us, ev));
                }
            }
        }
        drop(log);
        self.engine.pads.advance(t0);
        // Audio for this frame: the newest meters unless older than 500 ms (lost).
        match shared.audio.lock().unwrap().as_ref() {
            Some((at, a)) if t0 - at <= omoiiro_core::render::AudioInput::LOST_AFTER_US => {
                self.engine.audio.clone_from(a);
                self.engine.audio_fresh = true;
            }
            _ => self.engine.audio_fresh = false,
        }
        self.engine.audio.now_us = t0;
        let panics_before = self.engine.render_panics;
        let eval = self.engine.evaluate(&snap, &shared.overrides);
        if self.engine.render_panics != panics_before {
            let mut st = shared.status.lock().unwrap();
            st.render_panics = self.engine.render_panics;
            st.render_fault = self.engine.render_fault.clone();
        }
        if self.engine.safety_changed {
            self.engine.safety_changed = false;
            let mut st = shared.status.lock().unwrap();
            st.safety_warnings = self.engine.warnings.clone();
            st.muted_layers = self.engine.muted_layers.clone();
        }
        self.perf.render.record(now() - t0);
        self.perf.frames += 1;
        if let (Some(prev), Some(t)) = (self.last_show_time, eval.show_time_us) {
            if t < prev || t - prev > 1_000_000 {
                // Seek / loop: re-evaluated at the new position, nothing else (I07).
                self.jumps += 1;
            }
        }
        self.last_show_time = eval.show_time_us.or(self.last_show_time);
        let msg = FrameMsg { index: tick.index, render_start_us: t0, eval };
        shared.mailbox.put(msg);
        shared.preview.put(msg);
        let mut st = shared.status.lock().unwrap();
        st.frame_index = tick.index;
        st.show_time_us = eval.show_time_us;
        st.output = Some(match snap.output {
            OutputState::Follow => "follow",
            OutputState::Hold => "hold",
            OutputState::Black => "black",
        });
        st.gain = eval.gain;
        st.dark = eval.dark.map(DarkReason::name);
        st.revision = eval.revision;
        Some(msg)
    }
}

/// Where LED frames go.
pub enum LedSink {
    /// In-process virtual Launchpad X (no MIDI port).
    Virtual(VirtualLpx),
    /// A MIDI output port (exact name or `#index`).
    Port(OutputBinding),
}

struct VirtualOut<'a>(&'a mut VirtualLpx);

impl MidiSend for VirtualOut<'_> {
    fn send_msg(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.0.apply(bytes).map(|_| ())
    }
}

pub struct LedLoop<A: DeviceAdapter> {
    pub writer: LedWriter<A>,
    pub sink: LedSink,
    pub perf: SendPerf,
    pub log: Vec<String>,
    /// Device identification (port sinks to a real device): frames are
    /// written only once it is Ready. Replies come from the device input.
    pub identify: Option<(crate::output::identify::DeviceIdentify, Receiver<Vec<u8>>)>,
    /// Frames not written because the device was not identified yet.
    pub held_frames: u64,
}

impl<A: DeviceAdapter> LedLoop<A> {
    pub fn new(adapter: A, sink: LedSink) -> LedLoop<A> {
        LedLoop { writer: LedWriter::new(adapter), sink, perf: SendPerf::default(), log: Vec::new(), identify: None, held_frames: 0 }
    }

    /// Identify the device on every new connection before writing frames.
    pub fn with_identify(mut self, cfg: crate::output::identify::IdentifyConfig, replies: Receiver<Vec<u8>>) -> LedLoop<A> {
        self.identify = Some((crate::output::identify::DeviceIdentify::new(cfg), replies));
        self
    }

    fn send_packets(&mut self, packets: Vec<Vec<u8>>, now: i64) {
        if let LedSink::Port(b) = &mut self.sink {
            for p in packets {
                if let Err(e) = b.send(&p, now) {
                    self.log.push(format!("identify send failed: {e}"));
                    return;
                }
            }
        }
    }

    /// Feed device replies and retry timeouts (identification only).
    pub fn step_identify(&mut self, now: i64) {
        let Some((id, rx)) = self.identify.as_mut() else { return };
        let mut out = Vec::new();
        for bytes in rx.try_iter() {
            out.extend(id.on_input(&bytes, now));
        }
        out.extend(id.poll(now));
        if !out.is_empty() {
            self.send_packets(out, now);
        }
    }

    /// Port upkeep (presence, reconnect). A new connection re-initializes
    /// the device and forces a full refresh.
    pub fn maintain(&mut self, backend: Option<&mut dyn MidiBackend>, now: i64) {
        let (LedSink::Port(b), Some(backend)) = (&mut self.sink, backend) else { return };
        if let Some(ev) = b.check_presence(backend, now) {
            self.log.push(format!("{ev:?}"));
        }
        let mut start = None;
        if let Some(ev) = b.ensure(backend, now) {
            if let BindingEvent::Connected { generation, .. } = &ev {
                self.writer.reconnect(*generation);
                start = self.identify.as_mut().map(|(id, rx)| {
                    rx.try_iter().for_each(drop); // stale replies of the old connection
                    id.start(now)
                });
            }
            self.log.push(format!("{ev:?}"));
        }
        if let Some(packets) = start {
            self.send_packets(packets, now);
        }
    }

    /// Write one frame (the newest one taken from the mailbox).
    pub fn write(&mut self, msg: &FrameMsg, now: &dyn Fn() -> i64) {
        if self.identify.as_ref().is_some_and(|(id, _)| !id.ready()) {
            self.held_frames += 1;
            return;
        }
        let t0 = now();
        let result = match &mut self.sink {
            LedSink::Virtual(v) => self.writer.write(&mut VirtualOut(v), &msg.eval.frame),
            LedSink::Port(b) => match b.sender() {
                Some(out) => self.writer.write(out, &msg.eval.frame),
                None => return, // not connected: the frame is dropped, the base stays
            },
        };
        let t1 = now();
        match result {
            Ok(_) => {
                self.perf.frames += 1;
                self.perf.send.record(t1 - t0);
                self.perf.frame_latency.record(t1 - msg.render_start_us);
            }
            Err(e) => {
                self.perf.errors += 1;
                if let LedSink::Port(b) = &mut self.sink {
                    let ev = b.mark_lost(&format!("send failed: {e}"), t1);
                    self.log.push(format!("{ev:?}"));
                } else {
                    self.log.push(format!("virtual LPX rejected a packet: {e}"));
                }
            }
        }
    }

    pub fn state(&self) -> &'static str {
        match &self.sink {
            LedSink::Virtual(_) => "virtual",
            LedSink::Port(b) if b.state() == crate::connections::BindingState::Ready => match &self.identify {
                Some((id, _)) if !id.ready() => id.state().name(),
                _ => "ready",
            },
            LedSink::Port(b) => b.state().name(),
        }
    }

    pub fn virtual_lpx(&self) -> Option<&VirtualLpx> {
        match &self.sink {
            LedSink::Virtual(v) => Some(v),
            LedSink::Port(_) => None,
        }
    }

    /// Close and reopen the LED port at the next upkeep ("reconnect all").
    pub fn rebind(&mut self) {
        if let LedSink::Port(b) = &mut self.sink {
            let spec = b.spec().to_string();
            b.rebind(&spec);
            self.log.push("rebind requested".into());
        }
    }

    fn publish(&self, shared: &FollowShared) {
        let mut st = shared.status.lock().unwrap();
        st.led_state = Some(self.state());
        st.led_error = match &self.sink {
            LedSink::Port(b) => self.identify.as_ref().and_then(|(id, _)| id.fault.clone()).or_else(|| b.last_error().map(str::to_string)),
            LedSink::Virtual(_) => None,
        };
        st.led_generation = match &self.sink {
            LedSink::Port(b) => b.generation(),
            LedSink::Virtual(_) => None,
        };
        st.led_lit = self.writer.last_sent().map(|f| f.lit()).unwrap_or(0);
        // Virtual sink only: the text view of what the virtual device shows.
        st.virtual_view = self.virtual_lpx().map(|v| v.ascii());
        st.virtual_rgb7 = self.virtual_lpx().map(|v| v.frame.rgb7.to_vec());
    }

    /// Best effort: LEDs off, device mode restored.
    pub fn shutdown(&mut self) -> Result<(), String> {
        match &mut self.sink {
            LedSink::Virtual(v) => self.writer.shutdown(&mut VirtualOut(v)),
            LedSink::Port(b) => match b.sender() {
                Some(out) => self.writer.shutdown(out),
                None => Ok(()),
            },
        }
    }
}

pub type SharedBackend = Arc<Mutex<Box<dyn MidiBackend>>>;

/// How often bindings check port presence.
pub const MAINTAIN_US: i64 = 250_000;
/// How often threads publish their diagnostics into `FollowStatus`.
pub const DIAG_PUBLISH_US: i64 = 1_000_000;

pub fn spawn_render(shared: Arc<FollowShared>, clock: MonotonicClock, rate: RenderRate) -> JoinHandle<RenderLoop> {
    std::thread::Builder::new()
        .name("omoiiro-render".into())
        .spawn(move || {
            let now = || clock.now_us();
            let mut r = RenderLoop::new(now(), rate);
            let mut next_pub = now();
            while !shared.stop.load(Ordering::SeqCst) {
                if now() >= next_pub {
                    let mut st = shared.status.lock().unwrap();
                    st.diag.frames = r.perf.frames;
                    st.diag.skipped_frames = r.perf.skipped_frames;
                    st.diag.jumps = r.jumps;
                    st.diag.jitter = r.perf.jitter.summary();
                    st.diag.render = r.perf.render.summary();
                    st.diag.mailbox_overwritten = shared.mailbox.overwritten();
                    st.diag.ripple_spawned = r.pad_hits;
                    next_pub = now() + DIAG_PUBLISH_US;
                }
                if r.step(&shared, &now).is_none() {
                    let wait = r.scheduler.until_next_us(now()).clamp(0, 5_000);
                    std::thread::sleep(Duration::from_micros(wait as u64));
                }
            }
            shared.mailbox.close();
            r
        })
        .expect("spawn render thread")
}

pub fn spawn_led<A: DeviceAdapter + 'static>(shared: Arc<FollowShared>, clock: MonotonicClock, mut led: LedLoop<A>, backend: Option<SharedBackend>) -> JoinHandle<LedLoop<A>> {
    std::thread::Builder::new()
        .name("omoiiro-led".into())
        .spawn(move || {
            let now = || clock.now_us();
            let mut next_maintain = i64::MIN;
            let mut seen_reconnect = shared.reconnect_request.load(Ordering::SeqCst);
            let mut next_pub = now();
            loop {
                let t = now();
                let req = shared.reconnect_request.load(Ordering::SeqCst);
                if req != seen_reconnect {
                    seen_reconnect = req;
                    led.rebind();
                    next_maintain = i64::MIN;
                }
                if t >= next_pub {
                    let mut st = shared.status.lock().unwrap();
                    st.diag.led_frames = led.perf.frames;
                    st.diag.led_errors = led.perf.errors;
                    st.diag.led_send = led.perf.send.summary();
                    st.diag.render_to_sent = led.perf.frame_latency.summary();
                    next_pub = t + DIAG_PUBLISH_US;
                }
                if t >= next_maintain {
                    match &backend {
                        Some(b) => led.maintain(Some(b.lock().unwrap().as_mut()), t),
                        None => led.maintain(None, t),
                    }
                    next_maintain = t + MAINTAIN_US;
                }
                led.step_identify(now());
                match shared.mailbox.take_timeout(Duration::from_millis(50)) {
                    Some(msg) => led.write(&msg, &now),
                    None if shared.stop.load(Ordering::SeqCst) => break,
                    None => {}
                }
                led.publish(&shared);
            }
            if let Err(e) = led.shutdown() {
                led.log.push(format!("shutdown: {e}"));
            }
            led
        })
        .expect("spawn LED thread")
}

pub fn spawn_control(shared: Arc<FollowShared>, clock: MonotonicClock, mut route: ControlRoute, rx: Receiver<RawIn>, backend: SharedBackend) -> JoinHandle<ControlRoute> {
    std::thread::Builder::new()
        .name("omoiiro-control".into())
        .spawn(move || {
            let now = || clock.now_us();
            let mut next_maintain = i64::MIN;
            let mut seen_reconnect = shared.reconnect_request.load(Ordering::SeqCst);
            let mut input_ready = false;
            while !shared.stop.load(Ordering::SeqCst) {
                let req = shared.reconnect_request.load(Ordering::SeqCst);
                if req != seen_reconnect {
                    seen_reconnect = req;
                    let _ = shared.pad_tx.try_send(PadEvent::ReleaseAll { t_us: now() });
                    route.reconnect(now());
                    next_maintain = i64::MIN;
                }
                if shared.control_reset.swap(false, Ordering::SeqCst) {
                    route.reset_fault();
                    route.log.push("fault reset by command".into());
                }
                let t = now();
                if t >= next_maintain {
                    route.maintain(backend.lock().unwrap().as_mut(), t);
                    next_maintain = t + MAINTAIN_US;
                    // Input gone (unplugged): no release will come, so release every held pad.
                    let ready = route.input.state() == crate::connections::BindingState::Ready;
                    if input_ready && !ready {
                        let _ = shared.pad_tx.try_send(PadEvent::ReleaseAll { t_us: t });
                    }
                    input_ready = ready;
                }
                match rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(ev) => {
                        // Forward first; the pad copy for the renderer never delays it.
                        let out = route.handle(&ev, &now);
                        copy_pad(&shared, &ev, &out);
                        for ev in rx.try_iter() {
                            let out = route.handle(&ev, &now);
                            copy_pad(&shared, &ev, &out);
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                let mut st = shared.status.lock().unwrap();
                st.control_in = Some(route.input.state().name());
                st.control_out = Some(route.output.state().name());
                st.control_fault = route.router.fault_reason().map(str::to_string);
                st.control_pending_note_offs = route.pending_offs().len();
                st.control_in_error = route.input.last_error().map(str::to_string);
                st.control_out_error = route.output.last_error().map(str::to_string);
                st.diag.control_sent = route.perf.sent;
                st.diag.control_errors = route.perf.errors;
                st.diag.callback_to_send = route.perf.latency.summary();
            }
            route.shutdown(now());
            route
        })
        .expect("spawn control thread")
}

/// A forwarded grid-pad Note On, as a pad hit for the renderer (dropped if the queue is full).
fn copy_pad(shared: &FollowShared, ev: &RawIn, out: &RouteOutcome) {
    if let Some(e) = pad_event(ev.t_mono_us, out) {
        let _ = shared.pad_tx.try_send(e);
    }
}

/// Note On (velocity > 0) or Note Off of a Launchpad X grid pad in
/// Programmer Mode, as forwarded (Note On velocity 0 is already Note Off).
pub fn pad_event(t_us: i64, out: &RouteOutcome) -> Option<PadEvent> {
    let RouteOutcome::Send { bytes, .. } = out else { return None };
    let on = match bytes[0] & 0xF0 {
        0x90 if bytes[2] > 0 => true,
        0x80 | 0x90 => false,
        _ => return None,
    };
    let i = omoiiro_devices::launchpad_x::pad_to_led_index(bytes[1])?;
    let (x, y) = ((i % 8) as u8, (i / 8) as u8);
    Some(if on { PadEvent::Press { x, y, velocity: bytes[2], t_us } } else { PadEvent::Release { x, y, t_us } })
}

/// Summary JSON of a finished run (T07).
pub fn summary_json<A: DeviceAdapter>(render: &RenderLoop, led: &LedLoop<A>, control: Option<&ControlRoute>, mailbox_overwritten: u64) -> Value {
    json!({
        "render": {
            "frames": render.perf.frames,
            "skipped_frames": render.perf.skipped_frames,
            "jumps": render.jumps,
            "show_swaps": render.engine.swaps,
            "jitter": render.perf.jitter.summary_json(),
            "render_time": render.perf.render.summary_json(),
        },
        "led": {
            "sink": match &led.sink { LedSink::Virtual(_) => "virtual", LedSink::Port(_) => "port" },
            "state": led.state(),
            "frames_sent": led.perf.frames,
            "send_errors": led.perf.errors,
            "mailbox_overwritten": mailbox_overwritten,
            "full_refreshes": led.writer.stats.full_refreshes,
            "unchanged_frames": led.writer.stats.unchanged,
            "packets": led.writer.stats.packets,
            "bytes": led.writer.stats.bytes,
            "send_time": led.perf.send.summary_json(),
            "render_to_sent": led.perf.frame_latency.summary_json(),
            "log": led.log,
        },
        "control": control.map(|c| json!({
            "input": c.input.port_name(), "output": c.output.port_name(),
            "input_state": c.input.state().name(), "output_state": c.output.state().name(),
            "received": c.router.stats.received, "sent": c.router.stats.sent, "normalized": c.router.stats.normalized,
            "ignored": c.router.stats.ignored, "suppressed": c.router.stats.suppressed, "blocked": c.router.stats.blocked,
            "faults": c.router.stats.faults, "released": c.router.stats.released, "stale": c.stale,
            "fault": c.router.fault_reason(), "pending_note_offs": c.pending_offs().len(),
            "callback_to_send": c.perf.latency.summary_json(), "send_errors": c.perf.errors,
            "log": c.log,
        })),
    })
}

/// True while a port binding exists and is not ready (for status lines).
pub fn binding_down(b: &OutputBinding) -> bool {
    b.state() != BindingState::Ready
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::bridge::BridgeSample;
    use crate::sync::transport::TransportConfig;
    use omoiiro_core::time::{FrameRate, Timecode};
    use omoiiro_devices::adapter::DeviceFrame;
    use omoiiro_devices::launchpad_x::LaunchpadXAdapter;
    use omoiiro_midi::fake::fake_backend;

    const DEMO: &str = include_str!("../../../examples/demo.show.json");
    const LED_PORT: &str = "LPX MIDI";

    fn transport() -> Transport {
        let origin = Timecode::parse("01:00:00:00", FrameRate::Fps30).unwrap();
        Transport::new(TransportConfig::defaults(origin, 0, None))
    }

    /// 60 Hz bridge samples while playing from `song0_ms` at time 0.
    struct Feed {
        seq: i64,
        next_t: i64,
        song0_ms: f64,
    }

    impl Feed {
        fn new(song0_ms: f64) -> Feed {
            Feed { seq: 0, next_t: 0, song0_ms }
        }

        /// Deliver every sample due up to `now`.
        fn until(&mut self, shared: &FollowShared, now: i64) {
            let mut ev = Vec::new();
            while self.next_t <= now {
                self.seq += 1;
                let song_ms = self.song0_ms + self.next_t as f64 / 1000.0;
                let p = json!({"v": 2, "seq": self.seq, "kind": "listener", "event": "song_time", "song_ms": song_ms, "is_playing": true, "errors": []});
                shared.transport.lock().unwrap().feed_bridge(BridgeSample::from_payload(self.next_t, &p), &mut ev);
                self.next_t += 16_667;
            }
        }
    }

    #[test]
    fn follows_the_bridge_and_swaps_shows_at_frame_boundaries() {
        let (clock, time) = MonotonicClock::fake();
        let shared = FollowShared::new(transport());
        let mut r = RenderLoop::new(0, RenderRate::FPS30);
        let mut led = LedLoop::new(LaunchpadXAdapter, LedSink::Virtual(VirtualLpx::new()));

        // No show yet: black, but the scheduler runs.
        let now = || clock.now_us();
        let m = r.step(&shared, &now).unwrap();
        assert_eq!(m.eval.dark, Some(DarkReason::NoShow));

        let show = omoiiro_store::load_str(DEMO).unwrap();
        assert_eq!(shared.exchange.submit(&show), Ok(1));
        let mut bad = show.clone();
        bad.layers[0].target_id = "nope".into();
        assert!(shared.exchange.submit(&bad).is_err(), "an invalid show never becomes a revision");

        // Play from song time 10 s (base and sweep visible).
        let mut feed = Feed::new(10_000.0);
        let mut frames = Vec::new();
        while time.now_us() < 1_000_000 {
            feed.until(&shared, time.now_us());
            if let Some(m) = r.step(&shared, &|| clock.now_us()) {
                frames.push(m);
            }
            time.advance_us(1_000);
        }
        assert_eq!(r.engine.revision(), 1);
        let last = frames.last().unwrap();
        assert_eq!(last.eval.dark, None);
        let st = last.eval.show_time_us.unwrap();
        assert!((st - (10_000_000 + last.render_start_us)).abs() < 20_000, "show time {st} at {}", last.render_start_us);
        led.write(last, &|| clock.now_us());
        let v = led.virtual_lpx().unwrap();
        assert_eq!(v.frame, last.eval.frame);
        assert_eq!(v.frame.lit(), 64, "demo base layer covers the grid");

        // Blackout override: black frames, transport untouched.
        shared.overrides.set_blackout(true);
        time.advance_us(40_000);
        let m = r.step(&shared, &|| clock.now_us()).unwrap();
        assert_eq!((m.eval.dark, m.eval.frame), (Some(DarkReason::Blackout), DeviceFrame::black()));
        assert!(m.eval.show_time_us.is_some());
    }

    #[test]
    fn forwarded_pad_presses_reach_the_ripple_in_the_render_loop() {
        // Only grid Note Ons (velocity > 0) are pad hits; buttons, Note Off and CC are not.
        let send = |b: [u8; 3]| RouteOutcome::Send { bytes: b, normalized: false };
        assert_eq!(pad_event(5, &send([0x90, 11, 100])), Some(PadEvent::Press { x: 0, y: 0, velocity: 100, t_us: 5 }));
        assert_eq!(pad_event(5, &send([0x90, 88, 1])), Some(PadEvent::Press { x: 7, y: 7, velocity: 1, t_us: 5 }));
        assert_eq!(pad_event(5, &send([0x90, 19, 100])), None, "side button");
        assert_eq!(pad_event(5, &send([0x80, 11, 0])), Some(PadEvent::Release { x: 0, y: 0, t_us: 5 }));
        assert_eq!(pad_event(5, &send([0xB0, 11, 127])), None);
        assert_eq!(pad_event(5, &RouteOutcome::Ignored), None, "not forwarded, not drawn");

        // A show whose continuous layer is a ripple: a pad press lights the frame.
        let (clock, time) = MonotonicClock::fake();
        let shared = FollowShared::new(transport());
        let mut show = omoiiro_store::load_str(DEMO).unwrap();
        show.layers.truncate(0);
        show.presets.push(omoiiro_core::model::Preset { id: "ripple".into(), name: "波紋".into(), effect: omoiiro_core::edit::default_effect("builtin.ripple").unwrap() });
        let edit = omoiiro_core::edit::EditCommand::AddContinuousLayer { name: "r".into(), preset_id: "ripple".into(), index: None };
        let (show, _) = omoiiro_core::edit::apply(&show, &edit, omoiiro_core::edit::Snap::None).unwrap();
        shared.exchange.submit(&show).unwrap();
        let mut r = RenderLoop::new(0, RenderRate::FPS30);
        let mut feed = Feed::new(10_000.0);
        let mut last = None;
        while time.now_us() < 500_000 {
            feed.until(&shared, time.now_us());
            if time.now_us() == 300_000 {
                shared.pad_tx.send(pad_event(time.now_us(), &send([0x90, 44, 100])).unwrap()).unwrap();
            }
            if let Some(m) = r.step(&shared, &|| clock.now_us()) {
                if time.now_us() < 300_000 {
                    assert_eq!(m.eval.frame.lit(), 0, "nothing before a press");
                }
                last = Some(m);
            }
            time.advance_us(1_000);
        }
        assert_eq!(r.pad_hits, 1);
        assert!(last.unwrap().eval.frame.lit() > 0, "the ring is drawn");

        // 0.1.3: a recording catches presses with their show time; reset empties the input.
        shared.pad_log.lock().unwrap().recording = Some((0, Vec::new()));
        shared.pad_tx.send(PadEvent::Press { x: 2, y: 3, velocity: 90, t_us: time.now_us() }).unwrap();
        while r.step(&shared, &|| clock.now_us()).is_none() {
            feed.until(&shared, time.now_us());
            time.advance_us(1_000);
        }
        let (_, evs) = shared.pad_log.lock().unwrap().recording.take().unwrap();
        assert_eq!(evs.len(), 1);
        assert!(matches!(evs[0].1, PadEvent::Press { x: 2, y: 3, .. }) && evs[0].0 > 0, "{evs:?}");
        shared.pad_tx.send(PadEvent::Reset).unwrap();
        while r.step(&shared, &|| clock.now_us()).is_none() {
            time.advance_us(1_000);
        }
        assert!(r.engine.pads.hits.is_empty() && r.engine.pads.history.is_empty());

        // 0.1.3: fresh meters reach the engine; after 500 ms without a record they are lost.
        let a = omoiiro_core::render::AudioInput { now_us: 0, master: [0.5, 0.5], tracks: vec![] };
        *shared.audio.lock().unwrap() = Some((time.now_us(), a));
        while r.step(&shared, &|| clock.now_us()).is_none() {
            time.advance_us(1_000);
        }
        assert!(r.engine.audio_fresh && r.engine.audio.master == [0.5, 0.5]);
        time.advance_us(600_000);
        while r.step(&shared, &|| clock.now_us()).is_none() {
            time.advance_us(1_000);
        }
        assert!(!r.engine.audio_fresh, "lost after 500 ms");
    }

    /// M6 preparation: a (fake) Launchpad X is identified before any LED
    /// frame is written, and again after it is unplugged and plugged back.
    #[test]
    fn led_device_is_identified_before_frames() {
        use omoiiro_devices::launchpad_x::{mode_query, mode_select, Mode, DEVICE_INQUIRY};
        let (clock, time) = MonotonicClock::fake();
        let shared = FollowShared::new(transport());
        shared.exchange.submit(&omoiiro_store::load_str(DEMO).unwrap()).unwrap();
        let (be, ports) = fake_backend();
        let mut be: Box<dyn MidiBackend> = Box::new(be);
        ports.add_output(LED_PORT);
        let (reply_tx, reply_rx) = crossbeam_channel::bounded(16);
        let mut led = LedLoop::new(LaunchpadXAdapter, LedSink::Port(OutputBinding::new("led_out", LED_PORT)))
            .with_identify(crate::output::identify::IdentifyConfig { reply_timeout_us: 100_000, max_tries: 3 }, reply_rx);
        led.maintain(Some(be.as_mut()), 0);
        assert_eq!(ports.sent(LED_PORT), vec![DEVICE_INQUIRY.to_vec()], "only the inquiry is sent first");
        assert_eq!(led.state(), "identifying");

        let mut r = RenderLoop::new(0, RenderRate::FPS30);
        let mut feed = Feed::new(20_000.0);
        let mut step = |r: &mut RenderLoop, led: &mut LedLoop<LaunchpadXAdapter>| {
            time.advance_us(34_000);
            feed.until(&shared, time.now_us());
            r.step(&shared, &|| clock.now_us());
            if let Some(m) = shared.mailbox.try_take() {
                led.write(&m, &|| clock.now_us());
            }
            led.step_identify(clock.now_us());
        };
        step(&mut r, &mut led);
        assert_eq!(led.held_frames, 1, "no frame before the device is identified");
        assert_eq!(ports.sent(LED_PORT).len(), 1);

        // Launchpad X answers: Programmer Mode is selected and queried.
        reply_tx.send(vec![0xF0, 0x7E, 0x00, 0x06, 0x02, 0x00, 0x20, 0x29, 0x03, 0x01, 0x00, 0x00, 0, 4, 0, 7, 0xF7]).unwrap();
        led.step_identify(clock.now_us());
        assert_eq!(&ports.sent(LED_PORT)[1..], &[mode_select(Mode::Programmer), mode_query()]);
        assert_eq!(led.state(), "configuring");
        reply_tx.send(mode_select(Mode::Programmer)).unwrap();
        led.step_identify(clock.now_us());
        assert_eq!(led.state(), "ready");
        step(&mut r, &mut led);
        let sent = ports.sent(LED_PORT);
        assert_eq!(sent[3], mode_select(Mode::Programmer), "the writer initializes as usual");
        assert_eq!(sent[4].len(), 6 + 1 + 64 * 5 + 1, "then the first frame in full");

        // Unplug and back: identified again before frames.
        ports.unplug(LED_PORT);
        step(&mut r, &mut led);
        led.maintain(Some(be.as_mut()), clock.now_us()); // the periodic upkeep sees the port gone
        assert_eq!(led.state(), "recovering");
        ports.replug(LED_PORT, false, true);
        ports.clear_sent(LED_PORT);
        time.advance_us(crate::connections::RETRY_US);
        led.maintain(Some(be.as_mut()), clock.now_us());
        assert_eq!(ports.sent(LED_PORT), vec![DEVICE_INQUIRY.to_vec()]);
        assert_eq!(led.state(), "identifying");
        // No answer at all: retries, then a fault with the reason; frames stay held.
        for _ in 0..12 {
            step(&mut r, &mut led); // 12 x 34 ms > 3 tries x 100 ms
        }
        assert_eq!(led.state(), "fault");
        assert!(led.identify.as_ref().unwrap().0.fault.as_ref().unwrap().contains("応答がない"));
        assert!(ports.sent(LED_PORT).iter().all(|p| p == &DEVICE_INQUIRY.to_vec()), "nothing but inquiries to an unidentified device");
    }

    #[test]
    fn slow_led_port_skips_frames_and_recovers_after_unplug() {
        let (clock, time) = MonotonicClock::fake();
        let shared = FollowShared::new(transport());
        shared.exchange.submit(&omoiiro_store::load_str(DEMO).unwrap()).unwrap();
        let (be, ports) = fake_backend();
        let mut be: Box<dyn MidiBackend> = Box::new(be);
        ports.add_output(LED_PORT);
        // Each LED send costs 50 ms of (fake) time: slower than 30 fps.
        ports.set_send_cost(time.clone(), 50_000);
        let mut r = RenderLoop::new(0, RenderRate::FPS30);
        let mut led = LedLoop::new(LaunchpadXAdapter, LedSink::Port(OutputBinding::new("led_out", LED_PORT)));
        led.maintain(Some(be.as_mut()), 0);
        assert_eq!(led.state(), "ready");

        let mut feed = Feed::new(20_000.0);
        let mut taken = Vec::new();
        // Render and write in turn on one fake time line: each write takes
        // 50 ms, so frames are skipped and the next render jumps to the
        // current time.
        let mut k = 0;
        while time.now_us() < 2_000_000 {
            feed.until(&shared, time.now_us());
            // Every frame differs (brightness alternates), so every write sends.
            k += 1;
            shared.overrides.set_brightness(if k % 2 == 0 { 1.0 } else { 0.5 });
            let now = || clock.now_us();
            r.step(&shared, &now);
            if let Some(m) = shared.mailbox.try_take() {
                taken.push(m.index);
                led.write(&m, &now);
            }
            time.advance_us(1_000);
        }
        // 2 s at 30 fps = 60 slots; each write blocks for 50 ms, so about a third are skipped.
        assert!(r.perf.skipped_frames >= 15, "skipped {}", r.perf.skipped_frames);
        assert_eq!(r.perf.frames + r.perf.skipped_frames, 60);
        assert!(taken.windows(2).all(|w| w[1] > w[0]), "frames are never written out of order: {taken:?}");
        // Late frames are never rendered late: the render is for the slot that contains now.
        assert!(r.perf.jitter.max() < 33_334, "jitter max {}", r.perf.jitter.max());
        assert_eq!(led.writer.stats.full_refreshes, 1);
        let sent_before = ports.sent(LED_PORT).len();

        // Unplug: the write fails, the binding recovers, the next frame is full.
        ports.unplug(LED_PORT);
        time.advance_us(40_000);
        feed.until(&shared, time.now_us());
        r.step(&shared, &|| clock.now_us());
        let m = shared.mailbox.try_take().unwrap();
        led.write(&m, &|| clock.now_us());
        assert_eq!(led.state(), "recovering");
        assert_eq!(led.perf.errors, 1);
        ports.replug(LED_PORT, false, true);
        led.maintain(Some(be.as_mut()), clock.now_us());
        assert_eq!(led.state(), "ready");
        time.advance_us(40_000);
        feed.until(&shared, time.now_us());
        r.step(&shared, &|| clock.now_us());
        let m = shared.mailbox.try_take().unwrap();
        led.write(&m, &|| clock.now_us());
        let after = &ports.sent(LED_PORT)[sent_before..];
        assert_eq!(after[0], omoiiro_devices::launchpad_x::mode_select(omoiiro_devices::launchpad_x::Mode::Programmer));
        assert_eq!(after[1].len(), 6 + 1 + 64 * 5 + 1, "full refresh after reconnect");
        assert_eq!(led.writer.stats.full_refreshes, 2);
    }
}
