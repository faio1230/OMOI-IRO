//! `builtin.meter` (0.1.3, docs/14 §4-9): Live's output meter of the master
//! or of an `[LED]` track, from the bridge, as a bar, a glow or an orb.
//! Smoothing (attack / release) and the peak hold keep a small state per
//! layer between frames, on the monotonic clock of the audio input: like
//! the pad states, the picture follows what is heard, not the show time.

use crate::model::{MeterChannel, MeterMode, MeterParams, MeterSource};
use crate::render::AudioInput;
use crate::surface::{LayerFrame, Rgb, Rgba, GRID_H, GRID_LEDS, GRID_W};

#[derive(Clone, Debug, PartialEq)]
pub struct MeterLook {
    pub track: Option<String>,
    pub channel: MeterChannel,
    pub mode: MeterMode,
    pub low: Rgb,
    pub high: Rgb,
    pub gain: f64,
    pub floor: f64,
    pub attack_us: f64,
    pub release_us: f64,
    /// Bit x set = column x used (bar).
    pub columns: u8,
    pub peak_hold_us: i64,
}

impl MeterLook {
    pub fn compile(p: &MeterParams) -> Option<MeterLook> {
        Some(MeterLook {
            track: (p.source == MeterSource::Track).then(|| p.track_name.clone().unwrap_or_default()),
            channel: p.channel,
            mode: p.mode,
            low: Rgb::from_hex(&p.color_low)?,
            high: Rgb::from_hex(&p.color_high)?,
            gain: p.gain,
            floor: p.floor,
            attack_us: p.attack_ms as f64 * 1000.0,
            release_us: p.release_ms as f64 * 1000.0,
            columns: p.columns.as_ref().map_or(0xFF, |c| c.iter().filter(|x| **x < 8).fold(0, |m, x| m | 1 << x)),
            peak_hold_us: p.peak_hold_ms.unwrap_or(0) as i64 * 1000,
        })
    }

    /// The level 0..=1, or None without that source: a gate and stretch,
    /// `clamp((raw − floor) / (1 − floor), 0, 1) × gain`, at most 1. Live's
    /// smoothed meters sit high (0.75..0.92 under a kick), so a floor that only
    /// cut below it would leave nothing to react to.
    pub fn level(&self, audio: &AudioInput) -> Option<f64> {
        let [l, r] = match &self.track {
            None => audio.master,
            Some(name) => audio.tracks.iter().find(|(n, _)| n == name)?.1,
        };
        let v = match self.channel {
            MeterChannel::Mix => l.max(r),
            MeterChannel::Left => l,
            MeterChannel::Right => r,
        } as f64;
        let span = (1.0 - self.floor).max(1e-6);
        Some((((v - self.floor) / span).clamp(0.0, 1.0) * self.gain).min(1.0))
    }
}

/// Smoothed level and peak of one layer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterEnv {
    pub value: f64,
    pub peak: f64,
    pub peak_at: i64,
    last_us: Option<i64>,
}

impl MeterEnv {
    /// Move towards `target` with the attack or release time (0 = at once).
    pub fn update(&mut self, target: f64, now_us: i64, look: &MeterLook) {
        let dt = match self.last_us {
            Some(t) if now_us > t && now_us - t < 1_000_000 => (now_us - t) as f64,
            // First frame, or after a gap: no smoothing from stale state.
            _ => f64::INFINITY,
        };
        self.last_us = Some(now_us);
        let tau = if target > self.value { look.attack_us } else { look.release_us };
        let k = if tau <= 0.0 || dt.is_infinite() { 1.0 } else { 1.0 - (-dt / tau).exp() };
        self.value += (target - self.value) * k;
        if self.value >= self.peak || now_us - self.peak_at > look.peak_hold_us {
            self.peak = self.value;
            self.peak_at = now_us;
        }
    }
}

fn lerp(a: Rgb, b: Rgb, k: f64) -> Rgb {
    let k = k.clamp(0.0, 1.0) as f32;
    Rgb::new(a.r + (b.r - a.r) * k, a.g + (b.g - a.g) * k, a.b + (b.b - a.b) * k)
}

