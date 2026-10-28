//! Particles (0.1.3, docs/14 §4-5). Nothing is integrated: every particle
//! is placed from the time since its emission with the closed form of drag
//! and gravity, and its start values come from hash(seed, event, particle),
//! so any show time can be drawn directly (seek) and gives the same frame.
//!
//! An *event* is one emission: a burst (burst / rocket / implode) or a
//! fountain emitting `rate` per second for a while. Layers turn their time
//! or pad input into events (`render`); this module draws them.

use crate::model::{EdgeMode, ParticleMode, ParticlesParams};
use crate::render::hash01;
use crate::surface::{LayerFrame, Rgb, Rgba, GRID_H, GRID_W};

/// Events drawn per layer at most (newest first).
pub const MAX_EVENTS: usize = 16;
/// Particles drawn per layer at most; older ones are dropped first.
pub const MAX_PARTICLES: usize = 256;
/// Particles drawn per frame over the whole show (docs/14 §4-5): layers
/// draw bottom first, each newest first, until this is used up.
pub const MAX_SHOW_PARTICLES: usize = 768;

/// Grid walls: cell centres are 0..=7, a cell reaches half a cell further.
const LO: f64 = -0.5;
const HI: f64 = 7.5;

/// Particle parameters sampled at one time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleLook {
    pub mode: ParticleMode,
    pub count: u32,
    pub rate: f64,
    pub color: Rgb,
    pub color_end: Rgb,
    pub speed_min: f64,
    pub speed_max: f64,
    pub angle: f64,
    pub spread: f64,
    pub gravity: f64,
    pub drag: f64,
    pub lifetime_s: f64,
    pub size: f64,
    pub tail: f64,
    pub bounce: bool,
    pub origin: (f64, f64),
    pub seed: u32,
    pub rise_speed: f64,
    pub burst_height: f64,
    pub pause_s: f64,
    pub implode_radius: f64,
}

impl ParticleLook {
    /// Sample the parameters at `t` (µs on their time axis); None on a bad color.
    pub fn compile(p: &ParticlesParams, t: u64, col: impl Fn(&crate::model::Param<String>, u64) -> Option<Rgb>) -> Option<ParticleLook> {
        let color = col(&p.color, t)?;
        Some(ParticleLook {
            mode: p.mode.at(t),
            count: p.count.as_ref().map_or(16, |c| c.at(t)).clamp(1, 64),
            rate: p.rate.as_ref().map_or(8.0, |r| r.at(t)).max(0.01),
            color,
            color_end: match &p.color_end {
                Some(c) => col(c, t)?,
                None => color,
            },
            speed_min: p.speed_min.at(t),
            speed_max: p.speed_max.at(t),
            angle: p.angle.at(t),
            spread: p.spread.at(t),
            gravity: p.gravity.at(t),
            drag: p.drag.at(t),
            lifetime_s: p.lifetime_ms.at(t).max(1) as f64 / 1000.0,
            size: p.size.at(t),
            tail: p.tail.at(t),
            bounce: p.edge_mode.at(t) == EdgeMode::Bounce,
            origin: (p.origin_x.as_ref().map_or(3.5, |v| v.at(t)), p.origin_y.as_ref().map_or(3.5, |v| v.at(t))),
            seed: p.seed.at(t),
            rise_speed: p.rise_speed.as_ref().map_or(10.0, |v| v.at(t)).max(0.01),
            burst_height: p.burst_height.as_ref().map_or(5.0, |v| v.at(t)),
            pause_s: p.pause_ms.as_ref().map_or(200, |v| v.at(t)) as f64 / 1000.0,
            implode_radius: p.implode_radius.as_ref().map_or(4.0, |v| v.at(t)),
        })
    }

    /// Seconds before the burst of an event starting at row `y0`: the rocket
    /// rise, or the implode gathering (half the lifetime) plus its pause.
    pub fn pre_s(&self, y0: f64) -> f64 {
        match self.mode {
            ParticleMode::Rocket => (self.burst_height - y0).max(0.0) / self.rise_speed,
            ParticleMode::Implode => self.lifetime_s / 2.0 + self.pause_s,
            ParticleMode::Burst | ParticleMode::Fountain => 0.0,
        }
    }

