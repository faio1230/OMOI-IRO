//! Effects that read the pad input history (0.1.3, docs/14 §4-8): pads
//! (heat / plant / paint), cycle and constellation, and the charge of
//! tap_glow. Their picture depends on what was pressed since the last
//! input reset, so a seek does not reproduce it (recorded clips do: their
//! presses come from the clip's time). Paint and cycle retain fixed-size
//! counts and last presses independently of the bounded transient history.
//! Recorded counts are computed arithmetically on every seek.
//!
//! Also here: the input of one triggered clip, on the clip's own clock,
//! with its recording played in (`clip_input`).

use crate::model::{CycleParams, CycleScope, ConstellationParams, PadsMode, PadsParams, Recording, RippleFade};
use crate::render::{PadHit, PadInput};
use crate::surface::{LayerFrame, Rgb, Rgba, GRID_H, GRID_LEDS, GRID_W};
use crate::timeline::ClipSpan;
use std::sync::Arc;
use std::collections::BinaryHeap;

/// Fixed-size persistent state, independent of the expiring event window.
#[derive(Clone, Debug, PartialEq)]
pub struct PadState {
    pub total: u64,
    pub counts: [u64; GRID_LEDS],
    pub last: [Option<PadHit>; GRID_LEDS],
}

impl Default for PadState {
    fn default() -> Self {
        Self { total: 0, counts: [0; GRID_LEDS], last: [None; GRID_LEDS] }
    }
}

impl PadState {
    pub fn observe(&mut self, h: PadHit, count: u64) {
        let i = cell(&h);
        self.total += count;
        self.counts[i] += count;
        if self.last[i].is_none_or(|old| (old.t_us, old.serial) <= (h.t_us, h.serial)) {
            self.last[i] = Some(h);
        }
    }
}

/// One clip's live state; timestamps stay on the monotonic input clock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScopedPads {
    pub(crate) generation: u64,
    pub(crate) serial: u64,
    pub(crate) state: PadState,
}

fn hex(s: &Option<String>, default: &str) -> Option<Rgb> {
    Rgb::from_hex(s.as_deref().unwrap_or(default))
}

fn palette(p: &[String]) -> Option<Arc<[Rgb]>> {
    p.iter().map(|c| Rgb::from_hex(c)).collect::<Option<Vec<_>>>().map(Into::into)
}

fn lerp(a: Rgb, b: Rgb, k: f64) -> Rgb {
    let k = k.clamp(0.0, 1.0) as f32;
    Rgb::new(a.r + (b.r - a.r) * k, a.g + (b.g - a.g) * k, a.b + (b.b - a.b) * k)
}

/// `builtin.pads` with the defaults of its mode filled in.
#[derive(Clone, Debug, PartialEq)]
pub struct PadsLook {
    pub mode: PadsMode,
    pub low: Rgb,
    pub high: Rgb,
    pub color: Rgb,
    pub palette: Option<Arc<[Rgb]>>,
    pub gain: f64,
    pub decay_us: i64,
    pub attack_us: i64,
    pub sustain_us: i64,
    pub release_us: i64,
    pub size: f64,
}

