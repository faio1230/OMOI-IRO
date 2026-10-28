//! Fixed Trigger / Action (I07) and the operation route to Live (I03 + I06).
//!
//! Triggers are fixed in the MVP (docs/02 "TriggerとAction"):
//!
//! | Trigger | Action | Executed by |
//! | --- | --- | --- |
//! | device Note / CC | `RouteMidi` | `ControlRoute` (control output) |
//! | UI Blackout | `SetBlackout` | `Overrides` (LEDs only) |
//! | transport seek / loop | `Reevaluate` | render loop (no external output) |
//!
//! `RouteMidi` and `SetBlackout` are separate effects: Blackout never stops
//! the route to Live, and the route never changes the LEDs. A seek only
//! makes the render loop evaluate the new position; no MIDI is sent for it,
//! so past operations are never replayed.
//!
//! `ControlRoute` owns the control input / output bindings and the
//! `Router`. Dropped input (queue overflow), a failed send, a lost output or
//! a settings change release the sounding notes (Note Off for each, never a
//! channel-wide All Notes Off); overflow and send failure also fault the
//! route until `reset_fault`. Note Offs that cannot be sent because the
//! output is gone are sent when it reconnects.

use crate::connections::{BindingEvent, BindingState, InputBinding, OutputBinding};
use crate::diagnostics::perf::ControlPerf;
use crate::engine::Overrides;
use omoiiro_midi::backend::{Ingress, MidiBackend, RawIn};
use omoiiro_midi::routing::{RouteOutcome, RouteRules, Router};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger<'a> {
    DeviceMidi(&'a [u8]),
    UiBlackout(bool),
    TransportJump,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    RouteMidi { bytes: [u8; 3], normalized: bool },
    SetBlackout(bool),
    Reevaluate,
}

/// Fixed trigger rules. Device MIDI goes through the router (whitelist,
/// duplicates, fault); the others map one to one.
pub fn evaluate(router: &mut Router, t: Trigger<'_>) -> Option<Action> {
    match t {
        Trigger::DeviceMidi(bytes) => match router.handle(bytes) {
            RouteOutcome::Send { bytes, normalized } => Some(Action::RouteMidi { bytes, normalized }),
            _ => None,
        },
        Trigger::UiBlackout(on) => Some(Action::SetBlackout(on)),
        Trigger::TransportJump => Some(Action::Reevaluate),
    }
}

/// Apply a UI action that does not need the route (Blackout, seek).
pub fn apply_ui(action: Action, overrides: &Overrides) {
    if let Action::SetBlackout(on) = action {
        overrides.set_blackout(on);
    }
}

pub struct ControlRoute {
    pub router: Router,
    pub input: InputBinding,
    pub output: OutputBinding,
    ingress: Ingress,
    pending_offs: Vec<[u8; 3]>,
    seen_dropped: u64,
    seen_oversize: u64,
    pub stale: u64,
    pub perf: ControlPerf,
    pub log: Vec<String>,
    /// SysEx from the control input goes here too (device identification
    /// replies when the control input is the LED device).
    pub sysex_to: Option<crossbeam_channel::Sender<Vec<u8>>>,
}

impl ControlRoute {
    pub fn new(input_spec: &str, output_spec: &str, rules: RouteRules, ingress: Ingress) -> ControlRoute {
        ControlRoute {
            router: Router::new(rules),
            input: InputBinding::new("control_in", input_spec, true),
            output: OutputBinding::new("control_out", output_spec),
            ingress,
            pending_offs: Vec::new(),
            seen_dropped: 0,
            seen_oversize: 0,
            stale: 0,
            perf: ControlPerf::default(),
            log: Vec::new(),
            sysex_to: None,
        }
    }

    pub fn pending_offs(&self) -> &[[u8; 3]] {
        &self.pending_offs
    }

    /// Connection upkeep: presence, (re)open, flush held Note Offs.
    pub fn maintain(&mut self, backend: &mut dyn MidiBackend, now: i64) {
        if let Some(ev) = self.input.check_presence(backend, now) {
            self.log.push(format!("{ev:?}"));
            // The device that held the notes is gone: release them now.
            let offs = self.router.release_all();
            self.send_offs(offs, now);
        }
        if let Some(ev) = self.output.check_presence(backend, now) {
            self.log.push(format!("{ev:?}"));
            let offs = self.router.fault("control output lost");
            self.pending_offs.extend(offs);
        }
        if let Some(ev) = self.output.ensure(backend, now) {
            self.log.push(format!("{ev:?}"));
            if matches!(ev, BindingEvent::Connected { .. }) {
                let offs = std::mem::take(&mut self.pending_offs);
                self.send_offs(offs, now);
                if self.router.fault_reason() == Some("control output lost") {
                    self.router.reset();
                }
            }
        }
        if let Some(ev) = self.input.ensure(backend, &self.ingress, now) {
            self.log.push(format!("{ev:?}"));
        }
        self.check_overflow(now);
    }