    /// Seconds an event is visible: before its burst + emission + one lifetime.
    pub fn span_s(&self, y0: f64, emit_s: f64) -> f64 {
        match self.mode {
            ParticleMode::Fountain => emit_s + self.lifetime_s,
            _ => self.pre_s(y0) + self.lifetime_s,
        }
    }
}

/// One emission: where, its age, how long a fountain emits (seconds,
/// `f64::INFINITY` = until now) and a stable id for the hash.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub x: f64,
    pub y: f64,
    pub age_s: f64,
    pub emit_s: f64,
    pub id: u64,
    /// Local time (µs) the event started at: keyframed parameters are
    /// sampled there, so a flight keeps one set of parameters.
    pub start_us: u64,
}

/// Position on one axis after `t` seconds from `x0` at `v0` with constant
/// acceleration `g` and drag `k` (dv/dt = g − k v).
#[inline]
fn axis_pos(x0: f64, v0: f64, g: f64, k: f64, t: f64) -> f64 {
    if k < 1e-9 {
        x0 + v0 * t + 0.5 * g * t * t
    } else {
        let e = (-k * t).exp();
        x0 + (g / k) * t + (v0 - g / k) * (1.0 - e) / k
    }
}

#[inline]
fn axis_vel(v0: f64, g: f64, k: f64, t: f64) -> f64 {
    if k < 1e-9 {
        v0 + g * t
    } else {
        g / k + (v0 - g / k) * (-k * t).exp()
    }
}

/// Time in (0, t) where the velocity crosses zero (the one extremum), if any.
fn turn_time(v0: f64, g: f64, k: f64, t: f64) -> Option<f64> {
    let s = if k < 1e-9 {
        if g == 0.0 {
            return None;
        }
        -v0 / g
    } else {
        // g/k + (v0 − g/k)·e = 0  →  e = −g / (k·v0 − g)
        let d = k * v0 - g;
        if d == 0.0 {
            return None;
        }
        let e = -g / d;
        if !(e > 0.0 && e < 1.0) {
            return None;
        }
        -e.ln() / k
    };
    (s > 0.0 && s < t).then_some(s)
}

/// Fold a coordinate into the walls (elastic reflection = triangle wave).
#[inline]
fn fold(u: f64) -> f64 {
    let w = (u - LO).rem_euclid(2.0 * (HI - LO));
    LO + if w > HI - LO { 2.0 * (HI - LO) - w } else { w }
}

#[inline]
fn inside(u: f64) -> bool {
    (LO..=HI).contains(&u)
}

/// One particle in flight: start, velocity, accelerations.
#[derive(Clone, Copy)]
struct Flight {
    x0: f64,
    y0: f64,
    vx: f64,
    vy: f64,
    g: f64,
    k: f64,
    bounce: bool,
}

impl Flight {
    fn launch(look: &ParticleLook, x0: f64, y0: f64, id: u64, j: u64) -> Flight {
        let speed = look.speed_min + (look.speed_max - look.speed_min) * hash01(look.seed, id, j, 1);
        let dir = (look.angle + (hash01(look.seed, id, j, 2) - 0.5) * look.spread).to_radians();
        Flight { x0, y0, vx: speed * dir.cos(), vy: speed * dir.sin(), g: look.gravity, k: look.drag, bounce: look.bounce }
    }

    /// Unfolded position after `t` seconds (one exponential for both axes).
    #[inline]
    fn raw(&self, t: f64) -> (f64, f64) {
        if self.k < 1e-9 {
            return (self.x0 + self.vx * t, self.y0 + self.vy * t + 0.5 * self.g * t * t);
        }
        let d = (1.0 - (-self.k * t).exp()) / self.k;
        let gk = self.g / self.k;
        (self.x0 + self.vx * d, self.y0 + gk * t + (self.vy - gk) * d)
    }

    /// Position after `t` seconds; None once it has left the grid (vanish).
    fn at(&self, t: f64) -> Option<(f64, f64)> {
        let (x, y) = self.raw(t);
        if self.bounce {
            return Some((fold(x), fold(y)));
        }
        // x moves one way only; y may turn once (gravity): check the turn too.
        let turned_out = turn_time(self.vy, self.g, self.k, t).is_some_and(|s| !inside(axis_pos(self.y0, self.vy, self.g, self.k, s)));
        (inside(x) && inside(y) && !turned_out).then_some((x, y))
    }