impl PadsLook {
    pub fn compile(p: &PadsParams) -> Option<PadsLook> {
        let ms = |v: Option<u32>, d: u32| v.unwrap_or(d) as i64 * 1000;
        Some(PadsLook {
            mode: p.mode,
            low: hex(&p.color_low, "#200040")?,
            high: hex(&p.color_high, "#FF4020")?,
            color: hex(&p.color, if p.mode == PadsMode::Plant { "#40FF80" } else { "#FFFFFF" })?,
            palette: match &p.palette {
                Some(x) => Some(palette(x)?),
                None => None,
            },
            gain: p.gain.unwrap_or(0.25),
            decay_us: ms(p.decay_ms, if p.mode == PadsMode::Paint { 0 } else { 3000 }),
            attack_us: ms(p.attack_ms, 300),
            sustain_us: ms(p.sustain_ms, 1000),
            release_us: ms(p.release_ms, 1500),
            size: p.size.unwrap_or(1.5),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CycleLook {
    pub palette: Arc<[Rgb]>,
    pub per_pad: bool,
    pub lifetime_us: i64,
    pub radius: f64,
}

impl CycleLook {
    pub fn compile(p: &CycleParams) -> Option<CycleLook> {
        Some(CycleLook { palette: palette(&p.palette).filter(|x| !x.is_empty())?, per_pad: p.scope == CycleScope::PerPad, lifetime_us: p.lifetime_ms as i64 * 1000, radius: p.radius })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConstellationLook {
    pub color: Rgb,
    pub stroke: f64,
    pub lifetime_us: i64,
    pub fade: RippleFade,
    pub max_segments: usize,
    pub dots: bool,
}

impl ConstellationLook {
    pub fn compile(p: &ConstellationParams) -> Option<ConstellationLook> {
        Some(ConstellationLook { color: Rgb::from_hex(&p.color)?, stroke: p.stroke, lifetime_us: p.lifetime_ms.max(1) as i64 * 1000, fade: p.fade, max_segments: p.max_segments.clamp(1, 16) as usize, dots: p.dots })
    }
}

/// Brightest wins per cell (states of different pads do not add up).
fn put(layer: &mut LayerFrame, i: usize, c: Rgb, a: f64) {
    let s = Rgba::from_color(c, a as f32);
    if s.a > layer.px[i].a {
        layer.px[i] = s;
    }
}

fn cell(h: &PadHit) -> usize {
    h.y as usize * GRID_W + h.x as usize
}

/// Cells within `r` of a pad (the pad itself at least).
fn disc(h: &PadHit, r: f64, mut f: impl FnMut(usize, f64)) {
    for i in 0..GRID_LEDS {
        let d = ((i % GRID_W) as f64 - h.x as f64).hypot((i / GRID_W) as f64 - h.y as f64);
        if d <= r + 1e-9 {
            f(i, d);
        }
    }
}

/// Short-lived effects use the bounded history; paint uses persistent state.
pub fn paint_pads(look: &PadsLook, input: &PadInput, alpha: f64, layer: &mut LayerFrame) {
    let (presses, now) = (&input.history, input.now_us);
    match look.mode {
        PadsMode::Heat => {
            let mut v = [0.0f64; GRID_LEDS];
            let decay = look.decay_us.max(1) as f64;
            for h in presses {
                v[cell(h)] += look.gain * (1.0 - (now - h.t_us) as f64 / decay).max(0.0);
            }
            for (i, v) in v.iter().enumerate() {
                let v = v.min(1.0);
                if v > 0.0 {
                    // Fades in over the first 0.15 so a cold cell does not pop in at color_low.
                    put(layer, i, lerp(look.low, look.high, v), alpha * (v / 0.15).min(1.0));
                }
            }
        }
        PadsMode::Plant => {
            for h in presses {
                let age = now - h.t_us;
                let (grow, keep, fade) = (look.attack_us.max(1), look.sustain_us, look.release_us.max(1));
                let (r, a) = if age < grow {
                    (look.size * age as f64 / grow as f64, 1.0)
                } else if age < grow + keep {
                    (look.size, 1.0)
                } else if age < grow + keep + fade {
                    (look.size, 1.0 - (age - grow - keep) as f64 / fade as f64)
                } else {
                    continue;
                };
                disc(h, r + 0.5, |i, d| put(layer, i, look.color, alpha * a * crate::shape::fill_alpha(d - r, 1.0)));
            }
        }
        PadsMode::Paint => {
            for (i, h) in input.state.last.iter().enumerate() {
                if let Some(h) = h {
                    let c = look.palette.as_ref().map_or(look.color, |p| p[((input.state.counts[i] - 1) % p.len() as u64) as usize]);
                    let a = if look.decay_us == 0 { 1.0 } else { 1.0 - (now - h.t_us) as f64 / look.decay_us as f64 };
                    if a > 0.0 {
                        put(layer, i, c, alpha * a);
                    }
                }
            }
        }
    }
}

pub fn paint_cycle(look: &CycleLook, input: &PadInput, alpha: f64, layer: &mut LayerFrame) {
    let paint = |h: &PadHit, k: u64, layer: &mut LayerFrame| {
        let c = look.palette[(k % look.palette.len() as u64) as usize];
        let s = Rgba::from_color(c, alpha as f32);
        disc(h, look.radius, |i, _| layer.px[i] = s);
    };
    if look.lifetime_us == 0 {
        if look.per_pad {
            // Apply oldest first when neighbouring discs overlap.
            let mut last = input.state.last;
            last.sort_by_key(|h| h.map(|h| (h.t_us, h.serial)));
            for h in last.iter().flatten() {
                paint(h, input.state.counts[cell(h)] - 1, layer);
            }
        } else if let Some(h) = input.state.last.iter().flatten().max_by_key(|h| (h.t_us, h.serial)) {
            paint(h, input.state.total - 1, layer);
        }
        return;
    }
    let (presses, now) = (&input.history, input.now_us);
    let mut count = input.state.counts;
    for h in presses { count[cell(h)] -= 1; }
    let base = input.state.total - presses.len() as u64;
    for (n, h) in presses.iter().enumerate() {
        let k = if look.per_pad { count[cell(h)] } else { base + n as u64 };
        count[cell(h)] += 1;
        if now - h.t_us < look.lifetime_us {
            paint(h, k, layer);
        }
    }
}

pub fn paint_constellation(look: &ConstellationLook, presses: &[PadHit], now: i64, alpha: f64, layer: &mut LayerFrame) {
    let fade = |age: i64| {
        let life = age as f64 / look.lifetime_us as f64;
        match look.fade {
            RippleFade::Linear => 1.0 - life,
            RippleFade::Exp => (-4.0 * life).exp() * (1.0 - life),
        }
    };
    let segments = presses.windows(2).filter(|w| now - w[1].t_us < look.lifetime_us).collect::<Vec<_>>();
    let line = if look.dots { 0.6 } else { 1.0 };
    for w in segments.iter().rev().take(look.max_segments) {
        let a = alpha * fade(now - w[1].t_us);
        let (ax, ay, bx, by) = (w[0].x as f64, w[0].y as f64, w[1].x as f64, w[1].y as f64);
        for i in 0..GRID_LEDS {
            let d = crate::shape::segment_distance((i % GRID_W) as f64, (i / GRID_W) as f64, ax, ay, bx, by);
            let k = crate::shape::stroke_alpha(d, look.stroke, 0.0);
            if k > 0.0 {
                put(layer, i, look.color, a * k * line);
            }
        }
        if look.dots {
            put(layer, cell(&w[0]), look.color, a);
            put(layer, cell(&w[1]), look.color, a);
        }
    }
}

/// The input of one triggered clip at clip-local time `local`, on the
/// clip's clock (`now_us = local`): real presses that arrived inside the
/// clip (moved onto this clock, same ages) and the recording's presses up
/// to `local` (repeated every pass with loop). `out` is reused.
pub fn clip_input(real: Option<&PadInput>, span: &ClipSpan, local: i64, recording: Option<(&Recording, bool)>, out: &mut PadInput) {
    out.now_us = local;
    out.hits.clear();
    out.history.clear();
    out.state = Default::default();
    let shift = |t: i64, real_now: i64| local - (real_now - t);
    if let Some(r) = real {
        let window = (span.start as i64, span.end as i64);
        if r.windows.binary_search(&window).is_ok() {
            // Runtime/preview count every arrival, before history eviction.
            out.state = r.scopes.get(&window).map(|s| s.state.clone()).unwrap_or_default();
        } else {
            // Headless callers may supply unregistered input. Accumulate the
            // available window across calls instead of losing it after 60 s.
            let scope = out.scopes.entry(window).or_default();
            if scope.generation != r.generation || scope.serial > r.serial {
                *scope = ScopedPads { generation: r.generation, ..Default::default() };
            }
            for h in &r.history {
                if h.serial > scope.serial && h.show_us.is_some_and(|s| span.contains(s)) {
                    scope.state.observe(*h, 1);
                }
            }
            scope.serial = r.serial;
            out.state = scope.state.clone();
        }
        let shifted = |h: PadHit| PadHit {
            t_us: shift(h.t_us, r.now_us),
            release_us: h.release_us.map(|t| shift(t, r.now_us)),
            ..h
        };
        for h in out.state.last.iter_mut().flatten() { *h = shifted(*h); }
        out.history.extend(r.history.iter().filter(|h| h.show_us.is_some_and(|s| span.contains(s))).map(|h| shifted(*h)));
    }
    if let Some((rec, looping)) = recording {
        let period = rec.length() as i64;
        // One cursor per recorded event; merge the newest occurrences first.
        // Work is O((events + HISTORY_CAPACITY) * log(events)), independent of
        // the loop period and elapsed show time. Never expand all repetitions.
        let mut newest = BinaryHeap::with_capacity(rec.events.len());
        let hit = |i: usize, t: i64| {
            let e = &rec.events[i];
            let release = t + e.duration_us.unwrap_or(0) as i64;
            PadHit { serial: u64::MAX - rec.events.len() as u64 + i as u64, x: e.x, y: e.y, velocity: e.velocity, t_us: t, release_us: (release <= local).then_some(release), show_us: Some(span.start as i64 + t) }
        };
        for (i, e) in rec.events.iter().enumerate() {
            let t0 = e.t_us as i64;
            if t0 > local {
                continue;
            }
            let k = if looping { (local - t0) / period } else { 0 };
            let t = t0 + k * period;
            // Accumulate old repetitions arithmetically, including those outside
            // the short history. Persistent paint/cycle must survive that window.
            out.state.observe(hit(i, t), k as u64 + 1);
            if local - t <= PadInput::HISTORY_AGE_US {
                newest.push((t, i, k));
            }
        }
        for _ in 0..PadInput::HISTORY_CAPACITY {
            let Some((t, i, k)) = newest.pop() else { break };
            out.history.push(hit(i, t));
            if k > 0 && local - (t - period) <= PadInput::HISTORY_AGE_US {
                newest.push((t - period, i, k - 1));
            }
        }
    }
    out.history.sort_by_key(|h| (h.t_us, h.serial));
    if out.history.len() > PadInput::HISTORY_CAPACITY {
        out.history.drain(..out.history.len() - PadInput::HISTORY_CAPACITY);
    }
    out.hits.extend(out.history.iter().rev().take(PadInput::CAPACITY).filter(|h| local - h.t_us <= PadInput::MAX_AGE_US).copied());
    out.hits.reverse();
}

/// A recording from pad events stamped with the show time they arrived at
/// (oldest first), from `start_us` on: each press with the time it was held
/// (a press never released is held to the last event). At most
/// `MAX_RECORDED` presses; none before `start_us`.
pub fn recording_from(events: &[(i64, crate::render::PadEvent)], start_us: i64) -> Recording {
    let end_us = events.last().map_or(start_us, |e| e.0);
    let mut recording = recording_until(events, start_us, end_us);
    // Legacy CLI import without an explicit stop time keeps its inferred length.
    recording.length_us = None;
    recording
}

/// Capture the entire recording interval, including silence after the last
/// event. Held pads are released at the stop, not at the last input event.
pub fn recording_until(events: &[(i64, crate::render::PadEvent)], start_us: i64, end_us: i64) -> Recording {
    use crate::render::PadEvent;
    let mut out: Vec<crate::model::RecordedPad> = Vec::new();
    let mut held: Vec<(u8, u8, usize)> = Vec::new();
    let end = end_us.max(start_us);
    for &(show_us, ev) in events {
        if show_us < start_us || show_us > end { continue; }
        let t = show_us - start_us;
        match ev {
            PadEvent::Press { x, y, velocity, .. } if t >= 0 && out.len() < crate::validation::MAX_RECORDED => {
                held.push((x, y, out.len()));
                out.push(crate::model::RecordedPad { t_us: t as u64, x, y, velocity: velocity.max(1), duration_us: None });
            }
            PadEvent::Release { x, y, .. } => {
                if let Some(k) = held.iter().rposition(|h| h.0 == x && h.1 == y) {
                    let (_, _, i) = held.remove(k);
                    out[i].duration_us = Some((t.max(0) as u64).saturating_sub(out[i].t_us));
                }
            }
            PadEvent::ReleaseAll { .. } | PadEvent::Reset => {
                for (_, _, i) in held.drain(..) {
                    out[i].duration_us = Some((t.max(0) as u64).saturating_sub(out[i].t_us));
                }
            }
            _ => {}
        }
    }
    for (_, _, i) in held {
        out[i].duration_us = Some(((end - start_us).max(0) as u64).saturating_sub(out[i].t_us));
    }
    // A zero hold is a tap: written without duration.
    for e in &mut out {
        e.duration_us = e.duration_us.filter(|d| *d > 0);
    }
    Recording { events: out, length_us: Some((end - start_us).max(1) as u64) }
}

/// Grid size check for the painters above.
const _: () = assert!(GRID_W * GRID_H == GRID_LEDS);

#[cfg(test)]
mod tests {
    use super::*;

    fn press(x: u8, y: u8, t_us: i64) -> PadHit {
        PadHit::press(x, y, 100, t_us)
    }

    fn input(presses: &[PadHit], now: i64) -> PadInput {
        let mut input = PadInput::default();
        for h in presses { input.push(*h); }
        input.advance(now);
        input
    }

    fn lit(l: &LayerFrame) -> Vec<usize> {
        (0..GRID_LEDS).filter(|i| l.px[*i].a > 0.0).collect()
    }

    fn pads(mode: PadsMode, f: impl FnOnce(&mut PadsParams)) -> PadsLook {
        let mut p = PadsParams { mode, color_low: None, color_high: None, color: None, palette: None, gain: None, decay_ms: None, attack_ms: None, sustain_ms: None, release_ms: None, size: None };
        f(&mut p);
        PadsLook::compile(&p).unwrap()
    }

    #[test]
    fn heat_adds_up_and_cools() {
        let look = pads(PadsMode::Heat, |p| {
            p.gain = Some(0.5);
            p.decay_ms = Some(1000);
        });
        let paint = |presses: &[PadHit], now| {
            let mut l = LayerFrame::default();
            paint_pads(&look, &input(presses, now), 1.0, &mut l);
            l.px[0]
        };
        // One press: 0.5; two at once: 1 (the high color); after the decay: nothing.
        let one = paint(&[press(0, 0, 0)], 0);
        let two = paint(&[press(0, 0, 0), press(0, 0, 0)], 0);
        assert!(two.r > one.r && two.a == 1.0);
        assert_eq!(paint(&[press(0, 0, 0)], 1_000_000).a, 0.0);
    }

    #[test]
    fn plant_grows_and_paint_cycles_its_palette() {
        let look = pads(PadsMode::Plant, |p| {
            p.attack_ms = Some(1000);
            p.sustain_ms = Some(1000);
            p.release_ms = Some(1000);
            p.size = Some(2.0);
        });
        let size = |now| {
            let mut l = LayerFrame::default();
            paint_pads(&look, &input(&[press(3, 3, 0)], now), 1.0, &mut l);
            lit(&l).len()
        };
        assert!(size(100_000) < size(1_500_000), "grows");
        assert_eq!(size(3_100_000), 0, "gone after attack + sustain + release");
        let paint = pads(PadsMode::Paint, |p| p.palette = Some(vec!["#FF0000".into(), "#00FF00".into()]));
        let mut l = LayerFrame::default();
        paint_pads(&paint, &input(&[press(1, 0, 0), press(1, 0, 10), press(2, 0, 20)], 30), 1.0, &mut l);
        assert_eq!((l.px[1].g, l.px[2].r), (1.0, 1.0), "second press on pad 1 is green; pad 2 starts red");
    }

    #[test]
    fn cycle_counts_and_constellation_links() {
        let cyc = CycleLook::compile(&CycleParams { palette: vec!["#FF0000".into(), "#0000FF".into()], scope: CycleScope::Global, lifetime_ms: 0, radius: 0.0 }).unwrap();
        let mut l = LayerFrame::default();
        paint_cycle(&cyc, &input(&[press(0, 0, 0), press(5, 5, 10)], 20), 1.0, &mut l);
        assert_eq!(lit(&l), vec![45], "until the next press: only the newest");
        assert_eq!(l.px[45].b, 1.0, "second press: second color");
        let con = ConstellationLook::compile(&ConstellationParams { color: "#FFFFFF".into(), stroke: 1.0, lifetime_ms: 1000, fade: RippleFade::Linear, max_segments: 4, dots: true }).unwrap();
        let mut l = LayerFrame::default();
        paint_constellation(&con, &[press(0, 0, 0), press(7, 0, 100)], 100, 1.0, &mut l);
        assert_eq!(lit(&l), (0..8).collect::<Vec<_>>(), "the bottom row joined");
        assert!(l.px[0].a > l.px[3].a, "end points brighter");
    }

    #[test]
    fn recordings_from_stamped_events() {
        use crate::render::PadEvent;
        let ev = [
            (900, PadEvent::Press { x: 0, y: 0, velocity: 50, t_us: 0 }),
            (1_000, PadEvent::Press { x: 1, y: 2, velocity: 100, t_us: 0 }),
            (1_500, PadEvent::Release { x: 1, y: 2, t_us: 0 }),
            (2_000, PadEvent::Press { x: 3, y: 3, velocity: 0, t_us: 0 }),
            (2_000, PadEvent::Release { x: 3, y: 3, t_us: 0 }),
            (2_500, PadEvent::Press { x: 4, y: 4, velocity: 90, t_us: 0 }),
            (3_000, PadEvent::Release { x: 9, y: 9, t_us: 0 }),
        ];
        let r = recording_from(&ev, 1_000);
        let got: Vec<(u64, u8, u8, Option<u64>)> = r.events.iter().map(|e| (e.t_us, e.x, e.velocity, e.duration_us)).collect();
        assert_eq!(got, vec![(0, 1, 100, Some(500)), (1_000, 3, 1, None), (1_500, 4, 90, Some(500))], "before the start dropped; held to the end");
    }

    #[test]
    fn clip_input_plays_the_recording_on_the_clip_clock() {
        use crate::model::RecordedPad;
        let span = ClipSpan { start: 10_000_000, end: 20_000_000, fade_in: 0, fade_out: 0, order: 0 };
        let rec = Recording { events: vec![RecordedPad { t_us: 0, x: 1, y: 1, velocity: 100, duration_us: Some(200_000) }, RecordedPad { t_us: 500_000, x: 2, y: 2, velocity: 90, duration_us: None }], length_us: Some(1_000_000) };
        let mut out = PadInput::default();
        // 2.6 s into the clip with loop: passes at 0, 1, 2 s.
        clip_input(None, &span, 2_600_000, Some((&rec, true)), &mut out);
        let ts: Vec<i64> = out.history.iter().map(|h| h.t_us).collect();
        assert_eq!(ts, vec![0, 500_000, 1_000_000, 1_500_000, 2_000_000, 2_500_000]);
        assert_eq!(out.history[4].release_us, Some(2_200_000));
        // Without loop: one pass. A real press inside the clip joins with the same age.
        let mut real = PadInput::default();
        real.apply(crate::render::PadEvent::Press { x: 7, y: 7, velocity: 100, t_us: 5_000 }, Some(12_000_000));
        real.advance(105_000);
        clip_input(Some(&real), &span, 2_600_000, Some((&rec, false)), &mut out);
        assert_eq!(out.history.iter().map(|h| (h.x, h.t_us)).collect::<Vec<_>>(), vec![(1, 0), (2, 500_000), (7, 2_500_000)]);
    }

    #[test]
    fn microsecond_recording_loop_has_bounded_buffers_and_exact_counts() {
        let span = ClipSpan { start: 0, end: 1_000_000_000, fade_in: 0, fade_out: 0, order: 0 };
        let rec = Recording { events: vec![crate::model::RecordedPad { t_us: 0, x: 0, y: 0, velocity: 100, duration_us: None }], length_us: Some(1) };
        let mut out = PadInput::default();
        clip_input(None, &span, 120_000_000, Some((&rec, true)), &mut out);
        assert_eq!(out.history.len(), PadInput::HISTORY_CAPACITY);
        assert_eq!(out.hits.len(), PadInput::CAPACITY);
        assert!(out.history.capacity() <= 2 * PadInput::HISTORY_CAPACITY);
        assert!(out.hits.capacity() <= PadInput::CAPACITY);
        assert_eq!(out.history.first().unwrap().t_us, 120_000_000 - 1023);
        assert_eq!(out.state.total, 120_000_001);
        // A backwards seek recomputes the state, rather than accumulating it.
        clip_input(None, &span, 4, Some((&rec, true)), &mut out);
        assert_eq!(out.state.total, 5);
        assert_eq!(out.history.len(), 5);
    }

    #[test]
    fn bounded_recording_merge_matches_expansion_with_live_input_and_ties() {
        let span = ClipSpan { start: 0, end: 1_000_000, fade_in: 0, fade_out: 0, order: 0 };
        let rec = Recording { events: vec![
            crate::model::RecordedPad { t_us: 0, x: 0, y: 0, velocity: 100, duration_us: Some(7) },
            crate::model::RecordedPad { t_us: 4, x: 1, y: 0, velocity: 90, duration_us: None },
            crate::model::RecordedPad { t_us: 4, x: 2, y: 0, velocity: 80, duration_us: None },
        ], length_us: Some(10) };
        let mut real = PadInput::default();
        real.apply(crate::render::PadEvent::Press { x: 7, y: 7, velocity: 127, t_us: 24 }, Some(24));
        let mut out = PadInput::default();
        for now in [24, 40_004, 11, 40_004] {
            real.advance(now.max(24));
            clip_input(Some(&real), &span, now, Some((&rec, true)), &mut out);
            let mut expected = vec![(now - (real.now_us - 24), 7, 7)];
            for e in &rec.events {
                let mut t = e.t_us as i64;
                while t <= now {
                    expected.push((t, e.x, e.y));
                    t += 10;
                }
            }
            expected.sort_by_key(|e| e.0);
            let count = expected.len();
            if count > 1024 { expected.drain(..count - 1024); }
            assert_eq!(out.history.iter().map(|h| (h.t_us, h.x, h.y)).collect::<Vec<_>>(), expected);
            assert_eq!(out.state.total, count as u64);
            assert!(out.history.capacity() <= 2 * PadInput::HISTORY_CAPACITY);
        }
    }

    #[test]
    fn persistent_paint_and_cycle_survive_age_capacity_and_reset() {
        let paint = pads(PadsMode::Paint, |_| {});
        let cycle = CycleLook::compile(&CycleParams { palette: vec!["#FF0000".into(), "#00FF00".into(), "#0000FF".into()], scope: CycleScope::Global, lifetime_ms: 0, radius: 0.0 }).unwrap();
        let mut src = input(&[press(7, 7, 0)], 0);
        for n in 1..=1024 { src.push(press(0, 0, n)); }
        src.advance(61_000_000);
        assert!(src.history.is_empty());
        let mut frame = LayerFrame::default();
        paint_pads(&paint, &src, 1.0, &mut frame);
        assert_eq!(lit(&frame), vec![0, 63]);
        frame.clear();
        paint_cycle(&cycle, &src, 1.0, &mut frame);
        assert_eq!(frame.px[0].g, 1.0, "1025th input still uses the second colour");
        // A new finite-duration cycle also keeps its ordinal after expiration.
        src.push(press(0, 0, 61_000_000));
        frame.clear();
        let mut finite = cycle.clone(); finite.lifetime_us = 1_000_000;
        paint_cycle(&finite, &src, 1.0, &mut frame);
        assert_eq!(frame.px[0].b, 1.0);
        src.apply(crate::render::PadEvent::Reset, None);
        frame.clear();
        paint_pads(&paint, &src, 1.0, &mut frame);
        paint_cycle(&cycle, &src, 1.0, &mut frame);
        assert!(lit(&frame).is_empty());
        assert_eq!(src.state.total, 0);
    }

    #[test]
    fn live_clip_state_is_scoped_persistent_and_resettable() {
        let a = ClipSpan { start: 1_000_000, end: 200_000_000, fade_in: 0, fade_out: 0, order: 0 };
        let b = ClipSpan { start: 3_000_000, ..a };
        let mut real = PadInput::default();
        let mut out = PadInput::default();
        for (n, show) in [500_000, 2_000_000, 4_000_000].into_iter().enumerate() {
            real.apply(crate::render::PadEvent::Press { x: n as u8, y: 0, velocity: 100, t_us: n as i64 * 1000 }, Some(show));
        }
        real.advance(3000);
        clip_input(Some(&real), &a, 4_000_000, None, &mut out);
        assert_eq!(out.state.total, 2);
        clip_input(Some(&real), &b, 2_000_000, None, &mut out);
        assert_eq!(out.state.total, 1);
        real.advance(70_000_000);
        assert!(real.history.is_empty());
        clip_input(Some(&real), &a, 80_000_000, None, &mut out);
        assert_eq!(out.state.total, 2);
        assert!(out.state.last[1].is_some());
        clip_input(Some(&real), &b, 78_000_000, None, &mut out);
        assert_eq!(out.state.total, 1);
        real.apply(crate::render::PadEvent::Reset, None);
        clip_input(Some(&real), &a, 80_000_000, None, &mut out);
        assert_eq!(out.state.total, 0);
    }

    #[test]
    fn recorded_paint_persists_without_replaying_expired_events() {
        let span = ClipSpan { start: 0, end: 200_000_000, fade_in: 0, fade_out: 0, order: 0 };
        let rec = Recording { events: vec![crate::model::RecordedPad { t_us: 0, x: 2, y: 3, velocity: 100, duration_us: None }], length_us: Some(100_000_000) };
        let mut out = PadInput::default();
        clip_input(None, &span, 70_000_000, Some((&rec, false)), &mut out);
        assert!(out.history.is_empty());
        let mut frame = LayerFrame::default();
        paint_pads(&pads(PadsMode::Paint, |_| {}), &out, 1.0, &mut frame);
        assert_eq!(lit(&frame), vec![26]);
    }

    #[test]
    fn recording_stop_keeps_trailing_silence_and_releases_held_pads() {
        use crate::render::PadEvent;
        let events = vec![
            (10_000_000, PadEvent::Press { x: 0, y: 0, velocity: 100, t_us: 0 }),
            (11_000_000, PadEvent::Release { x: 0, y: 0, t_us: 1_000_000 }),
        ];
        let rec = recording_until(&events, 10_000_000, 14_000_000);
        assert_eq!(rec.length_us, Some(4_000_000));
        assert_eq!(rec.events[0].duration_us, Some(1_000_000));
        let held = recording_until(&events[..1], 10_000_000, 14_000_000);
        assert_eq!(held.events[0].duration_us, Some(4_000_000));
        assert_eq!(recording_from(&events, 10_000_000).length(), 1_000_000, "old import semantics stay intact");
    }
}
