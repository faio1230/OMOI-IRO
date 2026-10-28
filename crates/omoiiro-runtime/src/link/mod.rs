//! Ableton Link observation (runbook B3, docs/08-ableton-link.md).
//!
//! Observation only: this module never sets tempo, beat or transport and
//! never commits a session state. `LinkTracker` is pure and turns callback
//! notifications and periodic samples into events and a display state; the
//! SDK glue (`observer`, feature `link`) only stamps the session monotonic
//! time and `try_send`s from Link's callback threads.
//!
//! Transport rules:
//! * With no peers the transport is `unknown`.
//! * `is_playing = true` with peers is `playing`.
//! * `is_playing = false` is `stopped` only after a change was observed
//!   while peers were present. The initial `false` after joining is not a
//!   Stop, so it stays `unknown`. Losing all peers returns to `unknown`.
//! * Peer-count changes and `is_playing` changes are separate events.
//! * Tempo is shown only while peers are present. Alone, the SDK reports its
//!   own initial tempo (`AblLink::new`), which says nothing about Live, so it
//!   is recorded as `local, no peers` and displayed as `--`.

use serde_json::{json, Value};

/// Quantum used for phase (one 4/4 bar).
pub const QUANTUM: f64 = 4.0;

/// Notification from a Link callback, stamped with the session monotonic clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LinkRaw {
    Peers(u64),
    Tempo(f64),
    StartStop(bool),
}

/// Periodic capture of the app session state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkSample {
    /// Link's own clock. Recorded, never compared with `t_mono_us`.
    pub link_clock_us: i64,
    pub peers: u64,
    pub tempo: f64,
    pub beat: f64,
    pub phase: f64,
    pub is_playing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkTransport {
    Unknown,
    Playing,
    Stopped,
}

