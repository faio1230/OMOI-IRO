//! Effects, compositing and the headless renderer (tasks C06, C07, C10).
//!
//! `Renderer::new` validates and compiles a show once (colors parsed,
//! presets resolved, clip indexes built, buffers allocated). `render` then
//! evaluates any show time without JSON parsing or allocation, and is
//! deterministic: the same show, time and beat give the same frame (NFR08).
//!
//! Effects (docs/04-show-data.md):
//! * `builtin.solid`: the whole surface in one color.
//! * `builtin.chase`: a band `width` cells wide moves along `axis`, one lap
//!   of the 8 cells per `period_us` of local time, wrapping around; cells
//!   outside the band are transparent. `reverse` (0.1.2) runs it the other way.
//! * `builtin.pulse`: fixed color, alpha follows the quarter-note phase,
//!   maximum at phase 0. `depth` 0 keeps it at maximum, 1 dips to zero.
//!   Transparent while the beat phase is unknown.
//! * `builtin.strobe` (0.1.2): square wave, alpha 1 for `duty` of each
//!   cycle and 0 otherwise. Mode `hz`: `rate_hz` cycles per second of local
//!   time; mode `beat`: one cycle per `beats_per_cycle` of the beat phase
//!   (transparent while it is unknown). `phase_offset` shifts the cycle.
//! * `builtin.noise` (0.1.2): pattern `step = floor(local_s × rate_hz)`; each
//!   cell hashes (seed, cell, step) to 0..1 and is lit below `density`, at the
//!   highlight color dimmed by up to `depth` (a second hash); other cells
//!   show the base color. `smooth` cross-fades to the next step. Pure
//!   function of the time, so preview, device and seek agree.
//! * `builtin.ripple` (0.1.2): input driven, so outside `render(show, t)`'s
//!   purity: `render_input` takes the pad hits (`PadInput`) as an extra
//!   argument and only the input-driven effects (Ripple, TapGlow, Cross) read them. Each hit spreads a ring at
//!   `speed` cells/s from the pad: alpha 1 where `|dist - speed × age| <=
//!   ring_width / 2`, falling linearly to 0 over another half width, times
//!   the fade over `lifetime_ms`. Overlapping rings take the maximum. The
//!   newest `max_concurrent` live hits are drawn. `render` (no input) draws
//!   nothing for Ripple; the other effects are unchanged.
//! * `builtin.rain` (0.1.2): every lane (row or column across `direction`)
//!   has a slot each 0.5 s; a slot holds a drop with chance `density`, at a
//!   hashed offset and brightness. The head is `(local_s - spawn) × speed`
//!   cells in, a `tail` fades linearly behind it (`head_color` on the head
//!   cell), `flicker` dims tail cells by a hash per 8 Hz step. Pure function
//!   of the time.
//! * `builtin.wave` (0.1.2): curve `center + amplitude × sin(2π(pos / λ -
//!   speed × t))`; alpha 1 within `thickness / 2` of it, 0 at `thickness`.
//!   `beat_amplitude` scales the amplitude 0.3..1 with the beat phase.
//! * `builtin.orb` (0.1.2): disc of `radius` with a soft `edge`, brightness
//!   1 on the `light_angle` side down to `1 - shading`, `dither` grain per
//!   cell and step. `orbit` circles the centre; `bounce` reflects at the
//!   board edges (a triangle wave of the time, no state).
//! * `builtin.tap_glow` / `builtin.cross` (0.1.2): input driven like Ripple.
//!   TapGlow glows at the pad (start → mid → end color, `repeats` times);
//!   Cross runs four heads along the pad's row and column with tails.
//! * `builtin.regions` (0.1.2): the cells of `pattern` form groups lit in
//!   turn. The step position is `beat / step_beats` (transparent without a
//!   beat phase) or `local_s / step_s`; `order` walks the groups forward,
//!   reverse or back and forth. Within a step the group is lit for `duty`;
//!   `soft` ramps in and out over that many steps (1 = a smooth travelling
//!   peak). `color_b` fills the unlit groups (alternating look).
//!
//! Compositing: clips inside a layer are composited `over` in array order
//! into a premultiplied layer buffer (clip alpha = fade x effect alpha).
//! Layers are applied bottom to top onto black with their opacity:
//! `over` = `F (1 - a o) + L o`, `add` = `min(1, F + L o)`,
//! `max` = `max(F, L o)` per channel (L premultiplied). The master
//! brightness scales the result.

use crate::model::{
    Axis, Blend, Content, Effect, LayerMask, Mirror, OrbMotion, PathKind, ShapeKind, TriggerOn, RainDirection, RegionPattern, RippleFade, RippleShape, RippleTrigger, Show, StepMode, StepOrder, StrobeMode, Waveform,
};
use crate::particles::{paint_event, Event, ParticleLook, MAX_EVENTS, MAX_PARTICLES, MAX_SHOW_PARTICLES};
use crate::surface::{Frame, LayerFrame, Rgb, Rgba, GRID_H, GRID_LEDS, GRID_W};
use crate::time::{BeatState, ShowTimeUs};
use crate::timeline::{ClipIndex, ClipSpan};
use crate::validation::{validate, Issue};
use std::collections::HashMap;
use std::f64::consts::TAU;

/// An effect with its parameters parsed for rendering (cheap to clone:
/// frame data is shared).
#[derive(Clone, Debug, PartialEq)]
pub enum CompiledEffect {
    Solid { color: Rgb },
    Chase { color: Rgb, period_us: u64, width: u32, axis: Axis, reverse: bool },
    Pulse { color: Rgb, beats_per_cycle: f64, depth: f64, waveform: Waveform },
    Strobe { color: Rgb, mode: StrobeMode, rate_hz: f64, beats_per_cycle: f64, duty: f64, phase_offset: f64 },
    Noise { base: Rgb, highlight: Rgb, density: f64, rate_hz: f64, seed: u32, smooth: bool, depth: f64 },
    /// `interval`: rings at `(x, y)` every `every_us` of local time; None: pad input.
    Ripple { look: RippleLook, interval: Option<(f64, f64, i64)> },
    Rain { color: Rgb, head: Rgb, direction: RainDirection, speed: f64, density: f64, tail: f64, seed: u32, flicker: f64 },
    Wave { color: Rgb, axis: Axis, wavelength: f64, amplitude: f64, center: f64, speed: f64, thickness: f64, beat_amplitude: bool, phase: f64 },
    Orb {
        color: Rgb,
        center: (f64, f64),
        radius: f64,
        edge: f64,
        light: (f64, f64),
        shading: f64,
        dither: f64,
        dither_rate_hz: f64,
        seed: u32,
        motion: OrbMotion,
        speed: f64,
        orbit_radius: f64,
        bounce_dir: (f64, f64),
        orbit_phase: f64,
    },
    TapGlow { start: Rgb, mid: Rgb, end: Rgb, lifetime_us: i64, radius: f64, repeats: u32, repeat_us: i64, decay: f64 },
    Cross { color: Rgb, speed: f64, length: f64, lifetime_us: i64 },
    /// 0.1.3: particles; layers turn time or pads into events (`particles`).
    Particles(ParticleLook),
    /// 0.1.3: frames played by time (instances on a triggered layer).
    Frames(FramesLook),
    /// 0.1.3: text in a built-in font.
    Text(TextLook),
    Regions { pattern: RegionPattern, color: Rgb, color_b: Option<Rgb>, mode: StepMode, step_beats: f64, step_s: f64, order: StepOrder, soft: f64, duty: f64, custom: Option<CustomGroups> },
    Path(PathLook),
    Shape { kind: ShapeKind, color: Rgb, fill: bool, stroke: f64, center: (f64, f64), size: (f64, f64), angle: f64, edge: f64, invert: bool },
}

/// The groups of a `custom` regions pattern as cell bit masks, with each group's phase delay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CustomGroups {
    pub n: usize,
    pub masks: [u64; 32],
    pub offset: [f64; 32],
}

/// A compiled `builtin.path`: the path as at most 32 points plus how the spot runs on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathLook {
    pub color: Rgb,
    pub kind: PathKind,
    /// `line` / `points`: the polyline; `orbit` / `figure8`: centre in [0] and (radius, 0) in [1].
    pub pts: [(f64, f64); 32],
    pub n: usize,
    /// Length of the path in cells.
    pub length: f64,
    pub speed: f64,
    pub size: f64,
    pub tail: f64,
    pub looping: bool,
    pub phase: f64,
    pub mirror: Mirror,
}

/// One pad press for input-driven effects: grid cell (bottom-left origin),
/// the monotonic time it arrived, its release (if any) and the show time
/// when it arrived (triggered layers accept it only inside a clip).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadHit {
    pub x: u8,
    pub y: u8,
    pub velocity: u8,
    pub t_us: i64,
    pub release_us: Option<i64>,
    pub show_us: Option<i64>,
}

impl PadHit {
    pub fn press(x: u8, y: u8, velocity: u8, t_us: i64) -> PadHit {
        PadHit { x, y, velocity, t_us, release_us: None, show_us: None }
    }

    /// Release time, with a hold longer than `PadInput::MAX_HOLD_US` cut there.
    pub fn effective_release(&self, now_us: i64) -> Option<i64> {
        self.release_us.or((now_us - self.t_us > PadInput::MAX_HOLD_US).then_some(self.t_us + PadInput::MAX_HOLD_US))
    }
}

/// Operation input for the renderer, in arrival order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PadEvent {
    Press { x: u8, y: u8, velocity: u8, t_us: i64 },
    Release { x: u8, y: u8, t_us: i64 },
    /// The input went away (unplugged, reconnect): release every held pad.
    ReleaseAll { t_us: i64 },
}

/// Pad hits seen by the renderer at monotonic time `now_us` (oldest first).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadInput {
    pub now_us: i64,
    pub hits: Vec<PadHit>,
}

impl PadInput {
    /// Hits kept at most (the largest `max_concurrent`).
    pub const CAPACITY: usize = 16;
    /// Hits older than the longest effect window are dropped (tap_glow:
    /// 5 s lifetime + 7 repeats × 2 s).
    pub const MAX_AGE_US: i64 = 20_000_000;

    /// Add a hit; drop the oldest beyond `CAPACITY`. No allocation once full.
    pub fn push(&mut self, hit: PadHit) {
        if self.hits.len() == Self::CAPACITY {
            self.hits.remove(0);
        }
        self.hits.push(hit);
    }

    /// A hold is cut (released) after 10 s.
    pub const MAX_HOLD_US: i64 = 10_000_000;

    /// Take one event; `show_us` is the show time when it arrived.
    pub fn apply(&mut self, ev: PadEvent, show_us: Option<i64>) {
        match ev {
            PadEvent::Press { x, y, velocity, t_us } => self.push(PadHit { show_us, ..PadHit::press(x, y, velocity, t_us) }),
            // The newest held press of that pad.
            PadEvent::Release { x, y, t_us } => {
                if let Some(h) = self.hits.iter_mut().rev().find(|h| h.x == x && h.y == y && h.release_us.is_none()) {
                    h.release_us = Some(t_us);
                }
            }
            PadEvent::ReleaseAll { t_us } => self.hits.iter_mut().filter(|h| h.release_us.is_none()).for_each(|h| h.release_us = Some(t_us)),
        }
    }

    /// Set the time and drop expired hits.
    pub fn advance(&mut self, now_us: i64) {
        self.now_us = now_us;
        self.hits.retain(|h| now_us - h.t_us <= Self::MAX_AGE_US);
    }
}

/// A fired instance for instance-taking effects: position and age.
pub type Instance = (f64, f64, i64);

/// Instances a triggered layer keeps per clip and frame at most.
pub const MAX_INSTANCES: usize = 64;

impl CompiledEffect {
    pub fn compile(e: &Effect) -> Option<CompiledEffect> {
        CompiledEffect::compile_at(e, 0)
    }

    /// The effect with its parameters sampled at `t` on their time axis
    /// (keyframes; constant parameters are the same at any time).
    pub fn compile_at(e: &Effect, t: u64) -> Option<CompiledEffect> {
        Some(match e {
            // Parameters are constant in Schema 0.1.x (`Param::Value`).
            Effect::Solid { params, .. } => CompiledEffect::Solid { color: col(&params.color, t)? },
            Effect::Chase { params, .. } => CompiledEffect::Chase {
                color: col(&params.color, t)?,
                period_us: (params.period_us.at(t)).max(1),
                width: (params.width.at(t)).clamp(1, 8),
                axis: params.axis.at(t),
                reverse: params.reverse.as_ref().is_some_and(|r| r.at(t)),
            },
            Effect::Pulse { params, .. } => CompiledEffect::Pulse {
                color: col(&params.color, t)?,
                beats_per_cycle: params.beats_per_cycle.at(t),
                depth: params.depth.at(t),
                waveform: params.waveform.at(t),
            },
            Effect::Strobe { params, .. } => CompiledEffect::Strobe {
                color: col(&params.color, t)?,
                mode: params.mode.at(t),
                rate_hz: params.rate_hz.at(t),
                beats_per_cycle: params.beats_per_cycle.at(t),
                duty: params.duty.at(t),
                phase_offset: params.phase_offset.at(t),
            },
            Effect::Ripple { params, .. } => CompiledEffect::Ripple {
                look: RippleLook {
                    color: col(&params.color, t)?,
                    speed: params.speed.at(t),
                    ring_width: params.ring_width.at(t),
                    lifetime_us: params.lifetime_ms.at(t) as i64 * 1000,
                    fade: params.fade.at(t),
                    max_concurrent: (params.max_concurrent.at(t)).clamp(1, PadInput::CAPACITY as u32) as usize,
                    shape: params.shape.as_ref().map(|s| s.at(t)).unwrap_or(RippleShape::Circle),
                },
                interval: match params.trigger.at(t) {
                    RippleTrigger::AnyPad => None,
                    RippleTrigger::Interval => Some((
                        params.origin_x.as_ref()?.at(t),
                        params.origin_y.as_ref()?.at(t),
                        (params.interval_ms.as_ref()?.at(t)).max(1) as i64 * 1000,
                    )),
                },
            },
            Effect::Rain { params, .. } => {
                let color = col(&params.color, t)?;
                CompiledEffect::Rain {
                    color,
                    head: match &params.head_color {
                        Some(h) => col(h, t)?,
                        None => color,
                    },
                    direction: params.direction.at(t),
                    speed: params.speed.at(t),
                    density: params.density.at(t),
                    tail: params.tail.at(t),
                    seed: params.seed.at(t),
                    flicker: params.flicker.at(t),
                }
            }
            Effect::Wave { params, .. } => CompiledEffect::Wave {
                color: col(&params.color, t)?,
                axis: params.axis.at(t),
                wavelength: params.wavelength.at(t),
                amplitude: params.amplitude.at(t),
                center: params.center.at(t),
                speed: params.speed.at(t),
                thickness: params.thickness.at(t),
                beat_amplitude: params.beat_amplitude.at(t),
                phase: params.phase.as_ref().map(|p| p.at(t)).unwrap_or(0.0),
            },
            Effect::Orb { params, .. } => {
                let (la, ba) = (params.light_angle.at(t).to_radians(), params.bounce_angle.at(t).to_radians());
                CompiledEffect::Orb {
                    color: col(&params.color, t)?,
                    center: (params.center_x.at(t), params.center_y.at(t)),
                    radius: params.radius.at(t),
                    edge: params.edge.at(t),
                    light: (la.cos(), la.sin()),
                    shading: params.shading.at(t),
                    dither: params.dither.at(t),
                    dither_rate_hz: params.dither_rate_hz.at(t),
                    seed: params.seed.at(t),
                    motion: params.motion.at(t),
                    speed: params.speed.at(t),
                    orbit_radius: params.orbit_radius.at(t),
                    bounce_dir: (ba.cos(), ba.sin()),
                    orbit_phase: params.orbit_phase.as_ref().map(|p| p.at(t)).unwrap_or(0.0),
                }
            }
            Effect::TapGlow { params, .. } => CompiledEffect::TapGlow {
                start: col(&params.color_start, t)?,
                mid: col(&params.color_mid, t)?,
                end: col(&params.color_end, t)?,
                lifetime_us: (params.lifetime_ms.at(t)).max(1) as i64 * 1000,
                radius: params.radius.at(t),
                repeats: (params.repeats.at(t)).clamp(1, 8),
                repeat_us: params.repeat_interval_ms.at(t) as i64 * 1000,
                decay: params.repeat_decay.at(t),
            },
            Effect::Cross { params, .. } => CompiledEffect::Cross {
                color: col(&params.color, t)?,
                speed: params.speed.at(t),
                length: params.length.at(t),
                lifetime_us: (params.lifetime_ms.at(t)).max(1) as i64 * 1000,
            },
            Effect::Regions { params, .. } => CompiledEffect::Regions {
                pattern: params.pattern.at(t),
                color: col(&params.color, t)?,
                color_b: match &params.color_b {
                    Some(c) => Some(col(c, t)?),
                    None => None,
                },
                mode: params.step_mode.at(t),
                step_beats: params.step_beats.at(t),
                step_s: (params.step_ms.at(t)).max(1) as f64 / 1000.0,
                order: params.order.at(t),
                soft: params.soft.at(t),
                duty: params.duty.at(t),
                custom: custom_groups(params),
            },
            Effect::Path { params, .. } => CompiledEffect::Path(compile_path(params, t)?),
            Effect::Particles { params, .. } => CompiledEffect::Particles(ParticleLook::compile(params, t, col)?),
            Effect::Frames { params, .. } => CompiledEffect::Frames(FramesLook::compile(params)?),
            Effect::Text { params, .. } => CompiledEffect::Text(TextLook::compile(params, t)?),
            Effect::Shape { params, .. } => {
                let size = params.size.at(t);
                CompiledEffect::Shape {
                    kind: params.shape.at(t),
                    color: col(&params.color, t)?,
                    fill: params.fill.at(t),
                    stroke: params.stroke.at(t),
                    center: (params.center_x.at(t), params.center_y.at(t)),
                    size: (size, params.size_y.as_ref().map(|p| p.at(t)).unwrap_or(size)),
                    angle: params.angle.at(t),
                    edge: params.edge.at(t),
                    invert: params.invert.as_ref().is_some_and(|p| p.at(t)),
                }
            }
            Effect::Noise { params, .. } => CompiledEffect::Noise {
                base: col(&params.base_color, t)?,
                highlight: col(&params.highlight_color, t)?,
                density: params.density.at(t),
                rate_hz: params.rate_hz.at(t),
                seed: params.seed.at(t),
                smooth: params.smooth.at(t),
                depth: params.depth.at(t),
            },
        })
    }

