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

/// The particle load (0.1.3 stage 5): 8 continuous layers, each a fountain
/// at the per-layer cap (64 per second × 6 s lifetime = 384 alive, 256
/// drawn) with the largest spot and tail, bouncing so every particle stays.
pub fn load_show_particles() -> Show {
    let layers: Vec<Value> = (0..8)
        .map(|l| {
            let blend = ["over", "add", "max"][l % 3];
            json!({
                "id": format!("layer{l}"), "name": format!("layer {l}"), "target_id": "main-pad", "enabled": true,
                "opacity": 1, "blend": blend,
                "content": {"kind": "continuous", "preset_id": format!("p{l}")},
            })
        })
        .collect();
    let presets: Vec<Value> = (0..8)
        .map(|l| {
            json!({"id": format!("p{l}"), "name": format!("p{l}"), "effect": {"type": "builtin.particles", "version": 1, "params": {
                "mode": "fountain", "rate": 64, "color": "#FFC040", "color_end": "#2000FF",
                "speed_min": 2, "speed_max": 16, "angle": 45 * l, "spread": 360, "gravity": -8, "drag": 0.5,
                "lifetime_ms": 6000, "size": 2, "tail": 4, "edge_mode": "bounce", "origin_x": 3.5, "origin_y": 3.5, "seed": l,
            }}})
        })
        .collect();
    let doc = json!({
        "schema_version": omoiiro_core::model::SCHEMA_VERSION, "id": "nfr03-particles", "name": "NFR03 particle load", "seed": 1,
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
    omoiiro_store::load_str(&doc.to_string()).expect("particle load show is valid")
}

/// The compound load (0.1.3 stage 6): 8 layers, each looping one compound
/// of 4 inner layers (32 drawn layers), every inner layer a timeline of
/// overlapping 2 s clips with fades over chase / pulse / noise / rain /
/// orb; or, with `particles`, every inner layer a capped fountain.
pub fn load_show_compounds(particles: bool) -> Show {
    let effects = [
        json!({"type": "builtin.chase", "version": 1, "params": {"color": "#00DCC8", "period_us": 2_000_000, "width": 3, "axis": "x"}}),
        json!({"type": "builtin.pulse", "version": 1, "params": {"color": "#A020F0", "beats_per_cycle": 0.5, "depth": 0.7, "waveform": "triangle"}}),
        json!({"type": "builtin.noise", "version": 1, "params": {"base_color": "#101020", "highlight_color": "#FFFFFF", "density": 0.3, "rate_hz": 2, "seed": 1, "smooth": true, "depth": 0.5}}),
        json!({"type": "builtin.rain", "version": 1, "params": {"color": "#2060FF", "direction": "down", "speed": 8, "density": 0.8, "tail": 4, "seed": 2, "flicker": 0}}),
        json!({"type": "builtin.orb", "version": 1, "params": {"color": "#FF6020", "center_x": 3.5, "center_y": 3.5, "radius": 3, "edge": 1, "light_angle": 45, "shading": 0.6, "dither": 0, "dither_rate_hz": 8, "seed": 3, "motion": "orbit", "speed": 0.5, "orbit_radius": 2, "bounce_angle": 30}}),
        json!({"type": "builtin.particles", "version": 1, "params": {
            "mode": "fountain", "rate": 64, "color": "#FFC040", "color_end": "#2000FF", "speed_min": 2, "speed_max": 16, "angle": 90, "spread": 360,
            "gravity": -8, "drag": 0.5, "lifetime_ms": 6000, "size": 2, "tail": 4, "edge_mode": "bounce", "seed": 1}}),
    ];
    let presets: Vec<Value> = effects.iter().enumerate().map(|(i, e)| json!({"id": format!("p{i}"), "name": format!("p{i}"), "effect": e})).collect();
    let length: i64 = 60_000_000;
    let blends = ["over", "add", "max"];
    let compounds: Vec<Value> = (0..8)
        .map(|c| {
            let inner: Vec<Value> = (0..4)
                .map(|i| {
                    let id = format!("c{c}i{i}");
                    let blend = blends[(c + i) % 3];
                    if particles {
                        return json!({"id": id, "name": id, "enabled": true, "opacity": 1, "blend": blend, "content": {"kind": "continuous", "preset_id": "p5"}});
                    }
                    let mut clips = Vec::new();
                    let (mut start, mut k) = ((i as i64) * 250_000, 0);
                    while start < length {
                        clips.push(json!({"id": format!("{id}k{k}"), "preset_id": format!("p{}", (c + i + k) % 5), "start_us": start, "duration_us": 2_000_000, "fade_in_us": 500_000, "fade_out_us": 500_000}));
                        start += 1_500_000;
                        k += 1;
                    }
                    json!({"id": id, "name": id, "enabled": true, "opacity": 0.8, "blend": blend, "content": {"kind": "timeline", "clips": clips}})
                })
                .collect();
            json!({"id": format!("cp{c}"), "name": format!("cp{c}"), "duration_us": length, "layers": inner})
        })
        .collect();
    let layers: Vec<Value> = (0..8)
        .map(|l| {
            let blend = blends[l % 3];
            json!({
                "id": format!("layer{l}"), "name": format!("layer {l}"), "target_id": "main-pad", "enabled": true, "opacity": 1, "blend": blend,
                "content": {"kind": "timeline", "clips": [{"id": format!("place{l}"), "compound_id": format!("cp{l}"), "start_us": 0, "duration_us": 600_000_000, "fade_in_us": 0, "fade_out_us": 0, "loop": true, "tint": if l % 2 == 0 { Value::Null } else { json!("#40FF80") }}]},
            })
        })
        .collect();
    let mut doc = json!({
        "schema_version": omoiiro_core::model::SCHEMA_VERSION, "id": "nfr03-compounds", "name": "NFR03 compound load", "seed": 1,
        "sync": {
            "timecode": {"source": "live_bridge", "frame_rate": "30", "origin": "00:00:00:00", "input_offset_us": 0},
            "beat_source": "midi_clock", "stop_behavior": "hold",
            "loss": {"holdover_ms": 250, "blackout_after_ms": 1000, "recovery_fade_ms": 100},
        },
        "render": {"fps": 30, "master_brightness": 0.8},
        "preview": {"bpm": 128},
        "targets": [{"id": "main-pad", "name": "Launchpad X", "profile": omoiiro_core::surface::GRID64_PROFILE}],
        "presets": presets,
        "compounds": compounds,
        "layers": layers,
    });
    // No tint on even layers: drop the nulls.
    for l in doc["layers"].as_array_mut().into_iter().flatten() {
        if let Some(c) = l["content"]["clips"][0].as_object_mut() {
            c.retain(|_, v| !v.is_null());
        }
    }
    omoiiro_store::load_str(&doc.to_string()).expect("compound load show is valid")
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
    report_load(h, build, "64 pads, 8 layers, timeline clips overlapping with fades, solid/chase/pulse, over/add/max")
}

/// `report` for a named load (the particle load).
pub fn report_load(h: &Histogram, build: &str, load: &str) -> Value {
    json!({
        "nfr03": {
            "load": load,
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
