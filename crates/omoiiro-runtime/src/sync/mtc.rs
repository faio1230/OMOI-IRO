//! MTC (MIDI Time Code) decoder: a pure state machine with no I/O.
//!
//! Contract (see docs/02-architecture.md "MTC受信"):
//! * Quarter Frames (F1 nd) are assembled in forward order 0..7. Missing,
//!   duplicated or out-of-order pieces discard the partial sequence.
//! * A complete sequence yields the label of the frame during which piece 0
//!   was sent. The anchor pairs that label with the monotonic receive time of
//!   piece 0, so interpolation is `anchor_us + (now - anchor_t)`.
//! * `locked` needs two consecutive sequences whose labels differ by exactly
//!   2 frames. The first sequence after start, Full Frame or a discontinuity
//!   is `acquiring`.
//! * Full Frame (F0 7F 7F 01 01 hr mn sc fr F7) is a position update, not
//!   Play. It clears the partial sequence and the interpolation anchor.
//!   If the Full Frame position differs from the position reported just
//!   before it by more than `ff_jump_threshold_frames`, a
//!   `FullFrameDiscontinuity` is emitted and counted separately from the
//!   Quarter Frame `Discontinuity` (O13). LiveMTC reports seek, loop and
//!   tempo changes only this way.
//! * Optional (`ff_continue_when_clock_playing`, default off, O13 trial):
//!   when a Full Frame arrives while the caller reports MIDI Clock as
//!   playing (`set_clock_playing`), the position advances from the Full
//!   Frame at real time until the next lock, instead of holding still for
//!   the ~0.6 s LiveMTC needs before Quarter Frames lock. It freezes when
//!   the clock stops playing. At the lock, `FreewheelLocked` reports the
//!   difference between the advanced position and the locked label.
//! * P0 converts 24 / 25 / 30 NDF labels to time. 29.97 DF labels are decoded
//!   and displayed but reported as unsupported; they are never treated as
//!   30 fps.
//! * Reverse sequences (7..0) are detected and counted, never followed.
//! * Health by age since the last Quarter Frame: locked -> holdover
//!   (interpolate up to `holdover_us`) -> lost (hold the position reached at
//!   the holdover limit) -> lost + blackout after `blackout_us`.

use std::fmt;

/// The single frame-rate type of the workspace (M4). P0 MTC decoding converts
/// only non-drop rates to time (`!is_drop_frame()`).
pub use omoiiro_core::time::FrameRate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    pub rate: FrameRate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelError {
    Hours(u8),
    Minutes(u8),
    Seconds(u8),
    Frames(u8),
}

impl Label {
    pub fn new(hours: u8, minutes: u8, seconds: u8, frames: u8, rate: FrameRate) -> Label {
        Label { hours, minutes, seconds, frames, rate }
    }

    pub fn validate(&self) -> Result<(), LabelError> {
        if self.hours >= 24 {
            return Err(LabelError::Hours(self.hours));
        }
        if self.minutes >= 60 {
            return Err(LabelError::Minutes(self.minutes));
        }
        if self.seconds >= 60 {
            return Err(LabelError::Seconds(self.seconds));
        }
        if self.frames as u32 >= self.rate.nominal() {
            return Err(LabelError::Frames(self.frames));
        }
        Ok(())
    }

    /// Nominal frame count since 00:00:00:00. For NDF rates this is the
    /// true frame index. For 29.97 DF it is only a counting value.
    pub fn frame_index(&self) -> u64 {
        let fps = self.rate.nominal() as u64;
        (((self.hours as u64 * 60) + self.minutes as u64) * 60 + self.seconds as u64) * fps
            + self.frames as u64
    }

    pub fn from_frame_index(index: u64, rate: FrameRate) -> Label {
        let fps = rate.nominal() as u64;
        let frames = (index % fps) as u8;
        let total_seconds = index / fps;
        Label {
            hours: ((total_seconds / 3600) % 24) as u8,
            minutes: ((total_seconds / 60) % 60) as u8,
            seconds: (total_seconds % 60) as u8,
            frames,
            rate,
        }
    }

    /// Time since 00:00:00:00 in microseconds. `None` for unsupported rates.
    pub fn to_us(&self) -> Option<i64> {
        frame_index_to_us(self.frame_index(), self.rate)
    }
}

