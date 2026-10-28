//! MTC generator state machine (docs/10-live-bridge.md "MTC出力"). Pure: the
//! caller passes the time and the desired state and sends what comes back.
//!
//! * Running: Quarter Frames every `1/(fps*4)`. Each 8-piece sequence carries
//!   the label of the frame at which its piece 0 is sent, and consecutive
//!   sequences advance by 2 frames. The schedule runs on its own timeline and
//!   is slewed toward the source position (10 % of the error per sequence),
//!   so source jitter does not reach the labels.
//! * Start, jump (epoch change) or drift beyond 1 frame: Full Frame of the
//!   current frame, then the sequence restarts at the next frame boundary
//!   (no ~0.45 s wait as with LiveMTC).
//! * Hold (stopped): Quarter Frames stop, one Full Frame of the held
//!   position; another Full Frame whenever the held frame moves by more than 1.
//! * Off (source lost, or acquiring right after a jump): nothing is sent.
//!   Recovery starts with a Full Frame.
//! * Full Frame reasons: `start` (Play, or recovery without a jump), `jump`
//!   (the source's discontinuity epoch changed since the last run while
//!   playing, i.e. seek / loop / manual tempo change), `resync` (drift beyond
//!   one frame), `stop`, `hold` (first or moved position while stopped).
//! * 24 / 25 / 30 NDF only.

