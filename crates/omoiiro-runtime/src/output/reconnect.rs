//! An output port that reconnects by itself (U06): used for the timecode
//! (MTC) output, whose sender thread only knows `MidiSend`.
//!
//! * At most every `MAINTAIN_US` (from the sending thread): drop the
//!   connection if the port left the list, reopen it if it is due, and honor
//!   a "reconnect all" request. A new connection bumps the MTC epoch so the
//!   generator starts again with a Full Frame and receivers relock.
//! * While not connected, `send_msg` fails at once without touching the
//!   backend (no blocking on the sender's schedule).
//! * The binding's state, generation and last error are published to the
//!   follow status for the UI.

use crate::connections::{BindingEvent, BindingState, OutputBinding};
use crate::follow::{FollowShared, SharedBackend, MAINTAIN_US};
use omoiiro_midi::backend::MidiSend;
use omoiiro_midi::clock::MonotonicClock;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct ReconnectingOutput {
    binding: OutputBinding,
    backend: SharedBackend,
    clock: MonotonicClock,
    shared: Arc<FollowShared>,
    next_check: i64,
    seen_reconnect: u64,
    sent: u64,
    errors: u64,
}

impl ReconnectingOutput {
    pub fn new(binding: OutputBinding, backend: SharedBackend, clock: MonotonicClock, shared: Arc<FollowShared>) -> ReconnectingOutput {
        let seen_reconnect = shared.reconnect_request.load(Ordering::SeqCst);
        let mut o = ReconnectingOutput { binding, backend, clock, shared, next_check: i64::MIN, seen_reconnect, sent: 0, errors: 0 };
        o.maintain(o.clock.now_us());
        o
    }

    pub fn state(&self) -> BindingState {
        self.binding.state()
    }

    pub fn port_name(&self) -> Option<&str> {
        self.binding.port_name()
    }

    fn maintain(&mut self, now: i64) {
        let req = self.shared.reconnect_request.load(Ordering::SeqCst);
        if req != self.seen_reconnect {
            self.seen_reconnect = req;
            let spec = self.binding.spec().to_string();
            self.binding.rebind(&spec);
            self.next_check = i64::MIN;
        }
        if now < self.next_check {
            return;
        }
        self.next_check = now + MAINTAIN_US;
        {
            let mut be = self.backend.lock().unwrap();
            self.binding.check_presence(be.as_ref(), now);
            if let Some(BindingEvent::Connected { .. }) = self.binding.ensure(be.as_mut(), now) {
                self.shared.mtc_epoch_bump.fetch_add(1, Ordering::SeqCst);
            }
        }
        self.publish();
    }

    fn publish(&self) {
        let mut st = self.shared.status.lock().unwrap();
        st.timecode_out = Some(self.binding.state().name());
        st.timecode_generation = self.binding.generation();
        st.timecode_error = self.binding.last_error().map(str::to_string);
        st.diag.mtc_sent = self.sent;
        st.diag.mtc_send_errors = self.errors;
    }
}

impl MidiSend for ReconnectingOutput {
    fn send_msg(&mut self, bytes: &[u8]) -> Result<(), String> {
        let now = self.clock.now_us();
        self.maintain(now);
        if self.binding.state() != BindingState::Ready {
            self.errors += 1;
            return Err("timecode output not connected".into());
        }
        match self.binding.send(bytes, now) {
            Ok(()) => {
                self.sent += 1;
                Ok(())
            }
            Err(e) => {
                self.errors += 1;
                self.next_check = now; // look for the port again right away
                self.publish();
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::transport::{Transport, TransportConfig};
    use omoiiro_core::time::{FrameRate, Timecode};
    use omoiiro_midi::backend::MidiBackend;
    use omoiiro_midi::fake::fake_backend;
    use std::sync::Mutex;

    #[test]
    fn reconnects_after_unplug_and_on_request() {
        let (clock, time) = MonotonicClock::fake();
        let (be, ports) = fake_backend();
        ports.add_output("OMOI-IRO Timecode");
        let backend: SharedBackend = Arc::new(Mutex::new(Box::new(be) as Box<dyn MidiBackend>));
        let origin = Timecode::parse("01:00:00:00", FrameRate::Fps30).unwrap();
        let shared = FollowShared::new(Transport::new(TransportConfig::defaults(origin, 0, None)));
        let mut out = ReconnectingOutput::new(OutputBinding::new("timecode_out", "OMOI-IRO Timecode"), backend, clock, shared.clone());
        assert_eq!(out.state(), BindingState::Ready);
        assert_eq!(shared.mtc_epoch_bump.load(Ordering::SeqCst), 1);
        out.send_msg(&[0xF1, 0x00]).unwrap();

        ports.unplug("OMOI-IRO Timecode");
        assert!(out.send_msg(&[0xF1, 0x10]).is_err());
        assert_eq!(shared.status.lock().unwrap().timecode_out, Some("recovering"));
        ports.replug("OMOI-IRO Timecode", false, true);
        time.advance_us(MAINTAIN_US);
        out.send_msg(&[0xF1, 0x20]).unwrap();
        assert_eq!(shared.mtc_epoch_bump.load(Ordering::SeqCst), 2, "a reconnect restarts MTC with a Full Frame");

        // "Reconnect all": a new generation without unplugging.
        let gen = shared.status.lock().unwrap().timecode_generation;
        shared.reconnect_request.fetch_add(1, Ordering::SeqCst);
        out.send_msg(&[0xF1, 0x30]).unwrap();
        out.maintain(time.advance_us(MAINTAIN_US));
        assert_ne!(shared.status.lock().unwrap().timecode_generation, gen);
        assert_eq!(ports.sent("OMOI-IRO Timecode").len(), 3);
        let st = shared.status.lock().unwrap();
        assert_eq!((st.diag.mtc_sent, st.diag.mtc_send_errors), (3, 1));
    }
}
