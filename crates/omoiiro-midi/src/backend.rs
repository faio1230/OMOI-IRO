//! `MidiBackend` contract (I01) and the ingress callback rules.
//!
//! * Ingress: the callback takes the session monotonic time, checks the
//!   size, copies the bytes and `try_send`s a `RawIn`. Nothing else runs on
//!   the backend thread; a full channel counts a drop, an oversize message
//!   counts `oversize`. Every `RawIn` carries the ingress `tag` (role chosen
//!   by the caller, e.g. mtc / clock / control) and the connection
//!   `generation`, so one channel can carry several ports and stale
//!   connections are recognized after a reconnect.
//! * Backend timestamps are kept for the record only; they are never
//!   compared across connections.
//! * `MidiBackend` lists ports and opens them by spec. Inputs accept an
//!   exact name, `#index` or (when `exact` is false) a unique substring.
//!   Outputs accept only an exact, unique name or `#index`. Every open gets
//!   a new connection generation.
//! * Implementations: `midir_backend::MidirBackend` (OS ports) and
//!   `fake::FakeBackend` (tests: inject input, capture output, unplug,
//!   failing sends, send cost on a fake clock).

use crate::clock::MonotonicClock;
use crossbeam_channel::Sender;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Largest message accepted by an ingress (bounded SysEx, docs/02).
pub const MAX_INGRESS_BYTES: usize = 1024;

/// Raw input as captured by an ingress callback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawIn {
    pub t_mono_us: i64,
    pub backend_ts_us: u64,
    pub bytes: Vec<u8>,
    /// Caller-chosen role tag of the ingress (0 when untagged).
    pub tag: u16,
    /// Connection generation of the port that produced it.
    pub generation: u64,
}

impl RawIn {
    /// Untagged input (tests and replays).
    pub fn new(t_mono_us: i64, bytes: Vec<u8>) -> RawIn {
        RawIn { t_mono_us, backend_ts_us: 0, bytes, tag: 0, generation: 0 }
    }
}

/// Counters shared between an ingress and its owner.
#[derive(Debug, Default)]
pub struct IngressStats {
    pub delivered: AtomicU64,
    pub dropped: AtomicU64,
    pub oversize: AtomicU64,
}

impl IngressStats {
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
    pub fn oversize(&self) -> u64 {
        self.oversize.load(Ordering::Relaxed)
    }
    pub fn delivered(&self) -> u64 {
        self.delivered.load(Ordering::Relaxed)
    }
}

/// Where a backend delivers input. Cheap to clone; one per opened port.
#[derive(Clone, Debug)]
pub struct Ingress {
    clock: MonotonicClock,
    tx: Sender<RawIn>,
    stats: Arc<IngressStats>,
    tag: u16,
    generation: u64,
}

impl Ingress {
    pub fn new(clock: MonotonicClock, tx: Sender<RawIn>, stats: Arc<IngressStats>) -> Ingress {
        Ingress { clock, tx, stats, tag: 0, generation: 0 }
    }

    pub fn tagged(mut self, tag: u16) -> Ingress {
        self.tag = tag;
        self
    }

    /// Set by the backend when it opens the port.
    pub fn with_generation(mut self, generation: u64) -> Ingress {
        self.generation = generation;
        self
    }

    pub fn stats(&self) -> &Arc<IngressStats> {
        &self.stats
    }

