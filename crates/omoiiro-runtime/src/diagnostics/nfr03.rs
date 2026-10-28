//! NFR03 measurement: render time of the maximum MVP load (64 pads, 8
//! layers, overlapping timeline clips with fades, all three effects and
//! blend modes) at 30 fps show times. The result is a measurement on the
//! machine and build that ran it, not a device test.

use crate::diagnostics::perf::Histogram;
use crate::engine::{Engine, Overrides, ShowExchange};
use crate::sync::transport::{BeatHealth, Generations, OutputState, TransportSnapshot, TransportState};
use crate::sync::bridge::BridgeHealth;
use serde_json::{json, Value};
use omoiiro_core::model::Show;
use omoiiro_core::time::BeatState;
use std::time::Instant;

pub const NFR03_P95_LIMIT_US: i64 = 5_000;

/// The load show: 8 layers, each a timeline of 2 s clips overlapping by
/// 0.5 s (fades on both ends) over `minutes` minutes, cycling solid /
/// chase (x and y) / pulse (sine, triangle) and over / add / max.
pub fn load_show(minutes: u32) -> Show {
    let effects = [
        json!({"type": "builtin.solid", "version": 1, "params": {"color": "#204080"}}),
        json!({"type": "builtin.chase", "version": 1, "params": {"color": "#00DCC8", "period_us": 2_000_000, "width": 3, "axis": "x"}}),
        json!({"type": "builtin.chase", "version": 1, "params": {"color": "#FF6020", "period_us": 1_500_000, "width": 2, "axis": "y"}}),
        json!({"type": "builtin.pulse", "version": 1, "params": {"color": "#FFFFFF", "beats_per_cycle": 1, "depth": 1, "waveform": "sine"}}),
        json!({"type": "builtin.pulse", "version": 1, "params": {"color": "#A020F0", "beats_per_cycle": 0.5, "depth": 0.7, "waveform": "triangle"}}),
    ];
    let presets: Vec<Value> = effects.iter().enumerate().map(|(i, e)| json!({"id": format!("p{i}"), "name": format!("p{i}"), "effect": e})).collect();
    let blends = ["over", "add", "max"];
    let total_us: i64 = minutes as i64 * 60_000_000;
    let layers: Vec<Value> = (0..8)
        .map(|l| {
            let mut clips = Vec::new();
            let mut start = (l as i64) * 250_000;
            let mut k = 0;
            while start < total_us {
                clips.push(json!({
                    "id": format!("l{l}c{k}"), "preset_id": format!("p{}", (l + k) % presets.len()),
                    "start_us": start, "duration_us": 2_000_000, "fade_in_us": 500_000, "fade_out_us": 500_000,
                }));
                start += 1_500_000;
                k += 1;
            }
            json!({
                "id": format!("layer{l}"), "name": format!("layer {l}"), "target_id": "main-pad", "enabled": true,
                "opacity": 0.5 + 0.0625 * l as f64, "blend": blends[l % 3],
                "content": {"kind": "timeline", "clips": clips},
            })
        })
        .collect();
    let doc = json!({
        "schema_version": omoiiro_core::model::SCHEMA_VERSION, "id": "nfr03-load", "name": "NFR03 load", "seed": 1,
        "sync": {
            "timecode": {"source": "live_bridge", "frame_rate": "30", "origin": "00:00:00:00", "input_offset_us": 0},
            "beat_source": "midi_clock", "stop_behavior": "hold",
            "loss": {"holdover_ms": 250, "blackout_after_ms": 1000, "recovery_fade_ms": 100},
        },
        "render": {"fps": 30, "master_brightness": 0.8},
        "preview": {"bpm": 128},
        "targets": [{"id": "main-pad", "name": "Launchpad X", "profile": omoiiro_core::surface::GRID64_PROFILE}],
        "presets": presets,
        "layers": layers,
    });
    omoiiro_store::load_str(&doc.to_string()).expect("NFR03 load show is valid")
}

fn snapshot(t: i64, beat: f64) -> TransportSnapshot {
    TransportSnapshot {
        now_us: t,
        show_time_us: Some(t),
        advancing: true,
        transport: TransportState::Playing,
        stalled: false,
        time_health: BridgeHealth::Locked,
        output: OutputState::Follow,
        output_gain: 1.0,
        beat: Some(BeatState { beat, tempo_bpm: 128.0, phase_valid: true }),
        beat_health: BeatHealth::Locked,
        beat_seeded: false,
        clock_seeded: 0,
        mtc_label: None,
        generations: Generations::default(),
    }
}

/// Evaluate `frames` consecutive 30 fps frames (render + quantize, the work
/// of the render thread) and return the per-frame time histogram.
pub fn measure(show: &Show, frames: u64) -> Histogram {
    let ex = ShowExchange::new();
    ex.submit(show).expect("valid show");
    let mut engine = Engine::new();
    engine.adopt(&ex);
    let ov = Overrides::default();
    let mut h = Histogram::default();
    let mut lit = 0usize;
    for k in 0..frames {
        let t = (k as i64 * 1_000_000) / 30;
        let snap = snapshot(t, t as f64 / 1e6 * 128.0 / 60.0);
        let t0 = Instant::now();
        let e = engine.evaluate(&snap, &ov);
        h.record(t0.elapsed().as_micros() as i64);
        lit = lit.max(e.frame.lit());
    }
    assert_eq!(lit, 64, "the load lights every pad");
    h
}

pub fn report(h: &Histogram, build: &str) -> Value {
    json!({
        "nfr03": {
            "load": "64 pads, 8 layers, timeline clips overlapping with fades, solid/chase/pulse, over/add/max",
            "build": build,
            "render_time": h.summary_json(),
            "limit_p95_us": NFR03_P95_LIMIT_US,
            "pass": h.percentile(95.0).is_some_and(|p| p <= NFR03_P95_LIMIT_US),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfr03_render_p95_within_5ms() {
        let show = load_show(2);
        assert_eq!(show.layers.len(), 8);
        // 60 s of frames at 30 fps (debug build in `cargo test`; release is faster).
        let h = measure(&show, 1800);
        let p95 = h.percentile(95.0).unwrap();
        eprintln!("NFR03 (test build): {}", h.summary_json());
        assert!(p95 <= NFR03_P95_LIMIT_US, "render p95 {p95} us > {NFR03_P95_LIMIT_US} us");
    }
}
