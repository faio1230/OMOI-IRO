//! Fake `MidiBackend` for Runtime tests (docs/03 "テストの配置").
//!
//! `FakeBackend` is what the code under test owns; `FakePorts` is the test's
//! handle to the same state: add / unplug ports, inject input (stamped by the
//! ingress's own clock, normally a fake clock), read what was sent, make
//! sends fail, and charge a send cost on a `FakeTime` (load tests).

use crate::backend::{resolve_error, Ingress, InputConn, MidiBackend, MidiSend, OutputConn, PortError};
use crate::clock::FakeTime;
use crate::ports::{resolve, resolve_exact};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    inputs: Vec<String>,
    outputs: Vec<String>,
    /// Open inputs: (port name, ingress, alive flag dropped with the `InputConn`).
    open_inputs: Vec<(String, Ingress, Arc<AtomicBool>)>,
    sent: HashMap<String, Vec<Vec<u8>>>,
    failing: HashSet<String>,
    send_cost: Option<(FakeTime, i64)>,
    next_generation: u64,
    opens: u64,
}

/// The backend half (owned by the code under test).
pub struct FakeBackend {
    state: Arc<Mutex<State>>,
}

/// The test half.
#[derive(Clone)]
pub struct FakePorts {
    state: Arc<Mutex<State>>,
}

pub fn fake_backend() -> (FakeBackend, FakePorts) {
    let state = Arc::new(Mutex::new(State { next_generation: 1, ..State::default() }));
    (FakeBackend { state: state.clone() }, FakePorts { state })
}

impl FakePorts {
    pub fn add_input(&self, name: &str) {
        self.state.lock().unwrap().inputs.push(name.to_string());
    }

    pub fn add_output(&self, name: &str) {
        self.state.lock().unwrap().outputs.push(name.to_string());
    }

    /// Unplug every port with this name: it leaves the lists, open inputs
    /// stop delivering and sends to open outputs fail.
    pub fn unplug(&self, name: &str) {
        let mut s = self.state.lock().unwrap();
        s.inputs.retain(|n| n != name);
        s.outputs.retain(|n| n != name);
        s.open_inputs.retain(|(n, _, _)| n != name);
        s.failing.insert(name.to_string());
    }

    /// Plug a port back in (input and/or output). Old connections stay dead.
    pub fn replug(&self, name: &str, input: bool, output: bool) {
        let mut s = self.state.lock().unwrap();
        if input {
            s.inputs.push(name.to_string());
        }
        if output {
            s.outputs.push(name.to_string());
        }
    }

    /// Deliver `bytes` to every open input on `name` (as the OS callback would).
    pub fn inject(&self, name: &str, bytes: &[u8]) -> usize {
        let targets: Vec<Ingress> = {
            let s = self.state.lock().unwrap();
            s.open_inputs.iter().filter(|(n, _, alive)| n == name && alive.load(Ordering::SeqCst)).map(|(_, i, _)| i.clone()).collect()
        };
        for t in &targets {
            t.deliver(0, bytes);
        }
        targets.len()
    }

    /// Messages successfully sent to output `name`, in order.
    pub fn sent(&self, name: &str) -> Vec<Vec<u8>> {
        self.state.lock().unwrap().sent.get(name).cloned().unwrap_or_default()
    }

    pub fn clear_sent(&self, name: &str) {
        self.state.lock().unwrap().sent.remove(name);
    }

    /// Make sends to `name` fail (without unplugging it).
    pub fn set_failing(&self, name: &str, failing: bool) {
        let mut s = self.state.lock().unwrap();
        if failing {
            s.failing.insert(name.to_string());
        } else {
            s.failing.remove(name);
        }
    }

    /// Every successful send advances `time` by `cost_us` (simulated blocking send).
    pub fn set_send_cost(&self, time: FakeTime, cost_us: i64) {
        self.state.lock().unwrap().send_cost = Some((time, cost_us));
    }

    /// Number of successful opens so far.
    pub fn opens(&self) -> u64 {
        self.state.lock().unwrap().opens
    }

    pub fn open_input_count(&self, name: &str) -> usize {
        self.state.lock().unwrap().open_inputs.iter().filter(|(n, _, a)| n == name && a.load(Ordering::SeqCst)).count()
    }
}

struct InputGuard(Arc<AtomicBool>);

