//! Show time, timecode labels and beat state.
//!
//! * `ShowTimeUs`: integer microseconds since the song start (song start = 0,
//!   docs/10-live-bridge.md). Clip placement uses the same axis.
//! * `FrameRate`: timecode rates as exact rationals (29.97 = 30000/1001).
//! * `Timecode`: `HH:MM:SS:FF` labels with drop-frame validation and a
//!   continuous frame index. The MTC label of a show time is
//!   `origin + show_time` (`Timecode::add_us`).
//! * `BeatState`: quarter-note position and tempo from Link or MIDI Clock.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Microseconds since the song start (0 = song head).
pub type ShowTimeUs = i64;

/// Largest integer that JSON / JavaScript can carry exactly (2^53 - 1).
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FrameRate {
    #[serde(rename = "24")]
    Fps24,
    #[serde(rename = "25")]
    Fps25,
    #[serde(rename = "29.97df")]
    Fps2997Df,
    #[serde(rename = "30")]
    Fps30,
}

impl FrameRate {
    /// Label frames per second (the frame field range is `0..nominal`).
    pub fn nominal(self) -> u32 {
        match self {
            FrameRate::Fps24 => 24,
            FrameRate::Fps25 => 25,
            FrameRate::Fps2997Df | FrameRate::Fps30 => 30,
        }
    }

    /// Exact frames per second as `(numerator, denominator)`.
    pub fn rational(self) -> (u64, u64) {
        match self {
            FrameRate::Fps24 => (24, 1),
            FrameRate::Fps25 => (25, 1),
            FrameRate::Fps2997Df => (30_000, 1_001),
            FrameRate::Fps30 => (30, 1),
        }
    }

    pub fn is_drop_frame(self) -> bool {
        matches!(self, FrameRate::Fps2997Df)
    }

    /// Rate code of MTC Full Frame / Quarter Frame piece 7 (`rr` bits).
    pub fn from_mtc_code(code: u8) -> FrameRate {
        match code & 0x03 {
            0 => FrameRate::Fps24,
            1 => FrameRate::Fps25,
            2 => FrameRate::Fps2997Df,
            _ => FrameRate::Fps30,
        }
    }

    pub fn mtc_code(self) -> u8 {
        match self {
            FrameRate::Fps24 => 0,
            FrameRate::Fps25 => 1,
            FrameRate::Fps2997Df => 2,
            FrameRate::Fps30 => 3,
        }
    }

    /// Nominal interval between MTC Quarter Frames (4 per label frame).
    pub fn quarter_frame_interval_us(self) -> i64 {
        let (num, den) = self.rational();
        (den as i64 * 1_000_000) / (num as i64 * 4)
    }

    pub fn name(self) -> &'static str {
        match self {
            FrameRate::Fps24 => "24",
            FrameRate::Fps25 => "25",
            FrameRate::Fps2997Df => "29.97df",
            FrameRate::Fps30 => "30",
        }
    }

    /// Microseconds of `frames` frames, rounded down, without accumulated error.
    pub fn frames_to_us(self, frames: u64) -> u64 {
        let (num, den) = self.rational();
        ((frames as u128 * den as u128 * 1_000_000) / num as u128) as u64
    }

    /// Whole frames elapsed after `us` microseconds (rounded down).
    pub fn us_to_frames(self, us: u64) -> u64 {
        let (num, den) = self.rational();
        ((us as u128 * num as u128) / (den as u128 * 1_000_000)) as u64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimecodeError {
    Format,
    Hours,
    Minutes,
    Seconds,
    Frames,
    /// Frames 0 and 1 do not exist at the start of minutes not divisible by 10 (29.97 DF).
    DroppedFrame,
}

impl fmt::Display for TimecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            TimecodeError::Format => "label must be HH:MM:SS:FF",
            TimecodeError::Hours => "hours out of range (0..24)",
            TimecodeError::Minutes => "minutes out of range (0..60)",
            TimecodeError::Seconds => "seconds out of range (0..60)",
            TimecodeError::Frames => "frames out of range for the frame rate",
            TimecodeError::DroppedFrame => "frame dropped by 29.97 drop-frame counting",
        };
        f.write_str(s)
    }
}

