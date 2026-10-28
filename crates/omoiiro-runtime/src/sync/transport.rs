//! Transport integration (docs/02-architecture.md "時間モデル", docs/10-live-bridge.md,
//! tasks T03 / T04 / T08).
//!
//! * Time source: the OmoiIroBridge decoder (first source of the common clock).
//!   `show_time_us = song_ms * 1000 + input_offset_us`; the show's `origin`
//!   only labels MTC output (`origin + show_time`).
//! * Transport state comes from the bridge (`is_playing` is authoritative;
//!   Link Start Stop Sync is never used).
//! * Beat source: exactly one of Link or MIDI Clock, chosen by the show's
//!   `beat_source`. No automatic fallback to the other or to preview BPM.
//! * Time health and beat health are independent and each has its own last
//!   good reception time.
//! * Loss rules from the show's `sync.loss` / `stop_behavior`:
//!   time side: locked / holdover follow the position; lost (and fault) hold
//!   the last position until `blackout_after_ms` since the last good sample,
//!   then black; a normal Stop holds (`stop_behavior: hold`) or goes black
//!   (`blackout`); after a loss blackout, recovery fades in over
//!   `recovery_fade_ms`. Beat side: the beat extrapolates up to
//!   `holdover_ms`, then holds, and loses its phase (Pulse transparent) after
//!   `blackout_after_ms`.
//! * T03: every input is stamped with the session monotonic clock at
//!   arrival; backend timestamps are never compared across connections. A
//!   reconnect discards the old decoder state (`reconnect_*`).

use crate::link::{LinkRaw, LinkSample, LinkTracker};
use crate::sync::bridge::{BridgeDecoder, BridgeEvent, BridgeHealth, BridgeParams, BridgeSample, BridgeSnapshot, BridgeTransport};
use crate::sync::clock::{ClockDecoder, ClockEvent, Transport as ClockTransport};
use omoiiro_core::model::{BeatSource, Show, StopBehavior};
use omoiiro_core::time::{BeatState, FrameRate, ShowTimeUs, Timecode};

