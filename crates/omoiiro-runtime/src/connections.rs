//! Port bindings: connection management and recovery (I06).
//!
//! A binding is one role (control in / control out / LED out / clock in)
//! bound to a port spec. States follow docs/02 "MIDIの接続のライフサイクル"
//! as far as the M4 headless runtime needs them:
//!
//! * `Disconnected` -> (open succeeds) `Ready`; open fails because the port
//!   is missing -> `Recovering` and retried every `RETRY_US`.
//! * `Ready` -> `Recovering` when a send fails or the port leaves the port
//!   list (`check_presence`; midir does not notify removals). The
//!   connection is dropped; a later open is a new connection generation.
//! * Two ports with the same name are never chosen silently: the open fails
//!   with `Ambiguous` and the binding goes to `Fault` until the spec changes
//!   (`rebind`, e.g. to `#index`). Retries do not resolve a fault.
//! * Start order does not matter: a binding whose port appears later
//!   connects on a later `ensure`.
//!
//! Device identification (Device Inquiry) and mode handling for a real
//! Launchpad X belong to M6.

use omoiiro_midi::backend::{Ingress, InputConn, MidiBackend, MidiSend, OutputConn, PortError};

/// Retry interval while a port is missing.
pub const RETRY_US: i64 = 500_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingState {
    Disconnected,
    Ready,
    Recovering,
    Fault,
}

impl BindingState {
    pub fn name(self) -> &'static str {
        match self {
            BindingState::Disconnected => "disconnected",
            BindingState::Ready => "ready",
            BindingState::Recovering => "recovering",
            BindingState::Fault => "fault",
        }
    }
}

/// What changed during `ensure` / `check_presence`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingEvent {
    Connected { generation: u64, name: String },
    Lost { reason: String },
    Fault { reason: String },
}

#[derive(Debug)]
struct Common {
    role: &'static str,
    spec: String,
    state: BindingState,
    retry_at: i64,
    last_error: Option<String>,
    connects: u64,
    losses: u64,
}

impl Common {
    fn new(role: &'static str, spec: &str) -> Common {
        Common { role, spec: spec.to_string(), state: BindingState::Disconnected, retry_at: i64::MIN, last_error: None, connects: 0, losses: 0 }
    }

    fn may_try(&self, now: i64) -> bool {
        matches!(self.state, BindingState::Disconnected | BindingState::Recovering) && now >= self.retry_at
    }

    fn opened(&mut self, generation: u64, name: &str) -> BindingEvent {
        self.state = BindingState::Ready;
        self.last_error = None;
        self.connects += 1;
        BindingEvent::Connected { generation, name: name.to_string() }
    }

    fn open_failed(&mut self, e: PortError, now: i64) -> Option<BindingEvent> {
        let msg = format!("{}: {e}", self.role);
        let first = self.last_error.as_deref() != Some(msg.as_str());
        self.last_error = Some(msg.clone());
        if matches!(e, PortError::Ambiguous(_)) {
            self.state = BindingState::Fault;
            return Some(BindingEvent::Fault { reason: msg });
        }
        self.state = BindingState::Recovering;
        self.retry_at = now + RETRY_US;
        first.then_some(BindingEvent::Lost { reason: msg })
    }

    fn lost(&mut self, reason: String, now: i64) -> BindingEvent {
        self.state = BindingState::Recovering;
        self.retry_at = now;
        self.losses += 1;
        self.last_error = Some(reason.clone());
        BindingEvent::Lost { reason }
    }
}

/// Output role (control bus, LED port).
#[derive(Debug)]
pub struct OutputBinding {
    c: Common,
    conn: Option<OutputConn>,
}

impl OutputBinding {
    pub fn new(role: &'static str, spec: &str) -> OutputBinding {
        OutputBinding { c: Common::new(role, spec), conn: None }
    }

    pub fn role(&self) -> &'static str {
        self.c.role
    }
    pub fn spec(&self) -> &str {
        &self.c.spec
    }
    pub fn state(&self) -> BindingState {
        self.c.state
    }
    pub fn last_error(&self) -> Option<&str> {
        self.c.last_error.as_deref()
    }
    pub fn connects(&self) -> u64 {
        self.c.connects
    }
    pub fn losses(&self) -> u64 {
        self.c.losses
    }
    pub fn generation(&self) -> Option<u64> {
        self.conn.as_ref().map(|c| c.generation)
    }
    pub fn port_name(&self) -> Option<&str> {
        self.conn.as_ref().map(|c| c.name.as_str())
    }

    /// Open the port if it is due; returns what changed.
    pub fn ensure(&mut self, backend: &mut dyn MidiBackend, now: i64) -> Option<BindingEvent> {
        if self.conn.is_some() || !self.c.may_try(now) {
            return None;
        }
        match backend.open_output(&self.c.spec) {
            Ok(conn) => {
                let ev = self.c.opened(conn.generation, &conn.name);
                self.conn = Some(conn);
                Some(ev)
            }
            Err(e) => self.c.open_failed(e, now),
        }
    }

    /// Drop the connection if its port left the list.
    pub fn check_presence(&mut self, backend: &dyn MidiBackend, now: i64) -> Option<BindingEvent> {
        let name = self.conn.as_ref()?.name.clone();
        let present = backend.list_outputs().map(|l| l.contains(&name)).unwrap_or(true);
        if present {
            return None;
        }
        self.conn = None;
        Some(self.c.lost(format!("{}: port {name:?} disappeared", self.c.role), now))
    }

    /// Send; a failure drops the connection (`Recovering`) and is returned.
    pub fn send(&mut self, bytes: &[u8], now: i64) -> Result<(), String> {
        let conn = self.conn.as_mut().ok_or_else(|| format!("{} not connected", self.c.role))?;
        conn.send_msg(bytes).map_err(|e| {
            self.conn = None;
            self.c.lost(format!("{}: send failed: {e}", self.c.role), now);
            e
        })
    }

    /// Borrow the connection as a sender (LED writer). A failed send through
    /// it must be reported with `mark_lost`.
    pub fn sender(&mut self) -> Option<&mut OutputConn> {
        self.conn.as_mut()
    }

    pub fn mark_lost(&mut self, reason: &str, now: i64) -> BindingEvent {
        self.conn = None;
        self.c.lost(format!("{}: {reason}", self.c.role), now)
    }

    /// Settings change: close and bind to a new spec (connects on `ensure`).
    pub fn rebind(&mut self, spec: &str) {
        self.conn = None;
        self.c.spec = spec.to_string();
        self.c.state = BindingState::Disconnected;
        self.c.retry_at = i64::MIN;
        self.c.last_error = None;
    }
}