impl std::error::Error for TimecodeError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timecode {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    pub rate: FrameRate,
}

const DF_FRAMES_PER_10MIN: u64 = 17_982; // 10 * 60 * 30 - 9 * 2
const DF_FRAMES_PER_MIN: u64 = 1_798; // 60 * 30 - 2
const DAY_SECONDS: u64 = 24 * 3600;

impl Timecode {
    pub fn new(hours: u8, minutes: u8, seconds: u8, frames: u8, rate: FrameRate) -> Result<Timecode, TimecodeError> {
        let t = Timecode { hours, minutes, seconds, frames, rate };
        t.validate()?;
        Ok(t)
    }

    pub fn parse(label: &str, rate: FrameRate) -> Result<Timecode, TimecodeError> {
        let parts: Vec<&str> = label.split(':').collect();
        if parts.len() != 4 || parts.iter().any(|p| p.len() != 2 || !p.bytes().all(|b| b.is_ascii_digit())) {
            return Err(TimecodeError::Format);
        }
        let n = |i: usize| parts[i].parse::<u8>().map_err(|_| TimecodeError::Format);
        Timecode::new(n(0)?, n(1)?, n(2)?, n(3)?, rate)
    }

    pub fn validate(&self) -> Result<(), TimecodeError> {
        if self.hours >= 24 {
            return Err(TimecodeError::Hours);
        }
        if self.minutes >= 60 {
            return Err(TimecodeError::Minutes);
        }
        if self.seconds >= 60 {
            return Err(TimecodeError::Seconds);
        }
        if self.frames as u32 >= self.rate.nominal() {
            return Err(TimecodeError::Frames);
        }
        if self.rate.is_drop_frame() && self.seconds == 0 && self.frames < 2 && self.minutes % 10 != 0 {
            return Err(TimecodeError::DroppedFrame);
        }
        Ok(())
    }

    /// Continuous frame index since 00:00:00:00 (drop-frame aware).
    pub fn frame_index(&self) -> u64 {
        let nominal = self.rate.nominal() as u64;
        let total_minutes = self.hours as u64 * 60 + self.minutes as u64;
        let labelled = (total_minutes * 60 + self.seconds as u64) * nominal + self.frames as u64;
        if self.rate.is_drop_frame() {
            labelled - 2 * (total_minutes - total_minutes / 10)
        } else {
            labelled
        }
    }

    pub fn from_frame_index(index: u64, rate: FrameRate) -> Timecode {
        let nominal = rate.nominal() as u64;
        let mut i = index;
        if rate.is_drop_frame() {
            let day = DF_FRAMES_PER_10MIN * 6 * 24;
            i %= day;
            let tens = i / DF_FRAMES_PER_10MIN;
            let rem = i % DF_FRAMES_PER_10MIN;
            // Frame index -> labelled frame count: add back the dropped labels.
            let dropped = if rem < 2 { 0 } else { (rem - 2) / DF_FRAMES_PER_MIN };
            i = i + 18 * tens + 2 * dropped;
        } else {
            i %= DAY_SECONDS * nominal;
        }
        let frames = (i % nominal) as u8;
        let total_seconds = i / nominal;
        Timecode {
            hours: (total_seconds / 3600 % 24) as u8,
            minutes: (total_seconds / 60 % 60) as u8,
            seconds: (total_seconds % 60) as u8,
            frames,
            rate,
        }
    }

    /// Microseconds since 00:00:00:00 at the exact frame rate.
    pub fn to_us(&self) -> u64 {
        self.rate.frames_to_us(self.frame_index())
    }

    /// Label of `self + offset` (e.g. the MTC label of a show time with
    /// `self` = origin). Negative results clamp to 00:00:00:00; labels wrap
    /// at 24 h.
    pub fn add_us(&self, offset_us: ShowTimeUs) -> Timecode {
        let base = self.to_us() as i128 + offset_us as i128;
        let us = base.max(0) as u64;
        Timecode::from_frame_index(self.rate.us_to_frames(us), self.rate)
    }
}

impl fmt::Display for Timecode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}:{:02}:{:02}", self.hours, self.minutes, self.seconds, self.frames)
    }
}

