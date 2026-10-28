//! Semantic validation (docs/04-show-data.md "二段階の検証", task C03).
//! Runs after the JSON Schema check. Every issue carries a JSON Pointer.
//!
//! 1. ID uniqueness per kind (targets, presets, layers); clip IDs are
//!    unique across the whole show.
//! 2. References: layer target, clip / continuous preset; known profile and
//!    effect version.
//! 3. `start + duration` within the safe integer range; `fade_in +
//!    fade_out <= duration`.
//! 4. Origin label valid for the frame rate (drop-frame rule included);
//!    origin + last clip end + offset within the safe range.
//! 5. `blackout_after_ms > holdover_ms`.
//! 6. Resource limits (1 target, 8 layers, 256 presets, 4096 clips per
//!    timeline, string lengths) and value ranges the renderer relies on
//!    (colors, opacity, depth, beats per cycle, period, width).

use crate::model::{Content, Effect, Show, TriggerOn, TriggerSpec, SCHEMA_VERSION};
use crate::surface::{Rgb, GRID64_PROFILE};
use crate::time::{Timecode, MAX_SAFE_INTEGER};
use std::collections::{HashMap, HashSet};
use std::fmt;

pub const MAX_TARGETS: usize = 1;
pub const MAX_LAYERS: usize = 8;
pub const MAX_PRESETS: usize = 256;
pub const MAX_CLIPS_PER_TIMELINE: usize = 4096;
pub const MAX_NAME_CHARS: usize = 128;
pub const MAX_ID_CHARS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// JSON Pointer of the offending value, e.g. `/layers/1/content/clips/0/preset_id`.
    pub pointer: String,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.pointer, self.message)
    }
}

struct Ctx {
    issues: Vec<Issue>,
}

impl Ctx {
    fn push(&mut self, pointer: impl Into<String>, message: impl Into<String>) {
        self.issues.push(Issue { pointer: pointer.into(), message: message.into() });
    }

    fn unique<'a>(&mut self, seen: &mut HashSet<&'a str>, id: &'a str, pointer: String, kind: &str) {
        if !seen.insert(id) {
            self.push(pointer, format!("duplicate {kind} id {id:?}"));
        }
    }

    fn name(&mut self, s: &str, pointer: String) {
        let n = s.chars().count();
        if n == 0 || n > MAX_NAME_CHARS {
            self.push(pointer, format!("name must be 1..={MAX_NAME_CHARS} characters (got {n})"));
        }
    }

    fn id(&mut self, s: &str, pointer: String) {
        let ok = s.len() <= MAX_ID_CHARS
            && s.bytes().next().is_some_and(|c| c.is_ascii_lowercase())
            && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-');
        if !ok {
            self.push(pointer, format!("invalid id {s:?} (^[a-z][a-z0-9_-]{{0,63}}$)"));
        }
    }

    fn unit(&mut self, v: f64, pointer: String) {
        if !(0.0..=1.0).contains(&v) {
            self.push(pointer, format!("must be within 0..=1 (got {v})"));
        }
    }

    /// Counts (seed, repeats, ...) only step between keyframes.
    fn step_only<T>(&mut self, p: &crate::model::Param<T>, pointer: String) {
        if let crate::model::Param::Keyframes(k) = p {
            if k.iter().any(|x| x.ease != crate::model::Ease::Step) {
                self.push(pointer, "a count only steps: every keyframe ease must be \"step\"");
            }
        }
    }

    fn range(&mut self, v: f64, lo: f64, hi: f64, pointer: String) {
        if !(lo..=hi).contains(&v) {
            self.push(pointer, format!("must be within {lo}..={hi} (got {v})"));
        }
    }

    fn color(&mut self, s: &str, pointer: String) {
        if Rgb::from_hex(s).is_none() {
            self.push(pointer, format!("invalid color {s:?} (#RRGGBB)"));
        }
    }
}

