//! MIDI Clock decoder (alternative beat input). Pure state machine.
//!
//! * F8 ticks: 24 per quarter note. BPM is estimated from the last 24
//!   intervals, so it needs one quarter note of ticks before it is shown.
//! * FA Start resets the position to 0 and sets playing. FB Continue sets
//!   playing and keeps the position. FC Stop sets stopped.
//! * F2 Song Position Pointer is in 16th notes (6 ticks each).
//! * The quarter-note phase is only valid after Start or SPP. Joining a
//!   running clock stream gives a BPM estimate but no phase, unless the
//!   caller seeds the origin from another source (`seed_origin`; the
//!   transport does it once from the bridge's beats). Start or SPP replaces
//!   a seeded origin.
//! * Ticks received while stopped do not advance the position.
//! * Playing with no tick for `stall_us` (provisional 500 ms) is reported as
//!   `stalled`, not `playing`. Live stops the ticks without a Stop when its
//!   Clock Sync output is turned off (C7). BPM and beats keep their last
//!   values. A tick returns the transport to `playing` without Start or
//!   Continue, as Live does when Sync is turned back on.

use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Unknown,
    Playing,
    /// Playing per the last transport message, but ticks have stopped.
    Stalled,
    Stopped,
}

impl Transport {
    pub fn name(self) -> &'static str {
        match self {
            Transport::Unknown => "unknown",
            Transport::Playing => "playing",
            Transport::Stalled => "stalled",
            Transport::Stopped => "stopped",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockEvent {
    Start,
    Continue,
    Stop,
    SongPosition { sixteenths: u16 },
    /// No tick for `silent_us` while playing.
    Stalled { silent_us: i64 },
    /// Ticks came back after a stall.
    Resumed { silent_us: i64 },
    /// The origin was set from outside (no Start / SPP yet): `ticks` since it.
    Seeded { ticks: u64 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClockStats {
    pub ticks: u64,
    pub ticks_while_stopped: u64,
    pub starts: u64,
    pub continues: u64,
    pub stops: u64,
    pub song_positions: u64,
    pub stalls: u64,
    /// Origins set by `seed_origin`.
    pub seeded: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockSnapshot {
    pub bpm: Option<f64>,
    pub transport: Transport,
    /// True once Start or SPP fixed the tick origin.
    pub position_valid: bool,
    /// Quarter-note phase 0..1, only when `position_valid`.
    pub quarter_phase: Option<f64>,
    /// Quarter notes since the origin, only when `position_valid`.
    pub beats: Option<f64>,
    pub age_us: Option<i64>,
    /// The origin came from `seed_origin` and no Start / SPP has replaced it.
    pub origin_seeded: bool,
    pub stats: ClockStats,
}

/// Default tick silence after which a playing clock is `stalled`.
pub const DEFAULT_STALL_US: i64 = 500_000;

pub struct ClockDecoder {
    stall_us: i64,
    last_tick_us: Option<i64>,
    /// Last tick, Start or Continue: the reference for the stall timer.
    last_progress_us: Option<i64>,
    /// A `Stalled` event was emitted and no tick has come since.
    stall_reported: bool,
    intervals: VecDeque<i64>,
    ticks_since_origin: Option<u64>,
    origin_seeded: bool,
    transport: Transport,
    last_msg_us: Option<i64>,
    stats: ClockStats,
}

impl Default for ClockDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockDecoder {
    pub fn new() -> Self {
        Self::with_stall_us(DEFAULT_STALL_US)
    }

    pub fn with_stall_us(stall_us: i64) -> Self {
        ClockDecoder {
            stall_us,
            last_tick_us: None,
            last_progress_us: None,
            stall_reported: false,
            intervals: VecDeque::with_capacity(25),
            ticks_since_origin: None,
            origin_seeded: false,
            transport: Transport::Unknown,
            last_msg_us: None,
            stats: ClockStats::default(),
        }
    }

    pub fn stats(&self) -> ClockStats {
        self.stats
    }

    pub fn stall_us(&self) -> i64 {
        self.stall_us
    }

    /// True once Start, SPP or a seed fixed the tick origin.
    pub fn origin_known(&self) -> bool {
        self.ticks_since_origin.is_some()
    }

    /// Set the origin from outside while none is known (joining a running
    /// clock): `ticks` since the origin at the last tick, transport playing.
    /// Ignored once Start or SPP has set it. Start / SPP later replace it.
    pub fn seed_origin(&mut self, ticks: u64, out: &mut Vec<ClockEvent>) {
        if self.ticks_since_origin.is_some() {
            return;
        }
        self.ticks_since_origin = Some(ticks);
        self.origin_seeded = true;
        self.transport = Transport::Playing;
        self.stats.seeded += 1;
        out.push(ClockEvent::Seeded { ticks });
    }

    fn is_stalled(&self, now_us: i64) -> bool {
        self.transport == Transport::Playing
            && self.last_progress_us.is_some_and(|t| now_us - t > self.stall_us)
    }

    /// Report a stall once when the tick silence crosses `stall_us`. Call it
    /// periodically (and before each `feed`); `snapshot` does not need it.
    pub fn poll(&mut self, now_us: i64, out: &mut Vec<ClockEvent>) {
        if !self.stall_reported && self.is_stalled(now_us) {
            self.stall_reported = true;
            self.stats.stalls += 1;
            let silent_us = now_us - self.last_progress_us.expect("stalled implies progress");
            out.push(ClockEvent::Stalled { silent_us });
        }
    }

    pub fn feed(&mut self, t_us: i64, bytes: &[u8], out: &mut Vec<ClockEvent>) {
        let Some(status) = bytes.first().copied() else {
            return;
        };
        self.poll(t_us, out);
        match status {
            0xF8 => {
                if self.stall_reported {
                    self.stall_reported = false;
                    let silent_us = t_us - self.last_progress_us.expect("stall implies progress");
                    out.push(ClockEvent::Resumed { silent_us });
                }
                self.last_progress_us = Some(t_us);
                self.stats.ticks += 1;
                if let Some(last) = self.last_tick_us {
                    let d = t_us - last;
                    if d > 0 {
                        self.intervals.push_back(d);
                        while self.intervals.len() > 24 {
                            self.intervals.pop_front();
                        }
                    }
                }
                self.last_tick_us = Some(t_us);
                if self.transport == Transport::Playing {
                    if let Some(n) = self.ticks_since_origin.as_mut() {
                        *n += 1;
                    }
                } else {
                    self.stats.ticks_while_stopped += 1;
                }
                self.last_msg_us = Some(t_us);
            }
            0xFA => {
                self.stats.starts += 1;
                self.ticks_since_origin = Some(0);
                self.origin_seeded = false;
                self.transport = Transport::Playing;
                self.last_progress_us = Some(t_us);
                self.stall_reported = false;
                self.last_msg_us = Some(t_us);
                out.push(ClockEvent::Start);
            }
            0xFB => {
                self.stats.continues += 1;
                self.transport = Transport::Playing;
                self.last_progress_us = Some(t_us);
                self.stall_reported = false;
                self.last_msg_us = Some(t_us);
                out.push(ClockEvent::Continue);
            }
            0xFC => {
                self.stats.stops += 1;
                self.transport = Transport::Stopped;
                self.stall_reported = false;
                self.last_msg_us = Some(t_us);
                out.push(ClockEvent::Stop);
            }
            0xF2 if bytes.len() >= 3 => {
                let sixteenths = (bytes[1] as u16 & 0x7F) | ((bytes[2] as u16 & 0x7F) << 7);
                self.stats.song_positions += 1;
                self.ticks_since_origin = Some(sixteenths as u64 * 6);
                self.origin_seeded = false;
                self.last_msg_us = Some(t_us);
                out.push(ClockEvent::SongPosition { sixteenths });
            }
            _ => {}
        }
    }

    pub fn snapshot(&self, now_us: i64) -> ClockSnapshot {
        let bpm = if self.intervals.len() >= 6 {
            let sum: i64 = self.intervals.iter().sum();
            let n = self.intervals.len() as f64;
            Some(60_000_000.0 * n / (24.0 * sum as f64))
        } else {
            None
        };
        let (quarter_phase, beats) = match self.ticks_since_origin {
            Some(n) => (Some((n % 24) as f64 / 24.0), Some(n as f64 / 24.0)),
            None => (None, None),
        };
        let transport = if self.is_stalled(now_us) { Transport::Stalled } else { self.transport };
        ClockSnapshot {
            bpm,
            transport,
            position_valid: self.ticks_since_origin.is_some(),
            quarter_phase,
            beats,
            age_us: self.last_msg_us.map(|t| now_us - t),
            origin_seeded: self.origin_seeded,
            stats: self.stats,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK_120: i64 = 20_833; // 60e6 / (120 * 24), truncated

    fn ticks(dec: &mut ClockDecoder, t0: i64, n: usize, out: &mut Vec<ClockEvent>) -> i64 {
        for i in 0..n {
            dec.feed(t0 + i as i64 * TICK_120, &[0xF8], out);
        }
        t0 + n as i64 * TICK_120
    }

    #[test]
    fn start_then_ticks_gives_bpm_phase_and_beats() {
        let mut dec = ClockDecoder::new();
        let mut out = Vec::new();
        dec.feed(0, &[0xFA], &mut out);
        assert_eq!(out, vec![ClockEvent::Start]);
        let t = ticks(&mut dec, 0, 48, &mut out);
        let s = dec.snapshot(t);
        assert_eq!(s.transport, Transport::Playing);
        assert!(s.position_valid);
        assert_eq!(s.beats, Some(2.0));
        assert_eq!(s.quarter_phase, Some(0.0));
        let bpm = s.bpm.expect("bpm after 24 intervals");
        assert!((bpm - 120.0).abs() < 0.1, "bpm {bpm}");
        assert_eq!(s.stats.ticks, 48);
    }

    #[test]
    fn joining_a_running_stream_has_bpm_but_no_phase() {
        let mut dec = ClockDecoder::new();
        let mut out = Vec::new();
        let t = ticks(&mut dec, 0, 30, &mut out);
        let s = dec.snapshot(t);
        assert_eq!(s.transport, Transport::Unknown);
        assert!(!s.position_valid);
        assert_eq!(s.quarter_phase, None);
        assert!(s.bpm.is_some());
        assert_eq!(s.stats.ticks_while_stopped, 30);
    }

    #[test]
    fn stop_holds_position_and_continue_resumes() {
        let mut dec = ClockDecoder::new();
        let mut out = Vec::new();
        dec.feed(0, &[0xFA], &mut out);
        let t = ticks(&mut dec, 0, 12, &mut out);
        dec.feed(t, &[0xFC], &mut out);
        let t2 = ticks(&mut dec, t, 6, &mut out); // ticks while stopped
        let s = dec.snapshot(t2);
        assert_eq!(s.transport, Transport::Stopped);
        assert_eq!(s.beats, Some(0.5));
        assert_eq!(s.stats.ticks_while_stopped, 6);
        dec.feed(t2, &[0xFB], &mut out);
        let t3 = ticks(&mut dec, t2, 12, &mut out);
        let s = dec.snapshot(t3);
        assert_eq!(s.transport, Transport::Playing);
        assert_eq!(s.beats, Some(1.0));
        assert_eq!(out, vec![ClockEvent::Start, ClockEvent::Stop, ClockEvent::Continue]);
    }

    #[test]
    fn tick_silence_while_playing_is_stalled_then_resumes_without_start() {
        let mut dec = ClockDecoder::new();
        let mut out = Vec::new();
        dec.feed(0, &[0xFA], &mut out);
        let t = ticks(&mut dec, 0, 48, &mut out);
        let last_tick = t - TICK_120;
        assert_eq!(dec.snapshot(last_tick + DEFAULT_STALL_US).transport, Transport::Playing);
        let s = dec.snapshot(last_tick + DEFAULT_STALL_US + 1);
        assert_eq!(s.transport, Transport::Stalled);
        // Last values are kept while stalled.
        assert_eq!(s.beats, Some(2.0));
        assert!(s.bpm.is_some());
        assert_eq!(s.age_us, Some(DEFAULT_STALL_US + 1));
        out.clear();
        dec.poll(last_tick + 600_000, &mut out);
        dec.poll(last_tick + 700_000, &mut out);
        assert_eq!(out, vec![ClockEvent::Stalled { silent_us: 600_000 }]);
        out.clear();
        // Ticks resume without Start / Continue (Live Sync back on).
        let t2 = last_tick + 6_000_000;
        let t3 = ticks(&mut dec, t2, 24, &mut out);
        assert_eq!(out, vec![ClockEvent::Resumed { silent_us: 6_000_000 }]);
        let s = dec.snapshot(t3);
        assert_eq!(s.transport, Transport::Playing);
        assert_eq!(s.beats, Some(3.0));
        assert_eq!(s.stats.stalls, 1);
    }

    #[test]
    fn stop_and_unknown_are_never_stalled() {
        let mut dec = ClockDecoder::new();
        let mut out = Vec::new();
        let t = ticks(&mut dec, 0, 30, &mut out); // joined mid-stream: unknown
        assert_eq!(dec.snapshot(t + 5_000_000).transport, Transport::Unknown);
        dec.feed(t, &[0xFA], &mut out);
        dec.feed(t + 10_000, &[0xFC], &mut out);
        dec.poll(t + 5_000_000, &mut out);
        assert_eq!(dec.snapshot(t + 5_000_000).transport, Transport::Stopped);
        assert_eq!(dec.stats().stalls, 0);
    }

    #[test]
    fn start_without_ticks_stalls_after_threshold() {
        let mut dec = ClockDecoder::new();
        let mut out = Vec::new();
        dec.feed(1_000, &[0xFB], &mut out);
        assert_eq!(dec.snapshot(1_000 + 400_000).transport, Transport::Playing);
        assert_eq!(dec.snapshot(1_000 + 501_000).transport, Transport::Stalled);
    }

    #[test]
    fn song_position_sets_origin_in_sixteenths() {
        let mut dec = ClockDecoder::new();
        let mut out = Vec::new();
        dec.feed(0, &[0xF2, 16, 0], &mut out); // 16 sixteenths = 4 beats
        assert_eq!(out, vec![ClockEvent::SongPosition { sixteenths: 16 }]);
        dec.feed(1, &[0xFB], &mut out);
        let s = dec.snapshot(2);
        assert_eq!(s.beats, Some(4.0));
        assert_eq!(s.quarter_phase, Some(0.0));
        dec.feed(2, &[0xF2, 0x00, 0x01], &mut out); // 128 sixteenths
        assert_eq!(dec.snapshot(3).beats, Some(32.0));
    }
}