    fn speed(&self, t: f64) -> f64 {
        axis_vel(self.vx, 0.0, self.k, t).hypot(axis_vel(self.vy, self.g, self.k, t))
    }
}

/// Draw a round spot of radius `size` (1 at the centre, 0 at `size`).
fn spot(x: f64, y: f64, size: f64, color: Rgb, a: f64, layer: &mut LayerFrame) {
    if a <= 0.0 {
        return;
    }
    let (x0, x1) = ((x - size).ceil().max(0.0) as i64, (x + size).floor().min(GRID_W as f64 - 1.0) as i64);
    let (y0, y1) = ((y - size).ceil().max(0.0) as i64, (y + size).floor().min(GRID_H as f64 - 1.0) as i64);
    for cy in y0..=y1 {
        for cx in x0..=x1 {
            let d = (cx as f64 - x).hypot(cy as f64 - y);
            let k = 1.0 - d / size;
            if k > 0.0 {
                let px = &mut layer.px[cy as usize * GRID_W + cx as usize];
                let s = Rgba::from_color(color, (k * a) as f32);
                // Brightest wins: overlapping sparks do not pile up into white.
                if s.a > px.a {
                    *px = s;
                }
            }
        }
    }
}

fn lerp(a: Rgb, b: Rgb, k: f64) -> Rgb {
    let k = k.clamp(0.0, 1.0) as f32;
    Rgb::new(a.r + (b.r - a.r) * k, a.g + (b.g - a.g) * k, a.b + (b.b - a.b) * k)
}

/// Head plus a tail of `tail` cells behind it, sampled back in time at the
/// current speed (a bounce bends the tail with the flight).
fn draw_flight(look: &ParticleLook, f: &Flight, t: f64, color: Rgb, alpha: f64, layer: &mut LayerFrame) -> bool {
    let Some((x, y)) = f.at(t) else { return false };
    spot(x, y, look.size, color, alpha, layer);
    if look.tail > 0.0 {
        let back = (look.tail / f.speed(t).max(0.5)).min(t);
        let n = (look.tail * 3.0).ceil() as usize;
        for i in 1..=n {
            let ti = t - back * i as f64 / n as f64;
            if let Some((tx, ty)) = f.at(ti) {
                spot(tx, ty, look.size, color, alpha * (1.0 - i as f64 / (n + 1) as f64), layer);
            }
        }
    }
    true
}

/// A straight streak from `from` to `to` (the rocket rise and the implode
/// gathering): head at `to`, tail back towards `from`.
fn draw_streak(look: &ParticleLook, from: (f64, f64), to: (f64, f64), color: Rgb, alpha: f64, layer: &mut LayerFrame) {
    spot(to.0, to.1, look.size, color, alpha, layer);
    let len = (to.0 - from.0).hypot(to.1 - from.1);
    if look.tail > 0.0 && len > 0.0 {
        let back = look.tail.min(len);
        let n = (look.tail * 3.0).ceil() as usize;
        for i in 1..=n {
            let k = back * i as f64 / n as f64 / len;
            spot(to.0 + (from.0 - to.0) * k, to.1 + (from.1 - to.1) * k, look.size, color, alpha * (1.0 - i as f64 / (n + 1) as f64), layer);
        }
    }
}

