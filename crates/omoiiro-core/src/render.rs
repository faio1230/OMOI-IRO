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
//!   argument and only Ripple reads them. Each hit spreads a ring at
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
//!
//! Compositing: clips inside a layer are composited `over` in array order
//! into a premultiplied layer buffer (clip alpha = fade x effect alpha).
//! Layers are applied bottom to top onto black with their opacity:
//! `over` = `F (1 - a o) + L o`, `add` = `min(1, F + L o)`,
//! `max` = `max(F, L o)` per channel (L premultiplied). The master
//! brightness scales the result.

use crate::model::{Axis, Blend, Content, Effect, OrbMotion, RainDirection, RippleFade, Show, StrobeMode, Waveform};
use crate::surface::{Frame, LayerFrame, Rgb, Rgba, GRID_H, GRID_LEDS, GRID_W};
use crate::time::{BeatState, ShowTimeUs};
use crate::timeline::{ClipIndex, ClipSpan};
use crate::validation::{validate, Issue};
use std::collections::HashMap;
use std::f64::consts::TAU;

/// An effect with its parameters parsed for rendering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CompiledEffect {
    Solid { color: Rgb },
    Chase { color: Rgb, period_us: u64, width: u32, axis: Axis, reverse: bool },
    Pulse { color: Rgb, beats_per_cycle: f64, depth: f64, waveform: Waveform },
    Strobe { color: Rgb, mode: StrobeMode, rate_hz: f64, beats_per_cycle: f64, duty: f64, phase_offset: f64 },
    Noise { base: Rgb, highlight: Rgb, density: f64, rate_hz: f64, seed: u32, smooth: bool, depth: f64 },
    Ripple { color: Rgb, speed: f64, ring_width: f64, lifetime_us: i64, fade: RippleFade, max_concurrent: usize },
    Rain { color: Rgb, head: Rgb, direction: RainDirection, speed: f64, density: f64, tail: f64, seed: u32, flicker: f64 },
    Wave { color: Rgb, axis: Axis, wavelength: f64, amplitude: f64, center: f64, speed: f64, thickness: f64, beat_amplitude: bool },
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
    },
}

/// One pad press for input-driven effects: grid cell (bottom-left origin)
/// and the monotonic time it arrived.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadHit {
    pub x: u8,
    pub y: u8,
    pub velocity: u8,
    pub t_us: i64,
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
    /// Hits older than the longest lifetime are dropped.
    pub const MAX_AGE_US: i64 = 4_000_000;

    /// Add a hit; drop the oldest beyond `CAPACITY`. No allocation once full.
    pub fn push(&mut self, hit: PadHit) {
        if self.hits.len() == Self::CAPACITY {
            self.hits.remove(0);
        }
        self.hits.push(hit);
    }

    /// Set the time and drop expired hits.
    pub fn advance(&mut self, now_us: i64) {
        self.now_us = now_us;
        self.hits.retain(|h| now_us - h.t_us <= Self::MAX_AGE_US);
    }
}