/// Validate a show. An empty result means the show can be built into a renderer.
pub fn validate(show: &Show) -> Vec<Issue> {
    let mut c = Ctx { issues: Vec::new() };

    if show.schema_version != SCHEMA_VERSION {
        c.push("/schema_version", format!("unsupported schema_version {:?} (expected {SCHEMA_VERSION})", show.schema_version));
    }
    c.id(&show.id, "/id".into());
    c.name(&show.name, "/name".into());

    // 4. Timecode origin.
    let tc = &show.sync.timecode;
    let origin = match Timecode::parse(&tc.origin, tc.frame_rate) {
        Ok(t) => Some(t),
        Err(e) => {
            c.push("/sync/timecode/origin", format!("{e} ({:?} at {})", tc.origin, tc.frame_rate.name()));
            None
        }
    };
    if !(-10_000_000..=10_000_000).contains(&tc.input_offset_us) {
        c.push("/sync/timecode/input_offset_us", "must be within -10 s..=10 s");
    }

    // 5. Loss timing.
    let loss = &show.sync.loss;
    if loss.blackout_after_ms <= loss.holdover_ms {
        c.push(
            "/sync/loss/blackout_after_ms",
            format!("blackout_after_ms ({}) must be greater than holdover_ms ({})", loss.blackout_after_ms, loss.holdover_ms),
        );
    }

    if !matches!(show.render.fps, 30 | 60) {
        c.push("/render/fps", format!("render fps must be 30 or 60 (got {})", show.render.fps));
    }
    c.unit(show.render.master_brightness, "/render/master_brightness".into());
    if !(20.0..=300.0).contains(&show.preview.bpm) {
        c.push("/preview/bpm", "preview bpm must be within 20..=300");
    }

    // 6. Counts.
    if show.targets.is_empty() || show.targets.len() > MAX_TARGETS {
        c.push("/targets", format!("exactly {MAX_TARGETS} target is supported (got {})", show.targets.len()));
    }
    if show.presets.is_empty() || show.presets.len() > MAX_PRESETS {
        c.push("/presets", format!("1..={MAX_PRESETS} presets (got {})", show.presets.len()));
    }
    if show.layers.len() > MAX_LAYERS {
        c.push("/layers", format!("at most {MAX_LAYERS} layers (got {})", show.layers.len()));
    }

    // 1-2. Targets.
    let mut target_ids = HashSet::new();
    for (i, t) in show.targets.iter().enumerate() {
        let p = format!("/targets/{i}");
        c.id(&t.id, format!("{p}/id"));
        c.unique(&mut target_ids, &t.id, format!("{p}/id"), "target");
        c.name(&t.name, format!("{p}/name"));
        if t.profile != GRID64_PROFILE {
            c.push(format!("{p}/profile"), format!("unknown profile {:?}", t.profile));
        }
    }

    // 1-2, 6. Presets.
    let mut preset_ids: HashMap<&str, usize> = HashMap::new();
    for (i, pr) in show.presets.iter().enumerate() {
        let p = format!("/presets/{i}");
        c.id(&pr.id, format!("{p}/id"));
        if preset_ids.insert(&pr.id, i).is_some() {
            c.push(format!("{p}/id"), format!("duplicate preset id {:?}", pr.id));
        }
        c.name(&pr.name, format!("{p}/name"));
        let e = format!("{p}/effect");
        if pr.effect.version() != 1 {
            c.push(format!("{e}/version"), format!("{} supports version 1 (got {})", pr.effect.type_name(), pr.effect.version()));
        }
        match &pr.effect {
            Effect::Solid { params, .. } => {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
            },
            Effect::Chase { params, .. } => {
                {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
            };
                if !params.period_us.values().all(|v| (1_000..=3_600_000_000).contains(v)) {
                    c.push(format!("{e}/params/period_us"), "period_us must be within 1000..=3600000000");
                }
                if !params.width.values().all(|v| (1..=8).contains(v)) {
                    c.push(format!("{e}/params/width"), "width must be within 1..=8");
                }
                c.step_only(&params.width, format!("{e}/params/width"));
            }
            Effect::Pulse { params, .. } => {
                {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
            };
                if !params.beats_per_cycle.values().all(|v| [0.25, 0.5, 1.0].contains(v)) {
                    c.push(format!("{e}/params/beats_per_cycle"), "beats_per_cycle must be 0.25, 0.5 or 1");
                }
                for v in params.depth.values() {
                    c.unit(*v, format!("{e}/params/depth"));
                }
            }
            Effect::Strobe { params, .. } => {
                {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
            };
                for v in params.rate_hz.values() {
                    c.range(*v, 0.5, 30.0, format!("{e}/params/rate_hz"));
                }
                if !params.beats_per_cycle.values().all(|v| [0.25, 0.5, 1.0, 2.0].contains(v)) {
                    c.push(format!("{e}/params/beats_per_cycle"), "beats_per_cycle must be 0.25, 0.5, 1 or 2");
                }
                for v in params.duty.values() {
                    c.range(*v, 0.05, 0.95, format!("{e}/params/duty"));
                }
                for v in params.phase_offset.values() {
                    c.unit(*v, format!("{e}/params/phase_offset"));
                }
            }
            Effect::Rain { params, .. } => {
                {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
            };
                if let Some(h) = &params.head_color {
                    for v in h.values() {
                        c.color(v, format!("{e}/params/head_color"));
                    }
                }
                for v in params.speed.values() {
                    c.range(*v, 0.5, 16.0, format!("{e}/params/speed"));
                }
                for v in params.density.values() {
                    c.unit(*v, format!("{e}/params/density"));
                }
                for v in params.tail.values() {
                    c.range(*v, 0.0, 8.0, format!("{e}/params/tail"));
                }
                if params.seed.values().any(|v| *v > i32::MAX as u32) {
                    c.push(format!("{e}/params/seed"), "seed must be within 0..=2147483647");
                }
                c.step_only(&params.seed, format!("{e}/params/seed"));
                for v in params.flicker.values() {
                    c.unit(*v, format!("{e}/params/flicker"));
                }
            }
            Effect::Wave { params, .. } => {
                {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
            };
                for v in params.wavelength.values() {
                    c.range(*v, 2.0, 16.0, format!("{e}/params/wavelength"));
                }
                for v in params.amplitude.values() {
                    c.range(*v, 0.5, 3.5, format!("{e}/params/amplitude"));
                }
                for v in params.center.values() {
                    c.range(*v, 0.0, 7.0, format!("{e}/params/center"));
                }
                for v in params.speed.values() {
                    c.range(*v, -4.0, 4.0, format!("{e}/params/speed"));
                }
                for v in params.thickness.values() {
                    c.range(*v, 0.5, 3.0, format!("{e}/params/thickness"));
                }
                if let Some(p) = &params.phase {
                    for v in p.values() {
                        c.unit(*v, format!("{e}/params/phase"));
                    }
                }
            }
            Effect::Orb { params, .. } => {
                {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
            };
                for v in params.center_x.values() {
                    c.range(*v, 0.0, 7.0, format!("{e}/params/center_x"));
                }
                for v in params.center_y.values() {
                    c.range(*v, 0.0, 7.0, format!("{e}/params/center_y"));
                }
                for v in params.radius.values() {
                    c.range(*v, 0.5, 6.0, format!("{e}/params/radius"));
                }
                for v in params.edge.values() {
                    c.range(*v, 0.0, 2.0, format!("{e}/params/edge"));
                }
                for v in params.light_angle.values() {
                    c.range(*v, 0.0, 360.0, format!("{e}/params/light_angle"));
                }
                for v in params.shading.values() {
                    c.unit(*v, format!("{e}/params/shading"));
                }
                for v in params.dither.values() {
                    c.unit(*v, format!("{e}/params/dither"));
                }
                for v in params.dither_rate_hz.values() {
                    c.range(*v, 0.5, 15.0, format!("{e}/params/dither_rate_hz"));
                }
                if params.seed.values().any(|v| *v > i32::MAX as u32) {
                    c.push(format!("{e}/params/seed"), "seed must be within 0..=2147483647");
                }
                c.step_only(&params.seed, format!("{e}/params/seed"));
                // The speed range depends on the motion (unused for none); keyframed
                // motions take the widest range of the motions they use.
                let (lo, hi) = params.motion.values().fold((f64::MAX, 0.0f64), |(lo, hi), m| match m {
                    crate::model::OrbMotion::None => (lo.min(0.0), hi.max(12.0)),
                    crate::model::OrbMotion::Orbit => (lo.min(0.05), hi.max(2.0)),
                    crate::model::OrbMotion::Bounce => (lo.min(0.5), hi.max(12.0)),
                });
                for v in params.speed.values() {
                    c.range(*v, lo, hi, format!("{e}/params/speed"));
                }
                for v in params.orbit_radius.values() {
                    c.range(*v, 0.0, 4.0, format!("{e}/params/orbit_radius"));
                }
                for v in params.bounce_angle.values() {
                    c.range(*v, 0.0, 360.0, format!("{e}/params/bounce_angle"));
                }
                if let Some(p) = &params.orbit_phase {
                    for v in p.values() {
                        c.unit(*v, format!("{e}/params/orbit_phase"));
                    }
                }
            }
            Effect::Regions { params, .. } => {
                {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
            };
                if let Some(b) = &params.color_b {
                    for v in b.values() {
                        c.color(v, format!("{e}/params/color_b"));
                    }
                }
                if !params.step_beats.values().all(|v| [0.25, 0.5, 1.0, 2.0, 4.0].contains(v)) {
                    c.push(format!("{e}/params/step_beats"), "step_beats must be 0.25, 0.5, 1, 2 or 4");
                }
                if !params.step_ms.values().all(|v| (50..=5000).contains(v)) {
                    c.push(format!("{e}/params/step_ms"), "step_ms must be within 50..=5000");
                }
                for v in params.soft.values() {
                    c.unit(*v, format!("{e}/params/soft"));
                }
                for v in params.duty.values() {
                    c.range(*v, 0.05, 1.0, format!("{e}/params/duty"));
                }
            }
            Effect::TapGlow { params, .. } => {
                for (name, v) in [("color_start", &params.color_start), ("color_mid", &params.color_mid), ("color_end", &params.color_end)] {
                    for v in v.values() {
                        c.color(v, format!("{e}/params/{name}"));
                    }
                }
                if !params.lifetime_ms.values().all(|v| (100..=5000).contains(v)) {
                    c.push(format!("{e}/params/lifetime_ms"), "lifetime_ms must be within 100..=5000");
                }
                for v in params.radius.values() {
                    c.range(*v, 0.0, 3.0, format!("{e}/params/radius"));
                }
                if !params.repeats.values().all(|v| (1..=8).contains(v)) {
                    c.push(format!("{e}/params/repeats"), "repeats must be within 1..=8");
                }
                c.step_only(&params.repeats, format!("{e}/params/repeats"));
                if !params.repeat_interval_ms.values().all(|v| (50..=2000).contains(v)) {
                    c.push(format!("{e}/params/repeat_interval_ms"), "repeat_interval_ms must be within 50..=2000");
                }
                for v in params.repeat_decay.values() {
                    c.unit(*v, format!("{e}/params/repeat_decay"));
                }
            }
            Effect::Cross { params, .. } => {
                {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
            };
                for v in params.speed.values() {
                    c.range(*v, 1.0, 32.0, format!("{e}/params/speed"));
                }
                for v in params.length.values() {
                    c.range(*v, 0.5, 8.0, format!("{e}/params/length"));
                }
                if !params.lifetime_ms.values().all(|v| (100..=3000).contains(v)) {
                    c.push(format!("{e}/params/lifetime_ms"), "lifetime_ms must be within 100..=3000");
                }
            }
            Effect::Ripple { params, .. } => {
                {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
            };
                for v in params.speed.values() {
                    c.range(*v, 0.5, 16.0, format!("{e}/params/speed"));
                }
                for v in params.ring_width.values() {
                    c.range(*v, 0.5, 4.0, format!("{e}/params/ring_width"));
                }
                if !params.lifetime_ms.values().all(|v| (200..=4000).contains(v)) {
                    c.push(format!("{e}/params/lifetime_ms"), "lifetime_ms must be within 200..=4000");
                }
                if !params.max_concurrent.values().all(|v| (1..=16).contains(v)) {
                    c.push(format!("{e}/params/max_concurrent"), "max_concurrent must be within 1..=16");
                }
                c.step_only(&params.max_concurrent, format!("{e}/params/max_concurrent"));
                let interval = params.trigger.values().any(|t| *t == crate::model::RippleTrigger::Interval);
                for (name, v) in [("origin_x", &params.origin_x), ("origin_y", &params.origin_y)] {
                    match v {
                        Some(v) => v.values().for_each(|x| c.range(*x, 0.0, 7.0, format!("{e}/params/{name}"))),
                        None if interval => c.push(format!("{e}/params/{name}"), "required when trigger is interval"),
                        None => {}
                    }
                }
                match &params.interval_ms {
                    Some(v) if !v.values().all(|x| (100..=10_000).contains(x)) => c.push(format!("{e}/params/interval_ms"), "interval_ms must be within 100..=10000"),
                    None if interval => c.push(format!("{e}/params/interval_ms"), "required when trigger is interval"),
                    _ => {}
                }
            }
            Effect::Noise { params, .. } => {
                {
                for v in params.base_color.values() {
                    c.color(v, format!("{e}/params/base_color"));
                }
            };
                {
                for v in params.highlight_color.values() {
                    c.color(v, format!("{e}/params/highlight_color"));
                }
            };
                for v in params.density.values() {
                    c.unit(*v, format!("{e}/params/density"));
                }
                for v in params.rate_hz.values() {
                    c.range(*v, 0.5, 30.0, format!("{e}/params/rate_hz"));
                }
                if params.seed.values().any(|v| *v > i32::MAX as u32) {
                    c.push(format!("{e}/params/seed"), "seed must be within 0..=2147483647");
                }
                c.step_only(&params.seed, format!("{e}/params/seed"));
                for v in params.depth.values() {
                    c.unit(*v, format!("{e}/params/depth"));
                }
            }
        }
    }

    // 1-3, 6. Layers and clips.
    let mut layer_ids = HashSet::new();
    let mut clip_ids = HashSet::new();
    let mut max_end: u64 = 0;
    for (li, l) in show.layers.iter().enumerate() {
        let p = format!("/layers/{li}");
        c.id(&l.id, format!("{p}/id"));
        c.unique(&mut layer_ids, &l.id, format!("{p}/id"), "layer");
        c.name(&l.name, format!("{p}/name"));
        if !target_ids.contains(l.target_id.as_str()) {
            c.push(format!("{p}/target_id"), format!("unknown target {:?}", l.target_id));
        }
        c.unit(l.opacity, format!("{p}/opacity"));
        match &l.content {
            Content::Continuous { preset_id } => {
                if !preset_ids.contains_key(preset_id.as_str()) {
                    c.push(format!("{p}/content/preset_id"), format!("unknown preset {preset_id:?}"));
                }
            }
            Content::Timeline { clips } | Content::Triggered { clips, .. } => {
                if let Content::Triggered { trigger, .. } = &l.content {
                    check_trigger(&mut c, trigger, &format!("{p}/content/trigger"));
                    // Without coordinates of its own, only an effect that takes instances
                    // (ripple / tap_glow / cross) or a hold (drawn while held) makes sense.
                    for (ci, clip) in clips.iter().enumerate() {
                        if let Some(&pi) = preset_ids.get(clip.preset_id.as_str()) {
                            if trigger.on != TriggerOn::Hold && !show.presets[pi].effect.takes_instances() {
                                c.push(format!("{p}/content/clips/{ci}/preset_id"), format!("{} draws at fixed positions: use it with trigger on hold, or use ripple / tap_glow / cross", show.presets[pi].effect.type_name()));
                            }
                        }
                    }
                }
                if clips.len() > MAX_CLIPS_PER_TIMELINE {
                    c.push(format!("{p}/content/clips"), format!("at most {MAX_CLIPS_PER_TIMELINE} clips (got {})", clips.len()));
                }
                for (ci, clip) in clips.iter().enumerate() {
                    let cp = format!("{p}/content/clips/{ci}");
                    c.id(&clip.id, format!("{cp}/id"));
                    c.unique(&mut clip_ids, &clip.id, format!("{cp}/id"), "clip");
                    if !preset_ids.contains_key(clip.preset_id.as_str()) {
                        c.push(format!("{cp}/preset_id"), format!("unknown preset {:?}", clip.preset_id));
                    }
                    if clip.duration_us == 0 {
                        c.push(format!("{cp}/duration_us"), "duration_us must be positive");
                    }
                    match clip.start_us.checked_add(clip.duration_us) {
                        Some(end) if end <= MAX_SAFE_INTEGER => max_end = max_end.max(end),
                        _ => c.push(format!("{cp}/duration_us"), "start_us + duration_us exceeds the safe integer range"),
                    }
                    match clip.fade_in_us.checked_add(clip.fade_out_us) {
                        Some(f) if f <= clip.duration_us => {}
                        _ => c.push(format!("{cp}/fade_out_us"), "fade_in_us + fade_out_us must not exceed duration_us"),
                    }
                }
            }
        }
    }

    // 4. Offset conversion stays in range (origin + last end + offset).
    if let Some(o) = origin {
        let top = o.to_us() as i128 + max_end as i128 + tc.input_offset_us.max(0) as i128;
        if top > MAX_SAFE_INTEGER as i128 {
            c.push("/sync/timecode/origin", "origin + last clip end + input offset exceeds the safe integer range");
        }
    }

    c.issues
}

