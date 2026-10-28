//! Bridge decoder: the common clock from OmoiIroBridge samples
//! (docs/10-live-bridge.md "アプリ側の受信と共通時計"). Pure; no I/O.
//!
//! * Time axis: sample arrival on the session monotonic clock. `perf_ns`
//!   (Live's clock) is never compared with it.
//! * Position: while playing, `song_ms + (now - arrival)`; while stopped, the
//!   last `song_ms`. `is_playing` from the bridge is authoritative.
//! * Health: `unlocked` (nothing yet) -> `acquiring` (one sample, or right
//!   after a discontinuity) -> `locked` (two consecutive consistent
//!   samples) -> `holdover` (playing, no sample for > 50 ms, still
//!   interpolating) -> `lost` (> 250 ms, position held at the holdover
//!   limit; blackout after 1 s). While stopped only liveness counts: `lost`
//!   after 500 ms without a sample. `fault`: `errors` in a sample, missing
//!   song time, unknown format version, or a large `seq` regression that is
//!   not a script restart.
//! * Discontinuity (not suppressed): while playing, `|Δsong - Δarrival| >
//!   20 ms`; at Play / Stop, a move outside `[-20 ms, Δarrival + 20 ms]`;
//!   while stopped, a move of more than one frame (1/30 s). A playing jump
//!   within 50 ms of `is_playing` turning true is phase `start`, not
//!   `playing` (Live reports Play before moving to the start position).
//! * Stalled: `is_playing = true` but `song_ms` has not advanced for 300 ms
//!   of arrival time. Shown as transport `stalled` and never as playing. An
//!   unchanged song time while playing is a stall, not a discontinuity. As
//!   soon as two playing samples at least 30 ms apart show the same song time,
//!   the position stops being interpolated (`advancing = false`) until it
//!   moves again; the `stalled` transport still needs the full 300 ms.
//! * `meta stop` (the script is disconnecting): `lost` at once, without
//!   waiting for the 250 ms holdover. A later sample (script restart) clears it.
//! * Accepts format v2 (`song_ms`) and the P0 probe's v1 (`smpte_ms`).

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct BridgeSample {
    pub t_us: i64,
    pub v: Option<i64>,
    pub seq: Option<i64>,
    pub kind: String,
    pub event: String,
    pub song_ms: Option<f64>,
    pub beats: Option<f64>,
    pub tempo: Option<f64>,
    pub is_playing: Option<bool>,
    /// Names of fields Live could not read (v2), or the v1 error strings.
    pub errors: Vec<String>,
}