impl CompiledEffect {
    pub fn compile(e: &Effect) -> Option<CompiledEffect> {
        Some(match e {
            // Parameters are constant in Schema 0.1.x (`Param::Value`).
            Effect::Solid { params, .. } => CompiledEffect::Solid { color: Rgb::from_hex(params.color.value())? },
            Effect::Chase { params, .. } => CompiledEffect::Chase {
                color: Rgb::from_hex(params.color.value())?,
                period_us: (*params.period_us.value()).max(1),
                width: (*params.width.value()).clamp(1, 8),
                axis: *params.axis.value(),
                reverse: params.reverse.as_ref().is_some_and(|r| *r.value()),
            },
            Effect::Pulse { params, .. } => CompiledEffect::Pulse {
                color: Rgb::from_hex(params.color.value())?,
                beats_per_cycle: *params.beats_per_cycle.value(),
                depth: *params.depth.value(),
                waveform: *params.waveform.value(),
            },
            Effect::Strobe { params, .. } => CompiledEffect::Strobe {
                color: Rgb::from_hex(params.color.value())?,
                mode: *params.mode.value(),
                rate_hz: *params.rate_hz.value(),
                beats_per_cycle: *params.beats_per_cycle.value(),
                duty: *params.duty.value(),
                phase_offset: *params.phase_offset.value(),
            },
            Effect::Ripple { params, .. } => CompiledEffect::Ripple {
                color: Rgb::from_hex(params.color.value())?,
                speed: *params.speed.value(),
                ring_width: *params.ring_width.value(),
                lifetime_us: *params.lifetime_ms.value() as i64 * 1000,
                fade: *params.fade.value(),
                max_concurrent: (*params.max_concurrent.value()).clamp(1, PadInput::CAPACITY as u32) as usize,
            },
            Effect::Rain { params, .. } => {
                let color = Rgb::from_hex(params.color.value())?;
                CompiledEffect::Rain {
                    color,
                    head: match &params.head_color {
                        Some(h) => Rgb::from_hex(h.value())?,
                        None => color,
                    },
                    direction: *params.direction.value(),
                    speed: *params.speed.value(),
                    density: *params.density.value(),
                    tail: *params.tail.value(),
                    seed: *params.seed.value(),
                    flicker: *params.flicker.value(),
                }
            }
            Effect::Wave { params, .. } => CompiledEffect::Wave {
                color: Rgb::from_hex(params.color.value())?,
                axis: *params.axis.value(),
                wavelength: *params.wavelength.value(),
                amplitude: *params.amplitude.value(),
                center: *params.center.value(),
                speed: *params.speed.value(),
                thickness: *params.thickness.value(),
                beat_amplitude: *params.beat_amplitude.value(),
            },
            Effect::Orb { params, .. } => {
                let (la, ba) = (params.light_angle.value().to_radians(), params.bounce_angle.value().to_radians());
                CompiledEffect::Orb {
                    color: Rgb::from_hex(params.color.value())?,
                    center: (*params.center_x.value(), *params.center_y.value()),
                    radius: *params.radius.value(),
                    edge: *params.edge.value(),
                    light: (la.cos(), la.sin()),
                    shading: *params.shading.value(),
                    dither: *params.dither.value(),
                    dither_rate_hz: *params.dither_rate_hz.value(),
                    seed: *params.seed.value(),
                    motion: *params.motion.value(),
                    speed: *params.speed.value(),
                    orbit_radius: *params.orbit_radius.value(),
                    bounce_dir: (ba.cos(), ba.sin()),
                }
            }
            Effect::Noise { params, .. } => CompiledEffect::Noise {
                base: Rgb::from_hex(params.base_color.value())?,
                highlight: Rgb::from_hex(params.highlight_color.value())?,
                density: *params.density.value(),
                rate_hz: *params.rate_hz.value(),
                seed: *params.seed.value(),
                smooth: *params.smooth.value(),
                depth: *params.depth.value(),
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
            // Needs the pad input: drawn by `paint_input`.
            CompiledEffect::Ripple { .. } => {}
            CompiledEffect::Rain { color, head, direction, speed, density, tail, seed, flicker } => {
                paint_rain(local_us, color, head, direction, speed, density, tail, seed, flicker, alpha, layer);
            }
            CompiledEffect::Wave { color, axis, wavelength, amplitude, center, speed, thickness, beat_amplitude } => {
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
                    let curve = center + amp * (TAU * (along / wavelength - speed * t)).sin();
                    let d = (across - curve).abs();
                    let a = if d <= half { 1.0 } else { (1.0 - (d - half) / half).max(0.0) };
                    if a > 0.0 {
                        over(px, Rgba::from_color(color, (a * alpha) as f32));
                    }
                }
            }
            CompiledEffect::Orb { color, center, radius, edge, light, shading, dither, dither_rate_hz, seed, motion, speed, orbit_radius, bounce_dir } => {
                let t = local_us as f64 / 1e6;
                let (cx, cy) = match motion {
                    OrbMotion::None => center,
                    OrbMotion::Orbit => (center.0 + orbit_radius * (TAU * speed * t).cos(), center.1 + orbit_radius * (TAU * speed * t).sin()),
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
    pub fn paint_input(&self, local_us: u64, beat: Option<BeatState>, input: Option<&PadInput>, alpha: f64, layer: &mut LayerFrame) {
        match *self {
            CompiledEffect::Ripple { color, speed, ring_width, lifetime_us, fade, max_concurrent } => {
                if let Some(input) = input {
                    paint_ripple(input, color, speed, ring_width, lifetime_us, fade, max_concurrent, alpha, layer);
                }
            }
            _ => self.paint(local_us, beat, alpha, layer),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_ripple(input: &PadInput, color: Rgb, speed: f64, ring_width: f64, lifetime_us: i64, fade: RippleFade, max_concurrent: usize, alpha: f64, layer: &mut LayerFrame) {
    if alpha <= 0.0 {
        return;
    }
    let half = (ring_width / 2.0).max(1e-6);
    let mut cell_a = [0.0f64; GRID_LEDS];
    // Newest first, live ones only, at most `max_concurrent`.
    let live = input.hits.iter().rev().filter(|h| {
        let age = input.now_us - h.t_us;
        (0..lifetime_us).contains(&age)
    });
    for h in live.take(max_concurrent) {
        let age = (input.now_us - h.t_us) as f64;
        let life = age / lifetime_us as f64;
        let f = match fade {
            RippleFade::Linear => 1.0 - life,
            RippleFade::Exp => (-4.0 * life).exp() * (1.0 - life),
        };
        let radius = speed * age / 1e6;
        for (i, a) in cell_a.iter_mut().enumerate() {
            let (dx, dy) = ((i % GRID_W) as f64 - h.x as f64, (i / GRID_W) as f64 - h.y as f64);
            let off = ((dx * dx + dy * dy).sqrt() - radius).abs();
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
    Timeline { index: ClipIndex, effects: Vec<usize> },
}

#[derive(Clone, Debug)]
struct CompiledLayer {
    enabled: bool,
    opacity: f32,
    blend: Blend,
    content: LayerContent,
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
        for (i, p) in show.presets.iter().enumerate() {
            effects.push(CompiledEffect::compile(&p.effect).expect("validated color"));
            by_id.insert(p.id.as_str(), i);
        }
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
                            effects: clips.iter().map(|c| by_id[c.preset_id.as_str()]).collect(),
                        }
                    }
                };
                CompiledLayer { enabled: l.enabled, opacity: l.opacity as f32, blend: l.blend, content }
            })
            .collect();
        Ok(Renderer {
            effects,
            layers,
            master_brightness: show.render.master_brightness as f32,
            buf: LayerFrame::default(),
            active: Vec::with_capacity(max_clips),
            muted: vec![false; show.layers.len()],
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
        for (i, layer) in self.layers.iter().enumerate() {
            if !layer.enabled || layer.opacity <= 0.0 || t < 0 || self.muted[i] {
                continue;
            }
            self.buf.clear();
            match &layer.content {
                LayerContent::Continuous { effect } => {
                    self.effects[*effect].paint_input(t as u64, beat, input, 1.0, &mut self.buf);
                }
                LayerContent::Timeline { index, effects } => {
                    index.active(t, &mut self.active);
                    for span in &self.active {
                        self.effects[effects[span.order]].paint_input(span.local_us(t), beat, input, span.fade(t), &mut self.buf);
                    }
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
        let alpha = |e: CompiledEffect, beat: Option<BeatState>| {
            let mut l = LayerFrame::default();
            e.paint(0, beat, 1.0, &mut l);
            l.px[0].a
        };
        let sine = mk(1.0, Waveform::Sine);
        assert!((alpha(sine, Some(BeatState::new(4.0, 120.0))) - 1.0).abs() < 1e-6); // phase 0 = max
        assert!(alpha(sine, Some(BeatState::new(4.5, 120.0))) < 1e-6); // half phase = 0
        assert!((alpha(mk(0.0, Waveform::Sine), Some(BeatState::new(4.5, 120.0))) - 1.0).abs() < 1e-6); // depth 0
        assert!((alpha(mk(1.0, Waveform::Triangle), Some(BeatState::new(0.25, 120.0))) - 0.5).abs() < 1e-6);
        assert_eq!(alpha(sine, None), 0.0);
        let mut unknown = BeatState::new(4.0, 120.0);
        unknown.phase_valid = false;
        assert_eq!(alpha(sine, Some(unknown)), 0.0);
        // 0.25 beats per cycle: beat 0.125 is mid-cycle.
        let quarter = CompiledEffect::Pulse { color: Rgb::new(1.0, 1.0, 1.0), beats_per_cycle: 0.25, depth: 1.0, waveform: Waveform::Sine };
        assert!(alpha(quarter, Some(BeatState::new(0.125, 120.0))) < 1e-6);
    }

    #[test]
    fn strobe_is_a_square_wave_in_hz_or_on_the_beat() {
        let white = Rgb::new(1.0, 1.0, 1.0);
        let hz = CompiledEffect::Strobe { color: white, mode: StrobeMode::Hz, rate_hz: 4.0, beats_per_cycle: 1.0, duty: 0.25, phase_offset: 0.0 };
        let alpha = |e: CompiledEffect, local: u64, beat: Option<BeatState>| {
            let mut l = LayerFrame::default();
            e.paint(local, beat, 1.0, &mut l);
            assert!(l.px.iter().all(|p| p.a == l.px[0].a), "whole surface");
            l.px[0].a
        };
        // 4 Hz = 250 ms cycles, on for the first 62.5 ms.
        assert_eq!(alpha(hz, 0, None), 1.0);
        assert_eq!(alpha(hz, 62_000, None), 1.0);
        assert_eq!(alpha(hz, 63_000, None), 0.0);
        assert_eq!(alpha(hz, 250_000, None), 1.0);
        // phase_offset 0.5 shifts the on part by half a cycle.
        let off = CompiledEffect::Strobe { color: white, mode: StrobeMode::Hz, rate_hz: 4.0, beats_per_cycle: 1.0, duty: 0.25, phase_offset: 0.5 };
        assert_eq!(alpha(off, 0, None), 0.0);
        assert_eq!(alpha(off, 125_000, None), 1.0);
        // Beat mode: 2 beats per cycle, duty 0.5; transparent without a phase.
        let beat = CompiledEffect::Strobe { color: white, mode: StrobeMode::Beat, rate_hz: 1.0, beats_per_cycle: 2.0, duty: 0.5, phase_offset: 0.0 };
        assert_eq!(alpha(beat, 0, Some(BeatState::new(4.9, 120.0))), 1.0);
        assert_eq!(alpha(beat, 0, Some(BeatState::new(5.1, 120.0))), 0.0);
        assert_eq!(alpha(beat, 0, None), 0.0);
        let mut unknown = BeatState::new(4.0, 120.0);
        unknown.phase_valid = false;
        assert_eq!(alpha(beat, 0, Some(unknown)), 0.0);
    }

    #[test]
    fn noise_is_deterministic_dense_as_asked_and_smooth_on_request() {
        let base = Rgb::from_hex("#0D2346").unwrap();
        let hi = Rgb::from_hex("#9FE8FF").unwrap();
        let mk = |density: f64, smooth: bool, seed: u32| CompiledEffect::Noise { base, highlight: hi, density, rate_hz: 6.0, seed, smooth, depth: 0.5 };
        let paint = |e: CompiledEffect, local: u64| {
            let mut l = LayerFrame::default();
            e.paint(local, None, 1.0, &mut l);
            l
        };
        let e = mk(0.3, false, 1);
        // Same time, same picture; within one step (1/6 s) the pattern holds.
        assert_eq!(paint(e, 1_000_000).px, paint(e, 1_000_000).px);
        assert_eq!(paint(e, 1_000_000).px, paint(e, 1_100_000).px);
        assert_ne!(paint(e, 1_000_000).px, paint(e, 1_200_000).px);
        // Another seed gives another pattern.
        assert_ne!(paint(e, 1_000_000).px, paint(mk(0.3, false, 2), 1_000_000).px);
        // Density: the share of non-base cells over many steps is close to 0.3.
        let mut lit = 0;
        for step in 0..100u64 {
            let l = paint(e, step * 166_667 + 1);
            lit += l.px.iter().filter(|p| (p.b - base.b).abs() > 1e-6).count();
        }
        let share = lit as f64 / 6400.0;
        assert!((0.25..0.35).contains(&share), "{share}");
        // Density 0 is the base color everywhere, opaque.
        assert!(paint(mk(0.0, false, 1), 0).px.iter().all(|p| p.a == 1.0 && (p.r - base.r).abs() < 1e-6));
        // Smooth: halfway between two steps is the average of both patterns.
        let s = mk(0.3, true, 1);
        let (a, b, mid) = (paint(e, 1_000_000), paint(e, 1_166_667), paint(s, 1_083_334));
        for i in 0..64 {
            assert!((mid.px[i].r - (a.px[i].r + b.px[i].r) / 2.0).abs() < 0.01, "cell {i}");
        }
    }

    #[test]
    fn ripple_rings_spread_from_pad_hits_and_fade() {
        let e = CompiledEffect::Ripple { color: Rgb::new(1.0, 1.0, 1.0), speed: 4.0, ring_width: 1.0, lifetime_us: 1_000_000, fade: RippleFade::Linear, max_concurrent: 2 };
        let at = |input: &PadInput| {
            let mut l = LayerFrame::default();
            e.paint_input(0, None, Some(input), 1.0, &mut l);
            l
        };
        let hit = |x, y, t_us| PadHit { x, y, velocity: 100, t_us };
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
        // Capacity: the buffer keeps the 16 newest; advance drops hits older than 4 s.
        let mut input = PadInput::default();
        for k in 0..20 {
            input.push(hit(0, 0, k));
        }
        assert_eq!((input.hits.len(), input.hits[0].t_us), (16, 4));
        input.advance(5_000_000);
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
        let w = |beat_amplitude| CompiledEffect::Wave { color: Rgb::new(1.0, 1.0, 1.0), axis: Axis::X, wavelength: 8.0, amplitude: 2.5, center: 3.5, speed: 1.0, thickness: 1.0, beat_amplitude };
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
    fn renderer_passes_pad_input_only_to_ripple() {
        let mut s = demo();
        s.presets.push(crate::model::Preset { id: "ripple".into(), name: "波紋".into(), effect: crate::edit::default_effect("builtin.ripple").unwrap() });
        if let Content::Continuous { preset_id } = &mut s.layers[2].content {
            *preset_id = "ripple".into();
        }
        let mut r = Renderer::new(&s).unwrap();
        let mut input = PadInput::default();
        input.push(PadHit { x: 3, y: 3, velocity: 100, t_us: 1_000 });
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
