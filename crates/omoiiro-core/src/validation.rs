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
/// Inner layers of one compound (0.1.3 stage 6).
pub const MAX_COMPOUND_LAYERS: usize = 4;
pub const MAX_COMPOUNDS: usize = 64;
/// Frames of one `builtin.frames` preset.
pub const MAX_FRAMES: usize = 240;
/// Presses in one recording.
pub const MAX_RECORDED: usize = 1024;
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
/// Centre range of shapes and masks: a little off the grid so a wipe or a
/// large shape can cover all of it or none of it.
const CENTER_MIN: f64 = -4.0;
const CENTER_MAX: f64 = 11.0;

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
            }
            Effect::Chase { params, .. } => {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
                if !params.period_us.values().all(|v| (1_000..=3_600_000_000).contains(v)) {
                    c.push(format!("{e}/params/period_us"), "period_us must be within 1000..=3600000000");
                }
                if !params.width.values().all(|v| (1..=8).contains(v)) {
                    c.push(format!("{e}/params/width"), "width must be within 1..=8");
                }
                c.step_only(&params.width, format!("{e}/params/width"));
            }
            Effect::Pulse { params, .. } => {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
                if !params.beats_per_cycle.values().all(|v| [0.25, 0.5, 1.0].contains(v)) {
                    c.push(format!("{e}/params/beats_per_cycle"), "beats_per_cycle must be 0.25, 0.5 or 1");
                }
                for v in params.depth.values() {
                    c.unit(*v, format!("{e}/params/depth"));
                }
            }
            Effect::Strobe { params, .. } => {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
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
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
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
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
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
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
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
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
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
                // custom: its groups (and optional offsets) are required; other patterns have none.
                let custom = params.pattern.values().any(|p| *p == crate::model::RegionPattern::Custom);
                match (&params.groups, custom) {
                    (None, true) => c.push(format!("{e}/params/groups"), "required when pattern is custom"),
                    (Some(_), false) => c.push(format!("{e}/params/groups"), "only for pattern custom"),
                    (Some(g), true) => {
                        if !(1..=32).contains(&g.len()) {
                            c.push(format!("{e}/params/groups"), "groups needs 1..=32 groups");
                        }
                        for (gi, cells) in g.iter().enumerate() {
                            if !(1..=64).contains(&cells.len()) || cells.iter().any(|&i| i > 63) {
                                c.push(format!("{e}/params/groups/{gi}"), "a group has 1..=64 cell indexes within 0..=63");
                            }
                        }
                        if let Some(o) = &params.offset {
                            if o.len() != g.len() {
                                c.push(format!("{e}/params/offset"), format!("offset needs one value per group ({})", g.len()));
                            }
                            for v in o {
                                c.unit(*v, format!("{e}/params/offset"));
                            }
                        }
                    }
                    (None, false) => {
                        if params.offset.is_some() {
                            c.push(format!("{e}/params/offset"), "only for pattern custom");
                        }
                    }
                }
            }
            Effect::Path { params, .. } => {
                use crate::model::PathKind;
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
                let kinds: Vec<PathKind> = params.path.values().copied().collect();
                if kinds.contains(&PathKind::Points) {
                    match &params.points {
                        Some(pts) if (2..=32).contains(&pts.len()) => {
                            for p in pts.iter().flatten() {
                                c.range(*p, 0.0, 7.0, format!("{e}/params/points"));
                            }
                        }
                        Some(_) => c.push(format!("{e}/params/points"), "points needs 2..=32 points"),
                        None => c.push(format!("{e}/params/points"), "required when path is points"),
                    }
                }
                let line = kinds.contains(&PathKind::Line);
                for (name, v) in [("from_x", &params.from_x), ("from_y", &params.from_y), ("to_x", &params.to_x), ("to_y", &params.to_y), ("center_x", &params.center_x), ("center_y", &params.center_y)] {
                    match v {
                        Some(v) => v.values().for_each(|x| c.range(*x, 0.0, 7.0, format!("{e}/params/{name}"))),
                        None if line && (name.starts_with("from") || name.starts_with("to")) => c.push(format!("{e}/params/{name}"), "required when path is line"),
                        None => {}
                    }
                }
                if let Some(r) = &params.orbit_radius {
                    r.values().for_each(|x| c.range(*x, 0.0, 4.0, format!("{e}/params/orbit_radius")));
                }
                // Speed: cells/s for line / points, laps/s for the closed paths.
                let (lo, hi) = kinds.iter().fold((f64::MAX, 0.0f64), |(lo, hi), k| match k {
                    PathKind::Line | PathKind::Points => (lo.min(0.5), hi.max(16.0)),
                    _ => (lo.min(0.05), hi.max(4.0)),
                });
                for v in params.speed.values() {
                    c.range(*v, lo, hi, format!("{e}/params/speed"));
                }
                for v in params.size.values() {
                    c.range(*v, 0.5, 3.0, format!("{e}/params/size"));
                }
                for v in params.tail.values() {
                    c.range(*v, 0.0, 8.0, format!("{e}/params/tail"));
                }
                for v in params.phase.values() {
                    c.unit(*v, format!("{e}/params/phase"));
                }
            }
            Effect::Shape { params, .. } => {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
                for (name, p, lo, hi) in [("stroke", &params.stroke, 0.5, 3.0), ("center_x", &params.center_x, CENTER_MIN, CENTER_MAX), ("center_y", &params.center_y, CENTER_MIN, CENTER_MAX), ("size", &params.size, 0.0, 6.0), ("angle", &params.angle, 0.0, 360.0), ("edge", &params.edge, 0.0, 2.0)] {
                    for v in p.values() {
                        c.range(*v, lo, hi, format!("{e}/params/{name}"));
                    }
                }
                if let Some(p) = &params.size_y {
                    p.values().for_each(|v| c.range(*v, 0.0, 6.0, format!("{e}/params/size_y")));
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
            Effect::Pads { params, .. } => {
                for (name, v) in [("color_low", &params.color_low), ("color_high", &params.color_high), ("color", &params.color)] {
                    if let Some(v) = v {
                        c.color(v, format!("{e}/params/{name}"));
                    }
                }
                if let Some(p) = &params.palette {
                    if !(2..=8).contains(&p.len()) {
                        c.push(format!("{e}/params/palette"), "2..=8 colors");
                    }
                    p.iter().for_each(|v| c.color(v, format!("{e}/params/palette")));
                }
                if let Some(g) = params.gain {
                    c.range(g, 0.05, 1.0, format!("{e}/params/gain"));
                }
                if let Some(d) = params.decay_ms {
                    // paint may stay (0); heat needs a decay.
                    let ok = (200..=60_000).contains(&d) || (d == 0 && params.mode == crate::model::PadsMode::Paint);
                    if !ok {
                        c.push(format!("{e}/params/decay_ms"), "decay_ms must be within 200..=60000 (paint: 0 = stays)");
                    }
                }
                for (name, v) in [("attack_ms", params.attack_ms), ("sustain_ms", params.sustain_ms), ("release_ms", params.release_ms)] {
                    if v.is_some_and(|v| v > 60_000) {
                        c.push(format!("{e}/params/{name}"), "must be within 0..=60000");
                    }
                }
                if let Some(s) = params.size {
                    c.range(s, 0.5, 3.0, format!("{e}/params/size"));
                }
            }
            Effect::Cycle { params, .. } => {
                if !(2..=8).contains(&params.palette.len()) {
                    c.push(format!("{e}/params/palette"), "2..=8 colors");
                }
                params.palette.iter().for_each(|v| c.color(v, format!("{e}/params/palette")));
                if params.lifetime_ms > 10_000 {
                    c.push(format!("{e}/params/lifetime_ms"), "lifetime_ms must be within 0..=10000");
                }
                c.range(params.radius, 0.0, 3.0, format!("{e}/params/radius"));
            }
            Effect::Constellation { params, .. } => {
                c.color(&params.color, format!("{e}/params/color"));
                c.range(params.stroke, 0.5, 2.0, format!("{e}/params/stroke"));
                if !(200..=10_000).contains(&params.lifetime_ms) {
                    c.push(format!("{e}/params/lifetime_ms"), "lifetime_ms must be within 200..=10000");
                }
                if !(1..=16).contains(&params.max_segments) {
                    c.push(format!("{e}/params/max_segments"), "max_segments must be within 1..=16");
                }
            }
            Effect::Frames { params, .. } => {
                if !(1..=MAX_FRAMES).contains(&params.frames.len()) {
                    c.push(format!("{e}/params/frames"), format!("1..={MAX_FRAMES} frames (got {})", params.frames.len()));
                }
                for (i, f) in params.frames.iter().enumerate() {
                    if f.len() != 384 || !f.bytes().all(|b| b.is_ascii_hexdigit()) {
                        c.push(format!("{e}/params/frames/{i}"), "a frame is 64 colors as 384 hex digits (RRGGBB, top row first)");
                    }
                }
                c.range(params.fps, 1.0, 30.0, format!("{e}/params/fps"));
                if params.hold_ms.is_some_and(|h| h > 60_000) {
                    c.push(format!("{e}/params/hold_ms"), "hold_ms must be within 0..=60000");
                }
                if let Some(col) = &params.color {
                    c.color(col, format!("{e}/params/color"));
                }
            }
            Effect::Text { params, .. } => {
                let n = params.text.chars().count();
                if !(1..=64).contains(&n) || !params.text.chars().all(|ch| (' '..='~').contains(&ch)) {
                    c.push(format!("{e}/params/text"), "1..=64 printable ASCII characters");
                }
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
                let (lo, hi) = if params.mode == crate::model::TextMode::Sequential { (0.2, 8.0) } else { (0.5, 16.0) };
                c.range(params.speed, lo, hi, format!("{e}/params/speed"));
                if params.gap > 2 {
                    c.push(format!("{e}/params/gap"), "gap must be within 0..=2");
                }
            }
            Effect::Particles { params, .. } => {
                for v in params.color.values().chain(params.color_end.iter().flat_map(|c| c.values())) {
                    c.color(v, format!("{e}/params/color"));
                }
                let num = |c: &mut Ctx, name: &str, p: Option<&crate::model::Param<f64>>, lo: f64, hi: f64| {
                    for v in p.into_iter().flat_map(|p| p.values()) {
                        c.range(*v, lo, hi, format!("{e}/params/{name}"));
                    }
                };
                num(&mut c, "rate", params.rate.as_ref(), 0.5, 64.0);
                num(&mut c, "speed_min", Some(&params.speed_min), 0.0, 16.0);
                num(&mut c, "speed_max", Some(&params.speed_max), 0.0, 16.0);
                num(&mut c, "angle", Some(&params.angle), 0.0, 360.0);
                num(&mut c, "spread", Some(&params.spread), 0.0, 360.0);
                num(&mut c, "gravity", Some(&params.gravity), -32.0, 32.0);
                num(&mut c, "drag", Some(&params.drag), 0.0, 8.0);
                num(&mut c, "size", Some(&params.size), 0.5, 2.0);
                num(&mut c, "tail", Some(&params.tail), 0.0, 4.0);
                num(&mut c, "origin_x", params.origin_x.as_ref(), 0.0, 7.0);
                num(&mut c, "origin_y", params.origin_y.as_ref(), 0.0, 7.0);
                num(&mut c, "rise_speed", params.rise_speed.as_ref(), 1.0, 32.0);
                num(&mut c, "burst_height", params.burst_height.as_ref(), 0.0, 7.0);
                num(&mut c, "implode_radius", params.implode_radius.as_ref(), 1.0, 6.0);
                // speed_min <= speed_max at every keyframe value pair is not
                // checked: a larger min only swaps the range.
                if let Some(n) = &params.count {
                    if !n.values().all(|v| (1..=64).contains(v)) {
                        c.push(format!("{e}/params/count"), "count must be within 1..=64");
                    }
                    c.step_only(n, format!("{e}/params/count"));
                }
                if !params.lifetime_ms.values().all(|v| (100..=6000).contains(v)) {
                    c.push(format!("{e}/params/lifetime_ms"), "lifetime_ms must be within 100..=6000");
                }
                if let Some(p) = &params.pause_ms {
                    if !p.values().all(|v| *v <= 1000) {
                        c.push(format!("{e}/params/pause_ms"), "pause_ms must be within 0..=1000");
                    }
                }
                if params.seed.values().any(|v| *v > i32::MAX as u32) {
                    c.push(format!("{e}/params/seed"), "seed must be within 0..=2147483647");
                }
                c.step_only(&params.seed, format!("{e}/params/seed"));
            }
            Effect::Cross { params, .. } => {
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
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
                for v in params.color.values() {
                    c.color(v, format!("{e}/params/color"));
                }
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
                for v in params.base_color.values() {
                    c.color(v, format!("{e}/params/base_color"));
                }
                for v in params.highlight_color.values() {
                    c.color(v, format!("{e}/params/highlight_color"));
                }
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

    // 0.1.3 stage 6. Compounds: 1..=4 inner layers, a positive length.
    if show.compounds.len() > MAX_COMPOUNDS {
        c.push("/compounds", format!("at most {MAX_COMPOUNDS} compounds (got {})", show.compounds.len()));
    }
    let mut compound_ids: HashMap<&str, usize> = HashMap::new();
    for (i, cp) in show.compounds.iter().enumerate() {
        let p = format!("/compounds/{i}");
        c.id(&cp.id, format!("{p}/id"));
        if compound_ids.insert(&cp.id, i).is_some() {
            c.push(format!("{p}/id"), format!("duplicate compound id {:?}", cp.id));
        }
        c.name(&cp.name, format!("{p}/name"));
        if !(1..=MAX_COMPOUND_LAYERS).contains(&cp.layers.len()) {
            c.push(format!("{p}/layers"), format!("1..={MAX_COMPOUND_LAYERS} inner layers (got {})", cp.layers.len()));
        }
        match cp.duration_us {
            Some(d) if d == 0 || d > MAX_SAFE_INTEGER => c.push(format!("{p}/duration_us"), "duration_us must be positive and within the safe integer range"),
            None if cp.length_us() == 0 => c.push(format!("{p}/duration_us"), "required while the compound has no clips (its length would be 0)"),
            _ => {}
        }
    }

    // 1-3, 6. Layers and clips: the show's layers, then the compounds' inner
    // layers. Layer and clip IDs are unique across both (edits address them by ID).
    let mut layer_ids = HashSet::new();
    let mut clip_ids = HashSet::new();
    let mut max_end: u64 = 0;
    let all_layers = show.layers.iter().enumerate().map(|(li, l)| (l, format!("/layers/{li}"), false)).chain(
        show.compounds.iter().enumerate().flat_map(|(ci, cp)| cp.layers.iter().enumerate().map(move |(li, l)| (l, format!("/compounds/{ci}/layers/{li}"), true))),
    );
    for (l, p, inner) in all_layers {
        c.id(&l.id, format!("{p}/id"));
        c.unique(&mut layer_ids, &l.id, format!("{p}/id"), "layer");
        c.name(&l.name, format!("{p}/name"));
        if inner {
            if !l.target_id.is_empty() {
                c.push(format!("{p}/target_id"), "an inner layer uses the target of the layer placing the compound");
            }
            if matches!(l.content, Content::Triggered { .. }) {
                c.push(format!("{p}/content/kind"), "an inner layer is a timeline or continuous");
            }
        } else if !target_ids.contains(l.target_id.as_str()) {
            c.push(format!("{p}/target_id"), format!("unknown target {:?}", l.target_id));
        }
        c.unit(l.opacity, format!("{p}/opacity"));
        if let Some(m) = &l.mask {
            let mp = format!("{p}/mask");
            if m.shape.values().any(|k| *k == crate::model::ShapeKind::Line) {
                c.push(format!("{mp}/shape"), "a mask is circle, rect, diamond or wipe");
            }
            for (name, v, lo, hi) in [("center_x", Some(&m.center_x), CENTER_MIN, CENTER_MAX), ("center_y", Some(&m.center_y), CENTER_MIN, CENTER_MAX), ("size", Some(&m.size), 0.0, 8.0), ("size_y", m.size_y.as_ref(), 0.0, 8.0), ("angle", m.angle.as_ref(), 0.0, 360.0), ("edge", Some(&m.edge), 0.0, 2.0)] {
                for x in v.into_iter().flat_map(|v| v.values()) {
                    c.range(*x, lo, hi, format!("{mp}/{name}"));
                }
            }
        }
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
                    match &clip.compound_id {
                        None => {
                            if !preset_ids.contains_key(clip.preset_id.as_str()) {
                                c.push(format!("{cp}/preset_id"), format!("unknown preset {:?}", clip.preset_id));
                            }
                            if clip.tint.is_some() || clip.time_scale.is_some() {
                                c.push(cp.clone(), "tint and time_scale are for compound clips");
                            }
                            if clip.looping.is_some() && clip.recording.is_none() {
                                c.push(format!("{cp}/loop"), "loop is for compound clips and recordings");
                            }
                        }
                        Some(id) => {
                            if !clip.preset_id.is_empty() {
                                c.push(format!("{cp}/preset_id"), "a clip has a preset_id or a compound_id, not both");
                            }
                            if inner {
                                c.push(format!("{cp}/compound_id"), "compounds do not nest");
                            } else if !matches!(l.content, Content::Timeline { .. }) {
                                c.push(format!("{cp}/compound_id"), "compounds are placed on timeline layers");
                            }
                            if !compound_ids.contains_key(id.as_str()) {
                                c.push(format!("{cp}/compound_id"), format!("unknown compound {id:?}"));
                            }
                            if let Some(tint) = &clip.tint {
                                c.color(tint, format!("{cp}/tint"));
                            }
                            if let Some(s) = clip.time_scale {
                                c.range(s, 0.25, 4.0, format!("{cp}/time_scale"));
                            }
                        }
                    }
                    if clip.duration_us == 0 {
                        c.push(format!("{cp}/duration_us"), "duration_us must be positive");
                    }
                    // 0.1.3 stage 8: recordings on triggered clips only.
                    if let Some(rec) = &clip.recording {
                        let rp = format!("{cp}/recording");
                        if !matches!(l.content, Content::Triggered { .. }) {
                            c.push(rp.clone(), "recordings are on triggered layers");
                        }
                        if rec.events.len() > MAX_RECORDED {
                            c.push(format!("{rp}/events"), format!("at most {MAX_RECORDED} events (got {})", rec.events.len()));
                        }
                        for (ei, ev) in rec.events.iter().enumerate() {
                            if ev.x > 7 || ev.y > 7 || !(1..=127).contains(&ev.velocity) || ev.t_us > MAX_SAFE_INTEGER || ev.duration_us.is_some_and(|d| d > MAX_SAFE_INTEGER) {
                                c.push(format!("{rp}/events/{ei}"), "x and y within 0..=7, velocity 1..=127, times within the safe integer range");
                            }
                        }
                        if rec.events.windows(2).any(|w| w[1].t_us < w[0].t_us) {
                            c.push(format!("{rp}/events"), "events in time order");
                        }
                        if rec.length_us.is_some_and(|l| l == 0 || l > MAX_SAFE_INTEGER) {
                            c.push(format!("{rp}/length_us"), "length_us must be positive");
                        }
                    }
                    match clip.start_us.checked_add(clip.duration_us) {
                        Some(end) if end <= MAX_SAFE_INTEGER => {
                            if !inner {
                                max_end = max_end.max(end);
                            }
                        }
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
        let clip = |p: &str| Clip { id: format!("c-{p}"), preset_id: p.into(), start_us: 0, duration_us: 1_000_000, ..Default::default() };
        let layer = |p: &str, on| Layer {
            id: "trig".into(),
            name: "trig".into(),
            target_id: "main-pad".into(),
            enabled: true,
            opacity: 1.0,
            blend: crate::model::Blend::Over,
            content: Content::Triggered { clips: vec![clip(p)], trigger: TriggerSpec { on, interval_ms: None, origin_x: None, origin_y: None } },
            mask: None,
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
    fn path_shape_and_custom_regions_rules() {
        use crate::model::{PathKind, Preset, RegionPattern};
        let mut s = demo();
        let n = s.presets.len();
        let mut add = |t: &str, f: &dyn Fn(&mut Effect)| {
            let mut e = crate::edit::default_effect(t).unwrap();
            f(&mut e);
            s.presets.push(Preset { id: format!("p{}", s.presets.len()), name: "x".into(), effect: e });
        };
        add("builtin.path", &|e| {
            let Effect::Path { params, .. } = e else { unreachable!() };
            params.path = PathKind::Points.into(); // no points; speed 0.25 is below the cells/s range
        });
        add("builtin.path", &|e| {
            let Effect::Path { params, .. } = e else { unreachable!() };
            params.path = PathKind::Line.into(); // no from / to
            params.speed = 4.0.into();
        });
        add("builtin.shape", &|e| {
            let Effect::Shape { params, .. } = e else { unreachable!() };
            params.angle = 400.0.into();
        });
        add("builtin.regions", &|e| {
            let Effect::Regions { params, .. } = e else { unreachable!() };
            params.pattern = RegionPattern::Custom.into(); // no groups
        });
        add("builtin.regions", &|e| {
            let Effect::Regions { params, .. } = e else { unreachable!() };
            params.pattern = RegionPattern::Custom.into();
            params.groups = Some(vec![vec![0, 64], vec![]]);
            params.offset = Some(vec![0.5]);
        });
        add("builtin.regions", &|e| {
            let Effect::Regions { params, .. } = e else { unreachable!() };
            params.groups = Some(vec![vec![0]]); // not custom
        });
        let p = pointers(&s);
        let at = |i: usize, rest: &str| format!("/presets/{}/effect/params/{rest}", n + i);
        for want in [at(0, "points"), at(0, "speed"), at(1, "from_x"), at(1, "to_y"), at(2, "angle"), at(3, "groups"), at(4, "groups/0"), at(4, "groups/1"), at(4, "offset"), at(5, "groups")] {
            assert!(p.contains(&want), "{want} in {p:?}");
        }
        // The defaults are valid.
        let mut ok = demo();
        for t in ["builtin.path", "builtin.shape"] {
            ok.presets.push(Preset { id: t.replace("builtin.", ""), name: t.into(), effect: crate::edit::default_effect(t).unwrap() });
        }
        assert!(pointers(&ok).is_empty(), "{:?}", pointers(&ok));
    }

    #[test]
    fn particle_ranges() {
        use crate::model::{Effect, Preset};
        let mut e = crate::edit::default_effect("builtin.particles").unwrap();
        let mut s = demo();
        s.presets.push(Preset { id: "pt".into(), name: "x".into(), effect: e.clone() });
        assert!(pointers(&s).is_empty(), "defaults are valid: {:?}", pointers(&s));
        let Effect::Particles { params, .. } = &mut e else { unreachable!() };
        params.count = Some(65.into());
        params.lifetime_ms = 50.into();
        params.gravity = (-40.0).into();
        params.size = 3.0.into();
        params.pause_ms = Some(1500.into());
        params.implode_radius = Some(0.5.into());
        params.color_end = Some("red".to_string().into());
        s.presets.last_mut().unwrap().effect = e;
        let p = pointers(&s);
        let n = s.presets.len() - 1;
        for k in ["count", "lifetime_ms", "gravity", "size", "pause_ms", "implode_radius", "color"] {
            assert!(p.contains(&format!("/presets/{n}/effect/params/{k}")), "{k} in {p:?}");
        }
    }

    #[test]
    fn compound_rules() {
        use crate::model::{Clip, Compound, Layer, TriggerSpec};
        let inner = |id: &str, content| Layer { id: id.into(), name: id.into(), target_id: String::new(), enabled: true, opacity: 1.0, blend: crate::model::Blend::Over, content, mask: None };
        let mut s = demo();
        s.compounds.push(Compound {
            id: "cp".into(),
            name: "cp".into(),
            duration_us: None,
            layers: vec![
                Layer { target_id: "main-pad".into(), ..inner("a", Content::Continuous { preset_id: "base-blue".into() }) },
                inner("b", Content::Triggered { clips: vec![], trigger: TriggerSpec { on: TriggerOn::Press, interval_ms: None, origin_x: None, origin_y: None } }),
                inner("c", Content::Timeline { clips: vec![Clip { id: "nest".into(), compound_id: Some("cp".into()), duration_us: 1, ..Default::default() }] }),
                inner("d", Content::Continuous { preset_id: "base-blue".into() }),
                inner("base", Content::Continuous { preset_id: "base-blue".into() }),
            ],
        });
        if let Content::Timeline { clips } = &mut s.layers[0].content {
            clips[0].compound_id = Some("cp".into()); // and its preset_id is still set
            clips.push(Clip { id: "t".into(), preset_id: "base-blue".into(), duration_us: 1, tint: Some("#FFFFFF".into()), ..Default::default() });
            clips.push(Clip { id: "x".into(), compound_id: Some("nope".into()), duration_us: 1, time_scale: Some(8.0), ..Default::default() });
        }
        let p = pointers(&s);
        for want in [
            "/compounds/0/layers",
            "/compounds/0/layers/0/target_id",
            "/compounds/0/layers/1/content/kind",
            "/compounds/0/layers/2/content/clips/0/compound_id",
            "/compounds/0/layers/4/id",
            "/layers/0/content/clips/0/preset_id",
            "/layers/0/content/clips/1",
            "/layers/0/content/clips/2/compound_id",
            "/layers/0/content/clips/2/time_scale",
        ] {
            assert!(p.contains(&want.to_string()), "{want} in {p:?}");
        }
        // Without clips a compound needs its own length.
        let mut s = demo();
        s.compounds.push(Compound { id: "e".into(), name: "e".into(), duration_us: None, layers: vec![inner("in", Content::Timeline { clips: vec![] })] });
        assert_eq!(pointers(&s), vec!["/compounds/0/duration_us".to_string()]);
    }

    #[test]
    fn frames_and_text_rules() {
        use crate::model::Preset;
        let mut s = demo();
        let mut f = crate::edit::default_effect("builtin.frames").unwrap();
        let Effect::Frames { params, .. } = &mut f else { unreachable!() };
        params.frames.push("00".into());
        params.fps = 60.0;
        params.color = Some("white".into());
        let mut t = crate::edit::default_effect("builtin.text").unwrap();
        let Effect::Text { params, .. } = &mut t else { unreachable!() };
        params.text = "おもいいろ".into();
        params.speed = 20.0;
        params.gap = 3;
        s.presets.push(Preset { id: "f".into(), name: "f".into(), effect: f });
        s.presets.push(Preset { id: "t".into(), name: "t".into(), effect: t });
        let p = pointers(&s);
        for want in ["/presets/3/effect/params/frames/1", "/presets/3/effect/params/fps", "/presets/3/effect/params/color", "/presets/4/effect/params/text", "/presets/4/effect/params/speed", "/presets/4/effect/params/gap"] {
            assert!(p.contains(&want.to_string()), "{want} in {p:?}");
        }
    }

    #[test]
    fn input_state_rules() {
        use crate::model::{CycleParams, CycleScope, ConstellationParams, PadsMode, PadsParams, Preset, RecordedPad, Recording, RippleFade};
        let mut s = demo();
        let pads = PadsParams { mode: PadsMode::Heat, color_low: None, color_high: Some("red".into()), color: None, palette: Some(vec!["#FFFFFF".into()]), gain: Some(2.0), decay_ms: Some(0), attack_ms: None, sustain_ms: None, release_ms: None, size: None };
        s.presets.push(Preset { id: "p".into(), name: "p".into(), effect: Effect::Pads { version: 1, params: pads } });
        s.presets.push(Preset { id: "c".into(), name: "c".into(), effect: Effect::Cycle { version: 1, params: CycleParams { palette: vec!["#FFFFFF".into()], scope: CycleScope::Global, lifetime_ms: 20_000, radius: 4.0 } } });
        s.presets.push(Preset { id: "k".into(), name: "k".into(), effect: Effect::Constellation { version: 1, params: ConstellationParams { color: "#FFFFFF".into(), stroke: 3.0, lifetime_ms: 100, fade: RippleFade::Linear, max_segments: 20, dots: false } } });
        if let Content::Timeline { clips } = &mut s.layers[0].content {
            clips[0].recording = Some(Recording { events: vec![RecordedPad { t_us: 5, x: 8, y: 0, velocity: 0, duration_us: None }, RecordedPad { t_us: 1, x: 0, y: 0, velocity: 1, duration_us: None }], length_us: Some(0) });
        }
        let p = pointers(&s);
        for want in [
            "/presets/3/effect/params/color_high", "/presets/3/effect/params/palette", "/presets/3/effect/params/gain", "/presets/3/effect/params/decay_ms",
            "/presets/4/effect/params/palette", "/presets/4/effect/params/lifetime_ms", "/presets/4/effect/params/radius",
            "/presets/5/effect/params/stroke", "/presets/5/effect/params/lifetime_ms", "/presets/5/effect/params/max_segments",
            "/layers/0/content/clips/0/recording", "/layers/0/content/clips/0/recording/events/0", "/layers/0/content/clips/0/recording/events", "/layers/0/content/clips/0/recording/length_us",
        ] {
            assert!(p.contains(&want.to_string()), "{want} in {p:?}");
        }
    }

    #[test]
    fn layer_mask_rules() {
        use crate::model::{LayerMask, ShapeKind};
        let m = |shape: ShapeKind, size: f64, cx: f64| LayerMask { shape: shape.into(), center_x: cx.into(), center_y: 3.5.into(), size: size.into(), size_y: None, angle: Some(400.0.into()), edge: 0.5.into(), invert: false.into() };
        let mut s = demo();
        s.layers[0].mask = Some(m(ShapeKind::Line, 9.0, 12.0));
        let p = pointers(&s);
        for want in ["shape", "size", "center_x", "angle"] {
            assert!(p.contains(&format!("/layers/0/mask/{want}")), "{want} in {p:?}");
        }
        // A wipe from off the grid is fine (size is not used).
        s.layers[0].mask = Some(LayerMask { angle: None, ..m(ShapeKind::Wipe, 0.0, -2.0) });
        assert!(pointers(&s).is_empty(), "{:?}", pointers(&s));
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