impl BridgeSample {
    pub fn from_payload(t_us: i64, p: &Value) -> BridgeSample {
        let errors = match p.get("errors") {
            Some(Value::Array(a)) => a.iter().map(|e| e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string())).collect(),
            Some(Value::Null) | None => Vec::new(),
            Some(other) => vec![other.to_string()],
        };
        let mut errors = errors;
        if p.get("parse_error").is_some() {
            errors.push("parse_error".into());
        }
        BridgeSample {
            t_us,
            v: p.get("v").and_then(Value::as_i64),
            seq: p.get("seq").and_then(Value::as_i64),
            kind: p.get("kind").and_then(Value::as_str).unwrap_or("").to_string(),
            event: p.get("event").and_then(Value::as_str).unwrap_or("").to_string(),
            song_ms: p.get("song_ms").or_else(|| p.get("smpte_ms")).and_then(Value::as_f64),
            beats: p.get("beats").and_then(Value::as_f64),
            tempo: p.get("tempo").and_then(Value::as_f64),
            is_playing: p.get("is_playing").and_then(Value::as_bool),
            errors,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeHealth {
    Unlocked,
    Acquiring,
    Locked,
    Holdover,
    Lost,
    Fault,
}

impl BridgeHealth {
    pub fn name(self) -> &'static str {
        match self {
            BridgeHealth::Unlocked => "unlocked",
            BridgeHealth::Acquiring => "acquiring",
            BridgeHealth::Locked => "locked",
            BridgeHealth::Holdover => "holdover",
            BridgeHealth::Lost => "lost",
            BridgeHealth::Fault => "fault",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeTransport {
    Unknown,
    Playing,
    Stopped,
    Stalled,
}

impl BridgeTransport {
    pub fn name(self) -> &'static str {
        match self {
            BridgeTransport::Unknown => "unknown",
            BridgeTransport::Playing => "playing",
            BridgeTransport::Stopped => "stopped",
            BridgeTransport::Stalled => "stalled",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BridgeEvent {
    Locked { song_ms: f64 },
    /// `phase`: `playing`, `start`, `stop` or `stopped`.
    Discontinuity { phase: &'static str, from_ms: f64, to_ms: f64, elapsed_ms: f64, deviation_ms: f64 },
    Transport { playing: bool, song_ms: f64 },
    Stalled { song_ms: f64, for_ms: f64 },
    Resumed { song_ms: f64 },
    Fault { reason: String },
    /// The script sent `meta stop`.
    ScriptStopped { song_ms: f64 },
    Restart { seq: i64 },
    Reordered { seq: i64, last_seq: i64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BridgeParams {
    pub tolerance_ms: f64,
    pub stale_us: i64,
    pub holdover_us: i64,
    pub blackout_us: i64,
    pub stopped_timeout_us: i64,
    pub stall_us: i64,
    pub frame_ms: f64,
    /// Jumps this soon after `is_playing` turned true are phase `start`.
    pub start_window_us: i64,
    /// A `seq` drop larger than this is a fault unless the script restarted.
    pub seq_regression: i64,
}

impl Default for BridgeParams {
    fn default() -> Self {
        BridgeParams {
            tolerance_ms: 20.0,
            stale_us: 50_000,
            holdover_us: 250_000,
            blackout_us: 1_000_000,
            stopped_timeout_us: 500_000,
            stall_us: 300_000,
            frame_ms: 1000.0 / 30.0,
            start_window_us: 50_000,
            seq_regression: 100,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BridgeStats {
    pub samples: u64,
    pub discontinuities: u64,
    pub playing_discontinuities: u64,
    pub start_discontinuities: u64,
    pub locks: u64,
    pub faults: u64,
    pub stalls: u64,
    pub restarts: u64,
    pub reordered: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BridgeSnapshot {
    pub health: BridgeHealth,
    pub transport: BridgeTransport,
    pub fault: Option<String>,
    /// Song time in ms (interpolated while playing and healthy).
    pub song_ms: Option<f64>,
    pub advancing: bool,
    pub blackout: bool,
    pub age_us: Option<i64>,
    pub tempo: Option<f64>,
    pub beats: Option<f64>,
    /// Arrival of the sample on which `is_playing` turned true, while playing.
    pub play_since_us: Option<i64>,
    /// Samples received since `play_since_us` (the turning sample counts as 1).
    pub play_samples: u32,
    pub stats: BridgeStats,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Unlocked,
    Acquiring,
    Locked,
    Fault,
}

pub struct BridgeDecoder {
    params: BridgeParams,
    last: Option<BridgeSample>,
    state: State,
    fault: Option<String>,
    /// Arrival time and song time of the last advance while playing.
    advance: Option<(i64, f64)>,
    /// Arrival of the sample on which `is_playing` turned true.
    play_since: Option<i64>,
    play_samples: u32,
    /// The last playing pair showed no advance over >= 30 ms.
    static_pair: bool,
    script_stopped: bool,
    stalled: bool,
    stats: BridgeStats,
}

fn known_version(v: Option<i64>) -> bool {
    matches!(v, None | Some(1) | Some(2))
}

impl BridgeDecoder {
    pub fn new(params: BridgeParams) -> Self {
        BridgeDecoder { params, last: None, state: State::Unlocked, fault: None, advance: None, play_since: None, play_samples: 0, static_pair: false, script_stopped: false, stalled: false, stats: BridgeStats::default() }
    }

    pub fn params(&self) -> &BridgeParams {
        &self.params
    }

    fn set_fault(&mut self, reason: String, out: &mut Vec<BridgeEvent>) {
        if self.state != State::Fault || self.fault.as_deref() != Some(reason.as_str()) {
            self.stats.faults += 1;
            out.push(BridgeEvent::Fault { reason: reason.clone() });
        }
        self.state = State::Fault;
        self.fault = Some(reason);
    }

    pub fn feed(&mut self, s: BridgeSample, out: &mut Vec<BridgeEvent>) {
        self.stats.samples += 1;
        let p = self.params;

        if let (Some(seq), Some(last_seq)) = (s.seq, self.last.as_ref().and_then(|l| l.seq)) {
            if seq <= last_seq {
                let restart = s.event == "start" || seq <= 1;
                if restart {
                    self.stats.restarts += 1;
                    out.push(BridgeEvent::Restart { seq });
                    self.last = None;
                    self.state = State::Unlocked;
                    self.fault = None;
                    self.advance = None;
                    self.stalled = false;
                } else if last_seq - seq > p.seq_regression {
                    self.set_fault(format!("seq regression {last_seq} -> {seq}"), out);
                    self.last = None;
                    return;
                } else {
                    self.stats.reordered += 1;
                    out.push(BridgeEvent::Reordered { seq, last_seq });
                    return;
                }
            }
        }
        if !known_version(s.v) {
            self.set_fault(format!("format v={}", s.v.unwrap_or(-1)), out);
            return;
        }
        if !s.errors.is_empty() {
            self.set_fault(format!("errors: {}", s.errors.join(",")), out);
            self.last = Some(s);
            return;
        }
        let (Some(song), Some(playing)) = (s.song_ms, s.is_playing) else {
            self.set_fault("missing song_ms or is_playing".into(), out);
            return;
        };
        if self.state == State::Fault {
            self.fault = None;
            self.state = State::Acquiring;
            self.last = None;
        }
        if s.kind == "meta" && s.event == "stop" {
            self.script_stopped = true;
            out.push(BridgeEvent::ScriptStopped { song_ms: song });
            self.last = Some(s);
            return;
        }
        self.script_stopped = false;

        match self.last.as_ref() {
            None => self.state = State::Acquiring,
            Some(prev) => {
                let prev_song = prev.song_ms.unwrap_or(song);
                let prev_playing = prev.is_playing.unwrap_or(false);
                let elapsed_ms = (s.t_us - prev.t_us) as f64 / 1000.0;
                let step = song - prev_song;
                let just_started = self.play_since.is_some_and(|t| s.t_us - t <= p.start_window_us);
                let (phase, deviation, ok) = match (prev_playing, playing) {
                    (true, true) => {
                        // Moving again after a static pair (Live waiting to start) is a start too.
                        let phase = if just_started || self.static_pair { "start" } else { "playing" };
                        // An unchanged song time is a stall (handled below), not a jump.
                        let ok = (step - elapsed_ms).abs() <= p.tolerance_ms || step.abs() < 0.5;
                        (phase, step - elapsed_ms, ok)
                    }
                    (false, true) => ("start", step, step >= -p.tolerance_ms && step <= elapsed_ms + p.tolerance_ms),
                    (true, false) => ("stop", step, step >= -p.tolerance_ms && step <= elapsed_ms + p.tolerance_ms),
                    (false, false) => ("stopped", step, step.abs() <= p.frame_ms),
                };
                if prev_playing != playing {
                    out.push(BridgeEvent::Transport { playing, song_ms: song });
                }
                self.static_pair = prev_playing && playing && step.abs() < 0.5 && elapsed_ms >= 30.0;
                if ok {
                    if self.state != State::Locked {
                        self.state = State::Locked;
                        self.stats.locks += 1;
                        out.push(BridgeEvent::Locked { song_ms: song });
                    }
                } else {
                    self.stats.discontinuities += 1;
                    match phase {
                        "playing" => self.stats.playing_discontinuities += 1,
                        "start" => self.stats.start_discontinuities += 1,
                        _ => {}
                    }
                    out.push(BridgeEvent::Discontinuity { phase, from_ms: prev_song, to_ms: song, elapsed_ms, deviation_ms: deviation });
                    self.state = State::Acquiring;
                }
            }
        }

        if playing && self.last.as_ref().and_then(|l| l.is_playing) != Some(true) {
            self.play_since = Some(s.t_us);
            self.play_samples = 1;
        } else if playing {
            self.play_samples = self.play_samples.saturating_add(1);
        } else {
            self.play_since = None;
            self.play_samples = 0;
        }

        // Stall: playing but the song time does not move.
        if playing {
            let restart_timer = match (self.advance, self.last.as_ref().and_then(|l| l.is_playing)) {
                (None, _) | (_, Some(false)) | (_, None) => true,
                (Some((_, adv_song)), _) => song > adv_song + 0.5 || song < adv_song - p.tolerance_ms,
            };
            if restart_timer {
                if self.stalled {
                    self.stalled = false;
                    out.push(BridgeEvent::Resumed { song_ms: song });
                }
                self.advance = Some((s.t_us, song));
            } else if let Some((t_adv, _)) = self.advance {
                if !self.stalled && s.t_us - t_adv > p.stall_us {
                    self.stalled = true;
                    self.stats.stalls += 1;
                    out.push(BridgeEvent::Stalled { song_ms: song, for_ms: (s.t_us - t_adv) as f64 / 1000.0 });
                }
            }
        } else {
            self.advance = None;
            self.stalled = false;
        }

        self.last = Some(s);
    }

    pub fn snapshot(&self, now_us: i64) -> BridgeSnapshot {
        let p = self.params;
        let last = self.last.as_ref();
        let mut snap = BridgeSnapshot {
            health: BridgeHealth::Unlocked,
            transport: BridgeTransport::Unknown,
            fault: self.fault.clone(),
            song_ms: last.and_then(|l| l.song_ms),
            advancing: false,
            blackout: true,
            age_us: last.map(|l| now_us - l.t_us),
            tempo: last.and_then(|l| l.tempo),
            beats: last.and_then(|l| l.beats),
            play_since_us: self.play_since,
            play_samples: self.play_samples,
            stats: self.stats,
        };
        let Some(l) = last else {
            if self.state == State::Fault {
                snap.health = BridgeHealth::Fault;
            }
            return snap;
        };
        let age = now_us - l.t_us;
        if self.script_stopped {
            snap.health = BridgeHealth::Lost;
            snap.blackout = age > p.blackout_us;
            snap.transport = BridgeTransport::Unknown;
            return snap;
        }
        let playing = l.is_playing == Some(true);
        snap.transport = if !playing {
            BridgeTransport::Stopped
        } else if self.stalled {
            BridgeTransport::Stalled
        } else {
            BridgeTransport::Playing
        };
        match self.state {
            State::Unlocked => return snap,
            State::Fault => {
                snap.health = BridgeHealth::Fault;
                return snap;
            }
            State::Acquiring | State::Locked => {}
        }
        let base = if self.state == State::Locked { BridgeHealth::Locked } else { BridgeHealth::Acquiring };
        // Not moving right now: hold instead of interpolating.
        let interp_ok = !self.static_pair;
        let song = l.song_ms.unwrap_or(0.0);
        if snap.transport == BridgeTransport::Playing {
            let interp = self.state == State::Locked && interp_ok;
            if age <= p.stale_us {
                snap.health = base;
                snap.blackout = false;
                if interp {
                    snap.song_ms = Some(song + age as f64 / 1000.0);
                    snap.advancing = true;
                }
            } else if age <= p.holdover_us {
                snap.health = if interp { BridgeHealth::Holdover } else { base };
                snap.blackout = false;
                if interp {
                    snap.song_ms = Some(song + age as f64 / 1000.0);
                    snap.advancing = true;
                }
            } else {
                snap.health = BridgeHealth::Lost;
                snap.blackout = age > p.blackout_us;
                if interp {
                    snap.song_ms = Some(song + p.holdover_us as f64 / 1000.0);
                }
            }
        } else {
            // Stopped or stalled: hold the last position; only liveness counts.
            if age <= p.stopped_timeout_us {
                snap.health = base;
                snap.blackout = false;
            } else {
                snap.health = BridgeHealth::Lost;
                snap.blackout = age > p.blackout_us;
            }
        }
        snap
    }
}

pub fn event_line(e: &BridgeEvent) -> String {
    match e {
        BridgeEvent::Locked { song_ms } => format!("locked at {song_ms:.0} ms"),
        BridgeEvent::Discontinuity { phase, from_ms, to_ms, elapsed_ms, deviation_ms } => format!(
            "discontinuity({phase}) {from_ms:.0} -> {to_ms:.0} ms (elapsed {elapsed_ms:.1} ms, deviation {deviation_ms:+.1} ms)"
        ),
        BridgeEvent::Transport { playing, song_ms } => format!("{} at {song_ms:.0} ms", if *playing { "play" } else { "stop" }),
        BridgeEvent::Stalled { song_ms, for_ms } => format!("stalled at {song_ms:.0} ms (playing, no advance for {for_ms:.0} ms)"),
        BridgeEvent::Resumed { song_ms } => format!("resumed at {song_ms:.0} ms"),
        BridgeEvent::Fault { reason } => format!("fault [{reason}]"),
        BridgeEvent::ScriptStopped { song_ms } => format!("script stopped at {song_ms:.0} ms (lost)"),
        BridgeEvent::Restart { seq } => format!("script restart (seq {seq})"),
        BridgeEvent::Reordered { seq, last_seq } => format!("reordered sample seq {seq} after {last_seq} (ignored)"),
    }
}

/// `HH:MM:SS.mmm` of a song time in ms.
fn hms(ms: f64) -> String {
    crate::diagnostics::report::position_hms((ms * 1000.0) as i64)
}

pub fn status_line(s: &BridgeSnapshot) -> String {
    let pos = s.song_ms.map(hms).unwrap_or_else(|| "--".into());
    let age = s.age_us.map(|a| format!("{}ms", a / 1000)).unwrap_or_else(|| "--".into());
    let tempo = s.tempo.map(|t| format!("{t:.2}")).unwrap_or_else(|| "--".into());
    let fault = s.fault.as_ref().map(|f| format!(" [{f}]")).unwrap_or_default();
    format!(
        "BRIDGE {:<9} {:<8} song={pos}{} tempo={tempo} age={age} n={} disc={} (playing {}, start {}) stalls={}{fault}",
        s.health.name(),
        s.transport.name(),
        if s.advancing { " adv" } else { "    " },
        s.stats.samples,
        s.stats.discontinuities,
        s.stats.playing_discontinuities,
        s.stats.start_discontinuities,
        s.stats.stalls,
    )
}

pub fn snapshot_json(s: &BridgeSnapshot) -> Value {
    serde_json::json!({
        "health": s.health.name(),
        "transport": s.transport.name(),
        "fault": s.fault,
        "song_ms": s.song_ms,
        "advancing": s.advancing,
        "blackout": s.blackout,
        "age_us": s.age_us,
        "tempo": s.tempo,
        "beats": s.beats,
        "stats": {
            "samples": s.stats.samples,
            "discontinuities": s.stats.discontinuities,
            "playing_discontinuities": s.stats.playing_discontinuities,
            "start_discontinuities": s.stats.start_discontinuities,
            "locks": s.stats.locks,
            "faults": s.stats.faults,
            "stalls": s.stats.stalls,
            "restarts": s.stats.restarts,
            "reordered": s.stats.reordered,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Sim {
        dec: BridgeDecoder,
        seq: i64,
        out: Vec<BridgeEvent>,
    }

    impl Sim {
        fn new() -> Self {
            Sim { dec: BridgeDecoder::new(BridgeParams::default()), seq: 0, out: Vec::new() }
        }
        fn send(&mut self, t_us: i64, song_ms: f64, playing: bool) {
            self.seq += 1;
            let p = json!({"v": 2, "seq": self.seq, "kind": "listener", "event": "song_time",
                "song_ms": song_ms, "beats": song_ms / 500.0, "tempo": 120.0, "is_playing": playing, "errors": []});
            self.dec.feed(BridgeSample::from_payload(t_us, &p), &mut self.out);
        }
        /// Play from `song0` at `t0` for `n` samples every 16 ms.
        fn play(&mut self, t0: i64, song0: f64, n: i64) -> (i64, f64) {
            for k in 0..n {
                self.send(t0 + k * 16_000, song0 + k as f64 * 16.0, true);
            }
            (t0 + n * 16_000, song0 + n as f64 * 16.0)
        }
        fn discs(&self) -> Vec<&BridgeEvent> {
            self.out.iter().filter(|e| matches!(e, BridgeEvent::Discontinuity { .. })).collect()
        }
    }

    #[test]
    fn locks_after_two_consistent_samples_and_interpolates() {
        let mut s = Sim::new();
        s.send(0, 1000.0, true);
        assert_eq!(s.dec.snapshot(0).health, BridgeHealth::Acquiring);
        s.send(16_000, 1016.0, true);
        assert!(s.out.contains(&BridgeEvent::Locked { song_ms: 1016.0 }));
        let snap = s.dec.snapshot(26_000);
        assert_eq!((snap.health, snap.transport), (BridgeHealth::Locked, BridgeTransport::Playing));
        assert!(snap.advancing);
        assert_eq!(snap.song_ms, Some(1026.0));
    }

    #[test]
    fn silence_while_playing_goes_holdover_lost_blackout() {
        let mut s = Sim::new();
        let (t, _) = s.play(0, 0.0, 5);
        let last_t = t - 16_000;
        let last_song = 64.0;
        assert_eq!(s.dec.snapshot(last_t + 100_000).health, BridgeHealth::Holdover);
        assert_eq!(s.dec.snapshot(last_t + 100_000).song_ms, Some(last_song + 100.0));
        let lost = s.dec.snapshot(last_t + 400_000);
        assert_eq!(lost.health, BridgeHealth::Lost);
        assert!(!lost.advancing && !lost.blackout);
        assert_eq!(lost.song_ms, Some(last_song + 250.0));
        assert!(s.dec.snapshot(last_t + 1_500_000).blackout);
    }

    #[test]
    fn stopped_only_needs_liveness() {
        let mut s = Sim::new();
        s.send(0, 5000.0, false);
        s.send(100_000, 5000.0, false);
        let snap = s.dec.snapshot(400_000);
        assert_eq!((snap.health, snap.transport), (BridgeHealth::Locked, BridgeTransport::Stopped));
        assert_eq!(snap.song_ms, Some(5000.0));
        assert!(!snap.advancing);
        assert_eq!(s.dec.snapshot(700_000).health, BridgeHealth::Lost);
    }

    #[test]
    fn jumps_are_discontinuities_by_phase_and_tempo_automation_is_not() {
        let mut s = Sim::new();
        s.send(0, 0.0, false);
        s.send(100_000, 0.0, false);
        // Play from 0 (first playing sample 30 ms in).
        s.send(130_000, 30.0, true);
        let (t, song) = s.play(146_000, 46.0, 30);
        assert!(s.discs().is_empty(), "{:?}", s.discs());
        // Manual tempo change: Live's song time jumps by -895 ms.
        s.send(t, song - 895.0, true);
        // Loop wrap / seek: back to 22000 ms.
        let (t, _) = s.play(t + 16_000, song - 895.0 + 16.0, 10);
        s.send(t, 22_000.0, true);
        // Stop, then the position returns to the start while stopped.
        s.send(t + 16_000, 22_016.0, false);
        s.send(t + 116_000, 0.0, false);
        let phases: Vec<&str> = s
            .discs()
            .iter()
            .map(|e| match e {
                BridgeEvent::Discontinuity { phase, .. } => *phase,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(phases, vec!["playing", "playing", "stopped"]);
        assert_eq!(s.dec.snapshot(t + 116_000).stats.playing_discontinuities, 2);
    }

    #[test]
    fn jump_right_after_play_is_phase_start() {
        let mut s = Sim::new();
        s.send(0, 1378.0, false);
        s.send(100_000, 1378.0, false);
        // Live: is_playing first, the move to the start position 6 ms later.
        s.send(110_000, 1378.0, true);
        s.send(116_000, 5.0, true);
        s.play(132_000, 21.0, 5);
        let d = s.discs();
        assert_eq!(d.len(), 1);
        assert!(matches!(d[0], BridgeEvent::Discontinuity { phase: "start", .. }), "{d:?}");
        let st = s.dec.snapshot(200_000).stats;
        assert_eq!((st.playing_discontinuities, st.start_discontinuities), (0, 1));
        // After the window, the same jump is phase playing.
        let (t, song) = s.play(300_000, 200.0, 1);
        s.send(t, song + 5000.0, true);
        assert!(matches!(s.discs().last().unwrap(), BridgeEvent::Discontinuity { phase: "playing", .. }));
    }

    #[test]
    fn playing_without_advance_is_stalled_not_playing() {
        let mut s = Sim::new();
        s.send(0, 0.0, false);
        // Live says playing but the song time stays at 0 (S07 freeze).
        for k in 1..=40 {
            s.send(k * 16_000, 0.0, true);
        }
        assert!(s.out.iter().any(|e| matches!(e, BridgeEvent::Stalled { .. })));
        let snap = s.dec.snapshot(40 * 16_000);
        assert_eq!(snap.transport, BridgeTransport::Stalled);
        assert!(!snap.advancing);
        assert_eq!(snap.song_ms, Some(0.0));
        // Advancing again resumes.
        s.play(41 * 16_000, 16.0, 3);
        assert!(s.out.iter().any(|e| matches!(e, BridgeEvent::Resumed { .. })));
        assert_eq!(s.dec.snapshot(44 * 16_000).transport, BridgeTransport::Playing);
    }

    #[test]
    fn errors_version_and_seq_regression_are_faults_restart_is_not() {
        let mut s = Sim::new();
        s.play(0, 0.0, 3);
        let p = json!({"v": 2, "seq": 4, "song_ms": null, "is_playing": true, "errors": ["song_ms"]});
        s.dec.feed(BridgeSample::from_payload(48_000, &p), &mut s.out);
        assert_eq!(s.dec.snapshot(48_000).health, BridgeHealth::Fault);
        assert!(s.out.contains(&BridgeEvent::Fault { reason: "errors: song_ms".into() }));
        // Recovers with clean samples.
        s.seq = 4;
        s.play(64_000, 64.0, 2);
        assert_eq!(s.dec.snapshot(80_000).health, BridgeHealth::Locked);
        // Unknown format version.
        let p = json!({"v": 9, "seq": 100, "song_ms": 1.0, "is_playing": true, "errors": []});
        s.dec.feed(BridgeSample::from_payload(96_000, &p), &mut s.out);
        assert_eq!(s.dec.snapshot(96_000).fault.as_deref(), Some("format v=9"));
        // Script restart (seq back to 1) resets cleanly.
        s.seq = 0;
        s.play(200_000, 500.0, 2);
        assert!(s.out.iter().any(|e| matches!(e, BridgeEvent::Restart { seq: 1 })));
        assert_eq!(s.dec.snapshot(216_000).health, BridgeHealth::Locked);
    }

    #[test]
    fn meta_stop_is_lost_at_once_and_restart_recovers() {
        let mut s = Sim::new();
        let (t, song) = s.play(0, 0.0, 5);
        s.seq += 1;
        let p = json!({"v": 2, "seq": s.seq, "kind": "meta", "event": "stop", "song_ms": song, "is_playing": true, "errors": []});
        s.dec.feed(BridgeSample::from_payload(t, &p), &mut s.out);
        assert!(s.out.iter().any(|e| matches!(e, BridgeEvent::ScriptStopped { .. })));
        let snap = s.dec.snapshot(t + 10_000);
        assert_eq!(snap.health, BridgeHealth::Lost);
        assert!(!snap.advancing);
        // Script restart: seq back to 1.
        s.seq = 0;
        s.play(t + 5_000_000, 5000.0, 2);
        assert_eq!(s.dec.snapshot(t + 5_016_000).health, BridgeHealth::Locked);
    }

    #[test]
    fn stall_while_playing_is_not_a_discontinuity() {
        let mut s = Sim::new();
        s.send(0, 2000.0, false);
        // Playing, but the song time stays (timer samples every 100 ms).
        for k in 1..=20 {
            s.send(k * 100_000, 2000.0, true);
        }
        assert!(s.discs().is_empty(), "{:?}", s.discs());
        assert_eq!(s.dec.snapshot(2_000_000).transport, BridgeTransport::Stalled);
    }

    #[test]
    fn a_static_pair_stops_interpolation_before_the_stall_is_declared() {
        let mut s = Sim::new();
        s.play(0, 0.0, 5); // last sample at 64 ms, song 64
        s.send(164_000, 64.0, true); // 100 ms later, same song time
        let snap = s.dec.snapshot(200_000);
        assert_eq!(snap.transport, BridgeTransport::Playing);
        assert!(!snap.advancing);
        assert_eq!(snap.song_ms, Some(64.0));
        // A timer sample 1 ms after a listener sample with the same ms is not static.
        let mut s = Sim::new();
        s.play(0, 0.0, 5);
        s.send(65_000, 64.0, true);
        assert!(s.dec.snapshot(70_000).advancing);
    }

    #[test]
    fn play_since_and_sample_count_are_reported() {
        let mut s = Sim::new();
        s.send(0, 0.0, false);
        s.send(100_000, 0.0, true);
        s.send(116_000, 16.0, true);
        let snap = s.dec.snapshot(120_000);
        assert_eq!((snap.play_since_us, snap.play_samples), (Some(100_000), 2));
    }

    #[test]
    fn small_reorder_is_ignored() {
        let mut s = Sim::new();
        s.play(0, 0.0, 5);
        let p = json!({"v": 2, "seq": 3, "song_ms": 32.0, "is_playing": true, "errors": []});
        s.dec.feed(BridgeSample::from_payload(81_000, &p), &mut s.out);
        assert_eq!(s.dec.snapshot(81_000).stats.reordered, 1);
        assert!(s.discs().is_empty());
    }

    #[test]
    fn v1_probe_records_are_accepted() {
        let p = json!({"v": 1, "seq": 5, "kind": "timer", "event": "tick", "smpte_ms": 1234, "is_playing": false, "tempo": 90.0});
        let s = BridgeSample::from_payload(0, &p);
        assert_eq!(s.song_ms, Some(1234.0));
        assert!(s.errors.is_empty());
    }
}