    fn check_overflow(&mut self, now: i64) {
        let (d, o) = (self.ingress.stats().dropped(), self.ingress.stats().oversize());
        if d > self.seen_dropped || o > self.seen_oversize {
            let reason = if d > self.seen_dropped { "input queue overflow" } else { "oversize input" };
            self.seen_dropped = d;
            self.seen_oversize = o;
            self.log.push(format!("fault: {reason}"));
            let offs = self.router.fault(reason);
            self.send_offs(offs, now);
        }
    }

    fn send_offs(&mut self, offs: Vec<[u8; 3]>, now: i64) {
        for (i, off) in offs.iter().enumerate() {
            if self.output.state() != BindingState::Ready || self.output.send(off, now).is_err() {
                self.pending_offs.extend_from_slice(&offs[i..]);
                return;
            }
        }
    }

    /// Route one input event. `now` is taken after the send for latency.
    pub fn handle(&mut self, ev: &RawIn, now: &dyn Fn() -> i64) -> RouteOutcome {
        self.check_overflow(now());
        if Some(ev.generation) != self.input.generation() {
            // Input from a closed connection generation is never forwarded.
            self.stale += 1;
            return RouteOutcome::Ignored;
        }
        if ev.bytes.first() == Some(&0xF0) {
            if let Some(tx) = &self.sysex_to {
                let _ = tx.try_send(ev.bytes.clone());
            }
        }
        let outcome = self.router.handle(&ev.bytes);
        if let RouteOutcome::Send { bytes, .. } = outcome {
            let t = now();
            match self.output.send(&bytes, t) {
                Ok(()) => {
                    self.perf.sent += 1;
                    self.perf.latency.record(now() - ev.t_mono_us);
                }
                Err(e) => {
                    self.perf.errors += 1;
                    self.log.push(format!("fault: control send failed: {e}"));
                    let offs = self.router.fault("control send failed");
                    self.pending_offs.extend(offs);
                }
            }
        }
        outcome
    }

    /// "Reconnect all": release sounding notes (sent while the output still
    /// works, otherwise held), then close both ports; they reopen at the next
    /// `maintain` as new connection generations.
    pub fn reconnect(&mut self, now: i64) {
        let offs = self.router.release_all();
        self.send_offs(offs, now);
        let (i, o) = (self.input_spec(), self.output.spec().to_string());
        self.input.rebind(&i);
        self.output.rebind(&o);
        self.log.push("reconnect requested".into());
    }

    fn input_spec(&self) -> String {
        self.input.spec().to_string()
    }

    /// Settings change of the input: release notes first, then rebind.
    pub fn rebind_input(&mut self, spec: &str, now: i64) {
        let offs = self.router.release_all();
        self.send_offs(offs, now);
        self.input.rebind(spec);
    }

    /// Leave the fault state after the operator handled the cause.
    pub fn reset_fault(&mut self) {
        self.router.reset();
    }