    /// The callback body: stamp, check size, copy, `try_send`. Never blocks.
    pub fn deliver(&self, backend_ts_us: u64, bytes: &[u8]) {
        let t_mono_us = self.clock.now_us();
        if bytes.len() > MAX_INGRESS_BYTES {
            self.stats.oversize.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let ev = RawIn { t_mono_us, backend_ts_us, bytes: bytes.to_vec(), tag: self.tag, generation: self.generation };
        if self.tx.try_send(ev).is_err() {
            self.stats.dropped.fetch_add(1, Ordering::Relaxed);
        } else {
            self.stats.delivered.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Callback that only stamps, copies and `try_send`s (untagged; kept for the
/// P0 commands that open midir ports directly).
pub fn make_callback(
    clock: MonotonicClock,
    tx: Sender<RawIn>,
    dropped: Arc<AtomicU64>,
) -> impl FnMut(u64, &[u8], &mut ()) + Send + 'static {
    move |backend_ts_us, bytes, _| {
        let t_mono_us = clock.now_us();
        if tx.try_send(RawIn { t_mono_us, backend_ts_us, bytes: bytes.to_vec(), tag: 0, generation: 0 }).is_err() {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Something that can send one MIDI message (a midir connection, or a test double).
pub trait MidiSend {
    fn send_msg(&mut self, bytes: &[u8]) -> Result<(), String>;
}

impl MidiSend for midir::MidiOutputConnection {
    fn send_msg(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.send(bytes).map_err(|e| e.to_string())
    }
}

impl<S: MidiSend + ?Sized> MidiSend for Box<S> {
    fn send_msg(&mut self, bytes: &[u8]) -> Result<(), String> {
        (**self).send_msg(bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortError {
    /// The spec matched no port (or it disappeared).
    NotFound(String),
    /// The spec matched several ports (substring or duplicate names).
    Ambiguous(String),
    /// The OS refused to open the port.
    Open(String),
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PortError::NotFound(s) => write!(f, "port not found: {s}"),
            PortError::Ambiguous(s) => write!(f, "ambiguous port: {s}"),
            PortError::Open(s) => write!(f, "open failed: {s}"),
        }
    }
}

impl std::error::Error for PortError {}

/// Map a resolver message to `NotFound` / `Ambiguous`.
pub(crate) fn resolve_error(msg: String) -> PortError {
    if msg.contains("ambiguous") || msg.contains("select one with #index") {
        PortError::Ambiguous(msg)
    } else {
        PortError::NotFound(msg)
    }
}

/// An open input. Input stops when this is dropped.
pub struct InputConn {
    pub name: String,
    pub generation: u64,
    _inner: Box<dyn Send>,
}

impl InputConn {
    pub fn new(name: String, generation: u64, inner: Box<dyn Send>) -> InputConn {
        InputConn { name, generation, _inner: inner }
    }
}

impl fmt::Debug for InputConn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InputConn({:?}, gen {})", self.name, self.generation)
    }
}

/// An open output.
pub struct OutputConn {
    pub name: String,
    pub generation: u64,
    inner: Box<dyn MidiSend + Send>,
}

impl OutputConn {
    pub fn new(name: String, generation: u64, inner: Box<dyn MidiSend + Send>) -> OutputConn {
        OutputConn { name, generation, inner }
    }
}

impl MidiSend for OutputConn {
    fn send_msg(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.inner.send_msg(bytes)
    }
}

impl fmt::Debug for OutputConn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OutputConn({:?}, gen {})", self.name, self.generation)
    }
}

/// OS-independent MIDI port access (docs/03 "MidiBackend").
pub trait MidiBackend: Send {
    fn backend_name(&self) -> &'static str;
    fn list_inputs(&self) -> Result<Vec<String>, PortError>;
    fn list_outputs(&self) -> Result<Vec<String>, PortError>;
    /// Open an input. `exact` restricts the spec to an exact unique name or `#index`.
    fn open_input(&mut self, spec: &str, exact: bool, ingress: Ingress) -> Result<InputConn, PortError>;
    /// Open an output (exact unique name or `#index` only).
    fn open_output(&mut self, spec: &str) -> Result<OutputConn, PortError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::bounded;

    #[test]
    fn ingress_tags_counts_drops_and_oversize() {
        let (clock, t) = MonotonicClock::fake();
        let (tx, rx) = bounded(2);
        let stats = Arc::new(IngressStats::default());
        let ing = Ingress::new(clock, tx, stats.clone()).tagged(7).with_generation(3);
        t.advance_us(100);
        ing.deliver(5, &[0xF8]);
        ing.deliver(6, &[0xFA]);
        ing.deliver(7, &[0xFC]); // queue full
        ing.deliver(8, &vec![0xF0; MAX_INGRESS_BYTES + 1]);
        let got: Vec<RawIn> = rx.try_iter().collect();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], RawIn { t_mono_us: 100, backend_ts_us: 5, bytes: vec![0xF8], tag: 7, generation: 3 });
        assert_eq!((stats.delivered(), stats.dropped(), stats.oversize()), (2, 1, 1));
    }
}