/// Beat input at render time (Link or MIDI Clock). `beat` counts quarter
/// notes; phase-dependent effects need `phase_valid`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeatState {
    pub beat: f64,
    pub tempo_bpm: f64,
    /// False while the beat origin is not established (e.g. Clock joined
    /// without Start / SPP). Pulse is transparent then.
    pub phase_valid: bool,
}

impl BeatState {
    pub fn new(beat: f64, tempo_bpm: f64) -> BeatState {
        BeatState { beat, tempo_bpm, phase_valid: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_frame_labels_follow_the_rule() {
        let df = FrameRate::Fps2997Df;
        assert_eq!(Timecode::parse("00:01:00:00", df), Err(TimecodeError::DroppedFrame));
        assert_eq!(Timecode::parse("00:01:00:01", df), Err(TimecodeError::DroppedFrame));
        assert!(Timecode::parse("00:01:00:02", df).is_ok());
        assert!(Timecode::parse("00:10:00:00", df).is_ok());
        assert!(Timecode::parse("00:00:00:00", df).is_ok());
        // The same labels are fine at 30 NDF.
        assert!(Timecode::parse("00:01:00:00", FrameRate::Fps30).is_ok());
    }

    #[test]
    fn label_ranges_and_format() {
        let r = FrameRate::Fps25;
        assert_eq!(Timecode::parse("24:00:00:00", r), Err(TimecodeError::Hours));
        assert_eq!(Timecode::parse("00:60:00:00", r), Err(TimecodeError::Minutes));
        assert_eq!(Timecode::parse("00:00:60:00", r), Err(TimecodeError::Seconds));
        assert_eq!(Timecode::parse("00:00:00:25", r), Err(TimecodeError::Frames));
        assert_eq!(Timecode::parse("0:00:00:00", r), Err(TimecodeError::Format));
        assert_eq!(Timecode::parse("00:00:00", r), Err(TimecodeError::Format));
    }

    #[test]
    fn frame_index_round_trips_including_drop_frame() {
        for rate in [FrameRate::Fps24, FrameRate::Fps25, FrameRate::Fps30, FrameRate::Fps2997Df] {
            for idx in [0u64, 1, 1799, 1800, 1801, 17_981, 17_982, 17_983, 107_892, 1_000_000] {
                let tc = Timecode::from_frame_index(idx, rate);
                assert!(tc.validate().is_ok(), "{rate:?} {idx} {tc}");
                assert_eq!(tc.frame_index(), idx, "{rate:?} {tc}");
            }
        }
        // 29.97 DF: 00:01:00:02 is frame 1800 (two labels dropped).
        assert_eq!(Timecode::parse("00:01:00:02", FrameRate::Fps2997Df).unwrap().frame_index(), 1800);
        assert_eq!(Timecode::parse("00:10:00:00", FrameRate::Fps2997Df).unwrap().frame_index(), 17_982);
    }

    #[test]
    fn exact_rational_conversion() {
        assert_eq!(FrameRate::Fps30.frames_to_us(30 * 3600), 3_600_000_000);
        assert_eq!(FrameRate::Fps2997Df.frames_to_us(30_000), 1_001_000_000);
        assert_eq!(FrameRate::Fps25.us_to_frames(1_000_000), 25);
        let one_hour_30 = Timecode::parse("01:00:00:00", FrameRate::Fps30).unwrap();
        assert_eq!(one_hour_30.to_us(), 3_600_000_000);
    }

    #[test]
    fn mtc_label_is_origin_plus_show_time() {
        let origin = Timecode::parse("01:00:00:00", FrameRate::Fps30).unwrap();
        assert_eq!(origin.add_us(0).to_string(), "01:00:00:00");
        assert_eq!(origin.add_us(1_000_000).to_string(), "01:00:01:00");
        assert_eq!(origin.add_us(33_334).to_string(), "01:00:00:01");
        assert_eq!(origin.add_us(-1).to_string(), "00:59:59:29");
        let zero = Timecode::parse("00:00:00:00", FrameRate::Fps30).unwrap();
        assert_eq!(zero.add_us(-5_000_000).to_string(), "00:00:00:00");
    }
}