    /// Shutdown: stop input, send Note Offs for sounding notes.
    pub fn shutdown(&mut self, now: i64) {
        self.input.rebind("");
        let offs = self.router.release_all();
        self.send_offs(offs, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::{bounded, Receiver};
    use omoiiro_midi::backend::IngressStats;
    use omoiiro_midi::clock::{FakeTime, MonotonicClock};
    use omoiiro_midi::fake::{fake_backend, FakeBackend, FakePorts};
    use std::sync::Arc;

    const IN: &str = "LPX MIDI";
    const OUT: &str = "OMOI-IRO Control";

    fn setup(cap: usize) -> (FakeBackend, FakePorts, MonotonicClock, FakeTime, Receiver<RawIn>, ControlRoute) {
        let (mut be, ports) = fake_backend();
        let (clock, t) = MonotonicClock::fake();
        ports.add_input(IN);
        ports.add_output(OUT);
        let (tx, rx) = bounded(cap);
        let ing = Ingress::new(clock.clone(), tx, Arc::new(IngressStats::default()));
        let mut route = ControlRoute::new(IN, OUT, RouteRules::default(), ing);
        route.maintain(&mut be, 0);
        assert_eq!((route.input.state(), route.output.state()), (BindingState::Ready, BindingState::Ready));
        (be, ports, clock, t, rx, route)
    }

    fn drain(route: &mut ControlRoute, rx: &Receiver<RawIn>, clock: &MonotonicClock) {
        for ev in rx.try_iter() {
            route.handle(&ev, &|| clock.now_us());
        }
    }

    #[test]
    fn sysex_from_the_control_input_reaches_identification() {
        let (_be, ports, clock, _t, rx, mut route) = setup(64);
        let (tx, sysex) = crossbeam_channel::bounded(4);
        route.sysex_to = Some(tx);
        let reply = vec![0xF0, 0x00, 0x20, 0x29, 0x02, 0x0C, 0x0E, 0x01, 0xF7];
        ports.inject(IN, &reply);
        ports.inject(IN, &[0x90, 11, 100]);
        drain(&mut route, &rx, &clock);
        assert_eq!(sysex.try_iter().collect::<Vec<_>>(), vec![reply]);
        assert_eq!(ports.sent(OUT), vec![vec![0x90, 11, 100]], "SysEx is not forwarded to Live");
    }

    #[test]
    fn blackout_and_route_are_separate_effects() {
        let (_be, ports, clock, _t, rx, mut route) = setup(64);
        let ov = Overrides::default();
        apply_ui(evaluate(&mut route.router, Trigger::UiBlackout(true)).unwrap(), &ov);
        assert!(ov.blackout());
        ports.inject(IN, &[0x90, 11, 100]);
        drain(&mut route, &rx, &clock);
        assert_eq!(ports.sent(OUT), vec![vec![0x90, 11, 100]], "Blackout does not stop the route");
        // A seek is a re-evaluation only: nothing goes to Live.
        assert_eq!(evaluate(&mut route.router, Trigger::TransportJump), Some(Action::Reevaluate));
        assert_eq!(ports.sent(OUT).len(), 1);
        assert!(ov.blackout(), "routing does not touch the override");
    }

    #[test]
    fn queue_overflow_faults_and_releases_notes() {
        let (mut be, ports, clock, t, rx, mut route) = setup(4);
        ports.inject(IN, &[0x90, 11, 100]);
        drain(&mut route, &rx, &clock);
        // Burst larger than the queue while the pipeline is busy.
        for n in 20..30 {
            ports.inject(IN, &[0x90, n, 100]);
        }
        t.advance_us(1000);
        drain(&mut route, &rx, &clock);
        assert_eq!(route.router.fault_reason(), Some("input queue overflow"));
        let sent = ports.sent(OUT);
        assert_eq!(sent[0], vec![0x90, 11, 100]);
        assert_eq!(sent[1], vec![0x80, 11, 0], "the sounding note is released at the fault");
        assert_eq!(sent.len(), 2, "nothing is forwarded after the fault");
        route.maintain(&mut be, 2000);
        route.reset_fault();
        ports.inject(IN, &[0x90, 40, 1]);
        drain(&mut route, &rx, &clock);
        assert_eq!(ports.sent(OUT).last().unwrap(), &vec![0x90, 40, 1]);
    }

    #[test]
    fn disconnects_and_settings_change_release_notes() {
        let (mut be, ports, clock, t, rx, mut route) = setup(64);
        ports.inject(IN, &[0x90, 11, 100]);
        ports.inject(IN, &[0x90, 12, 100]);
        drain(&mut route, &rx, &clock);

        // Output (virtual bus) disappears: offs are held and sent on reconnect.
        ports.unplug(OUT);
        t.advance_us(1000);
        route.maintain(&mut be, clock.now_us());
        assert_eq!(route.pending_offs(), &[[0x80, 11, 0], [0x80, 12, 0]]);
        ports.replug(OUT, false, true);
        ports.clear_sent(OUT);
        t.advance_us(RETRY);
        route.maintain(&mut be, clock.now_us());
        assert_eq!(ports.sent(OUT), vec![vec![0x80, 11, 0], vec![0x80, 12, 0]]);
        assert_eq!(route.router.fault_reason(), None, "output-lost fault clears on reconnect");

        // Input device unplugged while a note sounds: released at once.
        ports.inject(IN, &[0x90, 13, 100]);
        drain(&mut route, &rx, &clock);
        ports.unplug(IN);
        route.maintain(&mut be, clock.now_us());
        assert_eq!(ports.sent(OUT).last().unwrap(), &vec![0x80, 13, 0]);
        // Replugged device: a new generation; queued input from the old one is stale.
        ports.replug(IN, true, false);
        t.advance_us(RETRY);
        route.maintain(&mut be, clock.now_us());
        assert_eq!(route.input.state(), BindingState::Ready);
        let old = RawIn { t_mono_us: clock.now_us(), backend_ts_us: 0, bytes: vec![0x90, 14, 1], tag: 0, generation: 1 };
        assert_eq!(route.handle(&old, &|| clock.now_us()), RouteOutcome::Ignored);
        assert_eq!(route.stale, 1);

        // Settings change while a note sounds.
        ports.inject(IN, &[0x90, 15, 100]);
        drain(&mut route, &rx, &clock);
        route.rebind_input(IN, clock.now_us());
        assert_eq!(ports.sent(OUT).last().unwrap(), &vec![0x80, 15, 0]);
        assert!(route.router.active_notes().is_empty());
    }

    const RETRY: i64 = crate::connections::RETRY_US;
}
