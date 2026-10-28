//! Shared formatting of decoder snapshots for the console and JSON output.

use crate::sync::clock::{ClockEvent, ClockSnapshot};
use crate::sync::mtc::{Label, MtcEvent, MtcSnapshot};
use serde_json::{json, Value};

pub fn us_to_secs(us: i64) -> f64 {
    us as f64 / 1_000_000.0
}

/// `HH:MM:SS.mmm` rendering of a position in microseconds.
pub fn position_hms(us: i64) -> String {
    let ms_total = us.div_euclid(1000);
    let ms = ms_total.rem_euclid(1000);
    let s_total = ms_total.div_euclid(1000);
    let s = s_total.rem_euclid(60);
    let m = (s_total.div_euclid(60)).rem_euclid(60);
    let h = s_total.div_euclid(3600);
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

pub fn mtc_line(s: &MtcSnapshot) -> String {
    let rate = s.rate.map(|r| r.name()).unwrap_or("--");
    let label = s.label.map(|l| l.to_string()).unwrap_or_else(|| "--:--:--:--".into());
    let pos = s
        .position_us
        .map(position_hms)
        .unwrap_or_else(|| "--".into());
    let age = s
        .age_us
        .map(|a| format!("{}ms", a / 1000))
        .unwrap_or_else(|| "--".into());
    let fault = s.fault.map(|f| format!(" [{f}]")).unwrap_or_default();
    format!(
        "MTC {rate:>7} {health:<9} raw={label} pos={pos}{adv} age={age} qf={qf} seq={seq} drop={drop} disc={disc} ff={ff} ff_disc={ffd}{fault}",
        health = s.health.name(),
        adv = if s.advancing { " adv" } else { "    " },
        qf = s.stats.quarter_frames,
        seq = s.stats.sequences,
        drop = s.stats.dropped,
        disc = s.stats.discontinuities,
        ff = s.stats.full_frames,
        ffd = s.stats.ff_discontinuities,
    )
}

pub fn mtc_json(s: &MtcSnapshot) -> Value {
    json!({
        "health": s.health.name(),
        "fault": s.fault,
        "rate": s.rate.map(|r| r.name()),
        "label": s.label.map(|l| l.to_string()),
        "position_us": s.position_us,
        "position_hms": s.position_us.map(position_hms),
        "advancing": s.advancing,
        "blackout": s.blackout,
        "age_us": s.age_us,
        "stats": {
            "quarter_frames": s.stats.quarter_frames,
            "full_frames": s.stats.full_frames,
            "sequences": s.stats.sequences,
            "reverse_sequences": s.stats.reverse_sequences,
            "dropped": s.stats.dropped,
            "stray_pieces": s.stats.stray_pieces,
            "discontinuities": s.stats.discontinuities,
            "ff_discontinuities": s.stats.ff_discontinuities,
            "rate_changes": s.stats.rate_changes,
            "invalid_labels": s.stats.invalid_labels,
            "locks": s.stats.locks,
        }
    })
}

pub fn mtc_event_line(e: &MtcEvent) -> String {
    match e {
        MtcEvent::FullFrame { label } => format!("full_frame {label} ({})", label.rate.name()),
        MtcEvent::Sequence { label, .. } => format!("sequence {label}"),
        MtcEvent::ReverseSequence { label } => format!("reverse_sequence {label}"),
        MtcEvent::Locked { label } => format!("locked {label}"),
        MtcEvent::Discontinuity { previous, current, delta_frames } => {
            format!("discontinuity {previous} -> {current} (delta {delta_frames:+} frames)")
        }
        MtcEvent::FullFrameDiscontinuity { previous_us, previous_health, current, delta_frames } => {
            // Previous position as the frame label it falls in, at the Full Frame rate.
            let fps = current.rate.nominal() as i64;
            let prev_index = (*previous_us as i128 * fps as i128).div_euclid(1_000_000) as u64;
            let previous = Label::from_frame_index(prev_index, current.rate);
            format!(
                "discontinuity(full_frame) {previous} -> {current} (delta {delta_frames:+} frames, from {})",
                previous_health.name()
            )
        }
        MtcEvent::FreewheelLocked { label, predicted_us, delta_us } => {
            let fps = label.rate.nominal() as f64;
            format!(
                "freewheel_locked {label} predicted={} (label - predicted {:+.2} frames, {:+} us)",
                position_hms(*predicted_us),
                *delta_us as f64 * fps / 1_000_000.0,
                delta_us
            )
        }
        MtcEvent::RateChange { previous, current } => {
            format!("rate_change {} -> {}", previous.name(), current.name())
        }
        MtcEvent::Dropped { reason } => format!("dropped {reason:?}"),
        MtcEvent::InvalidLabel { label, error } => format!("invalid_label {label} {error:?}"),
        MtcEvent::UnsupportedRate { rate } => format!("unsupported_rate {}", rate.name()),
    }
}

/// Events worth a console line (sequence completions are too frequent).
pub fn mtc_event_is_notable(e: &MtcEvent) -> bool {
    !matches!(e, MtcEvent::Sequence { .. })
}

pub fn clock_line(s: &ClockSnapshot) -> String {
    let bpm = s.bpm.map(|b| format!("{b:6.2}")).unwrap_or_else(|| "  --  ".into());
    let phase = s
        .quarter_phase
        .map(|p| format!("{p:.2}"))
        .unwrap_or_else(|| "--".into());
    let beats = s.beats.map(|b| format!("{b:.2}")).unwrap_or_else(|| "--".into());
    let age = s
        .age_us
        .map(|a| format!("{}ms", a / 1000))
        .unwrap_or_else(|| "--".into());
    format!(
        "CLK {bpm}bpm {transport:<8} phase={phase} beats={beats}{valid} age={age} ticks={ticks}",
        transport = s.transport.name(),
        valid = if s.position_valid { " pos-ok" } else { " pos-?" },
        ticks = s.stats.ticks,
    )
}

pub fn clock_json(s: &ClockSnapshot) -> Value {
    json!({
        "bpm": s.bpm,
        "transport": s.transport.name(),
        "position_valid": s.position_valid,
        "quarter_phase": s.quarter_phase,
        "beats": s.beats,
        "age_us": s.age_us,
        "stats": {
            "ticks": s.stats.ticks,
            "ticks_while_stopped": s.stats.ticks_while_stopped,
            "starts": s.stats.starts,
            "continues": s.stats.continues,
            "stops": s.stats.stops,
            "song_positions": s.stats.song_positions,
            "stalls": s.stats.stalls,
        }
    })
}

pub fn clock_event_line(e: &ClockEvent) -> String {
    match e {
        ClockEvent::Start => "start".into(),
        ClockEvent::Continue => "continue".into(),
        ClockEvent::Stop => "stop".into(),
        ClockEvent::SongPosition { sixteenths } => format!("song_position {sixteenths} (16ths)"),
        ClockEvent::Stalled { silent_us } => format!("stalled (no tick for {} ms)", silent_us / 1000),
        ClockEvent::Resumed { silent_us } => format!("resumed (ticks back after {} ms)", silent_us / 1000),
        ClockEvent::Seeded { ticks } => format!("origin seeded from the bridge beats ({:.2} beats)", *ticks as f64 / 24.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hms_formatting() {
        assert_eq!(position_hms(0), "00:00:00.000");
        assert_eq!(position_hms(3_600_066_666), "01:00:00.066");
        assert_eq!(position_hms(-1_000), "-1:59:59.999");
    }
}