    /// Composite this effect `over` `layer` with an extra alpha factor.
    pub fn paint(&self, local_us: u64, beat: Option<BeatState>, alpha: f64, layer: &mut LayerFrame) {
        if alpha <= 0.0 {
            return;
        }
        match *self {
            CompiledEffect::Solid { color } => {
                let src = Rgba::from_color(color, alpha as f32);
                for px in layer.px.iter_mut() {
                    over(px, src);
                }
            }
            CompiledEffect::Chase { color, period_us, width, axis, reverse } => {
                let cells = match axis {
                    Axis::X => GRID_W,
                    Axis::Y => GRID_H,
                } as u64;
                let head = ((local_us % period_us) as u128 * cells as u128 / period_us as u128) as u64;
                let src = Rgba::from_color(color, alpha as f32);
                for i in 0..GRID_LEDS {
                    let (x, y) = ((i % GRID_W) as u64, (i / GRID_W) as u64);
                    let pos = match axis {
                        Axis::X => x,
                        Axis::Y => y,
                    };
                    // Reverse mirrors the pattern, so the band runs the other way.
                    let pos = if reverse { cells - 1 - pos } else { pos };
                    // Cells head, head-1, ..., head-width+1 (the band trails behind the head).
                    let behind = (head + cells - pos) % cells;
                    if behind < width as u64 {
                        over(&mut layer.px[i], src);
                    }
                }
            }
            CompiledEffect::Pulse { color, beats_per_cycle, depth, waveform } => {
                let Some(b) = beat.filter(|b| b.phase_valid && b.beat.is_finite()) else {
                    return;
                };
                let phase = (b.beat / beats_per_cycle).rem_euclid(1.0);
                let v = match waveform {
                    Waveform::Sine => 0.5 * (1.0 + (TAU * phase).cos()),
                    Waveform::Triangle => (1.0 - 2.0 * phase).abs(),
                };
                let a = alpha * (1.0 - depth * (1.0 - v));
                let src = Rgba::from_color(color, a as f32);
                for px in layer.px.iter_mut() {
                    over(px, src);
                }
            }
            CompiledEffect::Strobe { color, mode, rate_hz, beats_per_cycle, duty, phase_offset } => {
                let cycles = match mode {
                    StrobeMode::Hz => local_us as f64 / 1e6 * rate_hz,
                    StrobeMode::Beat => match beat.filter(|b| b.phase_valid && b.beat.is_finite()) {
                        Some(b) => b.beat / beats_per_cycle,
                        None => return,
                    },
                };
                if (cycles + phase_offset).rem_euclid(1.0) >= duty {
                    return;
                }
                let src = Rgba::from_color(color, alpha as f32);
                for px in layer.px.iter_mut() {
                    over(px, src);
                }
            }
            // Pad-driven rings need the input (`paint_input`); interval rings are a function of the time.
            CompiledEffect::Ripple { look, interval: Some((x, y, every_us)) } => {
                let local = local_us as i64;
                let first = (local - look.lifetime_us) / every_us;
                let ages = (first.max(0)..=local / every_us).rev().map(|k| (x, y, local - k * every_us));
                paint_ripple(ages, look, alpha, layer);
            }
            CompiledEffect::Ripple { interval: None, .. } => {}
            // Pad driven: drawn by `paint_input`.
            CompiledEffect::TapGlow { .. } | CompiledEffect::Cross { .. } => {}
            CompiledEffect::Frames(ref f) => {
                if let Some(frame) = f.frame_at(local_us) {
                    paint_frame(frame, (0, 0), alpha, layer);
                }
            }
            CompiledEffect::Text(ref look) => paint_text(look, local_us, beat, alpha, layer),
            // On its own (a timeline clip, a preview): one event at the emitter from local 0.
            CompiledEffect::Particles(look) => {
                let ev = Event { x: look.origin.0, y: look.origin.1, age_s: local_us as f64 / 1e6, emit_s: f64::INFINITY, id: 0, start_us: 0 };
                paint_event(&ev, &|_| Some(look), alpha, layer, &mut MAX_PARTICLES.clone());
            }
            CompiledEffect::Regions { pattern, color, color_b, mode, step_beats, step_s, order, soft, duty, custom } => {
                let pos = match mode {
                    StepMode::Seconds => local_us as f64 / 1e6 / step_s,
                    StepMode::Beat => match beat.filter(|b| b.phase_valid && b.beat.is_finite()) {
                        Some(b) => b.beat / step_beats,
                        None => return,
                    },
                };
                let n = custom.map(|c| c.n).unwrap_or_else(|| region_count(pattern));
                // Brightness of each group: the steps around its (delayed) position that light it.
                let mut level = [0.0f64; 32];
                for (g, lv) in level.iter_mut().enumerate().take(n) {
                    let pg = pos - custom.map(|c| c.offset[g]).unwrap_or(0.0);
                    let k0 = pg.floor() as i64;
                    for k in k0 - 2..=k0 + 2 {
                        if step_group(k, n, order) == g {
                            *lv = lv.max(step_level(pg - k as f64, duty, soft));
                        }
                    }
                }
                for (i, px) in layer.px.iter_mut().enumerate() {
                    let a = match custom {
                        // A cell in several groups takes the brightest.
                        Some(c) => match (0..c.n).filter(|&g| c.masks[g] >> i & 1 == 1).map(|g| level[g]).reduce(f64::max) {
                            Some(a) => a,
                            None => continue,
                        },
                        None => match region_of(pattern, i % GRID_W, i / GRID_W) {
                            Some(g) => level[g],
                            None => continue,
                        },
                    };
                    match color_b {
                        Some(b) => {
                            let c = Rgb::new(b.r + (color.r - b.r) * a as f32, b.g + (color.g - b.g) * a as f32, b.b + (color.b - b.b) * a as f32);
                            over(px, Rgba::from_color(c, alpha as f32));
                        }
                        None if a > 0.0 => over(px, Rgba::from_color(color, (a * alpha) as f32)),
                        None => {}
                    }
                }
            }
            CompiledEffect::Path(look) => paint_path(&look, local_us, alpha, layer),
            CompiledEffect::Shape { kind, color, fill, stroke, center, size, angle, edge, invert } => {
                for (i, px) in layer.px.iter_mut().enumerate() {
                    let sd = shape_sd(kind, (i % GRID_W) as f64 - center.0, (i / GRID_W) as f64 - center.1, size, angle);
                    // A line has no inside: always its stroke. A wipe has no outline: always filled.
                    let a = if kind == ShapeKind::Wipe || (fill && kind != ShapeKind::Line) { crate::shape::fill_alpha(sd, edge) } else { crate::shape::stroke_alpha(sd, stroke, edge) };
                    let a = if invert { 1.0 - a } else { a };
                    if a > 0.0 {
                        over(px, Rgba::from_color(color, (a * alpha) as f32));
                    }
                }
            }
            CompiledEffect::Rain { color, head, direction, speed, density, tail, seed, flicker } => {
                paint_rain(local_us, color, head, direction, speed, density, tail, seed, flicker, alpha, layer);
            }
            CompiledEffect::Wave { color, axis, wavelength, amplitude, center, speed, thickness, beat_amplitude, phase } => {
                let t = local_us as f64 / 1e6;
                let amp = match beat.filter(|b| beat_amplitude && b.phase_valid && b.beat.is_finite()) {
                    // 1 on the beat, 0.3 half way.
                    Some(b) => amplitude * (0.3 + 0.7 * 0.5 * (1.0 + (TAU * b.beat).cos())),
                    None => amplitude,
                };
                let half = thickness / 2.0;
                for (i, px) in layer.px.iter_mut().enumerate() {
                    let (x, y) = ((i % GRID_W) as f64, (i / GRID_W) as f64);
                    let (along, across) = match axis {
                        Axis::X => (x, y),
                        Axis::Y => (y, x),
                    };
                    let curve = center + amp * (TAU * (along / wavelength - speed * t + phase)).sin();
                    let d = (across - curve).abs();
                    let a = if d <= half { 1.0 } else { (1.0 - (d - half) / half).max(0.0) };
                    if a > 0.0 {
                        over(px, Rgba::from_color(color, (a * alpha) as f32));
                    }
                }
            }
            CompiledEffect::Orb { color, center, radius, edge, light, shading, dither, dither_rate_hz, seed, motion, speed, orbit_radius, bounce_dir, orbit_phase } => {
                let t = local_us as f64 / 1e6;
                let (cx, cy) = match motion {
                    OrbMotion::None => center,
                    OrbMotion::Orbit => {
                        let a = TAU * (speed * t + orbit_phase);
                        (center.0 + orbit_radius * a.cos(), center.1 + orbit_radius * a.sin())
                    }
                    OrbMotion::Bounce => (bounce(center.0 + bounce_dir.0 * speed * t), bounce(center.1 + bounce_dir.1 * speed * t)),
                };
                let step = (t * dither_rate_hz).floor() as u64;
                for (i, px) in layer.px.iter_mut().enumerate() {
                    let (dx, dy) = ((i % GRID_W) as f64 - cx, (i / GRID_W) as f64 - cy);
                    let d = (dx * dx + dy * dy).sqrt();
                    let shape = if edge > 0.0 { ((radius - d) / edge + 0.5).clamp(0.0, 1.0) } else if d <= radius { 1.0 } else { 0.0 };
                    if shape <= 0.0 {
                        continue;
                    }
                    // Facing the light: 1; away from it: 1 - shading.
                    let n = if d > 1e-9 { ((dx * light.0 + dy * light.1) / d) * (d / radius).min(1.0) } else { 0.0 };
                    let mut b = 1.0 - shading * (1.0 - n) / 2.0;
                    if dither > 0.0 {
                        b *= 1.0 - dither * hash01(seed, i as u64, step, 2);
                    }
                    over(px, Rgba::from_color(color, (shape * b * alpha) as f32));
                }
            }            CompiledEffect::Noise { base, highlight, density, rate_hz, seed, smooth, depth } => {
                let pos = local_us as f64 / 1e6 * rate_hz;
                let step = pos.floor() as u64;
                let frac = (pos - pos.floor()) as f32;
                for (i, px) in layer.px.iter_mut().enumerate() {
                    let mut c = noise_cell(base, highlight, density, depth, seed, i as u64, step);
                    if smooth {
                        let next = noise_cell(base, highlight, density, depth, seed, i as u64, step + 1);
                        c = Rgb::new(c.r + (next.r - c.r) * frac, c.g + (next.g - c.g) * frac, c.b + (next.b - c.b) * frac);
                    }
                    over(px, Rgba::from_color(c, alpha as f32));
                }
            }
        }
    }
}

impl CompiledEffect {
    /// `paint` plus the input-driven effects (Ripple) when `input` is given.
    /// Continuous layers (0.1.2 behaviour): every press is an instance.
    pub fn paint_input(&self, local_us: u64, beat: Option<BeatState>, input: Option<&PadInput>, alpha: f64, layer: &mut LayerFrame) {
        let pad_driven = matches!(self, CompiledEffect::Ripple { interval: None, .. } | CompiledEffect::TapGlow { .. } | CompiledEffect::Cross { .. });
        if !pad_driven {
            return self.paint(local_us, beat, alpha, layer);
        }
        if let Some(input) = input {
            // Newest first.
            let mut inst = [(0.0, 0.0, 0); PadInput::CAPACITY];
            let n = input.hits.len().min(PadInput::CAPACITY);
            for (o, h) in inst.iter_mut().zip(input.hits.iter().rev()) {
                *o = (h.x as f64, h.y as f64, input.now_us - h.t_us);
            }
            self.paint_instances(&inst[..n], alpha, layer);
        }
    }

    /// True for effects drawn from instances (position + age).
    pub fn takes_instances(&self) -> bool {
        matches!(self, CompiledEffect::Ripple { .. } | CompiledEffect::TapGlow { .. } | CompiledEffect::Cross { .. } | CompiledEffect::Frames(_))
    }

    /// Draw instances (newest first). Other effects draw nothing here.
    pub fn paint_instances(&self, inst: &[Instance], alpha: f64, layer: &mut LayerFrame) {
        match *self {
            CompiledEffect::Ripple { look, .. } => paint_ripple(inst.iter().copied(), look, alpha, layer),
            CompiledEffect::TapGlow { start, mid, end, lifetime_us, radius, repeats, repeat_us, decay } => {
                paint_tap_glow(inst, [start, mid, end], lifetime_us, radius, repeats, repeat_us, decay, alpha, layer)
            }
            CompiledEffect::Cross { color, speed, length, lifetime_us } => paint_cross(inst, color, speed, length, lifetime_us, alpha, layer),
            // Each press shows the frames from its own start; `pad` moves cell (3, 3) onto the pad.
            CompiledEffect::Frames(ref f) => {
                for &(x, y, age) in inst {
                    let Some(frame) = (age >= 0).then(|| f.frame_at(age as u64)).flatten() else { continue };
                    let off = if f.pad { (x.round() as i64 - 3, y.round() as i64 - 3) } else { (0, 0) };
                    paint_frame(frame, off, alpha, layer);
                }
            }
            _ => {}
        }
    }
}

fn lerp_rgb(a: Rgb, b: Rgb, k: f32) -> Rgb {
    Rgb::new(a.r + (b.r - a.r) * k, a.g + (b.g - a.g) * k, a.b + (b.b - a.b) * k)
}

/// Each press glows `repeats` times, `repeat_us` apart, each weaker by `decay`;
/// a glow goes start → mid → end over its lifetime. The strongest wins per cell.
#[allow(clippy::too_many_arguments)]
fn paint_tap_glow(inst: &[Instance], colors: [Rgb; 3], lifetime_us: i64, radius: f64, repeats: u32, repeat_us: i64, decay: f64, alpha: f64, layer: &mut LayerFrame) {
    if alpha <= 0.0 {
        return;
    }
    let mut best = [(0.0f64, Rgb::new(0.0, 0.0, 0.0)); GRID_LEDS];
    for &(hx, hy, age) in inst {
        for r in 0..repeats {
            let a = age - r as i64 * repeat_us;
            if !(0..lifetime_us).contains(&a) {
                continue;
            }
            let life = a as f64 / lifetime_us as f64;
            let color = if life < 0.5 { lerp_rgb(colors[0], colors[1], (life * 2.0) as f32) } else { lerp_rgb(colors[1], colors[2], ((life - 0.5) * 2.0) as f32) };
            let strength = decay.powi(r as i32);
            for (i, b) in best.iter_mut().enumerate() {
                let (dx, dy) = ((i % GRID_W) as f64 - hx, (i / GRID_W) as f64 - hy);
                let d = (dx * dx + dy * dy).sqrt();
                let w = if radius > 0.0 { (1.0 - d / radius).max(0.0) } else if d == 0.0 { 1.0 } else { 0.0 };
                let s = w * strength;
                if s > b.0 {
                    *b = (s, color);
                }
            }
        }
    }
    for (px, (s, c)) in layer.px.iter_mut().zip(best) {
        if s > 0.0 {
            over(px, Rgba::from_color(c, (s * alpha) as f32));
        }
    }
}

/// Heads run `speed` cells/s along the pad's row and column; linear tails of
/// `length` cells behind them; everything fades over the lifetime.
fn paint_cross(inst: &[Instance], color: Rgb, speed: f64, length: f64, lifetime_us: i64, alpha: f64, layer: &mut LayerFrame) {
    if alpha <= 0.0 {
        return;
    }
    let mut cell_a = [0.0f64; GRID_LEDS];
    for &(hx, hy, age) in inst {
        if !(0..lifetime_us).contains(&age) {
            continue;
        }
        let fade = 1.0 - age as f64 / lifetime_us as f64;
        let head = speed * age as f64 / 1e6;
        for (i, a) in cell_a.iter_mut().enumerate() {
            let (x, y) = (i % GRID_W, i / GRID_W);
            // The row and the column through the instance's cell.
            let d = if (y as f64 - hy).abs() < 0.5 {
                (x as f64 - hx).abs()
            } else if (x as f64 - hx).abs() < 0.5 {
                (y as f64 - hy).abs()
            } else {
                continue;
            };
            let behind = head - d; // 0 at the head, growing towards the pad
            if behind < 0.0 || behind > length {
                continue;
            }
            *a = a.max((1.0 - behind / length) * fade);
        }
    }
    for (px, a) in layer.px.iter_mut().zip(cell_a) {
        if a > 0.0 {
            over(px, Rgba::from_color(color, (a * alpha) as f32));
        }
    }
}

/// How ripple rings look (shared by pad and interval triggers).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RippleLook {
    pub color: Rgb,
    pub speed: f64,
    pub ring_width: f64,
    pub lifetime_us: i64,
    pub fade: RippleFade,
    pub max_concurrent: usize,
    pub shape: RippleShape,
}

/// The shared shape metric of a ripple ring shape.
fn ripple_metric(s: RippleShape) -> crate::shape::Metric {
    match s {
        RippleShape::Circle => crate::shape::Metric::Circle,
        RippleShape::Diamond => crate::shape::Metric::Diamond,
        RippleShape::Square => crate::shape::Metric::Square,
    }
}

/// Rings from `(x, y, age_us)` sources, newest first; live ones only, at most `max_concurrent`.
fn paint_ripple(sources: impl Iterator<Item = (f64, f64, i64)>, look: RippleLook, alpha: f64, layer: &mut LayerFrame) {
    let RippleLook { color, speed, ring_width, lifetime_us, fade, max_concurrent, shape } = look;
    if alpha <= 0.0 {
        return;
    }
    let half = (ring_width / 2.0).max(1e-6);
    let mut cell_a = [0.0f64; GRID_LEDS];
    let live = sources.filter(|&(_, _, age)| (0..lifetime_us).contains(&age));
    for (x, y, age) in live.take(max_concurrent) {
        let age = age as f64;
        let life = age / lifetime_us as f64;
        let f = match fade {
            RippleFade::Linear => 1.0 - life,
            RippleFade::Exp => (-4.0 * life).exp() * (1.0 - life),
        };
        let radius = speed * age / 1e6;
        for (i, a) in cell_a.iter_mut().enumerate() {
            let dist = crate::shape::metric(ripple_metric(shape), (i % GRID_W) as f64 - x, (i / GRID_W) as f64 - y);
            let off = (dist - radius).abs();
            let ring = if off <= half { 1.0 } else { (1.0 - (off - half) / half).max(0.0) };
            *a = a.max(ring * f);
        }
    }
    for (px, a) in layer.px.iter_mut().zip(cell_a) {
        if a > 0.0 {
            over(px, Rgba::from_color(color, (a * alpha) as f32));
        }
    }
}

/// Number of groups of a region pattern.
pub fn region_count(p: RegionPattern) -> usize {
    match p {
        RegionPattern::HalvesLr | RegionPattern::HalvesTb | RegionPattern::Checker => 2,
        RegionPattern::Quadrants | RegionPattern::Fan => 4,
        RegionPattern::Columns | RegionPattern::Rows => 8,
        RegionPattern::Perimeter => 28,
        // Counted from `groups` (`CustomGroups::n`).
        RegionPattern::Custom => 1,
    }
}

/// Group of cell (x, y) (bottom-left origin), or None (perimeter interior).
pub fn region_of(p: RegionPattern, x: usize, y: usize) -> Option<usize> {
    Some(match p {
        RegionPattern::HalvesLr => (x >= 4) as usize,
        RegionPattern::HalvesTb => (y >= 4) as usize,
        RegionPattern::Checker => (x + y) % 2,
        // Bottom-left, bottom-right, top-right, top-left.
        RegionPattern::Quadrants => match (x >= 4, y >= 4) {
            (false, false) => 0,
            (true, false) => 1,
            (true, true) => 2,
            (false, true) => 3,
        },
        RegionPattern::Columns => x,
        RegionPattern::Rows => y,
        // Clockwise from the bottom-left: up the left edge, along the top, down the right, back along the bottom.
        RegionPattern::Perimeter => match (x, y) {
            (0, y) => y,
            (x, 7) => 7 + x,
            (7, y) => 21 - y,
            (x, 0) => 28 - x,
            _ => return None,
        },
        // The two middle columns first, then outwards in mirrored pairs.
        RegionPattern::Fan => if x < 4 { 3 - x } else { x - 4 },
        // Membership is in `CustomGroups` (a cell may be in several groups).
        RegionPattern::Custom => return None,
    })
}

/// Group lit by step `k` for `n` groups in `order`.
fn step_group(k: i64, n: usize, order: StepOrder) -> usize {
    let n = n as i64;
    (match order {
        StepOrder::Forward => k.rem_euclid(n),
        StepOrder::Reverse => n - 1 - k.rem_euclid(n),
        StepOrder::Pingpong => {
            let u = k.rem_euclid(2 * n - 2);
            if u < n {
                u
            } else {
                2 * n - 2 - u
            }
        }
    }) as usize
}