fn check_trigger(c: &mut Ctx, t: &TriggerSpec, p: &str) {
    match (t.on, t.interval_ms) {
        (TriggerOn::Interval, None) => c.push(format!("{p}/interval_ms"), "required when on is interval"),
        (_, Some(ms)) if !(100..=10_000).contains(&ms) => c.push(format!("{p}/interval_ms"), "interval_ms must be within 100..=10000"),
        _ => {}
    }
    for (name, v) in [("origin_x", t.origin_x), ("origin_y", t.origin_y)] {
        if let Some(v) = v {
            c.range(v, 0.0, 7.0, format!("{p}/{name}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_keyframe_value_is_checked_and_counts_only_step() {
        use crate::model::{Ease, Keyframe, Param};
        let mut s = crate::model::demo_for_tests();
        let mut tg = crate::edit::default_effect("builtin.tap_glow").unwrap();
        if let Effect::TapGlow { params, .. } = &mut tg {
            params.radius = Param::Keyframes(vec![Keyframe { t_us: 0, v: 1.0, ease: Ease::Linear }, Keyframe { t_us: 10, v: 9.0, ease: Ease::Linear }]);
            params.repeats = Param::Keyframes(vec![Keyframe { t_us: 0, v: 1, ease: Ease::Linear }, Keyframe { t_us: 10, v: 3, ease: Ease::Linear }]);
        }
        s.presets.push(crate::model::Preset { id: "tg".into(), name: "tg".into(), effect: tg });
        let e = validate(&s);
        assert!(e.iter().any(|i| i.pointer.ends_with("/params/radius")), "the 9.0 keyframe is out of range: {e:?}");
        assert!(e.iter().any(|i| i.pointer.ends_with("/params/repeats") && i.message.contains("step")), "{e:?}");
    }

    #[test]
    fn triggered_layers_check_their_trigger_and_effects() {
        use crate::model::{Clip, Layer, Preset};
        let mut s = crate::model::demo_for_tests();
        s.presets.push(Preset { id: "r".into(), name: "r".into(), effect: crate::edit::default_effect("builtin.ripple").unwrap() });
        let clip = |p: &str| Clip { id: format!("c-{p}"), preset_id: p.into(), start_us: 0, duration_us: 1_000_000, fade_in_us: 0, fade_out_us: 0 };
        let layer = |p: &str, on| Layer {
            id: "trig".into(),
            name: "trig".into(),
            target_id: "main-pad".into(),
            enabled: true,
            opacity: 1.0,
            blend: crate::model::Blend::Over,
            content: Content::Triggered { clips: vec![clip(p)], trigger: TriggerSpec { on, interval_ms: None, origin_x: None, origin_y: None } },
        };
        let with = |l: Layer| {
            let mut x = s.clone();
            x.layers.push(l);
            validate(&x)
        };
        assert!(with(layer("r", TriggerOn::Press)).is_empty());
        // A fixed-position effect only on hold.
        assert!(with(layer("base-blue", TriggerOn::Hold)).is_empty());
        let e = with(layer("base-blue", TriggerOn::Press));
        assert!(e[0].pointer.ends_with("/content/clips/0/preset_id"), "{e:?}");
        // Interval needs its interval.
        let e = with(layer("r", TriggerOn::Interval));
        assert!(e[0].pointer.ends_with("/content/trigger/interval_ms"), "{e:?}");
    }
    use crate::model::Show;

    fn demo() -> Show {
        crate::model::demo_for_tests()
    }

    fn pointers(s: &Show) -> Vec<String> {
        validate(s).into_iter().map(|i| i.pointer).collect()
    }

    #[test]
    fn demo_is_valid() {
        assert_eq!(validate(&demo()), vec![]);
    }

    #[test]
    fn duplicate_ids_and_unknown_references() {
        let mut s = demo();
        s.presets[1].id = "base-blue".into();
        s.layers[1].id = "base".into();
        if let Content::Timeline { clips } = &mut s.layers[1].content {
            clips[0].id = "intro-base".into(); // same as layer 0's clip
            clips[0].preset_id = "nope".into();
        }
        s.layers[2].target_id = "other-pad".into();
        let p = pointers(&s);
        assert!(p.contains(&"/presets/1/id".to_string()), "{p:?}");
        assert!(p.contains(&"/layers/1/id".to_string()), "{p:?}");
        assert!(p.contains(&"/layers/1/content/clips/0/id".to_string()), "{p:?}");
        assert!(p.contains(&"/layers/1/content/clips/0/preset_id".to_string()), "{p:?}");
        assert!(p.contains(&"/layers/2/target_id".to_string()), "{p:?}");
    }

    #[test]
    fn interval_and_fade_rules() {
        let mut s = demo();
        if let Content::Timeline { clips } = &mut s.layers[0].content {
            clips[0].fade_in_us = 40_000_000;
            clips[0].fade_out_us = 30_000_000; // 70 s of fades in a 60 s clip
        }
        if let Content::Timeline { clips } = &mut s.layers[1].content {
            clips[0].start_us = MAX_SAFE_INTEGER - 10;
        }
        let p = pointers(&s);
        assert!(p.contains(&"/layers/0/content/clips/0/fade_out_us".to_string()), "{p:?}");
        assert!(p.contains(&"/layers/1/content/clips/0/duration_us".to_string()), "{p:?}");
    }

    #[test]
    fn timecode_and_loss_rules() {
        let mut s = demo();
        s.sync.timecode.frame_rate = crate::time::FrameRate::Fps2997Df;
        s.sync.timecode.origin = "00:01:00:00".into(); // dropped label
        s.sync.loss.blackout_after_ms = 250; // equal to holdover
        let p = pointers(&s);
        assert!(p.contains(&"/sync/timecode/origin".to_string()), "{p:?}");
        assert!(p.contains(&"/sync/loss/blackout_after_ms".to_string()), "{p:?}");
        s.sync.timecode.origin = "00:10:00:00".into();
        s.sync.loss.blackout_after_ms = 1000;
        assert_eq!(validate(&s), vec![]);
    }

    #[test]
    fn resource_limits() {
        let mut s = demo();
        let l = s.layers[0].clone();
        for i in 0..6 {
            let mut x = l.clone();
            x.id = format!("extra-{i}");
            if let Content::Timeline { clips } = &mut x.content {
                clips[0].id = format!("extra-clip-{i}");
            }
            s.layers.push(x);
        }
        assert!(pointers(&s).contains(&"/layers".to_string()));
        let mut s = demo();
        s.targets.push(s.targets[0].clone());
        s.targets[1].id = "second".into();
        assert!(pointers(&s).contains(&"/targets".to_string()));
        let mut s = demo();
        s.name = "x".repeat(129);
        assert!(pointers(&s).contains(&"/name".to_string()));
    }

    #[test]
    fn effect_parameter_ranges() {
        let mut s = demo();
        if let Effect::Pulse { params, .. } = &mut s.presets[2].effect {
            params.beats_per_cycle = 2.0.into();
            params.depth = 1.5.into();
        }
        if let Effect::Chase { params, .. } = &mut s.presets[1].effect {
            params.width = 9.into();
        }
        let p = pointers(&s);
        assert!(p.contains(&"/presets/2/effect/params/beats_per_cycle".to_string()), "{p:?}");
        assert!(p.contains(&"/presets/2/effect/params/depth".to_string()), "{p:?}");
        assert!(p.contains(&"/presets/1/effect/params/width".to_string()), "{p:?}");
    }
}