impl Drop for InputGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

struct FakeOut {
    name: String,
    state: Arc<Mutex<State>>,
    /// Set when the port was unplugged after this connection opened.
    dead: bool,
}

impl MidiSend for FakeOut {
    fn send_msg(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        if self.dead || s.failing.contains(&self.name) || !s.outputs.contains(&self.name) {
            // An unplugged port stays dead for this connection even if a port
            // with the same name comes back (a new open is needed).
            self.dead |= !s.outputs.contains(&self.name);
            return Err(format!("fake send to {:?} failed", self.name));
        }
        s.sent.entry(self.name.clone()).or_default().push(bytes.to_vec());
        if let Some((t, cost)) = &s.send_cost {
            t.advance_us(*cost);
        }
        Ok(())
    }
}

impl MidiBackend for FakeBackend {
    fn backend_name(&self) -> &'static str {
        "fake"
    }

    fn list_inputs(&self) -> Result<Vec<String>, PortError> {
        Ok(self.state.lock().unwrap().inputs.clone())
    }

    fn list_outputs(&self) -> Result<Vec<String>, PortError> {
        Ok(self.state.lock().unwrap().outputs.clone())
    }

    fn open_input(&mut self, spec: &str, exact: bool, ingress: Ingress) -> Result<InputConn, PortError> {
        let mut s = self.state.lock().unwrap();
        let i = if exact { resolve_exact(&s.inputs, spec) } else { resolve(&s.inputs, spec) }.map_err(resolve_error)?;
        let name = s.inputs[i].clone();
        let generation = s.next_generation;
        s.next_generation += 1;
        s.opens += 1;
        let alive = Arc::new(AtomicBool::new(true));
        s.open_inputs.push((name.clone(), ingress.with_generation(generation), alive.clone()));
        Ok(InputConn::new(name, generation, Box::new(InputGuard(alive))))
    }

    fn open_output(&mut self, spec: &str) -> Result<OutputConn, PortError> {
        let mut s = self.state.lock().unwrap();
        let i = resolve_exact(&s.outputs, spec).map_err(resolve_error)?;
        let name = s.outputs[i].clone();
        let generation = s.next_generation;
        s.next_generation += 1;
        s.opens += 1;
        s.failing.remove(&name);
        let out = FakeOut { name: name.clone(), state: self.state.clone(), dead: false };
        Ok(OutputConn::new(name, generation, Box::new(out)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::IngressStats;
    use crate::clock::MonotonicClock;
    use crossbeam_channel::bounded;

    #[test]
    fn inject_send_unplug_and_generations() {
        let (mut be, ports) = fake_backend();
        let (clock, t) = MonotonicClock::fake();
        ports.add_input("LPX MIDI");
        ports.add_output("OMOI-IRO Control");
        let (tx, rx) = bounded(16);
        let ing = Ingress::new(clock, tx, Arc::new(IngressStats::default())).tagged(2);
        let conn = be.open_input("lpx", false, ing).unwrap();
        let mut out = be.open_output("OMOI-IRO Control").unwrap();
        assert_eq!((conn.generation, out.generation), (1, 2));
        assert!(be.open_output("OMOI-IRO").is_err(), "outputs need the exact name");

        t.advance_us(10);
        assert_eq!(ports.inject("LPX MIDI", &[0x90, 11, 100]), 1);
        let ev = rx.try_recv().unwrap();
        assert_eq!((ev.t_mono_us, ev.tag, ev.generation), (10, 2, 1));
        out.send_msg(&[0x90, 11, 100]).unwrap();
        assert_eq!(ports.sent("OMOI-IRO Control"), vec![vec![0x90, 11, 100]]);

        ports.unplug("OMOI-IRO Control");
        assert!(out.send_msg(&[0x80, 11, 0]).is_err());
        ports.replug("OMOI-IRO Control", false, true);
        assert!(out.send_msg(&[0x80, 11, 0]).is_err(), "old connection stays dead");
        let mut out2 = be.open_output("OMOI-IRO Control").unwrap();
        assert_eq!(out2.generation, 3);
        out2.send_msg(&[0x80, 11, 0]).unwrap();

        drop(conn);
        assert_eq!(ports.inject("LPX MIDI", &[0x90, 12, 1]), 0);
    }
}