/// Level of a step at offset `d` steps from its start: lit over `0..duty`,
/// ramping in and out over `soft` steps (centred on the edges).
fn step_level(d: f64, duty: f64, soft: f64) -> f64 {
    if soft <= 0.0 {
        return if (0.0..duty).contains(&d) { 1.0 } else { 0.0 };
    }
    let rise = ((d + soft / 2.0) / soft).clamp(0.0, 1.0);
    let fall = ((duty + soft / 2.0 - d) / soft).clamp(0.0, 1.0);
    rise.min(fall)
}

/// A color parameter at `t`, interpolated per sRGB channel (f32, no 8-bit rounding).
fn col(p: &crate::model::Param<String>, t: u64) -> Option<Rgb> {
    match p.segment(t) {
        (a, None) => Rgb::from_hex(a),
        (a, Some((b, f))) => Some(lerp_rgb(Rgb::from_hex(a)?, Rgb::from_hex(b)?, f as f32)),
    }
}

fn custom_groups(p: &crate::model::RegionsParams) -> Option<CustomGroups> {
    let groups = p.groups.as_ref()?;
    let mut c = CustomGroups { n: groups.len().min(32), masks: [0; 32], offset: [0.0; 32] };
    for (g, cells) in groups.iter().take(32).enumerate() {
        for &cell in cells {
            if (cell as usize) < GRID_LEDS {
                c.masks[g] |= 1 << cell;
            }
        }
        c.offset[g] = p.offset.as_ref().and_then(|o| o.get(g)).copied().unwrap_or(0.0);
    }
    Some(c)
}

/// Signed distance from a cell (offset from the shape centre) to a shape of
/// half sizes `size` turned by `angle` degrees.
pub fn shape_sd(kind: ShapeKind, dx: f64, dy: f64, size: (f64, f64), angle: f64) -> f64 {
    use crate::shape::{metric, sd_rect, segment_distance, unrotate, Metric};
    let (ux, uy) = unrotate(dx, dy, angle);
    match kind {
        ShapeKind::Circle => metric(Metric::Circle, dx, dy) - size.0,
        ShapeKind::Diamond => metric(Metric::Diamond, ux, uy) - size.0,
        ShapeKind::Rect => sd_rect(ux, uy, size.0, size.1),
        ShapeKind::Line => segment_distance(ux, uy, -size.0, 0.0, size.0, 0.0),
        // The side `angle` points to is inside.
        ShapeKind::Wipe => -ux,
    }
}

/// The 28 border cells clockwise from the bottom-left, as a closed square (length 28).
const PERIMETER: [(f64, f64); 5] = [(0.0, 0.0), (0.0, 7.0), (7.0, 7.0), (7.0, 0.0), (0.0, 0.0)];

fn compile_path(p: &crate::model::PathParams, t: u64) -> Option<PathLook> {
    let at = |v: &Option<crate::model::Param<f64>>, d: f64| v.as_ref().map(|x| x.at(t)).unwrap_or(d);
    let kind = p.path.at(t);
    let mut pts = [(0.0, 0.0); 32];
    let n = match kind {
        PathKind::Line => {
            pts[0] = (at(&p.from_x, 0.0), at(&p.from_y, 0.0));
            pts[1] = (at(&p.to_x, 7.0), at(&p.to_y, 7.0));
            2
        }
        PathKind::Points => {
            let src = p.points.as_ref()?;
            for (o, s) in pts.iter_mut().zip(src.iter()) {
                *o = (s[0], s[1]);
            }
            src.len().min(32)
        }
        PathKind::Perimeter => {
            pts[..5].copy_from_slice(&PERIMETER);
            5
        }
        PathKind::Orbit | PathKind::Figure8 => {
            pts[0] = (at(&p.center_x, 3.5), at(&p.center_y, 3.5));
            pts[1] = (at(&p.orbit_radius, 3.0), 0.0);
            2
        }
    };
    let mut look = PathLook {
        color: col(&p.color, t)?,
        kind,
        pts,
        n,
        length: 0.0,
        speed: p.speed.at(t),
        size: p.size.at(t),
        tail: p.tail.at(t),
        looping: p.looping.at(t),
        phase: p.phase.at(t),
        mirror: p.mirror.at(t),
    };
    look.length = match kind {
        PathKind::Orbit => TAU * pts[1].0,
        // Figure eight: measured on 64 chords.
        PathKind::Figure8 => (0..64).map(|i| dist2(path_point(&look, i as f64 / 64.0), path_point(&look, (i + 1) as f64 / 64.0))).sum(),
        _ => polyline_len(&pts[..n]),
    };
    Some(look)
}

fn dist2(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn polyline_len(pts: &[(f64, f64)]) -> f64 {
    pts.windows(2).map(|w| dist2(w[0], w[1])).sum()
}

/// Point at fraction `u` (0..=1) of the path's length.
fn path_point(l: &PathLook, u: f64) -> (f64, f64) {
    match l.kind {
        PathKind::Orbit => {
            let (c, r) = (l.pts[0], l.pts[1].0);
            (c.0 + r * (TAU * u).cos(), c.1 + r * (TAU * u).sin())
        }
        PathKind::Figure8 => {
            let (c, w) = (l.pts[0], l.pts[1].0);
            (c.0 + w * (TAU * u).sin(), c.1 + w / 2.0 * (2.0 * TAU * u).sin())
        }
        _ => {
            let pts = &l.pts[..l.n];
            let total = polyline_len(pts);
            let mut d = u.clamp(0.0, 1.0) * total;
            for w in pts.windows(2) {
                let s = dist2(w[0], w[1]);
                if d <= s && s > 0.0 {
                    let f = d / s;
                    return (w[0].0 + (w[1].0 - w[0].0) * f, w[0].1 + (w[1].1 - w[0].1) * f);
                }
                d -= s;
            }
            pts[pts.len() - 1]
        }
    }
}

/// Position after travelling `d` cells from the start (None before the start).
fn path_at_distance(l: &PathLook, d: f64) -> Option<(f64, f64)> {
    if d < 0.0 || l.length <= 0.0 {
        return (l.length <= 0.0).then(|| path_point(l, 0.0));
    }
    let closed = matches!(l.kind, PathKind::Perimeter | PathKind::Orbit | PathKind::Figure8);
    let len = l.length;
    let u = match (closed, l.looping) {
        (true, true) => (d / len).fract(),
        (false, true) => {
            // Back and forth: period 2 × length.
            let x = d.rem_euclid(2.0 * len);
            (if x <= len { x } else { 2.0 * len - x }) / len
        }
        (_, false) => (d / len).min(1.0),
    };
    Some(path_point(l, u))
}

/// The spot (radius `size`, linear edge) and its tail sampled back along the
/// path, plus mirrored twins. A function of the local time only.
fn paint_path(l: &PathLook, local_us: u64, alpha: f64, layer: &mut LayerFrame) {
    let closed = matches!(l.kind, PathKind::Perimeter | PathKind::Orbit | PathKind::Figure8);
    // Travelled distance in cells (closed paths: speed is laps per second).
    let per_s = if closed { l.speed * l.length } else { l.speed };
    let head = local_us as f64 / 1e6 * per_s + l.phase * l.length;
    let steps = (l.tail * 3.0).ceil() as usize;
    let mut cell_a = [0.0f64; GRID_LEDS];
    for j in 0..=steps {
        let back = if steps == 0 { 0.0 } else { l.tail * j as f64 / steps as f64 };
        let Some(p) = path_at_distance(l, head - back) else { continue };
        let fade = if l.tail > 0.0 { 1.0 - back / (l.tail + 1e-9) } else { 1.0 };
        let twins = match l.mirror {
            Mirror::None => [Some(p), None, None, None],
            Mirror::X => [Some(p), Some((7.0 - p.0, p.1)), None, None],
            Mirror::Y => [Some(p), Some((p.0, 7.0 - p.1)), None, None],
            Mirror::Xy => [Some(p), Some((7.0 - p.0, p.1)), Some((p.0, 7.0 - p.1)), Some((7.0 - p.0, 7.0 - p.1))],
        };
        for q in twins.into_iter().flatten() {
            for (i, a) in cell_a.iter_mut().enumerate() {
                let d = crate::shape::metric(crate::shape::Metric::Circle, (i % GRID_W) as f64 - q.0, (i / GRID_W) as f64 - q.1);
                let spot = (1.0 - d / l.size.max(1e-6)).max(0.0);
                *a = a.max(spot * fade);
            }
        }
    }
    for (px, a) in layer.px.iter_mut().zip(cell_a) {
        if a > 0.0 {
            over(px, Rgba::from_color(l.color, (a * alpha) as f32));
        }
    }
}

/// Reflect a coordinate into 0..=7 (triangle wave, period 14).
fn bounce(p: f64) -> f64 {
    let u = p.rem_euclid(14.0);
    if u <= 7.0 {
        u
    } else {
        14.0 - u
    }
}

/// Seconds between rain slots per lane.
const RAIN_SLOT_S: f64 = 0.5;

#[allow(clippy::too_many_arguments)]
fn paint_rain(local_us: u64, color: Rgb, head: Rgb, direction: RainDirection, speed: f64, density: f64, tail: f64, seed: u32, flicker: f64, alpha: f64, layer: &mut LayerFrame) {
    if alpha <= 0.0 || density <= 0.0 || speed <= 0.0 {
        return;
    }
    let t = local_us as f64 / 1e6;
    let flicker_step = (t * 8.0).floor() as u64;
    // A drop is visible from entry (head at 0) until its tail has left (head at 8 + tail).
    let travel_s = (GRID_W as f64 + tail + 1.0) / speed;
    let k_last = (t / RAIN_SLOT_S).floor() as i64;
    let k_first = ((t - travel_s) / RAIN_SLOT_S).floor() as i64 - 1;
    let mut cell_a = [0.0f64; GRID_LEDS];
    let mut cell_head = [false; GRID_LEDS];
    for lane in 0..8u64 {
        for k in k_first.max(0)..=k_last {
            let slot = k as u64;
            if hash01(seed, lane, slot, 10) >= density {
                continue;
            }
            let spawn = (k as f64 + hash01(seed, lane, slot, 11)) * RAIN_SLOT_S;
            let head_pos = (t - spawn) * speed; // cells from the entry edge
            if head_pos < 0.0 {
                continue;
            }
            let bright = 0.6 + 0.4 * hash01(seed, lane, slot, 12);
            for p in 0..8usize {
                let off = head_pos - p as f64; // 0..1 = head cell, then the tail
                let a = if (0.0..1.0).contains(&off) {
                    1.0
                } else if off >= 1.0 && off < tail + 1.0 {
                    1.0 - (off - 1.0) / (tail + 1.0)
                } else {
                    continue;
                };
                let (x, y) = match direction {
                    RainDirection::Down => (lane as usize, 7 - p),
                    RainDirection::Up => (lane as usize, p),
                    RainDirection::Left => (7 - p, lane as usize),
                    RainDirection::Right => (p, lane as usize),
                };
                let i = y * GRID_W + x;
                let mut a = a * bright;
                if off >= 1.0 && flicker > 0.0 {
                    a *= 1.0 - flicker * hash01(seed, i as u64, flicker_step, 13);
                }
                if a > cell_a[i] {
                    cell_a[i] = a;
                    cell_head[i] = off < 1.0;
                }
            }
        }
    }
    for (i, px) in layer.px.iter_mut().enumerate() {
        if cell_a[i] > 0.0 {
            over(px, Rgba::from_color(if cell_head[i] { head } else { color }, (cell_a[i] * alpha) as f32));
        }
    }
}

/// True when any parameter of the effect has keyframes (checked once at compile).
pub fn has_keyframes(e: &Effect) -> bool {
    serde_json::to_string(e).is_ok_and(|s| s.contains("\"keyframes\""))
}

/// Effect `i` at time `t` on its axis: the compiled constant unless it has keyframes.
fn sample(effects: &[CompiledEffect], keyed: &[Option<Effect>], i: usize, t: u64) -> CompiledEffect {
    match &keyed[i] {
        Some(src) => CompiledEffect::compile_at(src, t).unwrap_or_else(|| effects[i].clone()),
        None => effects[i].clone(),
    }
}

/// Instances of effect `i`; with keyframes each instance samples at its own age.
fn paint_instances_sampled(effects: &[CompiledEffect], keyed: &[Option<Effect>], i: usize, inst: &[Instance], alpha: f64, layer: &mut LayerFrame) {
    match &keyed[i] {
        None => effects[i].paint_instances(inst, alpha, layer),
        Some(src) => {
            // Draw newest first up to the (first-keyframe) concurrency limit.
            let limit = match effects[i] {
                CompiledEffect::Ripple { look, .. } => look.max_concurrent,
                _ => MAX_INSTANCES,
            };
            for one in inst.iter().filter(|x| x.2 >= 0).take(limit) {
                let e = CompiledEffect::compile_at(src, one.2 as u64).unwrap_or_else(|| effects[i].clone());
                e.paint_instances(std::slice::from_ref(one), alpha, layer);
            }
        }
    }
}

/// Frames parsed once: each frame's 64 cells in LED order (None = transparent).
#[derive(Clone, Debug, PartialEq)]
pub struct FramesLook {
    pub frames: std::sync::Arc<[[Option<Rgb>; GRID_LEDS]]>,
    pub fps: f64,
    pub looping: bool,
    pub hold_last: bool,
    pub hold_us: u64,
    pub pad: bool,
}

impl FramesLook {
    /// Parse the frames (top row first in the file, bottom-left origin here).
    pub fn compile(p: &crate::model::FramesParams) -> Option<FramesLook> {
        let color = match &p.color {
            Some(c) => Some(Rgb::from_hex(c)?),
            None => None,
        };
        let key = p.opacity_key.unwrap_or(true);
        let mut frames = Vec::with_capacity(p.frames.len());
        for s in &p.frames {
            if s.len() != 6 * GRID_LEDS || !s.is_ascii() {
                return None;
            }
            let mut f = [None; GRID_LEDS];
            for i in 0..GRID_LEDS {
                let c = Rgb::from_hex(&format!("#{}", &s[6 * i..6 * i + 6]))?;
                let black = c.r == 0.0 && c.g == 0.0 && c.b == 0.0;
                let (row_top, col) = (i / GRID_W, i % GRID_W);
                f[(GRID_H - 1 - row_top) * GRID_W + col] = match (black, key) {
                    (true, true) => None,
                    (true, false) => Some(c),
                    (false, _) => Some(color.unwrap_or(c)),
                };
            }
            frames.push(f);
        }
        if frames.is_empty() {
            return None;
        }
        Some(FramesLook {
            frames: frames.into(),
            fps: p.fps.max(0.001),
            looping: p.looping,
            hold_last: p.hold_last,
            hold_us: p.hold_ms.unwrap_or(0) as u64 * 1000,
            pad: p.anchor == crate::model::FrameAnchor::Pad,
        })
    }

    /// The frame `t_us` after the start: wraps with loop; else the last one
    /// stays (`hold_last`) or for `hold_us` after the end, then none.
    pub fn frame_at(&self, t_us: u64) -> Option<&[Option<Rgb>; GRID_LEDS]> {
        let n = self.frames.len() as u64;
        let k = (t_us as f64 / 1e6 * self.fps).floor() as u64;
        if self.looping {
            return Some(&self.frames[(k % n) as usize]);
        }
        if k < n {
            return Some(&self.frames[k as usize]);
        }
        let end_us = (n as f64 / self.fps * 1e6) as u64;
        (self.hold_last || t_us < end_us + self.hold_us).then(|| &self.frames[(n - 1) as usize])
    }
}

/// Draw a frame moved by `off` cells (what leaves the grid is cut).
fn paint_frame(frame: &[Option<Rgb>; GRID_LEDS], off: (i64, i64), alpha: f64, layer: &mut LayerFrame) {
    for (i, px) in layer.px.iter_mut().enumerate() {
        let (x, y) = ((i % GRID_W) as i64 - off.0, (i / GRID_W) as i64 - off.1);
        if (0..GRID_W as i64).contains(&x) && (0..GRID_H as i64).contains(&y) {
            if let Some(c) = frame[y as usize * GRID_W + x as usize] {
                over(px, Rgba::from_color(c, alpha as f32));
            }
        }
    }
}

/// Text: the glyphs of the text (font columns) and its layout parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLook {
    pub glyphs: std::sync::Arc<[crate::font::Glyph]>,
    pub w: usize,
    pub h: usize,
    pub color: Rgb,
    pub mode: crate::model::TextMode,
    pub speed: f64,
    pub gap: usize,
    pub beat_step: bool,
    pub looping: bool,
}

impl TextLook {
    pub fn compile(p: &crate::model::TextParams, t: u64) -> Option<TextLook> {
        let (w, h) = crate::font::size(p.font);
        Some(TextLook {
            glyphs: p.text.chars().map(|c| crate::font::glyph(p.font, c)).collect::<Vec<_>>().into(),
            w,
            h,
            color: col(&p.color, t)?,
            mode: p.mode,
            speed: p.speed.max(0.001),
            gap: p.gap as usize,
            beat_step: p.beat_step.unwrap_or(false),
            looping: p.looping,
        })
    }

    /// Glyph `ci`, column `x`, row `r` (from the top) lit?
    fn lit(&self, ci: usize, x: usize, r: usize) -> bool {
        x < self.w && r < self.h && self.glyphs[ci][x] >> r & 1 == 1
    }

    /// Grid row of glyph row 0 (5 × 7 from row 7 down, 3 × 5 from row 6).
    fn top(&self) -> i64 {
        if self.h >= 7 {
            7
        } else {
            6
        }
    }
}

/// Text at `local_us`: static (what fits, centred), scrolling left or up
/// (entering from the edge), or one character at a time (per `speed`, or
/// per beat counted from the clip start with `beat_step`).
fn paint_text(look: &TextLook, local_us: u64, beat: Option<BeatState>, alpha: f64, layer: &mut LayerFrame) {
    use crate::model::TextMode;
    let n = look.glyphs.len();
    if n == 0 {
        return;
    }
    let t = local_us as f64 / 1e6;
    let src = Rgba::from_color(look.color, alpha as f32);
    let mut put = |x: i64, y: i64| {
        if (0..GRID_W as i64).contains(&x) && (0..GRID_H as i64).contains(&y) {
            over(&mut layer.px[y as usize * GRID_W + x as usize], src);
        }
    };
    let top = look.top();
    // One glyph with its left column at x0.
    let glyph = |ci: usize, x0: i64, top: i64, put: &mut dyn FnMut(i64, i64)| {
        for x in 0..look.w {
            for r in 0..look.h {
                if look.lit(ci, x, r) {
                    put(x0 + x as i64, top - r as i64);
                }
            }
        }
    };
    let centred = |width: usize| (GRID_W as i64 - width as i64).div_euclid(2);
    // Cells moved since the start, wrapped over `period` cells with loop (None: done).
    let moved = |period: i64| -> Option<i64> {
        let k = (look.speed * t).floor() as i64;
        if look.looping {
            Some(k.rem_euclid(period))
        } else {
            (k < period).then_some(k)
        }
    };
    match look.mode {
        TextMode::Static => {
            let fit = (1..=n).rev().find(|m| m * look.w + (m - 1) * look.gap <= GRID_W).unwrap_or(1);
            let x0 = centred(fit * look.w + (fit - 1) * look.gap);
            for ci in 0..fit {
                glyph(ci, x0 + (ci * (look.w + look.gap)) as i64, top, &mut put);
            }
        }
        TextMode::ScrollLeft => {
            let unit = look.w + look.gap;
            let width = (n * unit - look.gap) as i64;
            let Some(k) = moved(width + GRID_W as i64) else { return };
            // The strip starts just right of the grid and moves left one cell per step.
            for ci in 0..n {
                glyph(ci, GRID_W as i64 - k + (ci * unit) as i64, top, &mut put);
            }
        }
        TextMode::ScrollUp => {
            let unit = look.h + look.gap;
            let height = (n * unit - look.gap) as i64;
            let Some(k) = moved(height + GRID_H as i64) else { return };
            // The stack starts just below the grid; its first glyph leads.
            for ci in 0..n {
                glyph(ci, centred(look.w), k - 1 - (ci * unit) as i64, &mut put);
            }
        }
        TextMode::Sequential => {
            let step = match beat.filter(|b| look.beat_step && b.phase_valid && b.beat.is_finite() && b.tempo_bpm > 0.0) {
                // Beats since the clip start (taken to be on a beat).
                Some(b) => {
                    let start = (b.beat - t * b.tempo_bpm / 60.0).round();
                    (b.beat - start).floor().max(0.0) as usize
                }
                None => (look.speed * t).floor() as usize,
            };
            let ci = if look.looping { step % n } else if step < n { step } else { return };
            glyph(ci, centred(look.w), top, &mut put);
        }
    }
}