impl LinkTransport {
    pub fn name(self) -> &'static str {
        match self {
            LinkTransport::Unknown => "unknown",
            LinkTransport::Playing => "playing",
            LinkTransport::Stopped => "stopped",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkStats {
    pub peer_changes: u64,
    pub tempo_changes: u64,
    pub is_playing_changes: u64,
    pub samples: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkSnapshot {
    pub peers: u64,
    pub tempo: Option<f64>,
    pub beat: Option<f64>,
    pub phase: Option<f64>,
    pub is_playing: bool,
    pub transport: LinkTransport,
    pub stats: LinkStats,
}

#[derive(Debug)]
pub struct LinkTracker {
    peers: u64,
    tempo: Option<f64>,
    is_playing: bool,
    /// An `is_playing` change was seen while peers were present.
    transport_confirmed: bool,
    last: Option<LinkSample>,
    stats: LinkStats,
}

impl Default for LinkTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl LinkTracker {
    pub fn new() -> Self {
        LinkTracker { peers: 0, tempo: None, is_playing: false, transport_confirmed: false, last: None, stats: LinkStats::default() }
    }

    fn set_peers(&mut self, n: u64, out: &mut Vec<String>) {
        if n == self.peers {
            return;
        }
        self.stats.peer_changes += 1;
        out.push(format!("peers {} -> {n}", self.peers));
        self.peers = n;
        if n == 0 {
            self.transport_confirmed = false;
            out.push("transport unknown (no peers)".into());
        }
    }

    fn set_tempo(&mut self, bpm: f64, out: &mut Vec<String>) {
        if self.tempo.is_some_and(|t| (t - bpm).abs() < 0.005) {
            return;
        }
        self.stats.tempo_changes += 1;
        let alone = if self.peers == 0 { " (local, no peers)" } else { "" };
        match self.tempo {
            Some(t) => out.push(format!("tempo {t:.2} -> {bpm:.2}{alone}")),
            None => out.push(format!("tempo {bpm:.2}{alone}")),
        }
        self.tempo = Some(bpm);
    }

    fn set_is_playing(&mut self, playing: bool, out: &mut Vec<String>) {
        if playing == self.is_playing {
            return;
        }
        self.stats.is_playing_changes += 1;
        self.is_playing = playing;
        if self.peers > 0 {
            self.transport_confirmed = true;
            out.push(format!("is_playing -> {playing} (peers {})", self.peers));
        } else {
            out.push(format!("is_playing -> {playing} (no peers; transport stays unknown)"));
        }
    }

    /// Apply a callback notification; returns event lines.
    pub fn feed(&mut self, raw: LinkRaw) -> Vec<String> {
        let mut out = Vec::new();
        match raw {
            LinkRaw::Peers(n) => self.set_peers(n, &mut out),
            LinkRaw::Tempo(bpm) => self.set_tempo(bpm, &mut out),
            LinkRaw::StartStop(p) => self.set_is_playing(p, &mut out),
        }
        out
    }

    /// Apply a periodic sample. Changes the callbacks missed become events too.
    pub fn sample(&mut self, s: LinkSample) -> Vec<String> {
        let mut out = Vec::new();
        self.stats.samples += 1;
        self.set_peers(s.peers, &mut out);
        self.set_tempo(s.tempo, &mut out);
        self.set_is_playing(s.is_playing, &mut out);
        self.last = Some(s);
        out
    }

    pub fn transport(&self) -> LinkTransport {
        if self.peers == 0 {
            LinkTransport::Unknown
        } else if self.is_playing {
            LinkTransport::Playing
        } else if self.transport_confirmed {
            LinkTransport::Stopped
        } else {
            LinkTransport::Unknown
        }
    }

    pub fn snapshot(&self) -> LinkSnapshot {
        LinkSnapshot {
            peers: self.peers,
            // The SDK's own tempo while alone is not session information.
            tempo: if self.peers > 0 { self.tempo } else { None },
            beat: self.last.map(|s| s.beat),
            phase: self.last.map(|s| s.phase),
            is_playing: self.is_playing,
            transport: self.transport(),
            stats: self.stats,
        }
    }
}

pub fn link_line(s: &LinkSnapshot) -> String {
    let tempo = s.tempo.map(|t| format!("{t:6.2}")).unwrap_or_else(|| "  --  ".into());
    let beat = s.beat.map(|b| format!("{b:.2}")).unwrap_or_else(|| "--".into());
    let phase = s.phase.map(|p| format!("{p:.2}")).unwrap_or_else(|| "--".into());
    format!(
        "LINK peers={} {tempo}bpm {:<8} beat={beat} phase={phase}/{QUANTUM} is_playing={}",
        s.peers,
        s.transport.name(),
        s.is_playing
    )
}

pub fn link_json(s: &LinkSnapshot) -> Value {
    json!({
        "peers": s.peers,
        "tempo": s.tempo,
        "beat": s.beat,
        "phase": s.phase,
        "quantum": QUANTUM,
        "is_playing": s.is_playing,
        "transport": s.transport.name(),
        "stats": {
            "peer_changes": s.stats.peer_changes,
            "tempo_changes": s.stats.tempo_changes,
            "is_playing_changes": s.stats.is_playing_changes,
            "samples": s.stats.samples,
        }
    })
}

pub fn sample_json(s: &LinkSample) -> Value {
    json!({
        "link_clock_us": s.link_clock_us,
        "peers": s.peers,
        "tempo": s.tempo,
        "beat": s.beat,
        "phase": s.phase,
        "quantum": QUANTUM,
        "is_playing": s.is_playing,
    })
}

/// SDK glue. Built only with `--features link` (needs cmake and libclang).
#[cfg(feature = "link")]
pub mod observer {
    use super::{LinkRaw, LinkSample, QUANTUM};
    use crate::clock::MonotonicClock;
    use crossbeam_channel::Sender;
    use rusty_link::{AblLink, SessionState};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    /// Tempo the SDK starts with while alone. It is never committed by this
    /// tool, but a session that starts here first is still a session at this
    /// tempo (see README: enable Link in Live first).
    pub const INITIAL_BPM: f64 = 120.0;

    /// Callback target. rusty_link 0.4.9 `set_*_callback` hands the C side a
    /// pointer to its by-value `closure` argument, which is gone when the call
    /// returns. A closure that captures anything is therefore read from freed
    /// stack memory on the Link thread (this crashed with a misaligned
    /// pointer in crossbeam). The closures passed below capture nothing
    /// (zero-sized, so the dangling pointer is never read) and reach their
    /// state through this process-wide slot instead.
    struct Target {
        clock: MonotonicClock,
        tx: Sender<(i64, LinkRaw)>,
        dropped: Arc<AtomicU64>,
    }

    static TARGET: Mutex<Option<Target>> = Mutex::new(None);

    fn deliver(raw: LinkRaw) {
        let Ok(guard) = TARGET.lock() else { return };
        if let Some(t) = guard.as_ref() {
            if t.tx.try_send((t.clock.now_us(), raw)).is_err() {
                t.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub struct LinkObserver {
        link: AblLink,
        state: SessionState,
    }

    impl LinkObserver {
        /// Join the Link session. `start_stop_sync` only controls whether this
        /// peer receives `is_playing`; it sends nothing. One observer per process.
        pub fn start(
            clock: &MonotonicClock,
            tx: Sender<(i64, LinkRaw)>,
            dropped: Arc<AtomicU64>,
            start_stop_sync: bool,
        ) -> Self {
            *TARGET.lock().unwrap_or_else(|e| e.into_inner()) = Some(Target { clock: clock.clone(), tx, dropped });
            let link = AblLink::new(INITIAL_BPM);
            link.enable_start_stop_sync(start_stop_sync);
            // Non-capturing closures only (see `Target`).
            link.set_num_peers_callback(|n| deliver(LinkRaw::Peers(n)));
            link.set_tempo_callback(|bpm| deliver(LinkRaw::Tempo(bpm)));
            link.set_start_stop_callback(|p| deliver(LinkRaw::StartStop(p)));
            link.enable(true);
            LinkObserver { link, state: SessionState::new() }
        }

        pub fn start_stop_sync(&self) -> bool {
            self.link.is_start_stop_sync_enabled()
        }

        /// Capture (never commit) the app session state.
        pub fn sample(&mut self) -> LinkSample {
            self.link.capture_app_session_state(&mut self.state);
            let t = self.link.clock_micros();
            LinkSample {
                link_clock_us: t,
                peers: self.link.num_peers(),
                tempo: self.state.tempo(),
                beat: self.state.beat_at_time(t, QUANTUM),
                phase: self.state.phase_at_time(t, QUANTUM),
                is_playing: self.state.is_playing(),
            }
        }
    }

    impl Drop for LinkObserver {
        fn drop(&mut self) {
            self.link.delete_num_peers_callback();
            self.link.delete_tempo_callback();
            self.link.delete_start_stop_callback();
            self.link.enable(false);
            *TARGET.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crossbeam_channel::bounded;

        /// Guard against the rusty_link closure lifetime bug: callbacks must
        /// not capture state, so their closures must be zero-sized.
        #[test]
        fn callback_closures_are_zero_sized() {
            fn size_of_val<T>(_: &T) -> usize {
                std::mem::size_of::<T>()
            }
            let a = |n: u64| deliver(LinkRaw::Peers(n));
            let b = |bpm: f64| deliver(LinkRaw::Tempo(bpm));
            let c = |p: bool| deliver(LinkRaw::StartStop(p));
            assert_eq!((size_of_val(&a), size_of_val(&b), size_of_val(&c)), (0, 0, 0));
        }

        #[test]
        fn deliver_reaches_the_registered_target_and_stops_after_reset() {
            let (tx, rx) = bounded(4);
            let dropped = Arc::new(AtomicU64::new(0));
            *TARGET.lock().unwrap() = Some(Target { clock: MonotonicClock::new(), tx, dropped });
            deliver(LinkRaw::Peers(1));
            assert_eq!(rx.try_recv().map(|(_, r)| r), Ok(LinkRaw::Peers(1)));
            *TARGET.lock().unwrap() = None;
            deliver(LinkRaw::Peers(2));
            assert!(rx.try_recv().is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(peers: u64, tempo: f64, is_playing: bool) -> LinkSample {
        LinkSample { link_clock_us: 0, peers, tempo, beat: 1.0, phase: 1.0, is_playing }
    }

    #[test]
    fn initial_not_playing_is_unknown_not_stopped() {
        let mut t = LinkTracker::new();
        assert_eq!(t.transport(), LinkTransport::Unknown);
        let ev = t.sample(sample(0, 120.0, false));
        assert_eq!(ev, vec!["tempo 120.00 (local, no peers)"]);
        assert_eq!(t.transport(), LinkTransport::Unknown);
        // The SDK's initial tempo is not shown as session tempo while alone.
        assert_eq!(t.snapshot().tempo, None);
        // Live joins; it is stopped, so is_playing never changes.
        assert_eq!(t.feed(LinkRaw::Peers(1)), vec!["peers 0 -> 1"]);
        t.sample(sample(1, 120.0, false));
        assert_eq!(t.transport(), LinkTransport::Unknown);
        assert_eq!(t.snapshot().tempo, Some(120.0));
        assert_eq!(t.feed(LinkRaw::Peers(0)), vec!["peers 1 -> 0", "transport unknown (no peers)"]);
        assert_eq!(t.snapshot().tempo, None);
    }

    #[test]
    fn playing_then_observed_stop_is_stopped() {
        let mut t = LinkTracker::new();
        t.feed(LinkRaw::Peers(1));
        assert_eq!(t.feed(LinkRaw::StartStop(true)), vec!["is_playing -> true (peers 1)"]);
        assert_eq!(t.transport(), LinkTransport::Playing);
        assert_eq!(t.feed(LinkRaw::StartStop(false)), vec!["is_playing -> false (peers 1)"]);
        assert_eq!(t.transport(), LinkTransport::Stopped);
        assert_eq!(t.snapshot().stats.is_playing_changes, 2);
        assert_eq!(t.snapshot().stats.peer_changes, 1);
    }

    #[test]
    fn losing_peers_returns_to_unknown_and_is_a_separate_event() {
        let mut t = LinkTracker::new();
        t.feed(LinkRaw::Peers(1));
        t.feed(LinkRaw::StartStop(true));
        t.feed(LinkRaw::StartStop(false));
        assert_eq!(t.transport(), LinkTransport::Stopped);
        assert_eq!(t.feed(LinkRaw::Peers(0)), vec!["peers 1 -> 0", "transport unknown (no peers)"]);
        assert_eq!(t.transport(), LinkTransport::Unknown);
        // A later false while alone is not a Stop.
        t.feed(LinkRaw::StartStop(true));
        assert_eq!(t.transport(), LinkTransport::Unknown);
        assert_eq!(t.feed(LinkRaw::StartStop(false)), vec!["is_playing -> false (no peers; transport stays unknown)"]);
        assert_eq!(t.transport(), LinkTransport::Unknown);
    }

    #[test]
    fn duplicate_notifications_and_samples_do_not_repeat_events() {
        let mut t = LinkTracker::new();
        t.feed(LinkRaw::Peers(1));
        t.feed(LinkRaw::Tempo(120.0));
        assert!(t.feed(LinkRaw::Peers(1)).is_empty());
        assert!(t.sample(sample(1, 120.0, false)).is_empty());
        assert_eq!(t.feed(LinkRaw::Tempo(128.0)), vec!["tempo 120.00 -> 128.00"]);
        // A change only the sample sees still becomes an event.
        assert_eq!(t.sample(sample(1, 128.0, true)), vec!["is_playing -> true (peers 1)"]);
    }

    #[test]
    fn this_module_never_writes_to_the_link_session() {
        let src = include_str!("mod.rs");
        let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        let forbidden = ["commit_", "set_tempo(", "set_is_playing(", "request_beat", "force_beat"];
        for f in forbidden {
            // The tracker's own private helpers are named set_tempo / set_is_playing
            // and are called on `self`; SDK calls go through `self.state` / `self.link`.
            for pat in [format!("state.{f}"), format!("link.{f}")] {
                assert!(!code.contains(&pat), "forbidden Link write {pat}");
            }
        }
    }
}