/// Update `env` from `audio` and draw. Without the source the layer is
/// transparent and the state starts again from 0.
pub fn paint_meter(look: &MeterLook, audio: Option<&AudioInput>, env: &mut MeterEnv, alpha: f64, layer: &mut LayerFrame) {
    let Some((audio, target)) = audio.and_then(|a| Some((a, look.level(a)?))) else {
        *env = MeterEnv::default();
        return;
    };
    env.update(target, audio.now_us, look);
    let v = env.value;
    match look.mode {
        MeterMode::Bar => {
            let rows = (v * GRID_H as f64).round() as usize;
            let peak_row = ((env.peak * GRID_H as f64).round() as usize).checked_sub(1).filter(|_| look.peak_hold_us > 0);
            for i in 0..GRID_LEDS {
                let (x, y) = (i % GRID_W, i / GRID_W);
                if look.columns >> x & 1 == 0 {
                    continue;
                }
                if y < rows {
                    layer.px[i] = Rgba::from_color(lerp(look.low, look.high, y as f64 / (GRID_H - 1) as f64), alpha as f32);
                } else if peak_row == Some(y) {
                    layer.px[i] = Rgba::from_color(look.high, alpha as f32);
                }
            }
        }
        MeterMode::Glow => {
            let s = Rgba::from_color(lerp(look.low, look.high, v), (v * alpha) as f32);
            layer.px.iter_mut().for_each(|p| *p = s);
        }
        MeterMode::Orb => {
            // Nothing at 0 (silence, or below the floor): no idle ball.
            if v < 1e-3 {
                return;
            }
            let r = 0.5 + v * 3.5;
            let c = lerp(look.low, look.high, v);
            for (i, px) in layer.px.iter_mut().enumerate() {
                let d = ((i % GRID_W) as f64 - 3.5).hypot((i / GRID_W) as f64 - 3.5);
                let a = crate::shape::fill_alpha(d - r, 0.5);
                if a > 0.0 {
                    *px = Rgba::from_color(c, (a * alpha) as f32);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(f: impl FnOnce(&mut MeterParams)) -> MeterLook {
        let mut p = MeterParams { source: MeterSource::Master, track_name: None, channel: MeterChannel::Mix, mode: MeterMode::Bar, color_low: "#00FF00".into(), color_high: "#FF0000".into(), gain: 1.0, floor: 0.0, attack_ms: 0, release_ms: 0, columns: None, peak_hold_ms: None };
        f(&mut p);
        MeterLook::compile(&p).unwrap()
    }

    fn audio(now_us: i64, master: [f32; 2]) -> AudioInput {
        AudioInput { now_us, master, tracks: vec![("Kick".into(), [0.9, 0.1])] }
    }

    fn lit(l: &LayerFrame) -> Vec<usize> {
        (0..GRID_LEDS).filter(|i| l.px[*i].a > 0.0).collect()
    }

    #[test]
    fn levels_from_source_channel_gain_and_floor() {
        let a = audio(0, [0.2, 0.4]);
        assert_eq!(look(|_| {}).level(&a), Some(0.4f32 as f64), "mix = the louder");
        assert_eq!(look(|p| p.channel = MeterChannel::Left).level(&a), Some(0.2f32 as f64));
        assert_eq!(look(|p| p.gain = 4.0).level(&a), Some(1.0), "at most 1");
        assert_eq!(look(|p| p.floor = 0.5).level(&a), Some(0.0), "gated");
        // Stretched above the floor: 0.75 with floor 0.5 is half way, then the gain.
        let hi = audio(0, [0.75, 0.75]);
        assert_eq!(look(|p| p.floor = 0.5).level(&hi), Some(0.5));
        assert_eq!(look(|p| { p.floor = 0.5; p.gain = 1.5 }).level(&hi), Some(0.75));
        let kick = look(|p| {
            p.source = MeterSource::Track;
            p.track_name = Some("Kick".into());
        });
        assert_eq!(kick.level(&a), Some(0.9f32 as f64));
        assert_eq!(look(|p| { p.source = MeterSource::Track; p.track_name = Some("Snare".into()) }).level(&a), None);
    }

    #[test]
    fn bar_glow_orb_and_nothing_without_meters() {
        let mut env = MeterEnv::default();
        let mut l = LayerFrame::default();
        paint_meter(&look(|p| p.columns = Some(vec![0])), Some(&audio(0, [0.5, 0.5])), &mut env, 1.0, &mut l);
        assert_eq!(lit(&l), vec![0, 8, 16, 24], "half of column 0");
        let mut l = LayerFrame::default();
        paint_meter(&look(|p| p.mode = MeterMode::Glow), Some(&audio(0, [0.25, 0.0])), &mut MeterEnv::default(), 1.0, &mut l);
        assert_eq!((lit(&l).len(), l.px[0].a), (64, 0.25));
        let orb = |v| {
            let mut l = LayerFrame::default();
            paint_meter(&look(|p| p.mode = MeterMode::Orb), Some(&audio(0, [v, v])), &mut MeterEnv::default(), 1.0, &mut l);
            lit(&l).len()
        };
        assert!(orb(0.0) < orb(1.0));
        assert_eq!(orb(0.0), 0, "no idle ball at 0");
        let mut l = LayerFrame::default();
        paint_meter(&look(|_| {}), None, &mut env, 1.0, &mut l);
        assert!(lit(&l).is_empty() && env == MeterEnv::default());
    }

    #[test]
    fn attack_release_and_peak_hold() {
        let lk = look(|p| {
            p.attack_ms = 100;
            p.release_ms = 1000;
            p.peak_hold_ms = Some(500);
        });
        let mut env = MeterEnv::default();
        env.update(0.0, 0, &lk);
        env.update(1.0, 100_000, &lk);
        assert!((env.value - (1.0 - (-1.0f64).exp())).abs() < 1e-9, "one attack time: 63 %");
        env.update(1.0, 2_000_000, &lk); // a gap of more than 1 s: no stale smoothing
        assert_eq!(env.value, 1.0);
        env.update(0.0, 2_100_000, &lk);
        assert!(env.value > 0.85 && env.peak == 1.0, "slow release, peak held");
        env.update(0.0, 2_700_000, &lk);
        assert!(env.peak < 1.0, "peak released after 0.5 s");
    }
}