/// Particle parameters of effect `i` at local time `t` (keyframes sampled).
fn particle_look(effects: &[CompiledEffect], keyed: &[Option<Effect>], i: usize, t: u64) -> Option<ParticleLook> {
    match sample(effects, keyed, i, t) {
        CompiledEffect::Particles(l) => Some(l),
        _ => None,
    }
}

/// Draw particle events (newest first) of effect `i`: at most `MAX_PARTICLES`
/// per call and what is left of the frame's `show_left` (`MAX_SHOW_PARTICLES`).
fn paint_particles(effects: &[CompiledEffect], keyed: &[Option<Effect>], i: usize, events: &[Event], alpha: f64, layer: &mut LayerFrame, show_left: &mut usize) {
    let mut budget = MAX_PARTICLES.min(*show_left);
    let start = budget;
    let look_at = |t: u64| particle_look(effects, keyed, i, t);
    for ev in events {
        paint_event(ev, &look_at, alpha, layer, &mut budget);
    }
    *show_left -= start - budget;
}

/// Events of a continuous particle layer at show time `t`: a fountain emits
/// from 0 on; the other modes repeat, one event per span (implode with its pause).
fn continuous_particle_events(look_at: &dyn Fn(u64) -> Option<ParticleLook>, t: u64, out: &mut Vec<Event>) {
    out.clear();
    let Some(look) = look_at(t) else { return };
    let (x, y) = look.origin;
    if look.mode == crate::model::ParticleMode::Fountain {
        out.push(Event { x, y, age_s: t as f64 / 1e6, emit_s: f64::INFINITY, id: 0, start_us: 0 });
    } else {
        let period = ((look.span_s(y, 0.0) * 1e6) as u64).max(1);
        let k = t / period;
        out.push(Event { x, y, age_s: (t - k * period) as f64 / 1e6, emit_s: 0.0, id: k, start_us: k * period });
    }
}

/// Events of one active triggered particle clip (newest first, at most
/// `MAX_EVENTS`). Pads emit at the pad (a rocket from the bottom of its
/// column); clip_start / interval at the effect's emitter. A fountain emits
/// while held on `hold`, otherwise for one lifetime.
fn triggered_particle_events(on: TriggerOn, interval_us: i64, span: &ClipSpan, local: i64, input: Option<&PadInput>, look_at: &dyn Fn(u64) -> Option<ParticleLook>, out: &mut Vec<Event>) {
    use crate::model::ParticleMode;
    out.clear();
    let mut push = |x: f64, y: f64, age_us: i64, emit_s: Option<f64>, id: u64| {
        let start_us = (local - age_us).max(0) as u64;
        let Some(look) = look_at(start_us) else { return };
        let emit_s = if look.mode == ParticleMode::Fountain { emit_s.unwrap_or(look.lifetime_s) } else { 0.0 };
        let age_s = age_us as f64 / 1e6;
        if age_us >= 0 && age_s < look.span_s(y, emit_s) {
            out.push(Event { x, y, age_s, emit_s, id, start_us });
        }
    };
    match on {
        TriggerOn::ClipStart => {
            if let Some(l) = look_at(0) {
                push(l.origin.0, l.origin.1, local, None, 0);
            }
        }
        TriggerOn::Interval => {
            let mut k = local / interval_us;
            while k >= 0 && local - k * interval_us <= PadInput::MAX_AGE_US {
                if let Some(l) = look_at((k * interval_us) as u64) {
                    push(l.origin.0, l.origin.1, local - k * interval_us, None, k as u64);
                }
                k -= 1;
            }
        }
        TriggerOn::Press | TriggerOn::Release | TriggerOn::Hold => {
            let Some(input) = input else { return };
            let now = input.now_us;
            for h in input.hits.iter().rev() {
                if !h.show_us.is_some_and(|s| span.contains(s)) {
                    continue;
                }
                let id = (h.t_us as u64) ^ ((h.x as u64) << 56) ^ ((h.y as u64) << 48);
                let release = h.effective_release(now);
                let (age, emit, id) = match (on, release) {
                    (TriggerOn::Release, Some(r)) => (now - r, None, id ^ 1),
                    (TriggerOn::Release, None) => continue,
                    // Hold: a fountain emits while the pad is held.
                    (TriggerOn::Hold, r) => (now - h.t_us, Some(r.unwrap_or(now).saturating_sub(h.t_us) as f64 / 1e6), id),
                    _ => (now - h.t_us, None, id),
                };
                let rocket = look_at((local - age).max(0) as u64).is_some_and(|l| l.mode == ParticleMode::Rocket);
                push(h.x as f64, if rocket { 0.0 } else { h.y as f64 }, age, emit, id);
            }
        }
    }
    out.truncate(MAX_EVENTS);
}

/// Instances of one active triggered clip (newest first, at most `MAX_INSTANCES`).
/// Pad events count only when they arrived while this clip was active.
fn triggered_instances(on: TriggerOn, interval_us: i64, origin: (f64, f64), span: &ClipSpan, local: i64, input: Option<&PadInput>, out: &mut Vec<Instance>) {
    out.clear();
    match on {
        TriggerOn::ClipStart => out.push((origin.0, origin.1, local)),
        TriggerOn::Interval => {
            let mut k = local / interval_us;
            while k >= 0 && out.len() < MAX_INSTANCES {
                let age = local - k * interval_us;
                if age > PadInput::MAX_AGE_US {
                    break;
                }
                out.push((origin.0, origin.1, age));
                k -= 1;
            }
        }
        TriggerOn::Press | TriggerOn::Release | TriggerOn::Hold => {
            let Some(input) = input else { return };
            let now = input.now_us;
            for h in input.hits.iter().rev() {
                if out.len() == MAX_INSTANCES {
                    break;
                }
                if !h.show_us.is_some_and(|s| span.contains(s)) {
                    continue;
                }
                let age = match (on, h.effective_release(now)) {
                    (TriggerOn::Press, _) => now - h.t_us,
                    (TriggerOn::Release, Some(r)) => now - r,
                    (TriggerOn::Hold, None) => now - h.t_us,
                    _ => continue,
                };
                out.push((h.x as f64, h.y as f64, age));
            }
        }
    }
}