/// Draw one event. `look_at(local_us)` samples the parameters (a fountain
/// samples each particle at its birth); `budget` counts particles left.
pub fn paint_event(ev: &Event, look_at: &dyn Fn(u64) -> Option<ParticleLook>, alpha: f64, layer: &mut LayerFrame, budget: &mut usize) {
    let Some(look) = look_at(ev.start_us) else { return };
    let a = ev.age_s;
    if a < 0.0 || *budget == 0 {
        return;
    }
    match look.mode {
        ParticleMode::Fountain => {
            // Particle n is born at n / rate; newest first, alive ones only.
            let newest = (a.min(ev.emit_s) * look.rate).floor();
            let oldest = ((a - look.lifetime_s) * look.rate).ceil().max(0.0);
            let mut n = newest;
            while n >= oldest && *budget > 0 {
                let born = n / look.rate;
                let l = look_at(ev.start_us + (born * 1e6) as u64).unwrap_or(look);
                let t = a - born;
                if t < l.lifetime_s {
                    let f = Flight::launch(&l, ev.x, ev.y, ev.id, n as u64);
                    draw_flight(&l, &f, t, lerp(l.color, l.color_end, t / l.lifetime_s), alpha, layer);
                    *budget -= 1;
                }
                n -= 1.0;
            }
        }
        ParticleMode::Burst | ParticleMode::Rocket | ParticleMode::Implode => {
            let pre = look.pre_s(ev.y);
            let centre = match look.mode {
                ParticleMode::Rocket => (ev.x, look.burst_height.max(ev.y)),
                _ => (ev.x, ev.y),
            };
            if a < pre {
                match look.mode {
                    ParticleMode::Rocket => {
                        draw_streak(&look, (ev.x, ev.y), (ev.x, ev.y + look.rise_speed * a), look.color, alpha, layer);
                        *budget -= 1;
                    }
                    ParticleMode::Implode => {
                        let gather = look.lifetime_s / 2.0;
                        let k = (a / gather).min(1.0);
                        for j in 0..look.count as u64 {
                            if *budget == 0 {
                                break;
                            }
                            let th = hash01(look.seed, ev.id, j, 3) * std::f64::consts::TAU;
                            let from = (centre.0 + look.implode_radius * th.cos(), centre.1 + look.implode_radius * th.sin());
                            let at = (from.0 + (centre.0 - from.0) * k, from.1 + (centre.1 - from.1) * k);
                            // Waiting at the centre: still, so no tail.
                            let from = if k < 1.0 { from } else { at };
                            draw_streak(&look, from, at, look.color, alpha, layer);
                            *budget -= 1;
                        }
                    }
                    _ => {}
                }
                return;
            }
            let t = a - pre;
            if t >= look.lifetime_s {
                return;
            }
            let color = lerp(look.color, look.color_end, t / look.lifetime_s);
            for j in 0..look.count as u64 {
                if *budget == 0 {
                    break;
                }
                let f = Flight::launch(&look, centre.0, centre.1, ev.id, j);
                draw_flight(&look, &f, t, color, alpha, layer);
                *budget -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_form_matches_small_steps() {
        // Integrate dv/dt = g − k v with tiny steps and compare.
        let (x0, v0, g, k) = (1.0, 6.0, -12.0, 2.0);
        let (mut x, mut v, dt) = (x0, v0, 1e-5);
        for _ in 0..50_000 {
            v += (g - k * v) * dt;
            x += v * dt;
        }
        assert!((axis_pos(x0, v0, g, k, 0.5) - x).abs() < 1e-3, "{x}");
        assert!((axis_vel(v0, g, k, 0.5) - v).abs() < 1e-3);
        assert_eq!(axis_pos(0.0, 2.0, -4.0, 0.0, 1.0), 0.0);
        // The turn of a thrown-up particle: v0 4 against g −8 turns at 0.5 s.
        assert_eq!(turn_time(4.0, -8.0, 0.0, 1.0), Some(0.5));
        assert_eq!(turn_time(4.0, -8.0, 0.0, 0.4), None);
    }

    #[test]
    fn fold_reflects_at_the_walls() {
        assert_eq!(fold(3.0), 3.0);
        assert_eq!(fold(8.5), 6.5, "1 past the top wall comes back 1");
        assert_eq!(fold(-1.5), 0.5);
        assert_eq!(fold(3.0 + 16.0), 3.0, "period of two widths");
    }

    #[test]
    fn vanish_is_final_even_if_gravity_brings_it_back() {
        // Thrown up hard from row 6: leaves the top, would fall back later.
        let f = Flight { x0: 3.0, y0: 6.0, vx: 0.0, vy: 8.0, g: -16.0, k: 0.0, bounce: false };
        assert!(f.at(0.05).is_some());
        assert!(f.at(0.5).is_none(), "out at the top");
        assert!(f.at(0.95).is_none(), "back inside, but it left: gone");
        let b = Flight { bounce: true, ..f };
        assert!(b.at(0.95).is_some());
    }
}