/// Input role (control in, clock in).
#[derive(Debug)]
pub struct InputBinding {
    c: Common,
    exact: bool,
    conn: Option<InputConn>,
}

impl InputBinding {
    pub fn new(role: &'static str, spec: &str, exact: bool) -> InputBinding {
        InputBinding { c: Common::new(role, spec), exact, conn: None }
    }

    pub fn role(&self) -> &'static str {
        self.c.role
    }
    pub fn spec(&self) -> &str {
        &self.c.spec
    }
    pub fn state(&self) -> BindingState {
        self.c.state
    }
    pub fn last_error(&self) -> Option<&str> {
        self.c.last_error.as_deref()
    }
    pub fn generation(&self) -> Option<u64> {
        self.conn.as_ref().map(|c| c.generation)
    }
    pub fn port_name(&self) -> Option<&str> {
        self.conn.as_ref().map(|c| c.name.as_str())
    }
    pub fn losses(&self) -> u64 {
        self.c.losses
    }

    pub fn ensure(&mut self, backend: &mut dyn MidiBackend, ingress: &Ingress, now: i64) -> Option<BindingEvent> {
        if self.conn.is_some() || !self.c.may_try(now) {
            return None;
        }
        match backend.open_input(&self.c.spec, self.exact, ingress.clone()) {
            Ok(conn) => {
                let ev = self.c.opened(conn.generation, &conn.name);
                self.conn = Some(conn);
                Some(ev)
            }
            Err(e) => self.c.open_failed(e, now),
        }
    }

    pub fn check_presence(&mut self, backend: &dyn MidiBackend, now: i64) -> Option<BindingEvent> {
        let name = self.conn.as_ref()?.name.clone();
        let present = backend.list_inputs().map(|l| l.contains(&name)).unwrap_or(true);
        if present {
            return None;
        }
        self.conn = None;
        Some(self.c.lost(format!("{}: port {name:?} disappeared", self.c.role), now))
    }

    pub fn rebind(&mut self, spec: &str) {
        self.conn = None;
        self.c.spec = spec.to_string();
        self.c.state = BindingState::Disconnected;
        self.c.retry_at = i64::MIN;
        self.c.last_error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omoiiro_midi::fake::fake_backend;

    #[test]
    fn late_port_reconnect_and_duplicate_names() {
        let (mut be, ports) = fake_backend();
        let mut out = OutputBinding::new("control_out", "OMOI-IRO Control");
        // Start order: the app starts before the virtual bus exists.
        assert!(matches!(out.ensure(&mut be, 0), Some(BindingEvent::Lost { .. })));
        assert_eq!(out.state(), BindingState::Recovering);
        ports.add_output("OMOI-IRO Control");
        assert_eq!(out.ensure(&mut be, 100_000), None, "retry waits RETRY_US");
        assert!(matches!(out.ensure(&mut be, RETRY_US), Some(BindingEvent::Connected { generation: 1, .. })));
        out.send(&[0x90, 1, 1], RETRY_US).unwrap();

        ports.unplug("OMOI-IRO Control");
        assert!(matches!(out.check_presence(&be, 1_000_000), Some(BindingEvent::Lost { .. })));
        assert!(out.send(&[0x80, 1, 0], 1_000_000).is_err());
        ports.replug("OMOI-IRO Control", false, true);
        assert!(matches!(out.ensure(&mut be, 1_000_000), Some(BindingEvent::Connected { generation: 2, .. })));
        assert_eq!(out.losses(), 1);

        // Send failure without a list change also recovers.
        ports.set_failing("OMOI-IRO Control", true);
        assert!(out.send(&[0x80, 1, 0], 2_000_000).is_err());
        assert_eq!(out.state(), BindingState::Recovering);
        assert!(matches!(out.ensure(&mut be, 2_000_000), Some(BindingEvent::Connected { generation: 3, .. })));

        // Two identical devices: fault, no retry, until rebound by index.
        ports.add_output("LPX MIDI");
        ports.add_output("LPX MIDI");
        let mut led = OutputBinding::new("led_out", "LPX MIDI");
        assert!(matches!(led.ensure(&mut be, 0), Some(BindingEvent::Fault { .. })));
        assert_eq!(led.ensure(&mut be, 10 * RETRY_US), None);
        led.rebind("#2");
        assert!(matches!(led.ensure(&mut be, 10 * RETRY_US), Some(BindingEvent::Connected { .. })));
        assert_eq!(led.port_name(), Some("LPX MIDI"));
    }
}