/// Exact conversion without accumulated rounding: `index * 1e6 / fps`.
pub fn frame_index_to_us(index: u64, rate: FrameRate) -> Option<i64> {
    if rate.is_drop_frame() {
        return None;
    }
    let fps = rate.nominal() as i128;
    Some((index as i128 * 1_000_000 / fps) as i64)
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02}:{:02}:{:02}:{:02}",
            self.hours, self.minutes, self.seconds, self.frames
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropReason {
    OutOfOrder { expected: u8, got: u8 },
    Timeout { gap_us: i64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MtcEvent {
    FullFrame { label: Label },
    Sequence { label: Label, t_first_piece_us: i64 },
    ReverseSequence { label: Label },
    Locked { label: Label },
    /// `delta_frames` is the deviation from the expected `previous + 2`.
    Discontinuity { previous: Label, current: Label, delta_frames: i64 },
    /// A Full Frame moved the position away from the one reported just before
    /// it (`previous_us`, in state `previous_health`). `delta_frames` is
    /// `current - previous` rounded to whole frames of the Full Frame rate.
    FullFrameDiscontinuity {
        previous_us: i64,
        previous_health: SyncHealth,
        current: Label,
        delta_frames: i64,
    },
    /// Lock after a Full Frame that was continued on the clock. `predicted_us`
    /// is the continued position at the time of the locking label's piece 0;
    /// `delta_us` is `label - predicted` (positive: the continued position
    /// was behind).
    FreewheelLocked { label: Label, predicted_us: i64, delta_us: i64 },
    RateChange { previous: FrameRate, current: FrameRate },
    Dropped { reason: DropReason },
    InvalidLabel { label: Label, error: LabelError },
    UnsupportedRate { rate: FrameRate },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncHealth {
    Unlocked,
    Acquiring,
    Locked,
    Holdover,
    Lost,
    Fault,
}

impl SyncHealth {
    pub fn name(self) -> &'static str {
        match self {
            SyncHealth::Unlocked => "unlocked",
            SyncHealth::Acquiring => "acquiring",
            SyncHealth::Locked => "locked",
            SyncHealth::Holdover => "holdover",
            SyncHealth::Lost => "lost",
            SyncHealth::Fault => "fault",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub quarter_frames: u64,
    pub full_frames: u64,
    pub sequences: u64,
    pub reverse_sequences: u64,
    pub dropped: u64,
    pub stray_pieces: u64,
    pub discontinuities: u64,
    /// Full Frame jumps beyond the threshold (not included in `discontinuities`).
    pub ff_discontinuities: u64,
    pub rate_changes: u64,
    pub invalid_labels: u64,
    pub locks: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Age after which a locked decoder is reported as holdover.
    pub stale_us: i64,
    /// Maximum interpolation beyond the last Quarter Frame.
    pub holdover_us: i64,
    /// Age after which the output should be black.
    pub blackout_us: i64,
    /// Maximum gap between two pieces of one sequence.
    pub piece_timeout_us: i64,
    /// A Full Frame further than this from the previous position is a
    /// `FullFrameDiscontinuity`. Provisional value 2 frames.
    pub ff_jump_threshold_frames: i64,
    /// Advance from a Full Frame while MIDI Clock is playing (O13 trial).
    pub ff_continue_when_clock_playing: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            stale_us: 50_000,
            holdover_us: 250_000,
            blackout_us: 1_000_000,
            piece_timeout_us: 100_000,
            ff_jump_threshold_frames: 2,
            ff_continue_when_clock_playing: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MtcSnapshot {
    pub health: SyncHealth,
    pub fault: Option<&'static str>,
    pub rate: Option<FrameRate>,
    /// Last decoded raw label (Full Frame or sequence).
    pub label: Option<Label>,
    /// Estimated position in microseconds since 00:00:00:00.
    pub position_us: Option<i64>,
    /// True while the position is being interpolated from a live stream.
    pub advancing: bool,
    pub blackout: bool,
    pub age_us: Option<i64>,
    pub stats: Stats,
}

#[derive(Clone, Copy, Debug)]
struct Partial {
    pieces: [u8; 8],
    next: u8,
    t_first: i64,
    t_last: i64,
}

#[derive(Clone, Copy, Debug)]
struct Anchor {
    position_us: i64,
    t_us: i64,
}

/// Position continued from a Full Frame while the clock is playing.
#[derive(Clone, Copy, Debug)]
struct Freewheel {
    position_us: i64,
    t_us: i64,
    /// Time the clock stopped playing; the position freezes there.
    end_us: Option<i64>,
}

impl Freewheel {
    fn position_at(&self, now_us: i64) -> i64 {
        let t = self.end_us.map_or(now_us, |e| e.min(now_us));
        self.position_us + (t - self.t_us)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LockState {
    Unlocked,
    Acquiring,
    Locked,
    Fault(&'static str),
}

pub struct MtcDecoder {
    params: Params,
    fwd: Option<Partial>,
    rev: Option<Partial>,
    anchor: Option<Anchor>,
    prev_seq: Option<Label>,
    consistent: u32,
    state: LockState,
    last_qf_us: Option<i64>,
    label: Option<Label>,
    rate: Option<FrameRate>,
    stats: Stats,
    clock_playing: bool,
    freewheel: Option<Freewheel>,
}

impl MtcDecoder {
    pub fn new(params: Params) -> Self {
        MtcDecoder {
            params,
            fwd: None,
            rev: None,
            anchor: None,
            prev_seq: None,
            consistent: 0,
            state: LockState::Unlocked,
            last_qf_us: None,
            label: None,
            rate: None,
            stats: Stats::default(),
            clock_playing: false,
            freewheel: None,
        }
    }

    /// Tell the decoder whether MIDI Clock is playing at `t_us`. Only used
    /// when `ff_continue_when_clock_playing` is on.
    pub fn set_clock_playing(&mut self, t_us: i64, playing: bool) {
        self.clock_playing = playing;
        if !playing {
            if let Some(f) = self.freewheel.as_mut() {
                f.end_us.get_or_insert(t_us);
            }
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Feed one complete MIDI message received at monotonic time `t_us`.
    /// Messages other than Quarter Frame and MTC Full Frame are ignored.
    pub fn feed(&mut self, t_us: i64, bytes: &[u8], out: &mut Vec<MtcEvent>) {
        match bytes.first() {
            Some(0xF1) if bytes.len() >= 2 => self.quarter_frame(t_us, bytes[1], out),
            Some(0xF0) => self.sysex(t_us, bytes, out),
            _ => {}
        }
    }

    fn sysex(&mut self, t_us: i64, bytes: &[u8], out: &mut Vec<MtcEvent>) {
        // Tolerate Real Time bytes interleaved inside the SysEx buffer.
        let clean: Vec<u8> = bytes.iter().copied().filter(|b| *b < 0xF8).collect();
        if clean.len() != 10
            || clean[1] != 0x7F
            || clean[2] != 0x7F
            || clean[3] != 0x01
            || clean[4] != 0x01
            || clean[9] != 0xF7
        {
            return;
        }
        let rate = FrameRate::from_mtc_code((clean[5] >> 5) & 0x03);
        let label = Label::new(clean[5] & 0x1F, clean[6], clean[7], clean[8], rate);
        // Position as reported just before this Full Frame (interpolated,
        // held or frozen, depending on the state). Faults have no reliable
        // position and are not compared.
        let before = self.snapshot(t_us);
        let previous = match before.health {
            SyncHealth::Acquiring | SyncHealth::Locked | SyncHealth::Holdover | SyncHealth::Lost => {
                before.position_us.map(|p| (p, before.health))
            }
            SyncHealth::Unlocked | SyncHealth::Fault => None,
        };
        self.stats.full_frames += 1;
        self.fwd = None;
        self.rev = None;
        self.anchor = None;
        self.freewheel = None;
        self.prev_seq = None;
        self.consistent = 0;
        if let Err(error) = label.validate() {
            self.stats.invalid_labels += 1;
            self.state = LockState::Fault("invalid full frame label");
            out.push(MtcEvent::InvalidLabel { label, error });
            return;
        }
        self.label = Some(label);
        self.rate = Some(rate);
        self.state = if !rate.is_drop_frame() {
            LockState::Acquiring
        } else {
            out.push(MtcEvent::UnsupportedRate { rate });
            LockState::Fault("unsupported frame rate")
        };
        out.push(MtcEvent::FullFrame { label });
        if self.params.ff_continue_when_clock_playing && self.clock_playing && self.state == LockState::Acquiring {
            if let Some(position_us) = label.to_us() {
                self.freewheel = Some(Freewheel { position_us, t_us, end_us: None });
            }
        }
        if let (Some((previous_us, previous_health)), Some(current_us)) = (previous, label.to_us()) {
            let fps = rate.nominal() as f64;
            let delta_frames = ((current_us - previous_us) as f64 * fps / 1_000_000.0).round() as i64;
            if delta_frames.abs() > self.params.ff_jump_threshold_frames {
                self.stats.ff_discontinuities += 1;
                out.push(MtcEvent::FullFrameDiscontinuity {
                    previous_us,
                    previous_health,
                    current: label,
                    delta_frames,
                });
            }
        }
    }

    fn quarter_frame(&mut self, t_us: i64, data: u8, out: &mut Vec<MtcEvent>) {
        let piece = (data >> 4) & 0x07;
        let nibble = data & 0x0F;
        self.stats.quarter_frames += 1;
        self.last_qf_us = Some(t_us);

        // Reverse tracker (detection only).
        if piece == 7 {
            self.rev = Some(Partial { pieces: [0; 8], next: 7, t_first: t_us, t_last: t_us });
        }
        if let Some(r) = self.rev.as_mut() {
            if r.next == piece {
                r.pieces[piece as usize] = nibble;
                r.t_last = t_us;
                if piece == 0 {
                    let label = decode_label(&r.pieces);
                    self.rev = None;
                    self.fwd = None;
                    self.stats.reverse_sequences += 1;
                    self.label = Some(label);
                    self.rate = Some(label.rate);
                    self.state = LockState::Fault("reverse sequence");
                    self.anchor = None;
                    self.freewheel = None;
                    self.prev_seq = None;
                    self.consistent = 0;
                    out.push(MtcEvent::ReverseSequence { label });
                    return;
                }
                r.next -= 1;
            } else if piece != 7 {
                self.rev = None;
            }
        }

        // Forward tracker.
        if let Some(p) = self.fwd {
            let gap = t_us - p.t_last;
            if gap > self.params.piece_timeout_us {
                self.fwd = None;
                self.stats.dropped += 1;
                out.push(MtcEvent::Dropped { reason: DropReason::Timeout { gap_us: gap } });
            }
        }
        match self.fwd.as_mut() {
            Some(p) if p.next == piece => {
                p.pieces[piece as usize] = nibble;
                p.t_last = t_us;
                p.next += 1;
                if p.next == 8 {
                    let done = *p;
                    self.fwd = None;
                    self.complete(done, out);
                }
            }
            Some(p) => {
                let expected = p.next;
                self.fwd = None;
                self.stats.dropped += 1;
                out.push(MtcEvent::Dropped {
                    reason: DropReason::OutOfOrder { expected, got: piece },
                });
                if piece == 0 {
                    self.start_forward(t_us, nibble);
                }
            }
            None => {
                if piece == 0 {
                    self.start_forward(t_us, nibble);
                } else {
                    self.stats.stray_pieces += 1;
                }
            }
        }
    }

    fn start_forward(&mut self, t_us: i64, nibble: u8) {
        let mut pieces = [0u8; 8];
        pieces[0] = nibble;
        self.fwd = Some(Partial { pieces, next: 1, t_first: t_us, t_last: t_us });
    }

    fn complete(&mut self, p: Partial, out: &mut Vec<MtcEvent>) {
        let label = decode_label(&p.pieces);
        self.stats.sequences += 1;
        if let Err(error) = label.validate() {
            self.stats.invalid_labels += 1;
            self.state = LockState::Fault("invalid label");
            self.anchor = None;
            self.freewheel = None;
            self.prev_seq = None;
            self.consistent = 0;
            out.push(MtcEvent::InvalidLabel { label, error });
            return;
        }
        out.push(MtcEvent::Sequence { label, t_first_piece_us: p.t_first });
        self.label = Some(label);

        if let Some(prev) = self.prev_seq {
            if prev.rate != label.rate {
                self.stats.rate_changes += 1;
                out.push(MtcEvent::RateChange { previous: prev.rate, current: label.rate });
            }
        }
        self.rate = Some(label.rate);

        if label.rate.is_drop_frame() {
            if self.state != LockState::Fault("unsupported frame rate") {
                out.push(MtcEvent::UnsupportedRate { rate: label.rate });
            }
            self.state = LockState::Fault("unsupported frame rate");
            self.anchor = None;
            self.freewheel = None;
            self.prev_seq = Some(label);
            self.consistent = 0;
            return;
        }

        let index = label.frame_index();
        match self.prev_seq {
            Some(prev) if prev.rate == label.rate => {
                let expected = prev.frame_index() + 2;
                if index == expected {
                    self.consistent += 1;
                } else {
                    self.stats.discontinuities += 1;
                    self.consistent = 0;
                    self.state = LockState::Acquiring;
                    out.push(MtcEvent::Discontinuity {
                        previous: prev,
                        current: label,
                        delta_frames: index as i64 - expected as i64,
                    });
                }
            }
            _ => {
                self.consistent = 0;
                self.state = LockState::Acquiring;
            }
        }

        self.anchor = Some(Anchor {
            position_us: label.to_us().expect("supported rate"),
            t_us: p.t_first,
        });
        if self.consistent >= 1 && self.state != LockState::Locked {
            self.state = LockState::Locked;
            self.stats.locks += 1;
            out.push(MtcEvent::Locked { label });
            if let Some(f) = self.freewheel.take() {
                let predicted_us = f.position_at(p.t_first);
                let label_us = label.to_us().expect("supported rate");
                out.push(MtcEvent::FreewheelLocked { label, predicted_us, delta_us: label_us - predicted_us });
            }
        }
        self.prev_seq = Some(label);
    }

    pub fn snapshot(&self, now_us: i64) -> MtcSnapshot {
        let age_us = self.last_qf_us.map(|t| now_us - t);
        let mut snap = MtcSnapshot {
            health: SyncHealth::Unlocked,
            fault: None,
            rate: self.rate,
            label: self.label,
            position_us: None,
            advancing: false,
            blackout: true,
            age_us,
            stats: self.stats,
        };
        match self.state {
            LockState::Unlocked => {}
            LockState::Fault(msg) => {
                snap.health = SyncHealth::Fault;
                snap.fault = Some(msg);
                snap.position_us = self.label.and_then(|l| l.to_us());
            }
            LockState::Acquiring => {
                snap.health = SyncHealth::Acquiring;
                snap.position_us = match (self.freewheel, self.anchor) {
                    (Some(f), _) => {
                        snap.advancing = f.end_us.is_none();
                        Some(f.position_at(now_us))
                    }
                    (None, Some(a)) => Some(a.position_us),
                    (None, None) => self.label.and_then(|l| l.to_us()),
                };
                snap.blackout = false;
            }
            LockState::Locked => {
                let anchor = self.anchor.expect("locked implies anchor");
                let last_qf = self.last_qf_us.expect("locked implies qf");
                let age = now_us - last_qf;
                let p = &self.params;
                if age <= p.stale_us {
                    snap.health = SyncHealth::Locked;
                    snap.position_us = Some(anchor.position_us + (now_us - anchor.t_us));
                    snap.advancing = true;
                    snap.blackout = false;
                } else if age <= p.holdover_us {
                    snap.health = SyncHealth::Holdover;
                    snap.position_us = Some(anchor.position_us + (now_us - anchor.t_us));
                    snap.advancing = true;
                    snap.blackout = false;
                } else {
                    snap.health = SyncHealth::Lost;
                    let frozen_t = last_qf + p.holdover_us;
                    snap.position_us = Some(anchor.position_us + (frozen_t - anchor.t_us));
                    snap.advancing = false;
                    snap.blackout = age > p.blackout_us;
                }
            }
        }
        snap
    }
}

fn decode_label(p: &[u8; 8]) -> Label {
    let frames = (p[0] & 0x0F) | ((p[1] & 0x01) << 4);
    let seconds = (p[2] & 0x0F) | ((p[3] & 0x07) << 4);
    let minutes = (p[4] & 0x0F) | ((p[5] & 0x07) << 4);
    let hours = (p[6] & 0x0F) | ((p[7] & 0x01) << 4);
    let rate = FrameRate::from_mtc_code((p[7] >> 1) & 0x03);
    Label { hours, minutes, seconds, frames, rate }
}

/// Encode a label into the 8 Quarter Frame data bytes (piece nibble included).
/// Used by tests and the synthetic fixture generator.
pub fn encode_quarter_frames(label: Label) -> [u8; 8] {
    let r = label.rate.mtc_code();
    [
        label.frames & 0x0F,
        0x10 | ((label.frames >> 4) & 0x01),
        0x20 | (label.seconds & 0x0F),
        0x30 | ((label.seconds >> 4) & 0x07),
        0x40 | (label.minutes & 0x0F),
        0x50 | ((label.minutes >> 4) & 0x07),
        0x60 | (label.hours & 0x0F),
        0x70 | ((label.hours >> 4) & 0x01) | (r << 1),
    ]
}

/// Encode a Full Frame message.
pub fn encode_full_frame(label: Label) -> [u8; 10] {
    [
        0xF0,
        0x7F,
        0x7F,
        0x01,
        0x01,
        (label.rate.mtc_code() << 5) | (label.hours & 0x1F),
        label.minutes,
        label.seconds,
        label.frames,
        0xF7,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const QF: i64 = 8_333; // 30 fps quarter frame interval (truncated)

    fn l30(h: u8, m: u8, s: u8, f: u8) -> Label {
        Label::new(h, m, s, f, FrameRate::Fps30)
    }

    /// Send one full forward sequence starting at `t0`; returns the time
    /// after the last piece.
    fn seq(dec: &mut MtcDecoder, label: Label, t0: i64, out: &mut Vec<MtcEvent>) -> i64 {
        let pieces = encode_quarter_frames(label);
        for (k, d) in pieces.iter().enumerate() {
            dec.feed(t0 + k as i64 * QF, &[0xF1, *d], out);
        }
        t0 + 8 * QF
    }

    #[test]
    fn frame_time_is_exact_without_accumulation() {
        assert_eq!(frame_index_to_us(0, FrameRate::Fps30), Some(0));
        assert_eq!(frame_index_to_us(1, FrameRate::Fps30), Some(33_333));
        assert_eq!(frame_index_to_us(3, FrameRate::Fps30), Some(100_000));
        assert_eq!(frame_index_to_us(30 * 3600, FrameRate::Fps30), Some(3_600_000_000));
        assert_eq!(frame_index_to_us(25, FrameRate::Fps25), Some(1_000_000));
        assert_eq!(frame_index_to_us(24, FrameRate::Fps24), Some(1_000_000));
        assert_eq!(frame_index_to_us(30, FrameRate::Fps2997Df), None);
    }

    #[test]
    fn label_round_trip_through_quarter_frames() {
        let label = l30(1, 23, 45, 29);
        let p = encode_quarter_frames(label);
        let mut masked = [0u8; 8];
        for (i, b) in p.iter().enumerate() {
            assert_eq!(b >> 4, i as u8);
            masked[i] = b & 0x0F;
        }
        assert_eq!(decode_label(&masked), label);
        assert_eq!(Label::from_frame_index(label.frame_index(), FrameRate::Fps30), label);
        assert_eq!(label.to_string(), "01:23:45:29");
    }

    #[test]
    fn single_sequence_is_acquiring_not_locked() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        let t = seq(&mut dec, l30(1, 0, 0, 0), 1_000, &mut out);
        assert_eq!(
            out,
            vec![MtcEvent::Sequence { label: l30(1, 0, 0, 0), t_first_piece_us: 1_000 }]
        );
        let s = dec.snapshot(t);
        assert_eq!(s.health, SyncHealth::Acquiring);
        assert_eq!(s.position_us, Some(3_600_000_000));
        assert!(!s.advancing);
    }

    #[test]
    fn two_consistent_sequences_lock_and_interpolate_from_first_piece() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        let t1 = seq(&mut dec, l30(1, 0, 0, 0), 1_000, &mut out);
        let t2 = seq(&mut dec, l30(1, 0, 0, 2), t1, &mut out);
        assert!(out.contains(&MtcEvent::Locked { label: l30(1, 0, 0, 2) }));
        let s = dec.snapshot(t2 + 10_000);
        assert_eq!(s.health, SyncHealth::Locked);
        assert!(s.advancing);
        // anchor: label 01:00:00:02 at t1 (first piece of 2nd sequence)
        let expected = 3_600_000_000 + 2 * 1_000_000 / 30 + (t2 + 10_000 - t1);
        assert_eq!(s.position_us, Some(expected));
        assert_eq!(s.stats.locks, 1);
    }

    #[test]
    fn full_frame_sets_position_without_play() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        dec.feed(500, &encode_full_frame(l30(0, 10, 0, 0)), &mut out);
        assert_eq!(out, vec![MtcEvent::FullFrame { label: l30(0, 10, 0, 0) }]);
        let s = dec.snapshot(100_000_000);
        assert_eq!(s.health, SyncHealth::Acquiring);
        assert_eq!(s.position_us, Some(600_000_000));
        assert!(!s.advancing);
        assert_eq!(s.stats.full_frames, 1);
    }

    #[test]
    fn full_frame_with_interleaved_realtime_bytes_is_decoded() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        let ff = encode_full_frame(l30(2, 3, 4, 5));
        let mut dirty = Vec::new();
        for (i, b) in ff.iter().enumerate() {
            dirty.push(*b);
            if i == 4 {
                dirty.push(0xF8);
            }
        }
        dec.feed(0, &dirty, &mut out);
        assert_eq!(out, vec![MtcEvent::FullFrame { label: l30(2, 3, 4, 5) }]);
    }

    #[test]
    fn full_frame_clears_partial_sequence() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        let pieces = encode_quarter_frames(l30(1, 0, 0, 0));
        for (k, d) in pieces.iter().take(4).enumerate() {
            dec.feed(k as i64 * QF, &[0xF1, *d], &mut out);
        }
        dec.feed(5 * QF, &encode_full_frame(l30(0, 0, 0, 0)), &mut out);
        // Remaining pieces of the old sequence must not complete a label.
        for (k, d) in pieces.iter().skip(4).enumerate() {
            dec.feed((6 + k as i64) * QF, &[0xF1, *d], &mut out);
        }
        assert_eq!(dec.stats().sequences, 0);
        assert_eq!(dec.stats().stray_pieces, 4);
    }

    #[test]
    fn full_frame_jump_while_locked_is_a_ff_discontinuity() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        let t1 = seq(&mut dec, l30(1, 0, 29, 0), 0, &mut out);
        let t2 = seq(&mut dec, l30(1, 0, 29, 2), t1, &mut out);
        out.clear();
        // Loop back to 01:00:22:00 (as LiveMTC does on a loop wrap).
        dec.feed(t2, &encode_full_frame(l30(1, 0, 22, 0)), &mut out);
        let previous_us = 3_629_000_000 + 2 * 1_000_000 / 30 + (t2 - t1);
        let current_us = 3_622_000_000;
        let delta = ((current_us - previous_us) as f64 * 30.0 / 1e6).round() as i64;
        assert_eq!(
            out,
            vec![
                MtcEvent::FullFrame { label: l30(1, 0, 22, 0) },
                MtcEvent::FullFrameDiscontinuity {
                    previous_us,
                    previous_health: SyncHealth::Locked,
                    current: l30(1, 0, 22, 0),
                    delta_frames: delta,
                },
            ]
        );
        assert!(delta < -200);
        let s = dec.snapshot(t2);
        assert_eq!(s.stats.ff_discontinuities, 1);
        // Quarter Frame discontinuities are counted separately.
        assert_eq!(s.stats.discontinuities, 0);
        assert_eq!(s.health, SyncHealth::Acquiring);
    }

    #[test]
    fn full_frame_near_the_running_position_is_not_a_discontinuity() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        let t1 = seq(&mut dec, l30(1, 0, 0, 0), 0, &mut out);
        let t2 = seq(&mut dec, l30(1, 0, 0, 2), t1, &mut out);
        out.clear();
        // Position at t2 is 01:00:00:02 + 8 QF, i.e. 01:00:00:04.
        dec.feed(t2, &encode_full_frame(l30(1, 0, 0, 4)), &mut out);
        assert_eq!(out, vec![MtcEvent::FullFrame { label: l30(1, 0, 0, 4) }]);
        // While acquiring, the held position is the reference. Exactly at the
        // threshold (2 frames) is still continuous.
        dec.feed(t2, &encode_full_frame(l30(1, 0, 0, 6)), &mut out);
        assert_eq!(dec.stats().ff_discontinuities, 0);
        // Beyond the threshold is not.
        dec.feed(t2, &encode_full_frame(l30(1, 0, 0, 9)), &mut out);
        assert_eq!(dec.stats().ff_discontinuities, 1);
    }

    #[test]
    fn first_full_frame_has_no_reference_then_acquiring_position_is_used() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        dec.feed(0, &encode_full_frame(l30(1, 0, 0, 0)), &mut out);
        // LiveMTC: origin Full Frame on Play, then the real position ~250 ms later.
        dec.feed(250_000, &encode_full_frame(l30(1, 0, 32, 0)), &mut out);
        assert_eq!(
            out,
            vec![
                MtcEvent::FullFrame { label: l30(1, 0, 0, 0) },
                MtcEvent::FullFrame { label: l30(1, 0, 32, 0) },
                MtcEvent::FullFrameDiscontinuity {
                    previous_us: 3_600_000_000,
                    previous_health: SyncHealth::Acquiring,
                    current: l30(1, 0, 32, 0),
                    delta_frames: 32 * 30,
                },
            ]
        );
    }

    fn freewheel_params() -> Params {
        Params { ff_continue_when_clock_playing: true, ..Params::default() }
    }

    #[test]
    fn full_frame_holds_still_when_the_option_is_off() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        dec.set_clock_playing(0, true);
        dec.feed(0, &encode_full_frame(l30(1, 0, 22, 0)), &mut out);
        let s = dec.snapshot(300_000);
        assert_eq!(s.position_us, Some(3_622_000_000));
        assert!(!s.advancing);
    }

    #[test]
    fn full_frame_continues_while_clock_plays_and_reports_the_lock_error() {
        let mut dec = MtcDecoder::new(freewheel_params());
        let mut out = Vec::new();
        dec.set_clock_playing(0, true);
        dec.feed(1_000, &encode_full_frame(l30(1, 0, 22, 0)), &mut out);
        let s = dec.snapshot(301_000);
        assert_eq!(s.health, SyncHealth::Acquiring);
        assert!(s.advancing);
        assert_eq!(s.position_us, Some(3_622_300_000));
        // Quarter Frames start 0.4 s later at the matching label 01:00:22:12.
        let t0 = 401_000;
        let t1 = seq(&mut dec, l30(1, 0, 22, 12), t0, &mut out);
        // Still continued (not the static anchor) while acquiring.
        assert_eq!(dec.snapshot(t1).position_us, Some(3_622_000_000 + (t1 - 1_000)));
        out.clear();
        seq(&mut dec, l30(1, 0, 22, 14), t1, &mut out);
        let predicted_us = 3_622_000_000 + (t1 - 1_000);
        let label_us = l30(1, 0, 22, 14).to_us().unwrap();
        assert!(out.contains(&MtcEvent::FreewheelLocked {
            label: l30(1, 0, 22, 14),
            predicted_us,
            delta_us: label_us - predicted_us,
        }));
    }

    #[test]
    fn freewheel_freezes_when_clock_stops_and_needs_clock_playing_to_start() {
        let mut dec = MtcDecoder::new(freewheel_params());
        let mut out = Vec::new();
        dec.feed(0, &encode_full_frame(l30(1, 0, 0, 0)), &mut out);
        assert!(!dec.snapshot(100_000).advancing); // clock not playing
        dec.set_clock_playing(100_000, true);
        dec.feed(200_000, &encode_full_frame(l30(1, 0, 10, 0)), &mut out);
        dec.set_clock_playing(500_000, false);
        let s = dec.snapshot(900_000);
        assert!(!s.advancing);
        assert_eq!(s.position_us, Some(3_610_300_000));
    }

    #[test]
    fn seek_is_a_discontinuity_then_relocks() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        let t1 = seq(&mut dec, l30(1, 0, 0, 0), 0, &mut out);
        let t2 = seq(&mut dec, l30(1, 0, 0, 2), t1, &mut out);
        out.clear();
        let t3 = seq(&mut dec, l30(1, 0, 10, 0), t2, &mut out);
        assert_eq!(
            out,
            vec![
                MtcEvent::Sequence { label: l30(1, 0, 10, 0), t_first_piece_us: t2 },
                MtcEvent::Discontinuity {
                    previous: l30(1, 0, 0, 2),
                    current: l30(1, 0, 10, 0),
                    delta_frames: 10 * 30 - 4,
                },
            ]
        );
        assert_eq!(dec.snapshot(t3).health, SyncHealth::Acquiring);
        out.clear();
        let t4 = seq(&mut dec, l30(1, 0, 10, 2), t3, &mut out);
        assert!(out.contains(&MtcEvent::Locked { label: l30(1, 0, 10, 2) }));
        let s = dec.snapshot(t4);
        assert_eq!(s.health, SyncHealth::Locked);
        assert_eq!(s.position_us, Some(3_610_000_000 + 2 * 1_000_000 / 30 + (t4 - t3)));
        assert_eq!(s.stats.discontinuities, 1);
        assert_eq!(s.stats.locks, 2);
    }

    #[test]
    fn missing_piece_drops_sequence_without_false_label() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        let pieces = encode_quarter_frames(l30(1, 0, 0, 0));
        let mut t = 0;
        for (k, d) in pieces.iter().enumerate() {
            if k == 3 {
                continue; // lost on the wire
            }
            dec.feed(t, &[0xF1, *d], &mut out);
            t += QF;
        }
        assert_eq!(
            out,
            vec![MtcEvent::Dropped { reason: DropReason::OutOfOrder { expected: 3, got: 4 } }]
        );
        assert_eq!(dec.stats().sequences, 0);
        assert_eq!(dec.snapshot(t).label, None);
        // Next clean sequences lock normally.
        let t1 = seq(&mut dec, l30(1, 0, 0, 2), t, &mut out);
        seq(&mut dec, l30(1, 0, 0, 4), t1, &mut out);
        assert!(out.contains(&MtcEvent::Locked { label: l30(1, 0, 0, 4) }));
    }

    #[test]
    fn piece_timeout_drops_partial() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        let pieces = encode_quarter_frames(l30(1, 0, 0, 0));
        dec.feed(0, &[0xF1, pieces[0]], &mut out);
        dec.feed(QF, &[0xF1, pieces[1]], &mut out);
        dec.feed(QF + 200_000, &[0xF1, pieces[2]], &mut out);
        assert_eq!(
            out,
            vec![MtcEvent::Dropped { reason: DropReason::Timeout { gap_us: 200_000 } }]
        );
    }

    #[test]
    fn silence_goes_holdover_then_lost_then_blackout() {
        let p = Params::default();
        let mut dec = MtcDecoder::new(p);
        let mut out = Vec::new();
        let t1 = seq(&mut dec, l30(1, 0, 0, 0), 0, &mut out);
        let t2 = seq(&mut dec, l30(1, 0, 0, 2), t1, &mut out);
        let last_qf = t2 - QF;
        let base = 3_600_000_000 + 2 * 1_000_000 / 30; // anchor position at t1
        let s = dec.snapshot(last_qf + 10_000);
        assert_eq!(s.health, SyncHealth::Locked);
        let s = dec.snapshot(last_qf + 100_000);
        assert_eq!(s.health, SyncHealth::Holdover);
        assert!(s.advancing);
        assert_eq!(s.position_us, Some(base + (last_qf + 100_000 - t1)));
        let s = dec.snapshot(last_qf + 300_000);
        assert_eq!(s.health, SyncHealth::Lost);
        assert!(!s.advancing);
        assert!(!s.blackout);
        let frozen = base + (last_qf + p.holdover_us - t1);
        assert_eq!(s.position_us, Some(frozen));
        let s = dec.snapshot(last_qf + 1_500_000);
        assert_eq!(s.health, SyncHealth::Lost);
        assert!(s.blackout);
        assert_eq!(s.position_us, Some(frozen));
    }

    #[test]
    fn reverse_sequence_is_detected_not_followed() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        let pieces = encode_quarter_frames(l30(1, 0, 0, 0));
        for (k, d) in pieces.iter().rev().enumerate() {
            dec.feed(k as i64 * QF, &[0xF1, *d], &mut out);
        }
        assert_eq!(out, vec![MtcEvent::ReverseSequence { label: l30(1, 0, 0, 0) }]);
        let s = dec.snapshot(8 * QF);
        assert_eq!(s.health, SyncHealth::Fault);
        assert!(!s.advancing);
        assert_eq!(s.stats.reverse_sequences, 1);
        assert_eq!(s.stats.sequences, 0);
    }

    #[test]
    fn drop_frame_rate_is_reported_unsupported_not_treated_as_30() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        let label = Label::new(0, 1, 0, 2, FrameRate::Fps2997Df);
        let t1 = seq(&mut dec, label, 0, &mut out);
        let next = Label::new(0, 1, 0, 4, FrameRate::Fps2997Df);
        let t2 = seq(&mut dec, next, t1, &mut out);
        assert!(out.contains(&MtcEvent::UnsupportedRate { rate: FrameRate::Fps2997Df }));
        assert!(!out.iter().any(|e| matches!(e, MtcEvent::Locked { .. })));
        let s = dec.snapshot(t2);
        assert_eq!(s.health, SyncHealth::Fault);
        assert_eq!(s.fault, Some("unsupported frame rate"));
        assert_eq!(s.rate, Some(FrameRate::Fps2997Df));
        assert_eq!(s.label, Some(next));
        assert_eq!(s.position_us, None);
    }

    #[test]
    fn rate_change_is_reported() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        let t1 = seq(&mut dec, l30(0, 0, 1, 0), 0, &mut out);
        let t2 = seq(&mut dec, l30(0, 0, 1, 2), t1, &mut out);
        out.clear();
        seq(&mut dec, Label::new(0, 0, 1, 4, FrameRate::Fps25), t2, &mut out);
        assert!(out.contains(&MtcEvent::RateChange {
            previous: FrameRate::Fps30,
            current: FrameRate::Fps25
        }));
        assert_eq!(dec.stats().rate_changes, 1);
        assert_eq!(dec.snapshot(t2 + 8 * QF).health, SyncHealth::Acquiring);
    }

    #[test]
    fn invalid_label_is_rejected() {
        let mut dec = MtcDecoder::new(Params::default());
        let mut out = Vec::new();
        // frames = 31 is impossible at 30 fps
        let bad = Label::new(0, 0, 0, 31, FrameRate::Fps30);
        seq(&mut dec, bad, 0, &mut out);
        assert_eq!(
            out,
            vec![MtcEvent::InvalidLabel { label: bad, error: LabelError::Frames(31) }]
        );
        assert_eq!(dec.snapshot(100_000).health, SyncHealth::Fault);
    }
}