/// SplitMix64 finalizer: a well-mixed 64-bit value from one input.
#[inline]
fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Deterministic 0..1 value for (seed, cell, step, salt).
#[inline]
pub fn hash01(seed: u32, cell: u64, step: u64, salt: u64) -> f64 {
    let h = mix64(mix64(mix64(seed as u64 ^ (salt << 32)) ^ cell) ^ step);
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Color of one noise cell at one step.
fn noise_cell(base: Rgb, highlight: Rgb, density: f64, depth: f64, seed: u32, cell: u64, step: u64) -> Rgb {
    if hash01(seed, cell, step, 0) < density {
        highlight.scale((1.0 - depth * hash01(seed, cell, step, 1)) as f32)
    } else {
        base
    }
}

#[inline]
fn over(dst: &mut Rgba, src: Rgba) {
    let k = 1.0 - src.a;
    dst.r = src.r + dst.r * k;
    dst.g = src.g + dst.g * k;
    dst.b = src.b + dst.b * k;
    dst.a = src.a + dst.a * k;
}

/// Apply a premultiplied layer onto the opaque frame.
pub fn blend_layer(frame: &mut Frame, layer: &LayerFrame, blend: Blend, opacity: f32) {
    for (f, l) in frame.leds.iter_mut().zip(layer.px.iter()) {
        let (r, g, b, a) = (l.r * opacity, l.g * opacity, l.b * opacity, l.a * opacity);
        match blend {
            Blend::Over => {
                let k = 1.0 - a;
                f.r = f.r * k + r;
                f.g = f.g * k + g;
                f.b = f.b * k + b;
            }
            Blend::Add => {
                f.r = (f.r + r).min(1.0);
                f.g = (f.g + g).min(1.0);
                f.b = (f.b + b).min(1.0);
            }
            Blend::Max => {
                f.r = f.r.max(r);
                f.g = f.g.max(g);
                f.b = f.b.max(b);
            }
        }
    }
}

#[derive(Clone, Debug)]
enum LayerContent {
    Continuous { effect: usize },
    /// `uses[i]`: the compound placement of clip i (index into `Renderer::uses`).
    Timeline { index: ClipIndex, effects: Vec<usize>, uses: Vec<Option<usize>> },
    Triggered { index: ClipIndex, effects: Vec<usize>, on: TriggerOn, interval_us: i64, origin: (f64, f64) },
}

/// Content of a compound's inner layer.
#[derive(Clone, Debug)]
enum InnerContent {
    Continuous { effect: usize },
    Timeline { index: ClipIndex, effects: Vec<usize> },
}

#[derive(Clone, Debug)]
struct InnerLayer {
    enabled: bool,
    opacity: f32,
    blend: Blend,
    content: InnerContent,
    mask: Option<LayerMask>,
}

/// One placement of a compound (a clip): its inner layers with the effects
/// of this placement (tinted copies when the clip has a tint).
#[derive(Clone, Debug)]
struct CompoundUse {
    length_us: u64,
    looping: bool,
    time_scale: f64,
    layers: Vec<InnerLayer>,
}

/// Color keys of effect parameters; the first three are the main color
/// (whichever the effect has), the rest follow it by brightness.
const COLOR_KEYS: [&str; 8] = ["color", "highlight_color", "color_start", "color_b", "head_color", "base_color", "color_mid", "color_end"];

fn luminance(c: Rgb) -> f32 {
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

/// The effect with every color replaced by `tint`. The main color becomes
/// `tint`; the other colors keep their brightness relative to the main one
/// (darker: `tint` scaled down; brighter: `tint` mixed towards white), so a
/// dark background or a white head stays one. Keyframed colors become the
/// constant derived from their first value.
pub fn tinted(e: &Effect, tint: Rgb) -> Effect {
    let Ok(mut v) = serde_json::to_value(e) else { return e.clone() };
    let Some(params) = v.get_mut("params").and_then(|p| p.as_object_mut()) else { return e.clone() };
    let first = |x: &serde_json::Value| -> Option<Rgb> {
        let s = x.as_str().or_else(|| x.get("keyframes")?.get(0)?.get("v")?.as_str())?;
        Rgb::from_hex(s)
    };
    let main = COLOR_KEYS[..3].iter().find_map(|k| params.get(*k).and_then(first)).map(luminance).unwrap_or(1.0).max(1e-3);
    for k in COLOR_KEYS {
        let Some(orig) = params.get(k).and_then(first) else { continue };
        let ratio = luminance(orig) / main;
        let c = if ratio <= 1.0 {
            tint.scale(ratio)
        } else {
            let w = (ratio - 1.0).min(1.0);
            Rgb::new(tint.r + (1.0 - tint.r) * w, tint.g + (1.0 - tint.g) * w, tint.b + (1.0 - tint.b) * w)
        };
        params.insert(k.to_string(), serde_json::Value::String(c.to_hex()));
    }
    serde_json::from_value(v).unwrap_or_else(|_| e.clone())
}

/// Composite a premultiplied layer onto a premultiplied buffer.
fn blend_into(dst: &mut LayerFrame, src: &LayerFrame, blend: Blend, opacity: f32) {
    for (d, s) in dst.px.iter_mut().zip(src.px.iter()) {
        let (r, g, b, a) = (s.r * opacity, s.g * opacity, s.b * opacity, s.a * opacity);
        match blend {
            Blend::Over => {
                let k = 1.0 - a;
                *d = Rgba { r: r + d.r * k, g: g + d.g * k, b: b + d.b * k, a: a + d.a * k };
            }
            Blend::Add => *d = Rgba { r: (d.r + r).min(1.0), g: (d.g + g).min(1.0), b: (d.b + b).min(1.0), a: (d.a + a).min(1.0) },
            Blend::Max => *d = Rgba { r: d.r.max(r), g: d.g.max(g), b: d.b.max(b), a: d.a.max(a) },
        }
    }
}

/// Timeline clips at `t` (layer time) into `buf`: presets only (particles included).
#[allow(clippy::too_many_arguments)]
fn paint_timeline_presets(effects: &[CompiledEffect], keyed: &[Option<Effect>], fx: &[usize], active: &[ClipSpan], t: ShowTimeUs, beat: Option<BeatState>, input: Option<&PadInput>, buf: &mut LayerFrame, show_left: &mut usize) {
    for span in active {
        let local = span.local_us(t);
        let ei = fx[span.order];
        if matches!(effects[ei], CompiledEffect::Particles(_)) {
            // One event from the clip start; parameters as at the start.
            let Some(l) = particle_look(effects, keyed, ei, 0) else { continue };
            let ev = Event { x: l.origin.0, y: l.origin.1, age_s: local as f64 / 1e6, emit_s: f64::INFINITY, id: span.order as u64, start_us: 0 };
            paint_particles(effects, keyed, ei, std::slice::from_ref(&ev), span.fade(t), buf, show_left);
            continue;
        }
        sample(effects, keyed, ei, local).paint_input(local, beat, input, span.fade(t), buf);
    }
}

/// A compound at its inner time `t` into `out` (cleared): each inner layer
/// drawn into `layer`, masked, then blended with its blend and opacity.
#[allow(clippy::too_many_arguments)]
fn paint_compound(effects: &[CompiledEffect], keyed: &[Option<Effect>], u: &CompoundUse, t: u64, beat: Option<BeatState>, input: Option<&PadInput>, active: &mut Vec<ClipSpan>, events: &mut Vec<Event>, layer: &mut LayerFrame, out: &mut LayerFrame, show_left: &mut usize) {
    out.clear();
    for l in &u.layers {
        if !l.enabled || l.opacity <= 0.0 {
            continue;
        }
        layer.clear();
        let mask_t = match &l.content {
            InnerContent::Continuous { effect } => {
                if matches!(effects[*effect], CompiledEffect::Particles(_)) {
                    continuous_particle_events(&|x| particle_look(effects, keyed, *effect, x), t, events);
                    paint_particles(effects, keyed, *effect, events, 1.0, layer, show_left);
                } else {
                    sample(effects, keyed, *effect, t).paint_input(t, beat, input, 1.0, layer);
                }
                Some(t)
            }
            InnerContent::Timeline { index, effects: fx } => {
                index.active(t as ShowTimeUs, active);
                paint_timeline_presets(effects, keyed, fx, active, t as ShowTimeUs, beat, input, layer, show_left);
                active.first().map(|s| s.local_us(t as ShowTimeUs))
            }
        };
        if let (Some(m), Some(mt)) = (&l.mask, mask_t) {
            apply_mask(m, mt, layer);
        }
        blend_into(out, layer, l.blend, l.opacity);
    }
}

#[derive(Clone, Debug)]
struct CompiledLayer {
    enabled: bool,
    opacity: f32,
    blend: Blend,
    content: LayerContent,
    mask: Option<LayerMask>,
}

/// Multiply a layer by its mask sampled at `t` (the layer's own time axis).
fn apply_mask(m: &LayerMask, t: u64, layer: &mut LayerFrame) {
    let size = m.size.at(t);
    let size = (size, m.size_y.as_ref().map_or(size, |p| p.at(t)));
    let (kind, cx, cy, edge, invert) = (m.shape.at(t), m.center_x.at(t), m.center_y.at(t), m.edge.at(t), m.invert.at(t));
    let angle = m.angle.as_ref().map_or(0.0, |p| p.at(t));
    for (i, px) in layer.px.iter_mut().enumerate() {
        let a = crate::shape::fill_alpha(shape_sd(kind, (i % GRID_W) as f64 - cx, (i / GRID_W) as f64 - cy, size, angle), edge);
        let k = (if invert { 1.0 - a } else { a }) as f32;
        if k < 1.0 {
            px.r *= k;
            px.g *= k;
            px.b *= k;
            px.a *= k;
        }
    }
}

/// Compiled show, ready to render any time.
#[derive(Clone, Debug)]
pub struct Renderer {
    effects: Vec<CompiledEffect>,
    layers: Vec<CompiledLayer>,
    master_brightness: f32,
    buf: LayerFrame,
    active: Vec<ClipSpan>,
    /// Layers muted by the flashing limit (safety); the show is unchanged.
    muted: Vec<bool>,
    /// Instances of one triggered clip (reused, never grows past `MAX_INSTANCES`).
    inst: Vec<Instance>,
    /// Presets with keyframes (same index as `effects`): sampled per frame
    /// on their time axis; None = the compiled constant in `effects`.
    keyed: Vec<Option<Effect>>,
    /// Particle events of one layer (reused, at most `MAX_EVENTS` kept).
    events: Vec<Event>,
    /// Compound placements, and the buffers and clip list they draw with.
    uses: Vec<CompoundUse>,
    inner_active: Vec<ClipSpan>,
    inner_buf: LayerFrame,
    compound_buf: LayerFrame,
}

impl Renderer {
    /// Validate and compile. Fails with the semantic issues if any.
    pub fn new(show: &Show) -> Result<Renderer, Vec<Issue>> {
        let issues = validate(show);
        if !issues.is_empty() {
            return Err(issues);
        }
        let mut by_id = HashMap::new();
        let mut effects = Vec::with_capacity(show.presets.len());
        let mut keyed = Vec::with_capacity(show.presets.len());
        for (i, p) in show.presets.iter().enumerate() {
            effects.push(CompiledEffect::compile(&p.effect).expect("validated color"));
            keyed.push(has_keyframes(&p.effect).then(|| p.effect.clone()));
            by_id.insert(p.id.as_str(), i);
        }
        // Compound placements: tinted presets are compiled as extra effects.
        let mut uses: Vec<CompoundUse> = Vec::new();
        let mut tinted_ids: HashMap<(usize, String), usize> = HashMap::new();
        let mut max_inner_clips = 0;
        let mut place = |clip: &crate::model::Clip, effects: &mut Vec<CompiledEffect>, keyed: &mut Vec<Option<Effect>>| -> Option<usize> {
            let cp = show.compounds.iter().find(|c| Some(&c.id) == clip.compound_id.as_ref())?;
            let tint = clip.tint.as_deref().and_then(Rgb::from_hex);
            let mut effect_of = |pid: &str| -> usize {
                let base = by_id[pid];
                let Some(tint) = tint else { return base };
                *tinted_ids.entry((base, clip.tint.clone().unwrap_or_default())).or_insert_with(|| {
                    let e = tinted(&show.presets[base].effect, tint);
                    effects.push(CompiledEffect::compile(&e).expect("tinted colors are valid"));
                    keyed.push(has_keyframes(&e).then(|| e.clone()));
                    effects.len() - 1
                })
            };
            let layers = cp
                .layers
                .iter()
                .map(|l| InnerLayer {
                    enabled: l.enabled,
                    opacity: l.opacity as f32,
                    blend: l.blend,
                    mask: l.mask.clone(),
                    content: match &l.content {
                        Content::Continuous { preset_id } => InnerContent::Continuous { effect: effect_of(preset_id) },
                        Content::Timeline { clips } | Content::Triggered { clips, .. } => {
                            max_inner_clips = max_inner_clips.max(clips.len());
                            InnerContent::Timeline { index: ClipIndex::new(clips), effects: clips.iter().map(|c| effect_of(&c.preset_id)).collect() }
                        }
                    },
                })
                .collect();
            uses.push(CompoundUse { length_us: cp.length_us(), looping: clip.looping.unwrap_or(false), time_scale: clip.time_scale.unwrap_or(1.0), layers });
            Some(uses.len() - 1)
        };
        let mut max_clips = 0;
        let layers = show
            .layers
            .iter()
            .map(|l| {
                let content = match &l.content {
                    Content::Continuous { preset_id } => LayerContent::Continuous { effect: by_id[preset_id.as_str()] },
                    Content::Timeline { clips } => {
                        max_clips = max_clips.max(clips.len());
                        LayerContent::Timeline {
                            index: ClipIndex::new(clips),
                            // Compound clips have no preset: their effects are in `uses`.
                            effects: clips.iter().map(|c| by_id.get(c.preset_id.as_str()).copied().unwrap_or(usize::MAX)).collect(),
                            uses: clips.iter().map(|c| place(c, &mut effects, &mut keyed)).collect(),
                        }
                    }
                    Content::Triggered { clips, trigger } => {
                        max_clips = max_clips.max(clips.len());
                        LayerContent::Triggered {
                            index: ClipIndex::new(clips),
                            effects: clips.iter().map(|c| by_id[c.preset_id.as_str()]).collect(),
                            on: trigger.on,
                            interval_us: trigger.interval_ms.unwrap_or(1000).max(1) as i64 * 1000,
                            origin: (trigger.origin_x.unwrap_or(3.5), trigger.origin_y.unwrap_or(3.5)),
                        }
                    }
                };
                CompiledLayer { enabled: l.enabled, opacity: l.opacity as f32, blend: l.blend, content, mask: l.mask.clone() }
            })
            .collect();
        Ok(Renderer {
            effects,
            layers,
            master_brightness: show.render.master_brightness as f32,
            buf: LayerFrame::default(),
            active: Vec::with_capacity(max_clips),
            muted: vec![false; show.layers.len()],
            inst: Vec::with_capacity(MAX_INSTANCES),
            keyed,
            events: Vec::with_capacity(4 * MAX_EVENTS),
            uses,
            inner_active: Vec::with_capacity(max_inner_clips),
            inner_buf: LayerFrame::default(),
            compound_buf: LayerFrame::default(),
        })
    }

    /// Mute (render as opacity 0) or unmute a layer by index. Used by the
    /// flashing limit; does not change the show.
    pub fn set_muted(&mut self, layer: usize, muted: bool) {
        if let Some(m) = self.muted.get_mut(layer) {
            *m = muted;
        }
    }

    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Render the frame at `t` (show time, song start = 0). No allocation.
    /// Input-driven effects (Ripple) draw nothing here.
    pub fn render(&mut self, t: ShowTimeUs, beat: Option<BeatState>) -> Frame {
        self.render_input(t, beat, None)
    }

    /// `render` with the pad hits for input-driven effects.
    pub fn render_input(&mut self, t: ShowTimeUs, beat: Option<BeatState>, input: Option<&PadInput>) -> Frame {
        let mut frame = Frame::default();
        // Particles left for this frame (all layers): lower layers draw first.
        let mut show_left = MAX_SHOW_PARTICLES;
        for (i, layer) in self.layers.iter().enumerate() {
            if !layer.enabled || layer.opacity <= 0.0 || t < 0 || self.muted[i] {
                continue;
            }
            self.buf.clear();
            match &layer.content {
                LayerContent::Continuous { effect } if matches!(self.effects[*effect], CompiledEffect::Particles(_)) => {
                    let (effects, keyed, ei) = (&self.effects, &self.keyed, *effect);
                    continuous_particle_events(&|x| particle_look(effects, keyed, ei, x), t as u64, &mut self.events);
                    paint_particles(effects, keyed, ei, &self.events, 1.0, &mut self.buf, &mut show_left);
                }
                LayerContent::Continuous { effect } => {
                    let e = sample(&self.effects, &self.keyed, *effect, t as u64);
                    match (&self.keyed[*effect], input) {
                        // Keyframed instance effects follow each instance's age.
                        (Some(_), Some(input)) if e.takes_instances() && !matches!(e, CompiledEffect::Ripple { interval: Some(_), .. }) => {
                            self.inst.clear();
                            self.inst.extend(input.hits.iter().rev().map(|h| (h.x as f64, h.y as f64, input.now_us - h.t_us)));
                            paint_instances_sampled(&self.effects, &self.keyed, *effect, &self.inst, 1.0, &mut self.buf);
                        }
                        _ => e.paint_input(t as u64, beat, input, 1.0, &mut self.buf),
                    }
                }
                LayerContent::Timeline { index, effects, uses } => {
                    index.active(t, &mut self.active);
                    for span in &self.active {
                        let Some(u) = uses[span.order] else {
                            paint_timeline_presets(&self.effects, &self.keyed, effects, std::slice::from_ref(span), t, beat, input, &mut self.buf, &mut show_left);
                            continue;
                        };
                        // A compound: inner time = local × time_scale; past its
                        // length it repeats (loop) or draws nothing.
                        let u = &self.uses[u];
                        let mut inner = (span.local_us(t) as f64 * u.time_scale) as u64;
                        if inner >= u.length_us {
                            if !u.looping || u.length_us == 0 {
                                continue;
                            }
                            inner %= u.length_us;
                        }
                        paint_compound(&self.effects, &self.keyed, u, inner, beat, input, &mut self.inner_active, &mut self.events, &mut self.inner_buf, &mut self.compound_buf, &mut show_left);
                        blend_into(&mut self.buf, &self.compound_buf, Blend::Over, span.fade(t) as f32);
                    }
                }
                LayerContent::Triggered { index, effects, on, interval_us, origin } => {
                    index.active(t, &mut self.active);
                    for span in &self.active {
                        let local = span.local_us(t) as i64;
                        let ei = effects[span.order];
                        if matches!(self.effects[ei], CompiledEffect::Particles(_)) {
                            let (fx, keyed) = (&self.effects, &self.keyed);
                            triggered_particle_events(*on, *interval_us, span, local, input, &|x| particle_look(fx, keyed, ei, x), &mut self.events);
                            paint_particles(fx, keyed, ei, &self.events, span.fade(t), &mut self.buf, &mut show_left);
                            continue;
                        }
                        triggered_instances(*on, *interval_us, *origin, span, local, input, &mut self.inst);
                        if self.effects[ei].takes_instances() {
                            paint_instances_sampled(&self.effects, &self.keyed, ei, &self.inst, span.fade(t), &mut self.buf);
                        } else if !self.inst.is_empty() {
                            // A fixed-position effect on `hold`: drawn while a pad is held.
                            sample(&self.effects, &self.keyed, ei, local as u64).paint(local as u64, beat, span.fade(t), &mut self.buf);
                        }
                    }
                }
            }
            if let Some(m) = &layer.mask {
                // Time axis: show time, or the local time of the (first) active clip.
                let local = match layer.content {
                    LayerContent::Continuous { .. } => Some(t as u64),
                    _ => self.active.first().map(|s| s.local_us(t)),
                };
                if let Some(local) = local {
                    apply_mask(m, local, &mut self.buf);
                }
            }
            blend_layer(&mut frame, &self.buf, layer.blend, layer.opacity);
        }
        if self.master_brightness != 1.0 {
            for c in frame.leds.iter_mut() {
                *c = c.scale(self.master_brightness);
            }
        }
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::LedId;

    fn demo() -> Show {
        crate::model::demo_for_tests()
    }

    fn close(a: Rgb, b: Rgb) -> bool {
        (a.r - b.r).abs() < 1e-5 && (a.g - b.g).abs() < 1e-5 && (a.b - b.b).abs() < 1e-5
    }

    #[test]
    fn solid_over_and_brightness() {
        let mut r = Renderer::new(&demo()).unwrap();
        // 30 s: base fully on (no fade), sweep active, pulse needs a beat (none).
        let f = r.render(30_000_000, None);
        let blue = Rgb::from_hex("#163B75").unwrap();
        // Pick a cell outside the sweep band: the base color times brightness 0.6.
        let off_band = (0..64).map(|i| f.leds[i]).find(|c| close(*c, blue.scale(0.6)));
        assert!(off_band.is_some(), "{:?}", f.rows_top_down());
    }

    #[test]
    fn before_start_and_after_end_are_black() {
        let mut r = Renderer::new(&demo()).unwrap();
        assert_eq!(r.render(-1, Some(BeatState::new(0.0, 120.0))), Frame::default());
        // After every timeline clip, only the continuous pulse remains (with a beat).
        let f = r.render(70_000_000, None);
        assert_eq!(f, Frame::default());
    }

    #[test]
    fn fade_in_scales_the_base() {
        let mut r = Renderer::new(&demo()).unwrap();
        let half = r.render(500_000, None); // fade_in 1 s -> 0.5
        let full = r.render(5_000_000, None);
        let a = half.get(LedId::new(0, 0).unwrap());
        let b = full.get(LedId::new(0, 0).unwrap());
        assert!((a.b - b.b * 0.5).abs() < 1e-5, "{a:?} {b:?}");
    }

    #[test]
    fn chase_band_moves_along_x_and_wraps() {
        let e = CompiledEffect::Chase { color: Rgb::new(1.0, 1.0, 1.0), period_us: 8_000, width: 2, axis: Axis::X, reverse: false };
        let lit = |local: u64| {
            let mut l = LayerFrame::default();
            e.paint(local, None, 1.0, &mut l);
            (0..8).filter(|&x| l.px[x].a > 0.0).collect::<Vec<_>>()
        };
        assert_eq!(lit(0), vec![0, 7]); // head at 0, band wraps to 7
        assert_eq!(lit(1_000), vec![0, 1]);
        assert_eq!(lit(7_999), vec![6, 7]);
        assert_eq!(lit(8_000), vec![0, 7]);
        // Every row of a lit column is lit (axis x).
        let mut l = LayerFrame::default();
        e.paint(3_000, None, 1.0, &mut l);
        assert!((0..8).all(|y| l.px[y * 8 + 3].a == 1.0));
        // Reverse: the mirror image, moving right to left.
        let r = CompiledEffect::Chase { color: Rgb::new(1.0, 1.0, 1.0), period_us: 8_000, width: 2, axis: Axis::X, reverse: true };
        let lit_r = |local: u64| {
            let mut l = LayerFrame::default();
            r.paint(local, None, 1.0, &mut l);
            (0..8).filter(|&x| l.px[x].a > 0.0).collect::<Vec<_>>()
        };
        assert_eq!(lit_r(0), vec![0, 7]);
        assert_eq!(lit_r(1_000), vec![6, 7]);
        assert_eq!(lit_r(2_000), vec![5, 6]);
        // Absent `reverse` compiles as before.
        let json = r##"{"type":"builtin.chase","version":1,"params":{"color":"#FFFFFF","period_us":8000,"width":2,"axis":"x"}}"##;
        let parsed: Effect = serde_json::from_str(json).unwrap();
        assert_eq!(CompiledEffect::compile(&parsed), Some(e));
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json, "not written when absent");
    }

    #[test]
    fn pulse_follows_phase_and_is_transparent_without_a_beat() {
        let mk = |depth: f64, wf: Waveform| CompiledEffect::Pulse { color: Rgb::new(1.0, 1.0, 1.0), beats_per_cycle: 1.0, depth, waveform: wf };
        let alpha = |e: &CompiledEffect, beat: Option<BeatState>| {
            let mut l = LayerFrame::default();
            e.paint(0, beat, 1.0, &mut l);
            l.px[0].a
        };
        let sine = mk(1.0, Waveform::Sine);
        assert!((alpha(&sine, Some(BeatState::new(4.0, 120.0))) - 1.0).abs() < 1e-6); // phase 0 = max
        assert!(alpha(&sine, Some(BeatState::new(4.5, 120.0))) < 1e-6); // half phase = 0
        assert!((alpha(&mk(0.0, Waveform::Sine), Some(BeatState::new(4.5, 120.0))) - 1.0).abs() < 1e-6); // depth 0
        assert!((alpha(&mk(1.0, Waveform::Triangle), Some(BeatState::new(0.25, 120.0))) - 0.5).abs() < 1e-6);
        assert_eq!(alpha(&sine, None), 0.0);
        let mut unknown = BeatState::new(4.0, 120.0);
        unknown.phase_valid = false;
        assert_eq!(alpha(&sine, Some(unknown)), 0.0);
        // 0.25 beats per cycle: beat 0.125 is mid-cycle.
        let quarter = CompiledEffect::Pulse { color: Rgb::new(1.0, 1.0, 1.0), beats_per_cycle: 0.25, depth: 1.0, waveform: Waveform::Sine };
        assert!(alpha(&quarter, Some(BeatState::new(0.125, 120.0))) < 1e-6);
    }

    #[test]
    fn strobe_is_a_square_wave_in_hz_or_on_the_beat() {
        let white = Rgb::new(1.0, 1.0, 1.0);
        let hz = CompiledEffect::Strobe { color: white, mode: StrobeMode::Hz, rate_hz: 4.0, beats_per_cycle: 1.0, duty: 0.25, phase_offset: 0.0 };
        let alpha = |e: &CompiledEffect, local: u64, beat: Option<BeatState>| {
            let mut l = LayerFrame::default();
            e.paint(local, beat, 1.0, &mut l);
            assert!(l.px.iter().all(|p| p.a == l.px[0].a), "whole surface");
            l.px[0].a
        };
        // 4 Hz = 250 ms cycles, on for the first 62.5 ms.
        assert_eq!(alpha(&hz, 0, None), 1.0);
        assert_eq!(alpha(&hz, 62_000, None), 1.0);
        assert_eq!(alpha(&hz, 63_000, None), 0.0);
        assert_eq!(alpha(&hz, 250_000, None), 1.0);
        // phase_offset 0.5 shifts the on part by half a cycle.
        let off = CompiledEffect::Strobe { color: white, mode: StrobeMode::Hz, rate_hz: 4.0, beats_per_cycle: 1.0, duty: 0.25, phase_offset: 0.5 };
        assert_eq!(alpha(&off, 0, None), 0.0);
        assert_eq!(alpha(&off, 125_000, None), 1.0);
        // Beat mode: 2 beats per cycle, duty 0.5; transparent without a phase.
        let beat = CompiledEffect::Strobe { color: white, mode: StrobeMode::Beat, rate_hz: 1.0, beats_per_cycle: 2.0, duty: 0.5, phase_offset: 0.0 };
        assert_eq!(alpha(&beat, 0, Some(BeatState::new(4.9, 120.0))), 1.0);
        assert_eq!(alpha(&beat, 0, Some(BeatState::new(5.1, 120.0))), 0.0);
        assert_eq!(alpha(&beat, 0, None), 0.0);
        let mut unknown = BeatState::new(4.0, 120.0);
        unknown.phase_valid = false;
        assert_eq!(alpha(&beat, 0, Some(unknown)), 0.0);
    }

    #[test]
    fn noise_is_deterministic_dense_as_asked_and_smooth_on_request() {
        let base = Rgb::from_hex("#0D2346").unwrap();
        let hi = Rgb::from_hex("#9FE8FF").unwrap();
        let mk = |density: f64, smooth: bool, seed: u32| CompiledEffect::Noise { base, highlight: hi, density, rate_hz: 6.0, seed, smooth, depth: 0.5 };
        let paint = |e: &CompiledEffect, local: u64| {
            let mut l = LayerFrame::default();
            e.paint(local, None, 1.0, &mut l);
            l
        };
        let e = mk(0.3, false, 1);
        // Same time, same picture; within one step (1/6 s) the pattern holds.
        assert_eq!(paint(&e, 1_000_000).px, paint(&e, 1_000_000).px);
        assert_eq!(paint(&e, 1_000_000).px, paint(&e, 1_100_000).px);
        assert_ne!(paint(&e, 1_000_000).px, paint(&e, 1_200_000).px);
        // Another seed gives another pattern.
        assert_ne!(paint(&e, 1_000_000).px, paint(&mk(0.3, false, 2), 1_000_000).px);
        // Density: the share of non-base cells over many steps is close to 0.3.
        let mut lit = 0;
        for step in 0..100u64 {
            let l = paint(&e, step * 166_667 + 1);
            lit += l.px.iter().filter(|p| (p.b - base.b).abs() > 1e-6).count();
        }
        let share = lit as f64 / 6400.0;
        assert!((0.25..0.35).contains(&share), "{share}");
        // Density 0 is the base color everywhere, opaque.
        assert!(paint(&mk(0.0, false, 1), 0).px.iter().all(|p| p.a == 1.0 && (p.r - base.r).abs() < 1e-6));
        // Smooth: halfway between two steps is the average of both patterns.
        let s = mk(0.3, true, 1);
        let (a, b, mid) = (paint(&e, 1_000_000), paint(&e, 1_166_667), paint(&s, 1_083_334));
        for i in 0..64 {
            assert!((mid.px[i].r - (a.px[i].r + b.px[i].r) / 2.0).abs() < 0.01, "cell {i}");
        }
    }

    #[test]
    fn ripple_rings_spread_from_pad_hits_and_fade() {
        let look = RippleLook { color: Rgb::new(1.0, 1.0, 1.0), speed: 4.0, ring_width: 1.0, lifetime_us: 1_000_000, fade: RippleFade::Linear, max_concurrent: 2, shape: RippleShape::Circle };
        let e = CompiledEffect::Ripple { look, interval: None };
        let at = |input: &PadInput| {
            let mut l = LayerFrame::default();
            e.paint_input(0, None, Some(input), 1.0, &mut l);
            l
        };
        let hit = |x, y, t_us| PadHit::press(x, y, 100, t_us);
        // No input: nothing (and `paint` alone never draws a ripple).
        let empty = PadInput::default();
        assert!(at(&empty).px.iter().all(|p| p.a == 0.0));
        let mut l = LayerFrame::default();
        e.paint(0, None, 1.0, &mut l);
        assert!(l.px.iter().all(|p| p.a == 0.0));
        // Pad (0, 0) at t = 0; at 500 ms the ring is 2 cells out, faded to 0.5.
        let mut input = PadInput::default();
        input.push(hit(0, 0, 0));
        input.advance(500_000);
        let l = at(&input);
        assert!((l.px[2].a - 0.5).abs() < 1e-6, "(2, 0) on the ring: {}", l.px[2].a); // x = 2, y = 0
        assert!((l.px[2 * 8].a - 0.5).abs() < 1e-6, "(0, 2) on the ring");
        assert_eq!(l.px[0].a, 0.0, "the centre is past");
        assert_eq!(l.px[5].a, 0.0, "(5, 0) not reached yet");
        // After the lifetime: gone.
        input.advance(1_000_000);
        assert!(at(&input).px.iter().all(|p| p.a == 0.0));
        // max_concurrent 2: of three live hits only the two newest draw.
        let mut input = PadInput::default();
        input.push(hit(0, 0, 0));
        input.push(hit(7, 7, 0));
        input.push(hit(7, 0, 0));
        input.advance(1);
        let l = at(&input);
        assert!(l.px[63].a > 0.9 && l.px[7].a > 0.9, "newest two");
        assert_eq!(l.px[0].a, 0.0, "the oldest is dropped");
        // Capacity: the buffer keeps the 16 newest; advance drops hits older than MAX_AGE_US.
        let mut input = PadInput::default();
        for k in 0..20 {
            input.push(hit(0, 0, k));
        }
        assert_eq!((input.hits.len(), input.hits[0].t_us), (16, 4));
        input.advance(PadInput::MAX_AGE_US + 100);
        assert!(input.hits.is_empty());
    }

    fn alpha_grid(e: &CompiledEffect, local_us: u64, beat: Option<BeatState>) -> [f32; 64] {
        let mut l = LayerFrame::default();
        e.paint(local_us, beat, 1.0, &mut l);
        let mut a = [0.0; 64];
        for (o, p) in a.iter_mut().zip(l.px.iter()) {
            *o = p.a;
        }
        a
    }

    #[test]
    fn rain_falls_with_a_tail_and_is_a_pure_function_of_time() {
        let green = Rgb::new(0.0, 0.75, 0.25);
        let rain = |dir, flicker| CompiledEffect::Rain { color: green, head: Rgb::new(0.8, 1.0, 0.85), direction: dir, speed: 8.0, density: 1.0, tail: 2.0, seed: 3, flicker };
        let down = rain(RainDirection::Down, 0.0);
        // Same time, same picture, in any order (seek safe).
        assert_eq!(alpha_grid(&down, 3_210_000, None), alpha_grid(&down, 3_210_000, None));
        let later = alpha_grid(&down, 3_330_000, None);
        let _ = alpha_grid(&down, 9_000_000, None);
        assert_eq!(alpha_grid(&down, 3_330_000, None), later);
        // Density 1: something falls in every lane over a few seconds.
        let mut lit_lanes = [false; 8];
        for k in 0..60u64 {
            let a = alpha_grid(&down, k * 50_000 + 2_000_000, None);
            for (i, v) in a.iter().enumerate() {
                if *v > 0.0 {
                    lit_lanes[i % 8] = true;
                }
            }
        }
        assert!(lit_lanes.iter().all(|l| *l), "{lit_lanes:?}");
        // Falling down: 1/8 s later every lit pattern has moved one row lower
        // (8 cells/s), except where drops enter, leave or start.
        let a = alpha_grid(&down, 4_040_000, None);
        let b = alpha_grid(&down, 4_165_000, None);
        let same = (0..7).flat_map(|y| (0..8).map(move |x| (x, y))).filter(|&(x, y)| a[(y + 1) * 8 + x] > 0.0 && (b[y * 8 + x] - a[(y + 1) * 8 + x]).abs() < 1e-6).count();
        let lit = (8..64).filter(|&i| a[i] > 0.0).count();
        assert!(same * 2 >= lit, "{same} of {lit} cells moved down by one");
        // Up is the vertical mirror of down for the same seed and time.
        let up = alpha_grid(&rain(RainDirection::Up, 0.0), 4_040_000, None);
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(up[y * 8 + x], a[(7 - y) * 8 + x]);
            }
        }
        // Flicker only dims (never brightens) tail cells.
        let f = alpha_grid(&rain(RainDirection::Down, 1.0), 4_040_000, None);
        assert!(f.iter().zip(a.iter()).all(|(f, a)| f <= a));
    }

    #[test]
    fn wave_follows_the_sine_curve_and_the_beat() {
        let w = |beat_amplitude| CompiledEffect::Wave { color: Rgb::new(1.0, 1.0, 1.0), axis: Axis::X, wavelength: 8.0, amplitude: 2.5, center: 3.5, speed: 1.0, thickness: 1.0, beat_amplitude, phase: 0.0 };
        // t = 0: curve(x) = 3.5 + 2.5 sin(2π x / 8): x = 2 → 6.0, x = 6 → 1.0, x = 0 → 3.5.
        let a = alpha_grid(&w(false), 0, None);
        assert_eq!(a[6 * 8 + 2], 1.0);
        assert_eq!(a[8 + 6], 1.0);
        assert_eq!(a[2], 0.0, "(2, 0) is far from the curve");
        assert!((a[3 * 8] - 1.0).abs() < 1e-6 && (a[4 * 8] - 1.0).abs() < 1e-6, "3.5 is half way between rows 3 and 4");
        // A quarter cycle later the curve has moved 2 cells along x.
        let b = alpha_grid(&w(false), 250_000, None);
        assert_eq!(b[6 * 8 + 4], 1.0);
        // Beat amplitude: half way between beats the amplitude is 0.3 × 2.5 = 0.75.
        let c = alpha_grid(&w(true), 0, Some(BeatState::new(0.5, 120.0)));
        assert_eq!(c[6 * 8 + 2], 0.0, "no longer reaches row 6");
        assert_eq!(c[4 * 8 + 2], 1.0, "3.5 + 0.75 = 4.25");
        // Unknown beat: fixed amplitude.
        assert_eq!(alpha_grid(&w(true), 0, None), a);
    }

    #[test]
    fn orb_is_a_shaded_disc_that_orbits_or_bounces() {
        let orb = |motion, dither| CompiledEffect::Orb {
            color: Rgb::new(1.0, 0.7, 0.25),
            center: (3.5, 3.5),
            radius: 3.0,
            edge: 0.0,
            light: (0.0, 1.0),
            shading: 0.8,
            dither,
            dither_rate_hz: 4.0,
            seed: 1,
            motion,
            speed: 0.5,
            orbit_radius: 2.0,
            bounce_dir: (1.0, 0.0),
            orbit_phase: 0.0,
        };
        let a = alpha_grid(&orb(OrbMotion::None, 0.0), 0, None);
        assert_eq!(a[0], 0.0, "corner outside the disc");
        assert!(a[3 * 8 + 3] > 0.0);
        // Light from above: the top of the disc is brighter than the bottom.
        assert!(a[6 * 8 + 3] > a[8 + 3], "{} vs {}", a[6 * 8 + 3], a[8 + 3]);
        // Orbit: half a turn (1 s at 0.5 turn/s) moves the centre by 2 × orbit_radius.
        let o0 = alpha_grid(&orb(OrbMotion::Orbit, 0.0), 0, None);
        let o1 = alpha_grid(&orb(OrbMotion::Orbit, 0.0), 1_000_000, None);
        assert_ne!(o0, o1);
        assert_eq!(alpha_grid(&orb(OrbMotion::Orbit, 0.0), 2_000_000, None), o0, "a full turn comes back");
        // Bounce: reflects at the edges; period 14 cells / 0.5 cells/s = 28 s.
        assert_eq!(bounce(9.0), 5.0);
        assert_eq!(bounce(-1.0), 1.0);
        assert_eq!(alpha_grid(&orb(OrbMotion::Bounce, 0.0), 28_000_000, None), alpha_grid(&orb(OrbMotion::Bounce, 0.0), 0, None));
        // Dither only dims, and is the same within a step.
        let d = alpha_grid(&orb(OrbMotion::None, 1.0), 100_000, None);
        assert!(d.iter().zip(a.iter()).all(|(d, a)| d <= a));
        assert_eq!(d, alpha_grid(&orb(OrbMotion::None, 1.0), 200_000, None));
    }

    #[test]
    fn d1_ripple_shapes_and_interval_wave_phase_orb_orbit_phase() {
        let look = |shape| RippleLook { color: Rgb::new(1.0, 1.0, 1.0), speed: 4.0, ring_width: 1.0, lifetime_us: 1_000_000, fade: RippleFade::Linear, max_concurrent: 4, shape };
        // Interval: a ring every 500 ms at (0, 0), no input; radius 2 at 500 ms after each start.
        let iv = CompiledEffect::Ripple { look: look(RippleShape::Circle), interval: Some((0.0, 0.0, 500_000)) };
        let a = alpha_grid(&iv, 1_000_000 + 500_000, None);
        assert!(a[2] > 0.0 && a[2 * 8] > 0.0, "ring of radius 2 from the start at 1.0 s");
        assert_eq!(alpha_grid(&iv, 1_500_000, None), alpha_grid(&iv, 1_500_000, None), "a function of the time");
        assert_eq!(alpha_grid(&iv, 3_500_000, None), a, "periodic: the same rings every interval");
        // Shapes at radius 2 from (3, 3): diamond reaches (5, 3) and (4, 4); square reaches (5, 5).
        let at_r2 = |shape| {
            let mut input = PadInput::default();
            input.push(PadHit::press(3, 3, 100, 0));
            input.advance(500_000);
            let mut l = LayerFrame::default();
            CompiledEffect::Ripple { look: look(shape), interval: None }.paint_input(0, None, Some(&input), 1.0, &mut l);
            l.px.map(|p| p.a)
        };
        let (c, d, s) = (at_r2(RippleShape::Circle), at_r2(RippleShape::Diamond), at_r2(RippleShape::Square));
        assert!(d[4 * 8 + 4] > 0.0 && d[5 * 8 + 5] == 0.0, "diamond: |dx| + |dy| = 2");
        assert!(s[5 * 8 + 5] > 0.0 && s[5 * 8 + 3] > 0.0, "square: max(|dx|, |dy|) = 2");
        // On the ring the alpha is the fade (half the lifetime: 0.5).
        assert!((c[5 * 8 + 3] - 0.5).abs() < 1e-6 && c[5 * 8 + 5] < s[5 * 8 + 5] && c[6 * 8 + 6] == 0.0, "circle: Euclidean");
        // Wave phase 0.25 = a quarter cycle: the same picture as 0.25 s later at 1 cycle/s, reversed in time.
        let wave = |phase| CompiledEffect::Wave { color: Rgb::new(1.0, 1.0, 1.0), axis: Axis::X, wavelength: 8.0, amplitude: 2.5, center: 3.5, speed: 1.0, thickness: 1.0, beat_amplitude: false, phase };
        assert_eq!(alpha_grid(&wave(0.25), 250_000, None), alpha_grid(&wave(0.0), 0, None));
        // Orb orbit_phase 0.5 starts on the opposite side: the same as half a turn later.
        let orb = |orbit_phase| CompiledEffect::Orb {
            color: Rgb::new(1.0, 1.0, 1.0),
            center: (3.5, 3.5),
            radius: 2.0,
            edge: 0.0,
            light: (0.0, 1.0),
            shading: 0.0,
            dither: 0.0,
            dither_rate_hz: 4.0,
            seed: 1,
            motion: OrbMotion::Orbit,
            speed: 0.5,
            orbit_radius: 2.0,
            bounce_dir: (1.0, 0.0),
            orbit_phase,
        };
        assert_eq!(alpha_grid(&orb(0.5), 0, None), alpha_grid(&orb(0.0), 1_000_000, None));
        // Absent optional fields: compiled as before and not written back.
        for t in ["builtin.ripple", "builtin.wave", "builtin.orb"] {
            let e = crate::edit::default_effect(t).unwrap();
            let json = serde_json::to_value(&e).unwrap();
            for k in ["shape", "origin_x", "origin_y", "interval_ms", "phase", "orbit_phase"] {
                assert!(json["params"].get(k).is_none(), "{t} writes {k}");
            }
        }
    }

    #[test]
    fn regions_groups_cover_the_board_as_specified() {
        use RegionPattern::*;
        for p in [HalvesLr, HalvesTb, Checker, Quadrants, Columns, Rows, Perimeter, Fan] {
            let n = region_count(p);
            let mut seen = vec![0; n];
            for y in 0..8 {
                for x in 0..8 {
                    if let Some(g) = region_of(p, x, y) {
                        seen[g] += 1;
                    }
                }
            }
            assert!(seen.iter().all(|c| *c > 0), "{p:?}: every group has cells {seen:?}");
            let total: usize = seen.iter().sum();
            assert_eq!(total, if p == Perimeter { 28 } else { 64 }, "{p:?}");
        }
        // Perimeter: clockwise from the bottom-left.
        assert_eq!([(0, 0), (0, 7), (1, 7), (7, 7), (7, 6), (7, 0), (6, 0), (1, 0)].map(|(x, y)| region_of(Perimeter, x, y)), [0, 7, 8, 14, 15, 21, 22, 27].map(Some));
        assert_eq!(region_of(Perimeter, 3, 3), None);
        // Quadrants forward: bottom-left, bottom-right, top-right, top-left; fan from the middle out.
        assert_eq!([(1, 1), (6, 1), (6, 6), (1, 6)].map(|(x, y)| region_of(Quadrants, x, y)), [0, 1, 2, 3].map(Some));
        assert_eq!([3, 4, 2, 5, 0, 7].map(|x| region_of(Fan, x, 0)), [0, 0, 1, 1, 3, 3].map(Some));
        // Orders.
        assert_eq!((0..8).map(|k| step_group(k, 4, StepOrder::Forward)).collect::<Vec<_>>(), [0, 1, 2, 3, 0, 1, 2, 3]);
        assert_eq!((0..4).map(|k| step_group(k, 4, StepOrder::Reverse)).collect::<Vec<_>>(), [3, 2, 1, 0]);
        assert_eq!((0..8).map(|k| step_group(k, 4, StepOrder::Pingpong)).collect::<Vec<_>>(), [0, 1, 2, 3, 2, 1, 0, 1]);
    }

    #[test]
    fn regions_step_by_beat_or_time_with_duty_soft_and_color_b() {
        let white = Rgb::new(1.0, 1.0, 1.0);
        let mk = |mode, soft, duty, color_b| CompiledEffect::Regions { pattern: RegionPattern::HalvesLr, color: white, color_b, mode, step_beats: 1.0, step_s: 0.5, order: StepOrder::Forward, soft, duty, custom: None };
        let hard = mk(StepMode::Beat, 0.0, 1.0, None);
        // Beat 0.x: left half; beat 1.x: right half; unknown beat: nothing.
        let a = alpha_grid(&hard, 0, Some(BeatState::new(0.3, 120.0)));
        assert!(a[0] == 1.0 && a[7] == 0.0);
        let b = alpha_grid(&hard, 0, Some(BeatState::new(1.3, 120.0)));
        assert!(b[0] == 0.0 && b[7] == 1.0);
        assert!(alpha_grid(&hard, 0, None).iter().all(|v| *v == 0.0));
        // Seconds: 0.5 s per step from the clip's local time.
        let secs = mk(StepMode::Seconds, 0.0, 1.0, None);
        assert_eq!(alpha_grid(&secs, 600_000, None)[7], 1.0);
        // Duty 0.5: dark in the second half of each step.
        let half = mk(StepMode::Beat, 0.0, 0.5, None);
        assert_eq!(alpha_grid(&half, 0, Some(BeatState::new(0.7, 120.0)))[0], 0.0);
        // Soft 1, duty 1: a travelling peak; at the step boundary both halves are half lit.
        let soft = mk(StepMode::Beat, 1.0, 1.0, None);
        let m = alpha_grid(&soft, 0, Some(BeatState::new(1.0, 120.0)));
        assert!((m[0] - 0.5).abs() < 1e-6 && (m[7] - 0.5).abs() < 1e-6, "{} {}", m[0], m[7]);
        let peak = alpha_grid(&soft, 0, Some(BeatState::new(0.5, 120.0)));
        assert!((peak[0] - 1.0).abs() < 1e-6 && peak[7] == 0.0);
        // color_b: the unlit half is opaque color_b.
        let mut l = LayerFrame::default();
        mk(StepMode::Beat, 0.0, 1.0, Some(Rgb::new(0.0, 0.0, 1.0))).paint(0, Some(BeatState::new(0.3, 120.0)), 1.0, &mut l);
        assert_eq!((l.px[7].a, l.px[7].b, l.px[7].r), (1.0, 1.0, 0.0));
        assert_eq!((l.px[0].a, l.px[0].r), (1.0, 1.0));
    }

    fn pad_layer(e: &CompiledEffect, x: u8, y: u8, age_us: i64) -> LayerFrame {
        let mut input = PadInput::default();
        input.push(PadHit::press(x, y, 100, 0));
        input.advance(age_us);
        let mut l = LayerFrame::default();
        e.paint_input(0, None, Some(&input), 1.0, &mut l);
        l
    }

    #[test]
    fn tap_glow_cools_through_three_colors_and_repeats() {
        let (s, m, e) = (Rgb::new(1.0, 1.0, 1.0), Rgb::new(1.0, 0.5, 0.0), Rgb::new(0.2, 0.0, 0.0));
        let glow = |radius, repeats| CompiledEffect::TapGlow { start: s, mid: m, end: e, lifetime_us: 1_000_000, radius, repeats, repeat_us: 300_000, decay: 0.5 };
        // At the pad: start color at 0, mid at half the lifetime.
        let l = pad_layer(&glow(1.5, 1), 3, 3, 0);
        assert!((l.px[3 * 8 + 3].g - 1.0).abs() < 1e-6);
        let l = pad_layer(&glow(1.5, 1), 3, 3, 500_000);
        assert!((l.px[3 * 8 + 3].g - 0.5).abs() < 1e-6 && l.px[3 * 8 + 3].a == 1.0);
        // Radius 1.5: the neighbour is weaker, two cells away is dark.
        assert!(l.px[3 * 8 + 4].a > 0.0 && l.px[3 * 8 + 4].a < 1.0 && l.px[3 * 8 + 5].a == 0.0);
        // Radius 0: the pad only.
        assert_eq!(pad_layer(&glow(0.0, 1), 3, 3, 0).px.iter().filter(|p| p.a > 0.0).count(), 1);
        // After the lifetime nothing, unless a repeat (0.3 s later, half as strong) is still glowing.
        assert!(pad_layer(&glow(1.5, 1), 3, 3, 1_100_000).px.iter().all(|p| p.a == 0.0));
        let r = pad_layer(&glow(1.5, 2), 3, 3, 1_100_000);
        assert!((r.px[3 * 8 + 3].a - 0.5).abs() < 1e-6);
        // Nothing without input.
        let mut l = LayerFrame::default();
        glow(1.5, 1).paint(0, None, 1.0, &mut l);
        assert!(l.px.iter().all(|p| p.a == 0.0));
    }

    #[test]
    fn cross_heads_run_along_the_row_and_column_with_tails() {
        let c = CompiledEffect::Cross { color: Rgb::new(1.0, 1.0, 1.0), speed: 10.0, length: 2.0, lifetime_us: 1_000_000 };
        // 0.3 s: heads 3 cells out from (2, 2), fade 0.7.
        let l = pad_layer(&c, 2, 2, 300_000);
        let a = |x: usize, y: usize| l.px[y * 8 + x].a;
        for (x, y) in [(5, 2), (2, 5)] {
            assert!((a(x, y) - 0.7).abs() < 1e-6, "head at ({x}, {y}): {}", a(x, y));
        }
        assert!((a(4, 2) - 0.35).abs() < 1e-6, "tail one cell behind");
        assert_eq!(a(2, 2), 0.0, "the pad is past the tail");
        assert_eq!(a(3, 3), 0.0, "only the row and the column");
        assert_eq!(a(6, 2), 0.0, "ahead of the head");
        assert!(pad_layer(&c, 2, 2, 1_000_000).px.iter().all(|p| p.a == 0.0), "after the lifetime");
    }

    /// A show with one triggered layer: clip 1..5 s of `preset`, firing `on`.
    fn triggered_show(effect: Effect, trigger: crate::model::TriggerSpec) -> Show {
        let mut s = demo();
        s.presets.push(crate::model::Preset { id: "p".into(), name: "p".into(), effect });
        let clip = crate::model::Clip { id: "w".into(), preset_id: "p".into(), start_us: 1_000_000, duration_us: 4_000_000, ..Default::default() };
        s.layers = vec![crate::model::Layer {
            id: "t".into(),
            name: "t".into(),
            target_id: "main-pad".into(),
            enabled: true,
            opacity: 1.0,
            blend: Blend::Over,
            content: Content::Triggered { clips: vec![clip], trigger },
            mask: None,
        }];
        s
    }

    fn on(on: TriggerOn) -> crate::model::TriggerSpec {
        crate::model::TriggerSpec { on, interval_ms: (on == TriggerOn::Interval).then_some(500), origin_x: None, origin_y: None }
    }

    fn lit(f: &Frame) -> usize {
        f.leds.iter().filter(|c| c.r + c.g + c.b > 0.0).count()
    }

    #[test]
    fn triggered_press_counts_only_inside_its_clip_and_ages_in_real_time() {
        let cross = crate::edit::default_effect("builtin.cross").unwrap(); // 12 cells/s, 800 ms
        let mut r = Renderer::new(&triggered_show(cross, on(TriggerOn::Press))).unwrap();
        let mut input = PadInput::default();
        // Before the clip (show 0.5 s): ignored. Inside (show 2 s): fires.
        input.apply(PadEvent::Press { x: 0, y: 0, velocity: 100, t_us: 0 }, Some(500_000));
        input.advance(100_000);
        assert_eq!(lit(&r.render_input(2_000_000, None, Some(&input))), 0, "pressed outside the clip");
        input.apply(PadEvent::Press { x: 3, y: 3, velocity: 100, t_us: 200_000 }, Some(2_000_000));
        input.advance(300_000);
        assert!(lit(&r.render_input(2_100_000, None, Some(&input))) > 0);
        // A seek back inside the clip does not replay or clear it: age follows real time.
        assert_eq!(r.render_input(1_200_000, None, Some(&input)), r.render_input(4_000_000, None, Some(&input)));
        // Outside the clip nothing is drawn.
        assert_eq!(lit(&r.render_input(6_000_000, None, Some(&input))), 0);
        // Without input the layer is empty.
        assert_eq!(lit(&r.render(2_100_000, None)), 0);
    }

    fn particles(f: impl FnOnce(&mut crate::model::ParticlesParams)) -> Effect {
        let mut e = crate::edit::default_effect("builtin.particles").unwrap();
        let Effect::Particles { params, .. } = &mut e else { unreachable!() };
        f(params);
        e
    }

    fn lit_cells(a: &[f32; 64]) -> Vec<usize> {
        (0..64).filter(|i| a[*i] > 0.0).collect()
    }

    #[test]
    fn burst_spreads_falls_and_is_a_function_of_time() {
        let e = CompiledEffect::compile(&particles(|_| {})).unwrap();
        // At its start every particle is at the emitter (3.5, 3.5): the four middle cells.
        assert_eq!(lit_cells(&alpha_grid(&e, 0, None)), vec![27, 28, 35, 36]);
        let mean_y = |t| {
            let a = alpha_grid(&e, t, None);
            let w: f32 = a.iter().sum();
            (0..64).map(|i| a[i] * (i / 8) as f32).sum::<f32>() / w
        };
        assert!(lit_cells(&alpha_grid(&e, 300_000, None)).len() > 4, "spread out");
        assert!(mean_y(900_000) < mean_y(100_000), "gravity pulls down");
        assert_eq!(lit_cells(&alpha_grid(&e, 1_300_000, None)), Vec::<usize>::new(), "gone after the lifetime");
        // Seek: drawing 0.7 s directly equals drawing it after other times.
        let direct = alpha_grid(&e, 700_000, None);
        let _ = alpha_grid(&e, 200_000, None);
        assert_eq!(alpha_grid(&e, 700_000, None), direct);
    }

    #[test]
    fn edges_vanish_or_bounce() {
        let one = |edge| {
            CompiledEffect::compile(&particles(|p| {
                p.count = Some(1.into());
                p.speed_min = 8.0.into();
                p.speed_max = 8.0.into();
                p.angle = 0.0.into();
                p.spread = 0.0.into();
                p.gravity = 0.0.into();
                p.drag = 0.0.into();
                p.lifetime_ms = 3000.into();
                p.tail = 0.0.into();
                p.size = 0.5.into();
                p.origin_x = Some(0.0.into());
                p.origin_y = Some(3.0.into());
                p.edge_mode = edge;
            }))
            .unwrap()
        };
        let (vanish, bounce) = (one(crate::model::EdgeMode::Vanish.into()), one(crate::model::EdgeMode::Bounce.into()));
        assert_eq!(lit_cells(&alpha_grid(&vanish, 500_000, None)), vec![3 * 8 + 4]);
        assert_eq!(lit_cells(&alpha_grid(&vanish, 1_500_000, None)), Vec::<usize>::new(), "x = 12: gone");
        assert_eq!(lit_cells(&alpha_grid(&bounce, 1_500_000, None)), vec![3 * 8 + 3], "x = 12 folds back to 3");
    }

    #[test]
    fn implode_gathers_waits_and_bursts() {
        let e = CompiledEffect::compile(&particles(|p| p.mode = crate::model::ParticleMode::Implode.into())).unwrap();
        // Start on a circle of radius 4: the middle is dark. Gathered (0.6 s) and
        // waiting (0.2 s): the middle only. Then the burst spreads again.
        let start = alpha_grid(&e, 0, None);
        assert_eq!((start[27], start[36]), (0.0, 0.0));
        assert_eq!(lit_cells(&alpha_grid(&e, 700_000, None)), vec![27, 28, 35, 36]);
        assert!(lit_cells(&alpha_grid(&e, 1_100_000, None)).len() > 4);
    }

    #[test]
    fn triggered_particles_follow_pads() {
        let lit_near = |f: &Frame, x: usize, y: usize| (y.saturating_sub(1)..=(y + 1).min(7)).any(|cy| (x.saturating_sub(1)..=(x + 1).min(7)).any(|cx| f.leds[cy * 8 + cx].r > 0.0));
        // Press burst at the pad.
        let mut r = Renderer::new(&triggered_show(particles(|_| {}), on(TriggerOn::Press))).unwrap();
        let mut input = PadInput::default();
        input.apply(PadEvent::Press { x: 1, y: 1, velocity: 100, t_us: 0 }, Some(2_000_000));
        input.advance(10_000);
        let f = r.render_input(2_010_000, None, Some(&input));
        assert!(lit_near(&f, 1, 1) && !lit_near(&f, 6, 6));
        // Rocket: rises from the bottom of the pressed column, bursts at row 5.
        let mut r = Renderer::new(&triggered_show(particles(|p| p.mode = crate::model::ParticleMode::Rocket.into()), on(TriggerOn::Press))).unwrap();
        let mut input = PadInput::default();
        input.apply(PadEvent::Press { x: 5, y: 6, velocity: 100, t_us: 0 }, Some(2_000_000));
        input.advance(100_000); // 10 cells/s: at row 1
        let f = r.render_input(2_100_000, None, Some(&input));
        assert!(lit_near(&f, 5, 1) && !lit_near(&f, 5, 6), "rising in column 5");
        input.advance(520_000); // burst at 0.5 s from (5, 5)
        assert!(lit_near(&r.render_input(2_520_000, None, Some(&input)), 5, 5));
        // Hold fountain: emits while held, the last ones fly one lifetime after release.
        let mut r = Renderer::new(&triggered_show(particles(|p| p.mode = crate::model::ParticleMode::Fountain.into()), on(TriggerOn::Hold))).unwrap();
        let mut input = PadInput::default();
        input.apply(PadEvent::Press { x: 3, y: 3, velocity: 100, t_us: 0 }, Some(2_000_000));
        input.apply(PadEvent::Release { x: 3, y: 3, t_us: 1_000_000 }, Some(3_000_000));
        for (now, any) in [(500_000, true), (1_500_000, true), (2_300_000, false)] {
            input.advance(now);
            assert_eq!(lit(&r.render_input(2_000_000 + now, None, Some(&input))) > 0, any, "at {now}");
        }
    }

    #[test]
    fn continuous_bursts_repeat_and_fountains_keep_going() {
        let mut s = demo();
        s.presets.push(crate::model::Preset { id: "p".into(), name: "p".into(), effect: particles(|_| {}) });
        s.layers.truncate(1);
        s.layers[0].content = Content::Continuous { preset_id: "p".into() };
        let mut r = Renderer::new(&s).unwrap();
        // A new burst (new random directions) every lifetime (1.2 s), each from the emitter.
        let middle = |f: &Frame| (0..64).filter(|i| f.leds[*i].r > 0.0).collect::<Vec<_>>();
        assert_eq!(middle(&r.render(1_200_000 * 7, None)), vec![27, 28, 35, 36]);
        assert_ne!(r.render(300_000, None), r.render(300_000 + 1_200_000 * 7, None));
        let Effect::Particles { params, .. } = &mut s.presets.last_mut().unwrap().effect else { unreachable!() };
        params.mode = crate::model::ParticleMode::Fountain.into();
        params.rate = Some(64.0.into());
        params.lifetime_ms = 6000.into();
        let mut r = Renderer::new(&s).unwrap();
        assert!(lit(&r.render(60_000_000, None)) > 0, "still emitting after a minute");
    }

    #[test]
    fn particles_are_capped_over_the_whole_show() {
        // Three capped fountains: 256 + 256 + 256 = 768, a fourth draws nothing.
        let e = CompiledEffect::compile(&particles(|p| {
            p.mode = crate::model::ParticleMode::Fountain.into();
            p.rate = Some(64.0.into());
            p.lifetime_ms = 6000.into();
        }))
        .unwrap();
        let ev = Event { x: 3.5, y: 3.5, age_s: 10.0, emit_s: f64::INFINITY, id: 0, start_us: 0 };
        let mut left = MAX_SHOW_PARTICLES;
        for k in 0..4 {
            let mut l = LayerFrame::default();
            paint_particles(std::slice::from_ref(&e), &[None], 0, &[ev], 1.0, &mut l, &mut left);
            assert_eq!(left, MAX_SHOW_PARTICLES.saturating_sub(256 * (k + 1)));
            assert_eq!(l.px.iter().any(|p| p.a > 0.0), k < 3, "layer {k}");
        }
    }

    #[test]
    fn particles_per_layer_are_capped() {
        // 64 per second for 6 s = 384 alive: only 256 are drawn.
        let CompiledEffect::Particles(look) = CompiledEffect::compile(&particles(|p| {
            p.mode = crate::model::ParticleMode::Fountain.into();
            p.rate = Some(64.0.into());
            p.lifetime_ms = 6000.into();
        }))
        .unwrap() else { unreachable!() };
        let ev = Event { x: 3.5, y: 3.5, age_s: 10.0, emit_s: f64::INFINITY, id: 0, start_us: 0 };
        let mut budget = MAX_PARTICLES;
        paint_event(&ev, &|_| Some(look), 1.0, &mut LayerFrame::default(), &mut budget);
        assert_eq!(budget, 0);
        let mut budget = MAX_PARTICLES;
        paint_event(&Event { age_s: 1.0, ..ev }, &|_| Some(look), 1.0, &mut LayerFrame::default(), &mut budget);
        assert_eq!(MAX_PARTICLES - budget, 65, "0..=64 born in the first second");
    }
    /// demo + a 2 s compound "cp" (solid base-blue, then a 0..1 s chase) placed
    /// by one clip at 5 s for 6 s on the only layer.
    fn compound_show(f: impl FnOnce(&mut crate::model::Clip)) -> Show {
        use crate::model::{Clip, Compound, Layer};
        let mut s = demo();
        let inner = |id: &str, content| Layer { id: id.into(), name: id.into(), target_id: String::new(), enabled: true, opacity: 1.0, blend: Blend::Over, content, mask: None };
        s.compounds.push(Compound {
            id: "cp".into(),
            name: "cp".into(),
            duration_us: Some(2_000_000),
            layers: vec![
                inner("in-base", Content::Continuous { preset_id: "base-blue".into() }),
                inner("in-sweep", Content::Timeline { clips: vec![Clip { id: "ic".into(), preset_id: "sweep-cyan".into(), duration_us: 1_000_000, ..Default::default() }] }),
            ],
        });
        s.layers.truncate(1);
        let mut clip = Clip { id: "place".into(), compound_id: Some("cp".into()), start_us: 5_000_000, duration_us: 6_000_000, ..Default::default() };
        f(&mut clip);
        if let Content::Timeline { clips } = &mut s.layers[0].content {
            *clips = vec![clip];
        }
        s
    }

    #[test]
    fn compound_clips_play_inner_time_with_scale_and_loop() {
        let plain = Renderer::new(&compound_show(|_| {})).unwrap().render(5_500_000, None); // inner 0.5 s
        let mut once = Renderer::new(&compound_show(|_| {})).unwrap();
        assert!(lit(&once.render(5_500_000, None)) > 0);
        assert_eq!(once.render(8_000_000, None), Frame::default(), "past its 2 s and no loop: nothing");
        let mut looped = Renderer::new(&compound_show(|c| c.looping = Some(true))).unwrap();
        assert_eq!(looped.render(7_500_000, None), plain, "inner 2.5 s wraps to 0.5 s");
        let mut fast = Renderer::new(&compound_show(|c| c.time_scale = Some(2.0))).unwrap();
        assert_eq!(fast.render(5_250_000, None), plain, "twice as fast: inner 0.5 s at local 0.25 s");
        // The inner chase (0..1 s) sits over the solid: at inner 1.5 s only the solid is left.
        let solid_only = once.render(6_500_000, None);
        assert!(solid_only.leds.iter().all(|c| *c == solid_only.leds[0]));
    }

    #[test]
    fn tint_replaces_colors_by_brightness() {
        let mut r = Renderer::new(&compound_show(|c| c.tint = Some("#00FF00".into()))).unwrap();
        let f = r.render(6_500_000, None); // the solid only
        assert_eq!((f.leds[0].r, f.leds[0].b), (0.0, 0.0));
        assert!(f.leds[0].g > 0.0);
        // Darker secondary colors stay darker; the main one becomes the tint.
        let regions = crate::edit::default_effect("builtin.regions").unwrap();
        let Effect::Regions { params, .. } = tinted(&regions, Rgb::new(1.0, 0.0, 0.0)) else { unreachable!() };
        assert_eq!(params.color.value(), "#FF0000");
        let Effect::Noise { params, .. } = tinted(&crate::edit::default_effect("builtin.noise").unwrap(), Rgb::new(0.0, 0.0, 1.0)) else { unreachable!() };
        let (base, high) = (Rgb::from_hex(params.base_color.value()).unwrap(), Rgb::from_hex(params.highlight_color.value()).unwrap());
        assert!(base.b < high.b && base.r == 0.0, "{base:?} {high:?}");
    }

    #[test]
    fn inner_layers_blend_inside_the_compound_then_onto_the_layer() {
        // Inner add of the chase over the solid inside; the compound then goes over the black frame.
        let mut s = compound_show(|_| {});
        s.compounds[0].layers[1].blend = Blend::Add;
        s.compounds[0].layers[1].opacity = 0.5;
        let mut r = Renderer::new(&s).unwrap();
        let f = r.render(5_100_000, None);
        // Add brightens some cells above the plain solid.
        let solid = r.render(6_500_000, None).leds[0];
        assert!(f.leds.iter().any(|c| c.g > solid.g));
    }
    /// A frame (top row first) with `cells` = (row from the top, column, hex color) lit.
    fn frame_hex(cells: &[(usize, usize, &str)]) -> String {
        let mut v = vec!["000000".to_string(); 64];
        for &(r, c, hex) in cells {
            v[r * 8 + c] = hex.into();
        }
        v.concat()
    }

    fn frames_effect(f: impl FnOnce(&mut crate::model::FramesParams)) -> Effect {
        let mut e = crate::edit::default_effect("builtin.frames").unwrap();
        let Effect::Frames { params, .. } = &mut e else { unreachable!() };
        params.frames = vec![frame_hex(&[(0, 0, "FF0000")]), frame_hex(&[(7, 7, "0000FF")])];
        params.fps = 2.0;
        params.looping = false;
        f(params);
        e
    }

    #[test]
    fn frames_play_by_time_hold_and_loop() {
        let lit = |e: &Effect, t| lit_cells(&alpha_grid(&CompiledEffect::compile(e).unwrap(), t, None));
        let once = frames_effect(|p| p.hold_ms = Some(500));
        // The file's top-left cell is LED 56 (top row, bottom-left origin); its bottom-right is LED 7.
        assert_eq!(lit(&once, 0), vec![56]);
        assert_eq!(lit(&once, 600_000), vec![7]);
        assert_eq!(lit(&once, 1_200_000), vec![7], "held 0.5 s after the end (1 s)");
        assert_eq!(lit(&once, 1_600_000), Vec::<usize>::new());
        assert_eq!(lit(&frames_effect(|p| p.hold_last = true), 9_000_000), vec![7]);
        assert_eq!(lit(&frames_effect(|p| p.looping = true), 1_200_000), vec![56]);
        // Black is see-through unless asked; `color` repaints the lit cells.
        assert_eq!(lit(&frames_effect(|p| p.opacity_key = Some(false)), 0).len(), 64);
        let tinted = CompiledEffect::compile(&frames_effect(|p| p.color = Some("#00FF00".into()))).unwrap();
        let mut l = LayerFrame::default();
        tinted.paint(0, None, 1.0, &mut l);
        assert_eq!((l.px[56].r, l.px[56].g), (0.0, 1.0));
    }

    #[test]
    fn frames_follow_the_pad_with_anchor_pad() {
        // Cell (3, 3) from the bottom-left is row 4 from the top, column 3 in the file.
        let star = frames_effect(|p| {
            p.frames = vec![frame_hex(&[(4, 3, "FFFFFF"), (4, 4, "FFFFFF")])];
            p.anchor = crate::model::FrameAnchor::Pad;
            p.hold_last = true;
        });
        let mut r = Renderer::new(&triggered_show(star, on(TriggerOn::Press))).unwrap();
        let mut input = PadInput::default();
        input.apply(PadEvent::Press { x: 6, y: 6, velocity: 100, t_us: 0 }, Some(2_000_000));
        input.advance(10_000);
        let f = r.render_input(2_010_000, None, Some(&input));
        let on: Vec<usize> = (0..64).filter(|i| f.leds[*i].r > 0.0).collect();
        assert_eq!(on, vec![6 * 8 + 6, 6 * 8 + 7], "cells (3, 3) and (4, 3) moved by (3, 3) onto the pad");
    }

    fn text_effect(text: &str, font: crate::model::TextFont, mode: crate::model::TextMode, f: impl FnOnce(&mut crate::model::TextParams)) -> CompiledEffect {
        let mut e = crate::edit::default_effect("builtin.text").unwrap();
        let Effect::Text { params, .. } = &mut e else { unreachable!() };
        params.text = text.into();
        params.font = font;
        params.mode = mode;
        params.speed = 1.0;
        params.looping = false;
        f(params);
        CompiledEffect::compile(&e).unwrap()
    }

    #[test]
    fn text_static_scroll_and_sequence() {
        use crate::model::{TextFont::*, TextMode::*};
        let row = |e: &CompiledEffect, t, y: usize, beat| {
            let a = alpha_grid(e, t, beat);
            (0..8).filter(|x| a[y * 8 + x] > 0.0).collect::<Vec<_>>()
        };
        // 5 × 7 "A", centred from column 1: its top row .XXX. is columns 2..=4 of row 7.
        let a = text_effect("AB", F5x7, Static, |_| {});
        assert_eq!(row(&a, 0, 7, None), vec![2, 3, 4], "only what fits (one glyph)");
        // 3 × 5 "42" with a gap of 1 fits (7 cells) from column 0; top row at row 6.
        let digits = text_effect("42", F3x5, Static, |p| p.gap = 1);
        assert_eq!(row(&digits, 0, 6, None), vec![0, 2, 4, 5, 6]);
        // Scrolling left at 1 cell/s: empty at 0, the first column of A at x 7 after 1 s.
        let s = text_effect("A", F5x7, ScrollLeft, |_| {});
        assert_eq!(lit_cells(&alpha_grid(&s, 0, None)), Vec::<usize>::new());
        assert_eq!(lit_cells(&alpha_grid(&s, 1_000_000, None)), (1..=6).map(|y| y * 8 + 7).collect::<Vec<_>>());
        assert_eq!(lit_cells(&alpha_grid(&s, 20_000_000, None)), Vec::<usize>::new(), "gone without loop");
        // One character per second, or per beat counted from the clip start.
        let seq = text_effect("12", F3x5, Sequential, |_| {});
        assert_eq!(row(&seq, 0, 6, None), vec![3], "\"1\" (top row .X.) centred from column 2");
        assert_ne!(alpha_grid(&seq, 1_500_000, None), alpha_grid(&seq, 0, None));
        assert_eq!(lit_cells(&alpha_grid(&seq, 2_500_000, None)), Vec::<usize>::new());
        let by_beat = text_effect("123", F3x5, Sequential, |p| {
            p.beat_step = Some(true);
            p.looping = true;
        });
        // 120 bpm, 1.25 s into the clip at beat 10.5: the clip began on beat 8, two beats ago → "3".
        let three = text_effect("3", F3x5, Static, |_| {});
        assert_eq!(alpha_grid(&by_beat, 1_250_000, Some(BeatState::new(10.5, 120.0))), alpha_grid(&three, 0, None));
    }
    #[test]
    fn hold_draws_while_held_and_release_fires_on_release() {
        // A fixed-position effect on hold: a blinder drawn only while a pad is held.
        let solid = crate::edit::default_effect("builtin.solid").unwrap();
        let mut r = Renderer::new(&triggered_show(solid, on(TriggerOn::Hold))).unwrap();
        let mut input = PadInput::default();
        input.apply(PadEvent::Press { x: 1, y: 1, velocity: 100, t_us: 0 }, Some(2_000_000));
        input.advance(500_000);
        assert_eq!(lit(&r.render_input(2_500_000, None, Some(&input))), 64, "held");
        input.apply(PadEvent::Release { x: 1, y: 1, t_us: 600_000 }, Some(2_600_000));
        input.advance(700_000);
        assert_eq!(lit(&r.render_input(2_700_000, None, Some(&input))), 0, "released");
        // A hold is cut after 10 s; a lost input (release all) ends every hold.
        let mut input = PadInput::default();
        input.apply(PadEvent::Press { x: 1, y: 1, velocity: 100, t_us: 0 }, Some(2_000_000));
        input.advance(PadInput::MAX_HOLD_US + 1);
        assert_eq!(lit(&r.render_input(2_500_000, None, Some(&input))), 0, "10 s cap");
        let mut input = PadInput::default();
        input.apply(PadEvent::Press { x: 1, y: 1, velocity: 100, t_us: 0 }, Some(2_000_000));
        input.apply(PadEvent::Press { x: 2, y: 2, velocity: 100, t_us: 0 }, Some(2_000_000));
        input.apply(PadEvent::ReleaseAll { t_us: 100 }, None);
        input.advance(200);
        assert_eq!(lit(&r.render_input(2_500_000, None, Some(&input))), 0, "release all");
        // On release: a ripple appears at the release, not at the press.
        let ripple = crate::edit::default_effect("builtin.ripple").unwrap();
        let mut r = Renderer::new(&triggered_show(ripple, on(TriggerOn::Release))).unwrap();
        let mut input = PadInput::default();
        input.apply(PadEvent::Press { x: 4, y: 4, velocity: 100, t_us: 0 }, Some(2_000_000));
        input.advance(300_000);
        assert_eq!(lit(&r.render_input(2_300_000, None, Some(&input))), 0, "still held");
        input.apply(PadEvent::Release { x: 4, y: 4, t_us: 300_000 }, Some(2_300_000));
        input.advance(300_001);
        assert!(lit(&r.render_input(2_300_001, None, Some(&input))) > 0, "fired by the release");
    }

    #[test]
    fn clip_start_and_interval_fire_from_the_clip_time() {
        let ripple = crate::edit::default_effect("builtin.ripple").unwrap(); // 6 cells/s, 1.2 s
        let mut r = Renderer::new(&triggered_show(ripple.clone(), on(TriggerOn::ClipStart))).unwrap();
        // 0.5 s into the clip: a ring of radius 3 around the centre; the same on a replay (seek safe).
        let a = r.render_input(1_500_000, None, None);
        assert!(lit(&a) > 0);
        assert_eq!(r.render(1_500_000, None), a);
        assert_eq!(lit(&r.render(3_000_000, None)), 0, "only once, at the clip start");
        let mut r = Renderer::new(&triggered_show(ripple, on(TriggerOn::Interval))).unwrap();
        assert!(lit(&r.render(3_000_000, None)) > 0, "every 500 ms");
        // Once the first rings have lived out their 1.2 s, the picture repeats every 500 ms.
        assert_eq!(r.render(3_100_000, None), r.render(3_600_000, None), "periodic in the clip's local time");
        assert_ne!(r.render(1_100_000, None), r.render(1_600_000, None), "but not before (one ring, then two)");
    }

    #[test]
    fn keyframes_follow_clip_time_and_instance_age() {
        use crate::model::{Ease, Keyframe, Param};
        let k = |a: f64, b: f64| Param::Keyframes(vec![Keyframe { t_us: 0, v: a, ease: Ease::Linear }, Keyframe { t_us: 4_000_000, v: b, ease: Ease::Linear }]);
        // A solid whose color goes black → white over the clip's first 4 s (clip at 1 s).
        let mut s = demo();
        let solid = Effect::Solid {
            version: 1,
            params: crate::model::SolidParams {
                color: Param::Keyframes(vec![Keyframe { t_us: 0, v: "#000000".into(), ease: Ease::Linear }, Keyframe { t_us: 4_000_000, v: "#FFFFFF".into(), ease: Ease::Linear }]),
            },
        };
        s.presets.push(crate::model::Preset { id: "fade".into(), name: "fade".into(), effect: solid });
        s.layers = vec![s.layers[0].clone()];
        if let Content::Timeline { clips } = &mut s.layers[0].content {
            clips.truncate(1);
            clips[0].preset_id = "fade".into();
            clips[0].start_us = 1_000_000;
            clips[0].duration_us = 10_000_000;
            clips[0].fade_in_us = 0;
            clips[0].fade_out_us = 0;
        }
        s.render.master_brightness = 1.0;
        let mut r = Renderer::new(&s).unwrap();
        let g = |r: &mut Renderer, t| r.render(t, None).leds[0].g;
        assert!((g(&mut r, 3_000_000) - 0.5).abs() < 1e-6, "half way (local 2 s)");
        assert_eq!(g(&mut r, 9_000_000), 1.0, "held after the last point");
        // Seek safe: the same time gives the same frame in any order.
        let a = r.render(2_000_000, None);
        let _ = r.render(8_000_000, None);
        assert_eq!(r.render(2_000_000, None), a);
        // An instance effect samples at each instance's age: a cross whose speed
        // ramps 4 → 32 cells/s draws a younger and an older instance differently.
        let mut cross = crate::edit::default_effect("builtin.cross").unwrap();
        if let Effect::Cross { params, .. } = &mut cross {
            params.speed = k(4.0, 32.0);
            params.lifetime_ms = 3000.into();
        }
        assert!(has_keyframes(&cross) && !has_keyframes(&crate::edit::default_effect("builtin.cross").unwrap()));
        let ce = CompiledEffect::compile(&cross).unwrap();
        let keyed = [Some(cross)];
        let paint = |age: i64| {
            let mut l = LayerFrame::default();
            paint_instances_sampled(std::slice::from_ref(&ce), &keyed, 0, &[(0.0, 0.0, age)], 1.0, &mut l);
            (0..8).filter(|&x| l.px[x].a > 0.0).max()
        };
        // Age 0.5 s: speed 7.5 → head at 3.75 cells; age 1 s: speed 11 → head at 11 (tail off the board).
        assert_eq!(paint(500_000), Some(3));
        assert_eq!(paint(1_000_000), None);
    }

    fn compile_default(t: &str, f: impl FnOnce(&mut Effect)) -> CompiledEffect {
        let mut e = crate::edit::default_effect(t).unwrap();
        f(&mut e);
        CompiledEffect::compile(&e).unwrap()
    }

    #[test]
    fn regions_custom_groups_with_offsets() {
        let e = compile_default("builtin.regions", |e| {
            let Effect::Regions { params, .. } = e else { unreachable!() };
            params.pattern = RegionPattern::Custom.into();
            params.step_mode = StepMode::Seconds.into();
            params.step_ms = 1000.into();
            // Two groups sharing cell 9; the second is delayed half a step.
            params.groups = Some(vec![vec![0, 9], vec![9, 63]]);
            params.offset = Some(vec![0.0, 0.5]);
        });
        // 0.2 s: step 0 → group 0 (cells 0 and 9); group 1 is still in step -1 → (2 groups) group 1.
        let a = alpha_grid(&e, 200_000, None);
        assert_eq!((a[0], a[9]), (1.0, 1.0));
        assert_eq!(a[63], 1.0, "group 1 delayed by 0.5: still lit from its previous step");
        assert_eq!(a[5], 0.0, "cells in no group stay transparent");
        // 0.7 s: group 0 (no delay) still lit; group 1 now at its step 0 (= group 0's turn) → dark.
        let b = alpha_grid(&e, 700_000, None);
        assert_eq!((b[0], b[63]), (1.0, 0.0));
        assert_eq!(b[9], 1.0, "a cell in two groups takes the brightest");
    }

    #[test]
    fn path_runs_along_its_path_with_tail_and_mirror() {
        // Perimeter at 0.25 laps/s: 7 cells per second up the left edge from (0, 0).
        let per = compile_default("builtin.path", |e| {
            let Effect::Path { params, .. } = e else { unreachable!() };
            params.tail = 0.0.into();
            params.size = 0.5.into();
        });
        let a = alpha_grid(&per, 1_000_000 / 7 * 3, None); // 3 cells up
        assert_eq!(a.iter().position(|v| *v > 0.9), Some(3 * 8), "at (0, 3)");
        assert_eq!(alpha_grid(&per, 4_000_000, None), alpha_grid(&per, 0, None), "one lap in 4 s");
        // A line without loop stops at its end; with loop it comes back.
        let line = |looping: bool| {
            compile_default("builtin.path", |e| {
                let Effect::Path { params, .. } = e else { unreachable!() };
                params.path = PathKind::Line.into();
                params.from_x = Some(0.0.into());
                params.from_y = Some(0.0.into());
                params.to_x = Some(7.0.into());
                params.to_y = Some(0.0.into());
                params.speed = 7.0.into();
                params.tail = 0.0.into();
                params.size = 0.5.into();
                params.looping = looping.into();
            })
        };
        let lit = |e: &CompiledEffect, t| alpha_grid(e, t, None).iter().position(|v| *v > 0.9);
        assert_eq!(lit(&line(false), 3_000_000), Some(7), "stopped at the end");
        assert_eq!(lit(&line(true), 1_428_571), Some(4), "1 s out, 3/7 s back");
        // Tail: cells behind the head fade; mirror x adds a twin at 7 - x.
        let tailed = compile_default("builtin.path", |e| {
            let Effect::Path { params, .. } = e else { unreachable!() };
            params.path = PathKind::Line.into();
            params.from_x = Some(0.0.into());
            params.from_y = Some(0.0.into());
            params.to_x = Some(7.0.into());
            params.to_y = Some(0.0.into());
            params.speed = 4.0.into();
            params.tail = 3.0.into();
            params.size = 0.5.into();
            params.looping = false.into();
            params.mirror = Mirror::Y.into();
        });
        let t = alpha_grid(&tailed, 1_000_000, None); // head at x = 4
        assert!(t[4] > t[3] && t[3] > t[2] && t[2] > t[1], "{:?}", &t[..8]);
        assert_eq!(t[7 * 8 + 4], t[4], "mirrored to the top row");
    }

    #[test]
    fn shapes_fill_or_outline_and_turn() {
        let shape = |kind: ShapeKind, fill: bool, angle: f64| {
            compile_default("builtin.shape", |e| {
                let Effect::Shape { params, .. } = e else { unreachable!() };
                params.shape = kind.into();
                params.fill = fill.into();
                params.edge = 0.0.into();
                params.size = 2.0.into();
                params.size_y = Some(1.0.into());
                params.center_x = 3.0.into();
                params.center_y = 3.0.into();
                params.angle = angle.into();
            })
        };
        let filled = alpha_grid(&shape(ShapeKind::Circle, true, 0.0), 0, None);
        assert_eq!((filled[3 * 8 + 3], filled[3 * 8 + 5], filled[3 * 8 + 6]), (1.0, 1.0, 0.0));
        let ring = alpha_grid(&shape(ShapeKind::Circle, false, 0.0), 0, None);
        assert_eq!((ring[3 * 8 + 3], ring[3 * 8 + 5]), (0.0, 1.0), "outline only");
        // A 2 × 1 (half sizes) rect: wide when flat, tall when turned 90°.
        let flat = alpha_grid(&shape(ShapeKind::Rect, true, 0.0), 0, None);
        let tall = alpha_grid(&shape(ShapeKind::Rect, true, 90.0), 0, None);
        assert_eq!((flat[3 * 8 + 5], flat[5 * 8 + 3]), (1.0, 0.0));
        assert_eq!((tall[3 * 8 + 5], tall[5 * 8 + 3]), (0.0, 1.0));
        let diamond = alpha_grid(&shape(ShapeKind::Diamond, true, 0.0), 0, None);
        assert_eq!((diamond[4 * 8 + 4], diamond[5 * 8 + 4]), (1.0, 0.0), "|dx| + |dy| <= 2");
        let line = alpha_grid(&shape(ShapeKind::Line, true, 0.0), 0, None);
        assert_eq!((line[3 * 8 + 1], line[3 * 8 + 5], line[4 * 8 + 3]), (1.0, 1.0, 0.0), "a line is its stroke");
        // The ripple ring shapes use the same metric as shape.
        assert_eq!(shape_sd(ShapeKind::Diamond, 1.0, 1.0, (2.0, 2.0), 0.0), crate::shape::metric(ripple_metric(RippleShape::Diamond), 1.0, 1.0) - 2.0);
    }

    #[test]
    fn wipe_and_invert_shapes() {
        let shape = |kind: ShapeKind, angle: f64, invert: bool| {
            compile_default("builtin.shape", |e| {
                let Effect::Shape { params, .. } = e else { unreachable!() };
                params.shape = kind.into();
                params.edge = 0.0.into();
                params.size = 2.0.into();
                params.angle = angle.into();
                params.invert = Some(invert.into());
            })
        };
        // A wipe at angle 0 paints the +x side of its centre (3.5); at 180 the other side.
        let right = alpha_grid(&shape(ShapeKind::Wipe, 0.0, false), 0, None);
        assert_eq!((right[3], right[4], right[7 * 8 + 7]), (0.0, 1.0, 1.0));
        let left = alpha_grid(&shape(ShapeKind::Wipe, 180.0, false), 0, None);
        assert_eq!((left[3], left[4]), (1.0, 0.0));
        // Inverted circle: dark inside, painted outside (an iris in black).
        let iris = alpha_grid(&shape(ShapeKind::Circle, 0.0, true), 0, None);
        assert_eq!((iris[3 * 8 + 3], iris[0], iris[7 * 8 + 7]), (0.0, 1.0, 1.0));
    }

    fn masked(mask: LayerMask, continuous: bool, clip_start_us: u64) -> Renderer {
        let mut s = demo();
        s.layers.truncate(1);
        let l = &mut s.layers[0];
        if continuous {
            l.content = Content::Continuous { preset_id: "base-blue".into() };
        } else if let Content::Timeline { clips } = &mut l.content {
            clips[0].start_us = clip_start_us;
        }
        l.mask = Some(mask);
        Renderer::new(&s).unwrap()
    }

    fn mask(shape: ShapeKind, size: crate::model::Param<f64>, invert: bool) -> LayerMask {
        LayerMask { shape: shape.into(), center_x: 3.5.into(), center_y: 3.5.into(), size, size_y: None, angle: None, edge: 0.0.into(), invert: invert.into() }
    }

    #[test]
    fn layer_mask_cuts_its_own_layer_on_the_layer_time() {
        use crate::model::{Keyframe, Param};
        let lit = |f: &Frame, i: usize| f.leds[i].b > 0.0;
        // Continuous: show time. A circle of radius 2 keeps the middle only.
        let mut r = masked(mask(ShapeKind::Circle, 2.0.into(), false), true, 0);
        let f = r.render(5_000_000, None);
        assert!(lit(&f, 3 * 8 + 3) && !lit(&f, 0) && !lit(&f, 7 * 8 + 7));
        // Inverted wipe at angle 0: the -x half stays.
        let mut r = masked(mask(ShapeKind::Wipe, 1.0.into(), true), true, 0);
        let f = r.render(0, None);
        assert!(lit(&f, 3) && !lit(&f, 4));
        // Timeline: the clip's local time. Radius 0 -> 8 over 8 s from the clip start (10 s).
        let grow = Param::Keyframes(vec![Keyframe { t_us: 0, v: 0.0, ease: Default::default() }, Keyframe { t_us: 8_000_000, v: 8.0, ease: Default::default() }]);
        let mut r = masked(mask(ShapeKind::Circle, grow, false), false, 10_000_000);
        let f = r.render(14_000_000, None); // radius 4
        assert!(lit(&f, 3 * 8 + 3) && !lit(&f, 0), "corner (4.95 away) not yet");
        let f = r.render(17_000_000, None); // radius 7
        assert!(lit(&f, 0) && lit(&f, 7 * 8 + 7));
        assert_eq!(r.render(5_000_000, None), Frame::default(), "before the clip nothing is drawn");
    }
    #[test]
    fn renderer_passes_pad_input_only_to_ripple() {
        let mut s = demo();
        s.presets.push(crate::model::Preset { id: "ripple".into(), name: "波紋".into(), effect: crate::edit::default_effect("builtin.ripple").unwrap() });
        if let Content::Continuous { preset_id } = &mut s.layers[2].content {
            *preset_id = "ripple".into();
        }
        let mut r = Renderer::new(&s).unwrap();
        let mut input = PadInput::default();
        input.push(PadHit::press(3, 3, 100, 1_000));
        input.advance(1_000);
        // Without input the frame is the same as before; with a fresh hit the pad lights up.
        let plain = r.render(5_000_000, None);
        assert_eq!(r.render_input(5_000_000, None, None), plain);
        let lit = r.render_input(5_000_000, None, Some(&input));
        assert_ne!(lit.leds[3 * 8 + 3], plain.leds[3 * 8 + 3]);
        assert_eq!(lit.leds[0], plain.leds[0], "far cells unchanged");
    }

    #[test]
    fn blend_modes() {
        let mut layer = LayerFrame::default();
        layer.px[0] = Rgba::from_color(Rgb::new(1.0, 0.0, 0.0), 0.5);
        let base = || {
            let mut f = Frame::default();
            f.leds[0] = Rgb::new(0.2, 0.4, 0.0);
            f
        };
        let mut f = base();
        blend_layer(&mut f, &layer, Blend::Over, 1.0);
        assert!(close(f.leds[0], Rgb::new(0.6, 0.2, 0.0)));
        let mut f = base();
        blend_layer(&mut f, &layer, Blend::Add, 1.0);
        assert!(close(f.leds[0], Rgb::new(0.7, 0.4, 0.0)));
        let mut f = base();
        blend_layer(&mut f, &layer, Blend::Max, 0.5);
        assert!(close(f.leds[0], Rgb::new(0.25, 0.4, 0.0)));
    }

    #[test]
    fn deterministic_rendering_nfr08() {
        let show = demo();
        let mut a = Renderer::new(&show).unwrap();
        let mut b = Renderer::new(&show).unwrap();
        let times = [0i64, 8_000_000, 9_123_457, 20_000_000, 31_999_999, 59_000_000];
        for (k, &t) in times.iter().enumerate() {
            let beat = Some(BeatState::new(k as f64 * 0.37, 120.0));
            let fa = a.render(t, beat);
            // Same renderer again, and a fresh renderer, in a different order.
            assert_eq!(fa, a.render(t, beat));
            assert_eq!(fa, b.render(t, beat));
        }
        for &t in times.iter().rev() {
            assert_eq!(a.render(t, None), b.render(t, None));
        }
    }

    #[test]
    fn invalid_show_is_refused() {
        let mut s = demo();
        s.layers[0].target_id = "missing".into();
        assert!(Renderer::new(&s).is_err());
    }
}