/// Bridge tempo and MIDI Clock BPM must agree within this ratio before the
/// clock origin is seeded from the bridge (design, 2026-10-28).
pub const SEED_TEMPO_TOLERANCE: f64 = 0.05;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeatSourceKind {
    Link,
    MidiClock,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransportConfig {
    /// `None`: no beat input (beat-driven layers stay transparent).
    pub beat_source: Option<BeatSourceKind>,
    pub input_offset_us: i64,
    /// Label of the song start for MTC output.
    pub origin: Timecode,
    pub holdover_us: i64,
    pub blackout_us: i64,
    pub stop_behavior: StopBehavior,
    pub recovery_fade_us: i64,
}

impl TransportConfig {
    pub fn from_show(show: &Show) -> Result<TransportConfig, String> {
        let tc = &show.sync.timecode;
        let origin = Timecode::parse(&tc.origin, tc.frame_rate).map_err(|e| format!("origin {:?}: {e}", tc.origin))?;
        Ok(TransportConfig {
            beat_source: Some(match show.sync.beat_source {
                BeatSource::AbletonLink => BeatSourceKind::Link,
                BeatSource::MidiClock => BeatSourceKind::MidiClock,
            }),
            input_offset_us: tc.input_offset_us,
            origin,
            holdover_us: show.sync.loss.holdover_ms as i64 * 1000,
            blackout_us: show.sync.loss.blackout_after_ms as i64 * 1000,
            stop_behavior: show.sync.stop_behavior,
            recovery_fade_us: show.sync.loss.recovery_fade_ms as i64 * 1000,
        })
    }

    /// The P0 defaults (250 ms holdover, 1 s blackout, 100 ms recovery, hold on Stop).
    pub fn defaults(origin: Timecode, input_offset_us: i64, beat_source: Option<BeatSourceKind>) -> TransportConfig {
        TransportConfig {
            beat_source,
            input_offset_us,
            origin,
            holdover_us: 250_000,
            blackout_us: 1_000_000,
            stop_behavior: StopBehavior::Hold,
            recovery_fade_us: 100_000,
        }
    }

    fn bridge_params(&self) -> BridgeParams {
        BridgeParams { holdover_us: self.holdover_us, blackout_us: self.blackout_us, ..BridgeParams::default() }
    }

    /// `origin + input_offset` in microseconds: add `song_ms * 1000` to get the MTC label position.
    pub fn origin_plus_offset_us(&self) -> f64 {
        self.origin.to_us() as f64 + self.input_offset_us as f64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportState {
    Playing,
    Stopped,
    Unknown,
}

impl TransportState {
    pub fn name(self) -> &'static str {
        match self {
            TransportState::Playing => "playing",
            TransportState::Stopped => "stopped",
            TransportState::Unknown => "unknown",
        }
    }
}

/// What the LED output should do with the time side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputState {
    /// Follow the advancing show time.
    Follow,
    /// Render the (held) show time without advancing it.
    Hold,
    /// Output black.
    Black,
}

impl OutputState {
    pub fn name(self) -> &'static str {
        match self {
            OutputState::Follow => "follow",
            OutputState::Hold => "hold",
            OutputState::Black => "black",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeatHealth {
    /// No beat input selected.
    Unselected,
    /// Selected, but no usable beat yet (no peers, no Start / SPP).
    Unknown,
    Locked,
    /// Extrapolating after the last good reception (<= holdover).
    Holdover,
    /// Beyond holdover: held; phase invalid after the blackout time.
    Lost,
}

impl BeatHealth {
    pub fn name(self) -> &'static str {
        match self {
            BeatHealth::Unselected => "unselected",
            BeatHealth::Unknown => "unknown",
            BeatHealth::Locked => "locked",
            BeatHealth::Holdover => "holdover",
            BeatHealth::Lost => "lost",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransportSnapshot {
    pub now_us: i64,
    /// Show time (song start = 0) to render, if any.
    pub show_time_us: Option<ShowTimeUs>,
    pub advancing: bool,
    pub transport: TransportState,
    /// True while the bridge reports playing without movement.
    pub stalled: bool,
    pub time_health: BridgeHealth,
    pub output: OutputState,
    /// Output gain 0..=1 (black = 0; recovery fade ramps it).
    pub output_gain: f32,
    pub beat: Option<BeatState>,
    pub beat_health: BeatHealth,
    /// MIDI Clock origin taken from the bridge's beats (joined a running
    /// clock); cleared by Start / SPP.
    pub beat_seeded: bool,
    /// Times the MIDI Clock origin was seeded from the bridge.
    pub clock_seeded: u64,
    /// MTC label of the show time (`origin + show_time`).
    pub mtc_label: Option<String>,
    pub generations: Generations,
}

/// Connection generations: bumped on every reconnect so stale state is never mixed in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Generations {
    pub bridge: u64,
    pub clock: u64,
    pub link: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dark {
    No,
    /// Black because of loss / fault / never locked: recovery fades in.
    Loss,
    /// Black because of a normal Stop with `stop_behavior: blackout`: no fade.
    Stop,
}

/// A21: the Live-less local time (`time_source: local`). The show time is a
/// play / pause / seek clock that wraps at the show length; the beat comes
/// from `bpm` (beat 0 at song start). Nothing is received from Live.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalTime {
    pub clock: crate::preview::LocalClock,
    pub bpm: f64,
}

pub struct Transport {
    cfg: TransportConfig,
    local: Option<LocalTime>,
    bridge: BridgeDecoder,
    clock: ClockDecoder,
    /// The clock origin was seeded once on this clock connection.
    clock_seed_done: bool,
    link: LinkTracker,
    link_last: Option<(i64, LinkSample)>,
    link_peers_lost_at: Option<i64>,
    link_had_peers: bool,
    gens: Generations,
    dark: Dark,
    recover_from: Option<i64>,
}

impl Transport {
    pub fn new(cfg: TransportConfig) -> Transport {
        Transport {
            bridge: BridgeDecoder::new(cfg.bridge_params()),
            clock: ClockDecoder::new(),
            clock_seed_done: false,
            link: LinkTracker::new(),
            link_last: None,
            link_peers_lost_at: None,
            link_had_peers: false,
            gens: Generations::default(),
            dark: Dark::No,
            recover_from: None,
            local: None,
            cfg,
        }
    }

    /// A21: driven by a local clock instead of the bridge (stopped at 0).
    pub fn new_local(cfg: TransportConfig, length_us: Option<i64>, bpm: f64) -> Transport {
        let mut t = Transport::new(cfg);
        t.local = Some(LocalTime { clock: crate::preview::LocalClock::new(length_us), bpm });
        t
    }

    pub fn local(&self) -> Option<&LocalTime> {
        self.local.as_ref()
    }

    pub fn local_mut(&mut self) -> Option<&mut LocalTime> {
        self.local.as_mut()
    }

    pub fn config(&self) -> &TransportConfig {
        &self.cfg
    }

    pub fn feed_bridge(&mut self, sample: BridgeSample, out: &mut Vec<BridgeEvent>) {
        self.bridge.feed(sample, out);
    }

    pub fn feed_clock(&mut self, t_us: i64, bytes: &[u8], out: &mut Vec<ClockEvent>) {
        self.clock.feed(t_us, bytes, out);
        if bytes.first() == Some(&0xF8) {
            self.seed_clock_origin(t_us, out);
        }
    }

    /// Joining a running clock gives ticks but no Start / SPP, so no phase
    /// (Pulse stays transparent until Live stops and plays again). Once per
    /// clock connection, take the origin from the bridge's beats when ticks
    /// are arriving, no origin is known, the bridge is locked and playing,
    /// and the two tempos agree within 5 % (otherwise another Set or clock
    /// source is suspected). Clock ticks then advance the phase as usual;
    /// Start / SPP replace the seed.
    fn seed_clock_origin(&mut self, t_us: i64, out: &mut Vec<ClockEvent>) {
        if self.clock_seed_done || self.cfg.beat_source != Some(BeatSourceKind::MidiClock) || self.clock.origin_known() {
            return;
        }
        let b = self.bridge.snapshot(t_us);
        let playing = b.health == BridgeHealth::Locked && b.transport == BridgeTransport::Playing && b.advancing;
        let (Some(beats), Some(tempo), Some(age)) = (b.beats, b.tempo, b.age_us) else {
            return;
        };
        let Some(bpm) = self.clock.snapshot(t_us).bpm else {
            return;
        };
        if !playing || tempo <= 0.0 || ((bpm - tempo) / tempo).abs() >= SEED_TEMPO_TOLERANCE {
            return;
        }
        // Live sends a tick on every 1/24 beat: the nearest one is this tick.
        let now_beats = beats + age.max(0) as f64 / 1e6 * tempo / 60.0;
        if now_beats < 0.0 {
            return;
        }
        self.clock.seed_origin((now_beats * 24.0).round() as u64, out);
        self.clock_seed_done = true;
    }

    /// Poll the clock decoder for a stall event (call periodically).
    pub fn poll_clock(&mut self, now_us: i64, out: &mut Vec<ClockEvent>) {
        self.clock.poll(now_us, out);
    }

    pub fn feed_link_raw(&mut self, t_us: i64, raw: LinkRaw) -> Vec<String> {
        let before = self.link.snapshot().peers;
        let ev = self.link.feed(raw);
        self.track_link_peers(t_us, before);
        ev
    }

    pub fn feed_link_sample(&mut self, t_us: i64, sample: LinkSample) -> Vec<String> {
        let before = self.link.snapshot().peers;
        let ev = self.link.sample(sample);
        self.link_last = Some((t_us, sample));
        self.track_link_peers(t_us, before);
        ev
    }

    fn track_link_peers(&mut self, t_us: i64, before: u64) {
        let after = self.link.snapshot().peers;
        if after > 0 {
            self.link_had_peers = true;
            self.link_peers_lost_at = None;
        } else if before > 0 {
            self.link_peers_lost_at = Some(t_us);
        }
    }

    /// T03: a new bridge connection (script restart is handled by the decoder;
    /// this is for a new socket / receiver).
    pub fn reconnect_bridge(&mut self) {
        self.bridge = BridgeDecoder::new(self.cfg.bridge_params());
        self.gens.bridge += 1;
    }

    pub fn reconnect_clock(&mut self) {
        self.clock = ClockDecoder::new();
        self.clock_seed_done = false;
        self.gens.clock += 1;
    }

    pub fn reconnect_link(&mut self) {
        self.link = LinkTracker::new();
        self.link_last = None;
        self.link_peers_lost_at = None;
        self.link_had_peers = false;
        self.gens.link += 1;
    }

    /// Bridge snapshot and discontinuity epoch, for the MTC generator.
    pub fn bridge_view(&self, now_us: i64) -> (BridgeSnapshot, u64) {
        let s = self.bridge.snapshot(now_us);
        let e = s.stats.discontinuities;
        (s, e)
    }

    pub fn snapshot(&mut self, now_us: i64) -> TransportSnapshot {
        if let Some(l) = self.local {
            return self.local_snapshot(&l, now_us);
        }
        let b = self.bridge.snapshot(now_us);
        let offset = self.cfg.input_offset_us;
        let show_time = b.song_ms.map(|ms| (ms * 1000.0).round() as i64 + offset);
        let transport = match b.transport {
            BridgeTransport::Playing => TransportState::Playing,
            BridgeTransport::Stopped => TransportState::Stopped,
            BridgeTransport::Unknown | BridgeTransport::Stalled => TransportState::Unknown,
        };
        let stalled = b.transport == BridgeTransport::Stalled;

        // Time side output.
        let (output, dark) = match b.health {
            BridgeHealth::Unlocked => (OutputState::Black, Dark::Loss),
            BridgeHealth::Fault | BridgeHealth::Lost => {
                let age = b.age_us.unwrap_or(i64::MAX);
                if age > self.cfg.blackout_us || show_time.is_none() {
                    (OutputState::Black, Dark::Loss)
                } else {
                    (OutputState::Hold, Dark::No)
                }
            }
            BridgeHealth::Acquiring | BridgeHealth::Locked | BridgeHealth::Holdover => match b.transport {
                BridgeTransport::Playing if b.advancing => (OutputState::Follow, Dark::No),
                BridgeTransport::Stopped if self.cfg.stop_behavior == StopBehavior::Blackout => (OutputState::Black, Dark::Stop),
                _ => (OutputState::Hold, Dark::No),
            },
        };
        if dark == Dark::No && self.dark == Dark::Loss {
            self.recover_from = Some(now_us);
        }
        self.dark = dark;
        let gain = match (output, self.recover_from) {
            (OutputState::Black, _) => 0.0,
            (_, Some(t0)) if self.cfg.recovery_fade_us > 0 && now_us - t0 < self.cfg.recovery_fade_us => {
                ((now_us - t0) as f64 / self.cfg.recovery_fade_us as f64).clamp(0.0, 1.0) as f32
            }
            _ => 1.0,
        };

        let (beat, beat_health) = self.beat(now_us);
        let c = self.clock.snapshot(now_us);
        let beat_seeded = self.cfg.beat_source == Some(BeatSourceKind::MidiClock) && c.origin_seeded;
        let mtc_label = show_time.map(|t| self.cfg.origin.add_us(t).to_string());
        TransportSnapshot {
            now_us,
            show_time_us: show_time,
            advancing: output == OutputState::Follow,
            transport,
            stalled,
            time_health: b.health,
            output,
            output_gain: gain,
            beat,
            beat_health,
            beat_seeded,
            clock_seeded: c.stats.seeded,
            mtc_label,
            generations: self.gens,
        }
    }

    /// A21: always locked; stopped holds the position (shown, not black).
    fn local_snapshot(&self, l: &LocalTime, now_us: i64) -> TransportSnapshot {
        let t = l.clock.position(now_us);
        let playing = l.clock.playing();
        TransportSnapshot {
            now_us,
            show_time_us: Some(t),
            advancing: playing,
            transport: if playing { TransportState::Playing } else { TransportState::Stopped },
            stalled: false,
            time_health: BridgeHealth::Locked,
            output: if playing { OutputState::Follow } else { OutputState::Hold },
            output_gain: 1.0,
            beat: Some(BeatState::new(t as f64 / 1e6 * l.bpm / 60.0, l.bpm)),
            beat_health: BeatHealth::Locked,
            beat_seeded: false,
            clock_seeded: 0,
            mtc_label: Some(self.cfg.origin.add_us(t).to_string()),
            generations: self.gens,
        }
    }

    /// Beat side with its own holdover / blackout (docs/02 "Clockの同期断").
    fn beat(&self, now_us: i64) -> (Option<BeatState>, BeatHealth) {
        let hold = self.cfg.holdover_us;
        let black = self.cfg.blackout_us;
        match self.cfg.beat_source {
            None => (None, BeatHealth::Unselected),
            Some(BeatSourceKind::MidiClock) => {
                let c = self.clock.snapshot(now_us);
                let (Some(beats), Some(age)) = (c.beats, c.age_us) else {
                    return (None, BeatHealth::Unknown);
                };
                let bpm = c.bpm.unwrap_or(0.0);
                match c.transport {
                    ClockTransport::Playing | ClockTransport::Stalled => {
                        // Extrapolate from the last tick up to the holdover limit.
                        let tick_us = if bpm > 0.0 { 60_000_000.0 / (bpm * 24.0) } else { 20_833.0 };
                        let ext = (age.min(hold) as f64 / 1e6) * bpm / 60.0;
                        let health = if (age as f64) <= 3.0 * tick_us {
                            BeatHealth::Locked
                        } else if age <= hold {
                            BeatHealth::Holdover
                        } else {
                            BeatHealth::Lost
                        };
                        let mut b = BeatState::new(beats + ext, bpm);
                        b.phase_valid = age <= black;
                        (Some(b), health)
                    }
                    // Stopped: position known, not advancing.
                    ClockTransport::Stopped => (Some(BeatState::new(beats, bpm)), BeatHealth::Locked),
                    ClockTransport::Unknown => (None, BeatHealth::Unknown),
                }
            }
            Some(BeatSourceKind::Link) => {
                let Some((t_s, s)) = self.link_last else {
                    return (None, BeatHealth::Unknown);
                };
                if !self.link_had_peers {
                    return (None, BeatHealth::Unknown);
                }
                // Link's own timeline keeps running; extrapolate from the last capture.
                let since_sample = (now_us - t_s).max(0);
                match self.link_peers_lost_at {
                    None => (Some(BeatState::new(s.beat + since_sample as f64 / 1e6 * s.tempo / 60.0, s.tempo)), BeatHealth::Locked),
                    Some(lost) => {
                        let age = now_us - lost;
                        let upto = (lost + age.min(hold) - t_s).max(0);
                        let mut b = BeatState::new(s.beat + upto as f64 / 1e6 * s.tempo / 60.0, s.tempo);
                        b.phase_valid = age <= black;
                        (Some(b), if age <= hold { BeatHealth::Holdover } else { BeatHealth::Lost })
                    }
                }
            }
        }
    }
}

/// One-line status for the probe CLI.
pub fn status_line(s: &TransportSnapshot) -> String {
    let t = s.show_time_us.map(|t| format!("{:.3}s", t as f64 / 1e6)).unwrap_or_else(|| "--".into());
    let beat = s
        .beat
        .map(|b| format!("{:.2}@{:.1}bpm{}", b.beat, b.tempo_bpm, if b.phase_valid { "" } else { " (no phase)" }))
        .unwrap_or_else(|| "--".into());
    format!(
        "TRANSPORT {:<8}{} show_time={t} tc={} output={}({:.2}) time={} beat={} [{}]",
        s.transport.name(),
        if s.stalled { " (stalled)" } else { "" },
        s.mtc_label.as_deref().unwrap_or("--:--:--:--"),
        s.output.name(),
        s.output_gain,
        s.time_health.name(),
        beat,
        s.beat_health.name(),
    )
}

pub fn snapshot_json(s: &TransportSnapshot) -> serde_json::Value {
    serde_json::json!({
        "show_time_us": s.show_time_us,
        "advancing": s.advancing,
        "transport": s.transport.name(),
        "stalled": s.stalled,
        "time_health": s.time_health.name(),
        "output": s.output.name(),
        "output_gain": s.output_gain,
        "beat": s.beat.map(|b| b.beat),
        "tempo_bpm": s.beat.map(|b| b.tempo_bpm),
        "beat_phase_valid": s.beat.map(|b| b.phase_valid),
        "beat_health": s.beat_health.name(),
        "beat_seeded": s.beat_seeded,
        "clock_seeded": s.clock_seeded,
        "mtc_label": s.mtc_label,
        "generations": { "bridge": s.generations.bridge, "clock": s.generations.clock, "link": s.generations.link },
    })
}

/// Parse an origin label at a frame rate (helper for CLIs).
pub fn origin_label(label: &str, rate: FrameRate) -> Result<Timecode, String> {
    Timecode::parse(label, rate).map_err(|e| format!("origin {label:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn origin() -> Timecode {
        Timecode::parse("01:00:00:00", FrameRate::Fps30).unwrap()
    }

    struct Sim {
        t: Transport,
        seq: i64,
        ev: Vec<BridgeEvent>,
    }

    impl Sim {
        fn new(cfg: TransportConfig) -> Sim {
            Sim { t: Transport::new(cfg), seq: 0, ev: Vec::new() }
        }
        fn bridge(&mut self, t_us: i64, song_ms: f64, playing: bool) {
            self.seq += 1;
            let p = json!({"v": 2, "seq": self.seq, "kind": "listener", "event": "song_time", "song_ms": song_ms, "is_playing": playing, "errors": []});
            self.t.feed_bridge(BridgeSample::from_payload(t_us, &p), &mut self.ev);
        }
        fn meta_stop(&mut self, t_us: i64, song_ms: f64) {
            self.seq += 1;
            let p = json!({"v": 2, "seq": self.seq, "kind": "meta", "event": "stop", "song_ms": song_ms, "is_playing": true, "errors": []});
            self.t.feed_bridge(BridgeSample::from_payload(t_us, &p), &mut self.ev);
        }
        fn play(&mut self, t0: i64, song0: f64, n: i64) -> (i64, f64) {
            for k in 0..n {
                self.bridge(t0 + k * 16_000, song0 + k as f64 * 16.0, true);
            }
            (t0 + (n - 1) * 16_000, song0 + (n - 1) as f64 * 16.0)
        }
    }

    #[test]
    fn show_time_is_song_time_plus_offset_and_label_adds_origin() {
        let mut s = Sim::new(TransportConfig::defaults(origin(), 40_000, None));
        let (t, _) = s.play(0, 10_000.0, 5); // last sample song 10064 ms at 64 ms
        let snap = s.t.snapshot(t + 10_000);
        assert_eq!(snap.show_time_us, Some(10_074_000 + 40_000));
        assert_eq!(snap.output, OutputState::Follow);
        assert_eq!(snap.transport, TransportState::Playing);
        assert_eq!(snap.mtc_label.as_deref(), Some("01:00:10:03"));
        assert_eq!(snap.beat_health, BeatHealth::Unselected);
    }

    #[test]
    fn unlocked_is_black_and_loss_holds_then_blacks_out_then_recovers_with_fade() {
        let mut s = Sim::new(TransportConfig::defaults(origin(), 0, None));
        assert_eq!(s.t.snapshot(0).output, OutputState::Black);
        let (t, song) = s.play(0, 0.0, 10);
        // Within holdover: still following.
        assert_eq!(s.t.snapshot(t + 200_000).output, OutputState::Follow);
        // Lost: hold the frozen position.
        let lost = s.t.snapshot(t + 400_000);
        assert_eq!((lost.output, lost.output_gain), (OutputState::Hold, 1.0));
        assert_eq!(lost.show_time_us, Some(((song + 250.0) * 1000.0) as i64));
        // Beyond blackout: black.
        assert_eq!(s.t.snapshot(t + 1_100_000).output, OutputState::Black);
        // Recovery: two consistent samples, then a 100 ms fade.
        let t2 = t + 2_000_000;
        s.play(t2, 5_000.0, 2);
        let r0 = s.t.snapshot(t2 + 16_000);
        assert_eq!(r0.output, OutputState::Follow);
        assert!(r0.output_gain < 0.01, "{}", r0.output_gain);
        assert!((s.t.snapshot(t2 + 66_000).output_gain - 0.5).abs() < 0.01);
        assert_eq!(s.t.snapshot(t2 + 200_000).output_gain, 1.0);
    }

    #[test]
    fn stop_holds_or_blacks_out_per_stop_behavior_without_recovery_fade() {
        for (behavior, expect) in [(StopBehavior::Hold, OutputState::Hold), (StopBehavior::Blackout, OutputState::Black)] {
            let mut cfg = TransportConfig::defaults(origin(), 0, None);
            cfg.stop_behavior = behavior;
            let mut s = Sim::new(cfg);
            let (t, song) = s.play(0, 0.0, 5);
            s.bridge(t + 16_000, song + 16.0, false);
            s.bridge(t + 116_000, song + 16.0, false);
            let snap = s.t.snapshot(t + 120_000);
            assert_eq!((snap.transport, snap.output), (TransportState::Stopped, expect), "{behavior:?}");
            // Play again: no recovery fade after a normal Stop.
            s.play(t + 200_000, song + 16.0, 3);
            assert_eq!(s.t.snapshot(t + 240_000).output_gain, 1.0, "{behavior:?}");
        }
    }

    #[test]
    fn script_stop_is_lost_at_once() {
        let mut s = Sim::new(TransportConfig::defaults(origin(), 0, None));
        let (t, song) = s.play(0, 0.0, 5);
        s.meta_stop(t + 10_000, song);
        let snap = s.t.snapshot(t + 20_000);
        assert_eq!(snap.time_health, BridgeHealth::Lost);
        assert_eq!(snap.output, OutputState::Hold);
    }

    #[test]
    fn midi_clock_beat_has_its_own_holdover_and_blackout() {
        let mut s = Sim::new(TransportConfig::defaults(origin(), 0, Some(BeatSourceKind::MidiClock)));
        let mut ev = Vec::new();
        assert_eq!(s.t.snapshot(0).beat_health, BeatHealth::Unknown);
        s.t.feed_clock(0, &[0xFA], &mut ev);
        let tick = 20_833;
        for k in 1..=48 {
            s.t.feed_clock(k * tick, &[0xF8], &mut ev);
        }
        let last = 48 * tick;
        let locked = s.t.snapshot(last + 10_000);
        assert_eq!(locked.beat_health, BeatHealth::Locked);
        let b = locked.beat.unwrap();
        assert!((b.beat - (2.0 + 0.01 * 2.0)).abs() < 0.01, "{b:?}"); // 2 beats + 10 ms at 120 bpm
        assert_eq!(s.t.snapshot(last + 150_000).beat_health, BeatHealth::Holdover);
        let lost = s.t.snapshot(last + 600_000);
        assert_eq!(lost.beat_health, BeatHealth::Lost);
        assert!(lost.beat.unwrap().phase_valid);
        assert!((lost.beat.unwrap().beat - 2.5).abs() < 0.01); // held at the 250 ms holdover limit
        assert!(!s.t.snapshot(last + 1_100_000).beat.unwrap().phase_valid);
        // The time side is independent (no bridge at all here).
        assert_eq!(s.t.snapshot(last).output, OutputState::Black);
    }

    #[test]
    fn link_beat_without_peers_is_unknown_and_peer_loss_holds() {
        let mut s = Sim::new(TransportConfig::defaults(origin(), 0, Some(BeatSourceKind::Link)));
        let sample = |peers, beat| LinkSample { link_clock_us: 0, peers, tempo: 120.0, beat, phase: beat % 4.0, is_playing: false };
        s.t.feed_link_sample(0, sample(0, 1.0));
        assert_eq!(s.t.snapshot(10).beat_health, BeatHealth::Unknown);
        s.t.feed_link_sample(1_000_000, sample(1, 10.0));
        let b = s.t.snapshot(1_500_000);
        assert_eq!(b.beat_health, BeatHealth::Locked);
        assert!((b.beat.unwrap().beat - 11.0).abs() < 1e-9);
        s.t.feed_link_raw(2_000_000, LinkRaw::Peers(0));
        assert_eq!(s.t.snapshot(2_100_000).beat_health, BeatHealth::Holdover);
        let lost = s.t.snapshot(2_600_000);
        assert_eq!(lost.beat_health, BeatHealth::Lost);
        assert!((lost.beat.unwrap().beat - (10.0 + 1.25 * 2.0)).abs() < 1e-9); // held at loss + 250 ms
        assert!(!s.t.snapshot(3_100_000).beat.unwrap().phase_valid);
    }

    #[test]
    fn reconnect_discards_the_old_state_t03() {
        let mut s = Sim::new(TransportConfig::defaults(origin(), 0, Some(BeatSourceKind::MidiClock)));
        let mut ev = Vec::new();
        s.t.feed_clock(0, &[0xFA], &mut ev);
        for k in 1..=24 {
            s.t.feed_clock(k * 20_833, &[0xF8], &mut ev);
        }
        s.play(0, 0.0, 3);
        assert!(s.t.snapshot(500_000).beat.is_some());
        s.t.reconnect_clock();
        s.t.reconnect_bridge();
        let snap = s.t.snapshot(500_000);
        assert_eq!(snap.beat, None);
        assert_eq!(snap.time_health, BridgeHealth::Unlocked);
        assert_eq!(snap.generations, Generations { bridge: 1, clock: 1, link: 0 });
        // Ticks after the reconnect without Start / SPP give no phase.
        for k in 0..30 {
            s.t.feed_clock(600_000 + k * 20_833, &[0xF8], &mut ev);
        }
        assert_eq!(s.t.snapshot(1_300_000).beat_health, BeatHealth::Unknown);
    }

    /// One bridge sample at `t_us` while Live plays at `tempo` (song 10 s at t = 0).
    fn beat_sample(s: &mut Sim, t_us: i64, tempo: f64) {
        s.seq += 1;
        let song = 10_000.0 + t_us as f64 / 1000.0;
        let p = json!({"v": 2, "seq": s.seq, "kind": "listener", "event": "song_time", "song_ms": song,
            "beats": song / 1000.0 * tempo / 60.0, "tempo": tempo, "is_playing": true, "errors": []});
        s.t.feed_bridge(BridgeSample::from_payload(t_us, &p), &mut s.ev);
    }

    /// Bridge samples every 16 ms up to t = 0, then clock ticks k = 0..n at
    /// 120 bpm (beat = 20 + k / 24 at 120 bpm), each right after a bridge
    /// sample as in Live (samples every ~12 ms keep the bridge locked).
    fn join_running(s: &mut Sim, tempo: f64, n: i64, ev: &mut Vec<ClockEvent>) {
        for k in (1..40).rev() {
            beat_sample(s, -k * 16_000, tempo);
        }
        for k in 0..n {
            beat_sample(s, k * 20_833, tempo);
            s.t.feed_clock(k * 20_833, &[0xF8], ev);
        }
    }

    #[test]
    fn joining_a_running_clock_seeds_the_origin_from_the_bridge_once() {
        let mut s = Sim::new(TransportConfig::defaults(origin(), 0, Some(BeatSourceKind::MidiClock)));
        let mut ev = Vec::new();
        // Ticks without Start / SPP while the bridge plays at 120 bpm.
        let tick = 20_833;
        join_running(&mut s, 120.0, 12, &mut ev);
        let seeds: Vec<_> = ev.iter().filter_map(|e| if let ClockEvent::Seeded { ticks } = e { Some(*ticks) } else { None }).collect();
        assert_eq!(seeds.len(), 1, "{ev:?}");
        // The bpm estimate needs 6 intervals: seeded on the 7th tick (k = 6), beat 20.25.
        assert_eq!(seeds[0], 20 * 24 + 6);
        let snap = s.t.snapshot(11 * tick);
        assert!(snap.beat_seeded);
        assert_eq!(snap.clock_seeded, 1);
        assert_eq!(snap.beat_health, BeatHealth::Locked);
        let b = snap.beat.unwrap();
        assert!(b.phase_valid);
        assert!((b.beat - (20.0 + 11.0 / 24.0)).abs() < 0.01, "{b:?}");
        // Once only: later ticks do not seed again.
        ev.clear();
        for k in 12..60 {
            s.t.feed_clock(k * tick, &[0xF8], &mut ev);
        }
        assert!(ev.is_empty(), "{ev:?}");
        assert_eq!(s.t.snapshot(59 * tick).clock_seeded, 1);
    }

    #[test]
    fn no_seed_without_a_playing_bridge_or_with_another_tempo() {
        let tick = 20_833; // 120 bpm clock
        // (a) bridge not playing, (b) bridge at 100 bpm (> 5 % off), (c) no bridge at all.
        for case in ["stopped", "tempo", "none"] {
            let mut s = Sim::new(TransportConfig::defaults(origin(), 0, Some(BeatSourceKind::MidiClock)));
            let mut ev = Vec::new();
            if case == "stopped" {
                s.play(0, 10_000.0, 5);
                s.bridge(80_000, 10_064.0, false);
                s.bridge(96_000, 10_064.0, false);
            }
            if case == "tempo" {
                join_running(&mut s, 100.0, 30, &mut ev);
            } else {
                for k in 0..30 {
                    s.t.feed_clock(100_000 + k * tick, &[0xF8], &mut ev);
                }
            }
            assert!(!ev.iter().any(|e| matches!(e, ClockEvent::Seeded { .. })), "{case}: {ev:?}");
            let snap = s.t.snapshot(if case == "tempo" { 29 * tick } else { 100_000 + 29 * tick });
            assert_eq!((snap.beat_seeded, snap.clock_seeded, snap.beat), (false, 0, None), "{case}");
        }
        // The Link beat source never seeds the clock.
        let mut s = Sim::new(TransportConfig::defaults(origin(), 0, Some(BeatSourceKind::Link)));
        let mut ev = Vec::new();
        join_running(&mut s, 120.0, 30, &mut ev);
        assert_eq!(s.t.snapshot(29 * tick).clock_seeded, 0);
    }

    #[test]
    fn a_real_song_position_replaces_the_seed() {
        let mut s = Sim::new(TransportConfig::defaults(origin(), 0, Some(BeatSourceKind::MidiClock)));
        let mut ev = Vec::new();
        let tick = 20_833;
        join_running(&mut s, 120.0, 12, &mut ev);
        assert!(s.t.snapshot(11 * tick).beat_seeded);
        // Live stops and plays from bar 2: SPP 16 sixteenths (= beat 4), Continue, ticks.
        s.t.feed_clock(12 * tick, &[0xFC], &mut ev);
        s.t.feed_clock(13 * tick, &[0xF2, 16, 0], &mut ev);
        s.t.feed_clock(13 * tick + 1, &[0xFB], &mut ev);
        for k in 14..20 {
            s.t.feed_clock(k * tick, &[0xF8], &mut ev);
        }
        let snap = s.t.snapshot(19 * tick);
        assert!(!snap.beat_seeded);
        assert_eq!(snap.clock_seeded, 1);
        assert!((snap.beat.unwrap().beat - (4.0 + 6.0 / 24.0)).abs() < 0.01, "{:?}", snap.beat);
    }

    #[test]
    fn config_from_the_demo_show() {
        let show: Show = serde_json::from_str(include_str!("../../../../examples/demo.show.json")).unwrap();
        let cfg = TransportConfig::from_show(&show).unwrap();
        assert_eq!(cfg.beat_source, Some(BeatSourceKind::Link));
        assert_eq!((cfg.holdover_us, cfg.blackout_us, cfg.recovery_fade_us), (250_000, 1_000_000, 100_000));
        assert_eq!(cfg.origin.to_string(), "01:00:00:00");
        assert_eq!(cfg.origin_plus_offset_us(), 3_600_000_000.0);
    }
}