use crate::sync::bridge::{BridgeHealth, BridgeSnapshot, BridgeTransport};
use crate::sync::mtc::{encode_full_frame, encode_quarter_frames, FrameRate, Label};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Desired {
    /// Playing; label position (us since 00:00:00:00) at `now`.
    Run { pos_us: f64 },
    /// Stopped (or stalled); held label position.
    Hold { pos_us: f64 },
    /// No usable source: send nothing.
    Off,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GenOut {
    FullFrame { label: Label, reason: &'static str, bytes: [u8; 10] },
    Qf { piece: u8, data: u8, label: Label, t_sched_us: f64 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GenStats {
    pub quarter_frames: u64,
    pub full_frames: u64,
    pub starts: u64,
    pub jumps: u64,
    pub resyncs: u64,
    pub stops: u64,
}

#[derive(Clone, Copy, Debug)]
struct Run {
    t0: f64,
    frame0: u64,
    n: u64,
    epoch: u64,
}

pub struct MtcGen {
    rate: FrameRate,
    frame_us: f64,
    qf_us: f64,
    run: Option<Run>,
    held_frame: Option<u64>,
    /// Epoch of the last run, kept across `Off` and cleared by a stop.
    last_run_epoch: Option<u64>,
    pub slew: f64,
    pub stats: GenStats,
}

impl MtcGen {
    pub fn new(rate: FrameRate) -> Result<MtcGen, String> {
        if rate.is_drop_frame() {
            return Err(format!("MTC output supports 24 / 25 / 30 NDF, not {}", rate.name()));
        }
        let fps = rate.nominal() as f64;
        Ok(MtcGen { rate, frame_us: 1e6 / fps, qf_us: 1e6 / (fps * 4.0), run: None, held_frame: None, last_run_epoch: None, slew: 0.1, stats: GenStats::default() })
    }

    pub fn rate(&self) -> FrameRate {
        self.rate
    }

    pub fn is_running(&self) -> bool {
        self.run.is_some()
    }

    fn frame_at(&self, pos_us: f64) -> u64 {
        // The epsilon keeps exact frame times (e.g. 01:00:00:00) from flooring
        // to the previous frame through f64 rounding.
        (pos_us / self.frame_us + 1e-6).floor().max(0.0) as u64
    }

    fn label(&self, frame: u64) -> Label {
        Label::from_frame_index(frame, self.rate)
    }

    fn full_frame(&mut self, frame: u64, reason: &'static str, out: &mut Vec<GenOut>) {
        let label = self.label(frame);
        self.stats.full_frames += 1;
        out.push(GenOut::FullFrame { label, reason, bytes: encode_full_frame(label) });
    }

    fn start(&mut self, now: f64, pos_us: f64, epoch: u64, reason: &'static str, out: &mut Vec<GenOut>) -> f64 {
        match reason {
            "start" => self.stats.starts += 1,
            "jump" => self.stats.jumps += 1,
            _ => self.stats.resyncs += 1,
        }
        let f = self.frame_at(pos_us);
        self.full_frame(f, reason, out);
        let b = f + 1;
        let t0 = now + (b as f64 * self.frame_us - pos_us);
        self.run = Some(Run { t0, frame0: b, n: 0, epoch });
        self.held_frame = None;
        self.last_run_epoch = Some(epoch);
        t0
    }

    /// Advance to `now_us`. Returns the next time to call again.
    pub fn step(&mut self, now_us: f64, desired: Desired, epoch: u64, out: &mut Vec<GenOut>) -> f64 {
        match desired {
            Desired::Off => {
                self.run = None;
                self.held_frame = None;
                now_us + 5_000.0
            }
            Desired::Hold { pos_us } => {
                let f = self.frame_at(pos_us);
                self.last_run_epoch = None;
                if self.run.take().is_some() {
                    self.stats.stops += 1;
                    self.full_frame(f, "stop", out);
                    self.held_frame = Some(f);
                } else if self.held_frame.map_or(true, |h| (h as i64 - f as i64).abs() > 1) {
                    self.full_frame(f, "hold", out);
                    self.held_frame = Some(f);
                }
                now_us + 10_000.0
            }
            Desired::Run { pos_us } => {
                let Some(run) = self.run else {
                    // Coming back from Off: a jump if the epoch moved while we were running.
                    let reason = match self.last_run_epoch {
                        Some(e) if e != epoch => "jump",
                        _ => "start",
                    };
                    return self.start(now_us, pos_us, epoch, reason, out);
                };
                if run.epoch != epoch {
                    return self.start(now_us, pos_us, epoch, "jump", out);
                }
                let mut run = run;
                let t = run.t0 + run.n as f64 * self.qf_us;
                if now_us < t {
                    return t;
                }
                let piece = (run.n % 8) as u8;
                let seq = run.n / 8;
                let frame = run.frame0 + 2 * seq;
                if piece == 0 {
                    // Source position at the scheduled time vs this sequence's frame.
                    let err = (pos_us - (now_us - t)) - frame as f64 * self.frame_us;
                    if err.abs() > self.frame_us {
                        return self.start(now_us, pos_us, epoch, "resync", out);
                    }
                    run.t0 -= err * self.slew;
                }
                let label = self.label(frame);
                let data = encode_quarter_frames(label)[piece as usize];
                self.stats.quarter_frames += 1;
                out.push(GenOut::Qf { piece, data, label, t_sched_us: t });
                run.n += 1;
                self.run = Some(run);
                run.t0 + run.n as f64 * self.qf_us
            }
        }
    }
}

/// After `is_playing` turns true, wait this long for the first position
/// update before the start Full Frame (Live moves to the start position a few
/// ms after reporting Play).
pub const PLAY_SETTLE_US: i64 = 50_000;

/// What the generator should do for a bridge snapshot at `now_us`.
pub fn desired_from(s: &BridgeSnapshot, origin_plus_offset_us: f64, now_us: i64) -> Desired {
    let pos = |ms: f64| ms * 1000.0 + origin_plus_offset_us;
    if s.transport == BridgeTransport::Playing
        && s.play_samples < 2
        && s.play_since_us.is_some_and(|t| now_us - t < PLAY_SETTLE_US)
    {
        return Desired::Off;
    }
    match (s.health, s.transport, s.song_ms) {
        (BridgeHealth::Locked | BridgeHealth::Holdover, BridgeTransport::Playing, Some(ms)) if s.advancing => Desired::Run { pos_us: pos(ms) },
        // Playing but not moving (Live waiting to start, or stalling): hold.
        (BridgeHealth::Locked | BridgeHealth::Holdover, BridgeTransport::Playing, Some(ms)) => Desired::Hold { pos_us: pos(ms) },
        (BridgeHealth::Locked | BridgeHealth::Acquiring | BridgeHealth::Holdover, BridgeTransport::Stopped | BridgeTransport::Stalled, Some(ms)) => {
            Desired::Hold { pos_us: pos(ms) }
        }
        _ => Desired::Off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::bridge::BridgeStats;
    use crate::sync::mtc::{MtcDecoder, MtcEvent, Params, SyncHealth};

    const H1: f64 = 3_600_000_000.0; // 01:00:00:00

    /// Drive the generator with an ideal clock: the source position is
    /// `pos0 + (t - t_play)` while playing. Feeds everything into an MTC
    /// decoder (the receiver) at the scheduled times.
    struct Bench {
        g: MtcGen,
        rx: MtcDecoder,
        ev: Vec<MtcEvent>,
        sent: Vec<(f64, GenOut)>,
        t: f64,
        wake: f64,
    }

    impl Bench {
        fn new() -> Self {
            Bench { g: MtcGen::new(FrameRate::Fps30).unwrap(), rx: MtcDecoder::new(Params::default()), ev: Vec::new(), sent: Vec::new(), t: 0.0, wake: 0.0 }
        }
        fn run_until(&mut self, end: f64, desired: impl Fn(f64) -> Desired, epoch: impl Fn(f64) -> u64) {
            while self.t < end {
                self.t = self.wake.max(self.t);
                if self.t >= end {
                    break;
                }
                let mut out = Vec::new();
                self.wake = self.g.step(self.t, desired(self.t), epoch(self.t), &mut out);
                for o in out {
                    let bytes: Vec<u8> = match &o {
                        GenOut::FullFrame { bytes, .. } => bytes.to_vec(),
                        GenOut::Qf { data, .. } => vec![0xF1, *data],
                    };
                    self.rx.feed(self.t as i64, &bytes, &mut self.ev);
                    self.sent.push((self.t, o));
                }
                if self.wake <= self.t {
                    self.wake = self.t + 1.0;
                }
            }
        }
    }

    #[test]
    fn rejects_drop_frame() {
        assert!(MtcGen::new(FrameRate::Fps2997Df).is_err());
    }

    #[test]
    fn play_starts_with_full_frame_and_receiver_locks_within_two_sequences() {
        let mut b = Bench::new();
        let pos0 = H1 + 10_000.0; // 01:00:00:00 + 10 ms
        b.run_until(1_000_000.0, |t| Desired::Run { pos_us: pos0 + t }, |_| 0);
        assert!(matches!(&b.sent[0].1, GenOut::FullFrame { reason: "start", label, .. } if label.to_string() == "01:00:00:00"));
        // First Quarter Frame at the next frame boundary (33.333 ms - 10 ms).
        let first_qf = b.sent.iter().find(|(_, o)| matches!(o, GenOut::Qf { .. })).unwrap().0;
        assert!((first_qf - 23_333.3).abs() < 1.0, "{first_qf}");
        let lock_t = b.ev.iter().position(|e| matches!(e, MtcEvent::Locked { .. })).map(|_| ()).and(Some(()));
        assert!(lock_t.is_some());
        let s = b.rx.snapshot(1_000_000);
        assert_eq!(s.health, SyncHealth::Locked);
        // Receiver position tracks the source within one frame.
        let err = s.position_us.unwrap() as f64 - (pos0 + 1_000_000.0);
        assert!(err.abs() < 34_000.0, "err {err}");
        assert_eq!(s.stats.discontinuities, 0);
        assert_eq!(b.g.stats.full_frames, 1);
        // 120 QF/s after the first boundary.
        assert!((b.g.stats.quarter_frames as i64 - 117).abs() <= 2, "{}", b.g.stats.quarter_frames);
    }

    #[test]
    fn seek_sends_full_frame_and_restarts_immediately() {
        let mut b = Bench::new();
        let pos = |t: f64| if t < 500_000.0 { H1 + t } else { H1 + 30_000_000.0 + (t - 500_000.0) };
        b.run_until(900_000.0, |t| Desired::Run { pos_us: pos(t) }, |t| if t < 500_000.0 { 0 } else { 1 });
        let jump = b.sent.iter().find(|(_, o)| matches!(o, GenOut::FullFrame { reason: "jump", .. })).expect("jump FF");
        assert!(jump.0 >= 500_000.0 && jump.0 < 510_000.0);
        assert_eq!(b.g.stats.jumps, 1);
        // The receiver relocks at the new position within ~2 sequences + 1 frame.
        let s = b.rx.snapshot(900_000);
        assert_eq!(s.health, SyncHealth::Locked);
        let err = s.position_us.unwrap() as f64 - pos(900_000.0);
        assert!(err.abs() < 34_000.0, "err {err}");
        assert_eq!(s.stats.discontinuities, 0, "a Full Frame jump is not a QF discontinuity");
    }

    #[test]
    fn jump_through_an_off_gap_is_reported_as_jump_and_lost_recovery_as_start() {
        let mut b = Bench::new();
        b.run_until(300_000.0, |t| Desired::Run { pos_us: H1 + t }, |_| 0);
        // Bridge acquiring for one sample after a seek: Off, epoch 1.
        b.run_until(316_000.0, |_| Desired::Off, |_| 1);
        b.run_until(500_000.0, |t| Desired::Run { pos_us: H1 + 30_000_000.0 + t }, |_| 1);
        // Bridge lost and back without a discontinuity: start.
        b.run_until(800_000.0, |_| Desired::Off, |_| 1);
        b.run_until(900_000.0, |t| Desired::Run { pos_us: H1 + 30_000_000.0 + t }, |_| 1);
        // Stop, seek while stopped, play: hold then start.
        b.run_until(1_000_000.0, |_| Desired::Hold { pos_us: H1 }, |_| 1);
        b.run_until(1_100_000.0, |_| Desired::Hold { pos_us: H1 + 5_000_000.0 }, |_| 2);
        b.run_until(1_200_000.0, |t| Desired::Run { pos_us: H1 + 5_000_000.0 + (t - 1_100_000.0) }, |_| 2);
        let reasons: Vec<&str> = b.sent.iter().filter_map(|(_, o)| match o { GenOut::FullFrame { reason, .. } => Some(*reason), _ => None }).collect();
        assert_eq!(reasons, vec!["start", "jump", "start", "stop", "hold", "start"]);
    }

    #[test]
    fn source_jitter_does_not_reach_the_labels() {
        let mut b = Bench::new();
        // +-4 ms deterministic jitter on the reported position.
        let jit = |t: f64| ((t / 16_000.0).floor() as i64 % 5 - 2) as f64 * 2_000.0;
        b.run_until(3_000_000.0, |t| Desired::Run { pos_us: H1 + t + jit(t) }, |_| 0);
        assert_eq!(b.g.stats.resyncs, 0);
        assert_eq!(b.rx.snapshot(3_000_000).stats.discontinuities, 0);
        assert_eq!(b.g.stats.full_frames, 1);
    }

    #[test]
    fn drift_beyond_a_frame_resyncs() {
        let mut b = Bench::new();
        // Source runs 10 % fast: the slewed schedule cannot follow, so it resyncs.
        b.run_until(2_000_000.0, |t| Desired::Run { pos_us: H1 + 1.1 * t }, |_| 0);
        assert!(b.g.stats.resyncs >= 1);
    }

    #[test]
    fn stop_holds_with_one_full_frame_and_moves_send_another() {
        let mut b = Bench::new();
        b.run_until(300_000.0, |t| Desired::Run { pos_us: H1 + t }, |_| 0);
        let qf_before = b.g.stats.quarter_frames;
        b.run_until(600_000.0, |_| Desired::Hold { pos_us: H1 + 300_000.0 }, |_| 0);
        assert_eq!(b.g.stats.quarter_frames, qf_before);
        assert_eq!(b.g.stats.stops, 1);
        let ffs: Vec<&str> = b.sent.iter().filter_map(|(_, o)| match o { GenOut::FullFrame { reason, .. } => Some(*reason), _ => None }).collect();
        assert_eq!(ffs, vec!["start", "stop"]);
        // Position change while stopped.
        b.run_until(700_000.0, |_| Desired::Hold { pos_us: H1 }, |_| 0);
        assert_eq!(b.g.stats.full_frames, 3);
        // Receiver holds the stopped position.
        let s = b.rx.snapshot(700_000);
        assert_eq!(s.position_us, Some(3_600_000_000));
        assert!(!s.advancing);
    }

    #[test]
    fn off_sends_nothing_and_recovery_starts_with_full_frame() {
        let mut b = Bench::new();
        b.run_until(200_000.0, |t| Desired::Run { pos_us: H1 + t }, |_| 0);
        let n = b.sent.len();
        b.run_until(500_000.0, |_| Desired::Off, |_| 0);
        assert_eq!(b.sent.len(), n);
        b.run_until(600_000.0, |t| Desired::Run { pos_us: H1 + t }, |_| 0);
        assert!(matches!(&b.sent[n].1, GenOut::FullFrame { reason: "start", .. }));
        assert_eq!(b.g.stats.starts, 2);
    }

    fn snap(health: BridgeHealth, transport: BridgeTransport, ms: Option<f64>) -> BridgeSnapshot {
        let advancing = transport == BridgeTransport::Playing;
        BridgeSnapshot { health, transport, fault: None, song_ms: ms, advancing, blackout: false, age_us: Some(0), tempo: None, beats: None, play_since_us: None, play_samples: 0, stats: BridgeStats::default() }
    }

    #[test]
    fn desired_state_follows_bridge_health_and_transport() {
        let o = 3_600_000_000.0;
        assert_eq!(desired_from(&snap(BridgeHealth::Locked, BridgeTransport::Playing, Some(1.0)), o, 0), Desired::Run { pos_us: o + 1000.0 });
        assert_eq!(desired_from(&snap(BridgeHealth::Holdover, BridgeTransport::Playing, Some(1.0)), o, 0), Desired::Run { pos_us: o + 1000.0 });
        assert_eq!(desired_from(&snap(BridgeHealth::Acquiring, BridgeTransport::Playing, Some(1.0)), o, 0), Desired::Off);
        assert_eq!(desired_from(&snap(BridgeHealth::Lost, BridgeTransport::Playing, Some(1.0)), o, 0), Desired::Off);
        assert_eq!(desired_from(&snap(BridgeHealth::Fault, BridgeTransport::Stopped, Some(1.0)), o, 0), Desired::Off);
        assert_eq!(desired_from(&snap(BridgeHealth::Locked, BridgeTransport::Stopped, Some(2.0)), o, 0), Desired::Hold { pos_us: o + 2000.0 });
        assert_eq!(desired_from(&snap(BridgeHealth::Locked, BridgeTransport::Stalled, Some(2.0)), o, 0), Desired::Hold { pos_us: o + 2000.0 });
        let mut waiting = snap(BridgeHealth::Locked, BridgeTransport::Playing, Some(2.0));
        waiting.advancing = false;
        assert_eq!(desired_from(&waiting, o, 0), Desired::Hold { pos_us: o + 2000.0 });
        // Play settle: one sample since Play, within 50 ms -> wait; afterwards or with a 2nd sample -> run.
        let mut p = snap(BridgeHealth::Locked, BridgeTransport::Playing, Some(1.0));
        p.play_since_us = Some(1_000_000);
        p.play_samples = 1;
        assert_eq!(desired_from(&p, o, 1_030_000), Desired::Off);
        assert_eq!(desired_from(&p, o, 1_050_000), Desired::Run { pos_us: o + 1000.0 });
        p.play_samples = 2;
        assert_eq!(desired_from(&p, o, 1_010_000), Desired::Run { pos_us: o + 1000.0 });
    }
}
