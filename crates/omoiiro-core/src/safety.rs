//! Safety limits (docs/12 "安全性"): a brightness ceiling and a flashing
//! speed limit for this terminal.
//!
//! * Brightness: every color channel of the final frame is clipped to
//!   `max_brightness` before device quantization (a ceiling, not a gain).
//! * Flashing is judged statically from the show, not from rendered frames:
//!   Pulse flashes at `tempo / 60 / beats_per_cycle` Hz; a Chase band passes
//!   one cell `(8 / width) / period_s` times per second; Strobe flashes at
//!   `rate_hz` or `tempo / 60 / beats_per_cycle`; Noise cells blink at
//!   `rate_hz / 2`; Rain flicker 4 Hz, Orb dither `rate / 2` (Wave: `|speed| × 2`,
//!   warned about but never muted); Regions one switch per step (none with
//!   `soft >= 0.5`). A preset above
//!   `max_flash_hz` gives a warning (with the value) for each layer that
//!   uses it. Only when `limit_flash` is on are those layers muted
//!   (opacity 0 at render time); the show itself is never rewritten.
//! * 3 Hz is the default: whole-field flashing faster than that is treated
//!   as fast flashing (common photosensitivity guidance).

use crate::model::{Content, Effect, Param, Show, StrobeMode};
use crate::surface::Frame;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SafetyLimits {
    /// Ceiling of every color channel, 0..=1.
    pub max_brightness: f64,
    /// Flashing faster than this (Hz) is warned about.
    pub max_flash_hz: f64,
    /// Mute layers whose presets exceed `max_flash_hz`.
    pub limit_flash: bool,
}

impl Default for SafetyLimits {
    fn default() -> Self {
        SafetyLimits { max_brightness: 1.0, max_flash_hz: 3.0, limit_flash: false }
    }
}

impl SafetyLimits {
    /// Problems with the values themselves (for the settings check).
    pub fn issues(&self) -> Vec<String> {
        let mut v = Vec::new();
        if !(self.max_brightness.is_finite() && (0.05..=1.0).contains(&self.max_brightness)) {
            v.push(format!("輝度の上限は 0.05〜1（{}）", self.max_brightness));
        }
        if !(self.max_flash_hz.is_finite() && (0.5..=30.0).contains(&self.max_flash_hz)) {
            v.push(format!("点滅の上限は 0.5〜30 Hz（{}）", self.max_flash_hz));
        }
        v
    }
}

/// Clip every channel to `cap` (0..=1). No allocation.
pub fn cap_frame(frame: &mut Frame, cap: f32) {
    if cap >= 1.0 {
        return;
    }
    let cap = cap.max(0.0);
    for c in frame.leds.iter_mut() {
        c.r = c.r.min(cap);
        c.g = c.g.min(cap);
        c.b = c.b.min(cap);
    }
}

/// How an effect flashes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FlashSource {
    /// Depends on the tempo.
    Pulse { beats_per_cycle: f64 },
    /// Fixed rate per cell.
    Chase { hz: f64 },
    /// Strobe at a fixed rate.
    Strobe { hz: f64 },
    /// Strobe on the beat (depends on the tempo).
    StrobeBeat { beats_per_cycle: f64 },
    /// Noise cells blink at half the pattern rate.
    Noise { hz: f64 },
    /// Per-cell grain: rain flicker (4 Hz) or orb dither (rate / 2).
    Grain { hz: f64 },
    /// Wave: `|speed| × 2` per cell. A warning only: never muted (continuous motion).
    Wave { hz: f64 },
    /// Path: a spot passing a cell. A warning only, like Wave.
    Path { hz: f64 },
    /// Regions stepping by time: one switch per step.
    Step { hz: f64 },
    /// Regions stepping on the beat (depends on the tempo).
    StepBeat { beats_per_step: f64 },
}

impl FlashSource {
    pub fn hz(&self, tempo_bpm: f64) -> f64 {
        match self {
            FlashSource::Pulse { beats_per_cycle } | FlashSource::StrobeBeat { beats_per_cycle } => tempo_bpm / 60.0 / beats_per_cycle,
            FlashSource::StepBeat { beats_per_step } => tempo_bpm / 60.0 / beats_per_step,
            FlashSource::Step { hz } => *hz,
            FlashSource::Chase { hz } | FlashSource::Strobe { hz } | FlashSource::Noise { hz } | FlashSource::Grain { hz } | FlashSource::Wave { hz } | FlashSource::Path { hz } => *hz,
        }
    }

    /// Played `k` times faster (a compound clip's time_scale): rates in time
    /// scale, rates on the beat follow the song and do not.
    pub fn scaled(self, k: f64) -> FlashSource {
        match self {
            FlashSource::Chase { hz } => FlashSource::Chase { hz: hz * k },
            FlashSource::Strobe { hz } => FlashSource::Strobe { hz: hz * k },
            FlashSource::Noise { hz } => FlashSource::Noise { hz: hz * k },
            FlashSource::Grain { hz } => FlashSource::Grain { hz: hz * k },
            FlashSource::Wave { hz } => FlashSource::Wave { hz: hz * k },
            FlashSource::Path { hz } => FlashSource::Path { hz: hz * k },
            FlashSource::Step { hz } => FlashSource::Step { hz: hz * k },
            beat => beat,
        }
    }
}

/// `(8 / width) / period_s`: how often the band passes one cell.
pub fn chase_hz(period_us: u64, width: u32) -> f64 {
    let period_s = period_us.max(1) as f64 / 1e6;
    (8.0 / width.clamp(1, 8) as f64) / period_s
}

fn max(p: &Param<f64>) -> f64 {
    p.values().copied().fold(f64::MIN, f64::max)
}

fn max_abs(p: &Param<f64>) -> f64 {
    p.values().map(|v| v.abs()).fold(0.0, f64::max)
}

fn min(p: &Param<f64>) -> f64 {
    p.values().copied().fold(f64::MAX, f64::min)
}

/// How an effect flashes, judged at the worst values of its keyframes
/// (fastest rate, shortest period / step, deepest pulse; a mode switched by
/// keyframes counts as the time mode if it is ever used).
pub fn effect_flash(e: &Effect) -> Option<FlashSource> {
    match e {
        Effect::Solid { .. } => None,
        Effect::Chase { params, .. } => {
            let period = params.period_us.values().copied().min().unwrap_or(1);
            let width = params.width.values().copied().min().unwrap_or(1);
            Some(FlashSource::Chase { hz: chase_hz(period, width) })
        }
        // A Pulse with depth 0 does not change brightness.
        Effect::Pulse { params, .. } if max(&params.depth) <= 0.0 => None,
        Effect::Pulse { params, .. } => Some(FlashSource::Pulse { beats_per_cycle: min(&params.beats_per_cycle) }),
        Effect::Strobe { params, .. } => Some(if params.mode.values().any(|m| *m == StrobeMode::Hz) {
            FlashSource::Strobe { hz: max(&params.rate_hz) }
        } else {
            FlashSource::StrobeBeat { beats_per_cycle: min(&params.beats_per_cycle) }
        }),
        Effect::Noise { params, .. } => Some(FlashSource::Noise { hz: max(&params.rate_hz) / 2.0 }),
        // Driven by pad presses, not by time: not judged statically.
        Effect::Ripple { .. } | Effect::TapGlow { .. } | Effect::Cross { .. } => None,
        Effect::Shape { .. } => None,
        // Continuous motion, not a flash (docs/14 §4-5).
        Effect::Particles { .. } => None,
        // A spot passing a cell: (cells per second) / (spot diameter). A warning only.
        Effect::Path { params, .. } => {
            let length = |k: &crate::model::PathKind| match k {
                crate::model::PathKind::Perimeter => 28.0,
                crate::model::PathKind::Orbit => std::f64::consts::TAU * params.orbit_radius.as_ref().map(max).unwrap_or(3.0),
                crate::model::PathKind::Figure8 => 6.1 * params.orbit_radius.as_ref().map(max).unwrap_or(3.0),
                _ => 1.0,
            };
            let per_s = params.path.values().map(|k| max_abs(&params.speed) * length(k)).fold(0.0, f64::max);
            Some(FlashSource::Path { hz: per_s / (2.0 * min(&params.size).max(0.5)) })
        }
        Effect::Rain { params, .. } if max(&params.flicker) > 0.0 => Some(FlashSource::Grain { hz: 4.0 }),
        Effect::Rain { .. } => None,
        Effect::Wave { params, .. } => Some(FlashSource::Wave { hz: params.speed.values().map(|s| s.abs()).fold(0.0, f64::max) * 2.0 }),
        Effect::Orb { params, .. } if max(&params.dither) > 0.0 => Some(FlashSource::Grain { hz: max(&params.dither_rate_hz) / 2.0 }),
        Effect::Orb { .. } => None,
        // A soft hand-over (>= 0.5) does not switch abruptly.
        Effect::Regions { params, .. } if min(&params.soft) >= 0.5 => None,
        Effect::Regions { params, .. } => Some(if params.step_mode.values().any(|m| *m == crate::model::StepMode::Seconds) {
            FlashSource::Step { hz: 1000.0 / params.step_ms.values().copied().min().unwrap_or(1).max(1) as f64 }
        } else {
            FlashSource::StepBeat { beats_per_step: min(&params.step_beats) }
        }),
    }
}
/// A flashing preset used by a layer, with the clips that use it there.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerPresetFlash {
    pub preset_id: String,
    pub preset_name: String,
    pub source: FlashSource,
    pub clips: Vec<String>,
}

/// Flashing presets per layer (index = `show.layers` index).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlashProfile {
    pub layers: Vec<(String, String, Vec<LayerPresetFlash>)>,
}

impl FlashProfile {
    pub fn of(show: &Show) -> FlashProfile {
        let mut layers = Vec::with_capacity(show.layers.len());
        for l in &show.layers {
            let mut entries: Vec<LayerPresetFlash> = Vec::new();
            // `k`: time scale of a compound clip (1 otherwise); the fastest use is kept.
            let mut add = |preset_id: &str, clip: Option<&str>, k: f64| {
                let Some(p) = show.presets.iter().find(|p| p.id == preset_id) else { return };
                let Some(source) = effect_flash(&p.effect).map(|s| s.scaled(k)) else { return };
                if let Some(e) = entries.iter_mut().find(|e| e.preset_id == preset_id) {
                    e.clips.extend(clip.map(str::to_string).filter(|c| !e.clips.contains(c)));
                    if source.hz(120.0) > e.source.hz(120.0) {
                        e.source = source;
                    }
                    return;
                }
                entries.push(LayerPresetFlash { preset_id: p.id.clone(), preset_name: p.name.clone(), source, clips: clip.map(str::to_string).into_iter().collect() });
            };
            match &l.content {
                Content::Timeline { clips } | Content::Triggered { clips, .. } => {
                    for c in clips {
                        match c.compound_id.as_deref().and_then(|id| show.compounds.iter().find(|x| x.id == id)) {
                            // Every preset inside counts for the placing layer, at its time scale.
                            Some(cp) => {
                                for il in &cp.layers {
                                    match &il.content {
                                        Content::Continuous { preset_id } => add(preset_id, Some(&c.id), c.time_scale.unwrap_or(1.0)),
                                        _ => il.content.clips().into_iter().flatten().for_each(|ic| add(&ic.preset_id, Some(&c.id), c.time_scale.unwrap_or(1.0))),
                                    }
                                }
                            }
                            None => add(&c.preset_id, Some(&c.id), 1.0),
                        }
                    }
                }
                Content::Continuous { preset_id } => add(preset_id, None, 1.0),
            }
            layers.push((l.id.clone(), l.name.clone(), entries));
        }
        FlashProfile { layers }
    }

    /// Highest flashing rate of each layer at `tempo_bpm` (0 = none). No allocation.
    pub fn layer_hz(&self, index: usize, tempo_bpm: f64) -> f64 {
        // Wave is continuous motion: warned about, never muted.
        self.layers.get(index).map(|(_, _, e)| e.iter().filter(|x| !matches!(x.source, FlashSource::Wave { .. } | FlashSource::Path { .. })).map(|x| x.source.hz(tempo_bpm)).fold(0.0, f64::max)).unwrap_or(0.0)
    }

    pub fn warnings(&self, tempo_bpm: f64, max_hz: f64) -> Vec<FlashWarning> {
        let mut out = Vec::new();
        for (layer_id, layer_name, entries) in &self.layers {
            for e in entries {
                let hz = e.source.hz(tempo_bpm);
                if hz > max_hz {
                    let (kind, reason) = match e.source {
                        FlashSource::Pulse { beats_per_cycle } => ("pulse", format!("Pulse {beats_per_cycle} 拍周期 × {tempo_bpm:.1} bpm = {hz:.2} Hz（上限 {max_hz} Hz）")),
                        FlashSource::Chase { .. } => ("chase", format!("Chase の1セルの点滅 {hz:.2} Hz（上限 {max_hz} Hz）")),
                        FlashSource::Strobe { .. } => ("strobe", format!("Strobe {hz:.2} Hz（上限 {max_hz} Hz）")),
                        FlashSource::StrobeBeat { beats_per_cycle } => ("strobe", format!("Strobe {beats_per_cycle} 拍周期 × {tempo_bpm:.1} bpm = {hz:.2} Hz（上限 {max_hz} Hz）")),
                        FlashSource::Noise { .. } => ("noise", format!("Noise のセルの明滅 {hz:.2} Hz（模様の更新の半分、上限 {max_hz} Hz）")),
                        FlashSource::Step { .. } => ("regions", format!("領域ステップの切り替え {hz:.2} Hz（上限 {max_hz} Hz）")),
                        FlashSource::StepBeat { beats_per_step } => ("regions", format!("領域ステップ {beats_per_step} 拍ごと × {tempo_bpm:.1} bpm = {hz:.2} Hz（上限 {max_hz} Hz）")),
                        FlashSource::Grain { .. } => ("grain", format!("粒の揺らぎ {hz:.2} Hz（上限 {max_hz} Hz）")),
                        FlashSource::Path { .. } => ("path", format!("経路の光点がセルを通る {hz:.2} Hz（警告のみ、消灯の対象外。上限 {max_hz} Hz）")),
                        FlashSource::Wave { .. } => ("wave", format!("Wave のセルから見た明滅 {hz:.2} Hz（警告のみ、消灯の対象外。上限 {max_hz} Hz）")),
                    };
                    out.push(FlashWarning {
                        layer_id: layer_id.clone(),
                        layer_name: layer_name.clone(),
                        preset_id: e.preset_id.clone(),
                        preset_name: e.preset_name.clone(),
                        kind,
                        hz: (hz * 100.0).round() / 100.0,
                        limit_hz: max_hz,
                        tempo_bpm,
                        clips: e.clips.clone(),
                        reason,
                    });
                }
            }
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FlashWarning {
    pub layer_id: String,
    pub layer_name: String,
    pub preset_id: String,
    pub preset_name: String,
    pub kind: &'static str,
    pub hz: f64,
    pub limit_hz: f64,
    pub tempo_bpm: f64,
    pub clips: Vec<String>,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChaseParams, Param, Preset, PulseParams, Waveform};
    use crate::surface::Rgb;

    fn demo() -> Show {
        crate::model::demo_for_tests()
    }

    #[test]
    fn flash_rates() {
        assert_eq!(FlashSource::Pulse { beats_per_cycle: 1.0 }.hz(120.0), 2.0);
        assert_eq!(FlashSource::Pulse { beats_per_cycle: 0.25 }.hz(120.0), 8.0);
        assert_eq!(chase_hz(4_000_000, 2), 1.0); // 8 / 2 cells per 4 s
        assert_eq!(chase_hz(1_000_000, 1), 8.0);
        assert_eq!(chase_hz(500_000, 8), 2.0);
    }

    #[test]
    fn cap_clips_channels_not_gain() {
        let mut f = Frame::default();
        f.leds[0] = Rgb::new(1.0, 0.5, 0.2);
        cap_frame(&mut f, 0.6);
        assert_eq!(f.leds[0], Rgb::new(0.6, 0.5, 0.2), "only channels above the ceiling change");
        cap_frame(&mut f, 1.0);
        assert_eq!(f.leds[0], Rgb::new(0.6, 0.5, 0.2));
    }

    #[test]
    fn warnings_follow_tempo_and_limits() {
        let mut s = demo();
        let p = FlashProfile::of(&s);
        // Demo: Pulse 1 beat per cycle on a continuous layer, Chase 4 s / width 2.
        assert!(p.warnings(120.0, 3.0).is_empty(), "2 Hz at 120 bpm");
        let w = p.warnings(200.0, 3.0);
        assert_eq!(w.len(), 1);
        assert_eq!((w[0].kind, w[0].hz), ("pulse", 3.33));
        assert!(w[0].reason.contains("200.0 bpm"));
        // A fast chase.
        s.presets.push(Preset { id: "strobe".into(), name: "速い流れ".into(), effect: Effect::Chase { version: 1, params: ChaseParams { color: "#FFFFFF".to_string().into(), period_us: 500_000.into(), width: 1.into(), axis: crate::model::Axis::X.into(), reverse: None } } });
        if let Content::Timeline { clips } = &mut s.layers[0].content {
            clips[0].preset_id = "strobe".into();
        }
        let p = FlashProfile::of(&s);
        let w = p.warnings(120.0, 3.0);
        assert_eq!(w.len(), 1);
        assert_eq!((w[0].layer_id.as_str(), w[0].hz, w[0].clips.clone()), ("base", 16.0, vec!["intro-base".to_string()]));
        assert_eq!(p.layer_hz(0, 120.0), 16.0);
        // Depth 0 pulse does not flash.
        let quiet = Effect::Pulse { version: 1, params: PulseParams { color: "#FFFFFF".to_string().into(), beats_per_cycle: 0.25.into(), depth: Param::Value(0.0), waveform: Waveform::Sine.into() } };
        assert_eq!(effect_flash(&quiet), None);
    }

    #[test]
    fn strobe_and_noise_flash_rates() {
        use crate::edit::default_effect;
        // Defaults: strobe on the beat (2 Hz at 120 bpm), noise 6 Hz pattern = 3 Hz blinking.
        let strobe = default_effect("builtin.strobe").unwrap();
        assert_eq!(effect_flash(&strobe).unwrap().hz(120.0), 2.0);
        let noise = default_effect("builtin.noise").unwrap();
        assert_eq!(effect_flash(&noise).unwrap(), FlashSource::Noise { hz: 3.0 });
        // A 12 Hz strobe on a layer: warned with its own reason, and the layer rate is 12.
        let mut s = demo();
        let Effect::Strobe { version, mut params } = strobe else { unreachable!() };
        params.mode = crate::model::StrobeMode::Hz.into();
        params.rate_hz = 12.0.into();
        s.presets.push(Preset { id: "fast".into(), name: "速い点滅".into(), effect: Effect::Strobe { version, params } });
        if let Content::Timeline { clips } = &mut s.layers[0].content {
            clips[0].preset_id = "fast".into();
        }
        let p = FlashProfile::of(&s);
        let w = p.warnings(120.0, 3.0);
        assert_eq!(w.len(), 1);
        assert_eq!((w[0].kind, w[0].hz), ("strobe", 12.0));
        assert_eq!(p.layer_hz(0, 120.0), 12.0);
        // Raising the terminal limit lets it through.
        assert!(p.warnings(120.0, 15.0).is_empty());
    }

    #[test]
    fn keyframed_parameters_are_judged_at_their_worst() {
        use crate::model::{Ease, Keyframe};
        let k = |v: &[f64]| Param::Keyframes(v.iter().enumerate().map(|(i, x)| Keyframe { t_us: i as u64 * 1000, v: *x, ease: Ease::Linear }).collect());
        let Effect::Strobe { version, mut params } = crate::edit::default_effect("builtin.strobe").unwrap() else { unreachable!() };
        params.mode = Param::Keyframes(vec![Keyframe { t_us: 0, v: StrobeMode::Beat, ease: Ease::Step }, Keyframe { t_us: 5000, v: StrobeMode::Hz, ease: Ease::Step }]);
        params.rate_hz = k(&[1.0, 12.0, 2.0]);
        assert_eq!(effect_flash(&Effect::Strobe { version, params }), Some(FlashSource::Strobe { hz: 12.0 }), "the time mode is used once, at its fastest");
        let Effect::Regions { version, mut params } = crate::edit::default_effect("builtin.regions").unwrap() else { unreachable!() };
        params.soft = k(&[1.0, 0.2]);
        assert!(effect_flash(&Effect::Regions { version, params }).is_some(), "soft dips under 0.5");
    }

    #[test]
    fn regions_flash_at_the_step_rate_unless_soft() {
        use crate::edit::default_effect;
        // Default: halves on the beat = 2 Hz at 120 bpm.
        let Effect::Regions { version, mut params } = default_effect("builtin.regions").unwrap() else { unreachable!() };
        assert_eq!(effect_flash(&Effect::Regions { version, params: params.clone() }).unwrap().hz(120.0), 2.0);
        params.step_mode = crate::model::StepMode::Seconds.into();
        params.step_ms = 100.into();
        assert_eq!(effect_flash(&Effect::Regions { version, params: params.clone() }), Some(FlashSource::Step { hz: 10.0 }));
        params.soft = 0.5.into();
        assert_eq!(effect_flash(&Effect::Regions { version, params }), None);
    }

    #[test]
    fn rain_wave_orb_flash_rules() {
        use crate::edit::default_effect;
        // Defaults: rain without flicker, still orb without dither: not flashing.
        assert_eq!(effect_flash(&default_effect("builtin.rain").unwrap()), None);
        assert_eq!(effect_flash(&default_effect("builtin.orb").unwrap()), None);
        let Effect::Rain { version, mut params } = default_effect("builtin.rain").unwrap() else { unreachable!() };
        params.flicker = 0.5.into();
        assert_eq!(effect_flash(&Effect::Rain { version, params }), Some(FlashSource::Grain { hz: 4.0 }));
        let Effect::Orb { version, mut params } = default_effect("builtin.orb").unwrap() else { unreachable!() };
        params.dither = 0.3.into();
        params.dither_rate_hz = 10.0.into();
        assert_eq!(effect_flash(&Effect::Orb { version, params }), Some(FlashSource::Grain { hz: 5.0 }));
        // A fast wave is warned about but never muted (layer_hz ignores it).
        let Effect::Wave { version, mut params } = default_effect("builtin.wave").unwrap() else { unreachable!() };
        params.speed = (-3.0).into();
        let mut s = demo();
        s.presets.push(Preset { id: "wave".into(), name: "速い波".into(), effect: Effect::Wave { version, params } });
        if let Content::Timeline { clips } = &mut s.layers[0].content {
            clips[0].preset_id = "wave".into();
        }
        let p = FlashProfile::of(&s);
        let w = p.warnings(120.0, 3.0);
        assert_eq!(w.len(), 1);
        assert_eq!((w[0].kind, w[0].hz), ("wave", 6.0));
        assert_eq!(p.layer_hz(0, 120.0), 0.0, "not muted");
    }

    #[test]
    fn path_is_warned_only_and_shape_never_flashes() {
        use crate::edit::default_effect;
        assert_eq!(effect_flash(&default_effect("builtin.shape").unwrap()), None);
        // Perimeter (28 cells) at 1 lap/s with a 1-cell spot: 28 / 2 = 14 Hz seen by a cell.
        let Effect::Path { version, mut params } = default_effect("builtin.path").unwrap() else { unreachable!() };
        params.speed = 1.0.into();
        assert_eq!(effect_flash(&Effect::Path { version, params: params.clone() }), Some(FlashSource::Path { hz: 14.0 }));
        let mut s = demo();
        s.presets.push(Preset { id: "path".into(), name: "速い経路".into(), effect: Effect::Path { version, params } });
        if let Content::Timeline { clips } = &mut s.layers[0].content {
            clips[0].preset_id = "path".into();
        }
        let p = FlashProfile::of(&s);
        let w = p.warnings(120.0, 3.0);
        assert_eq!((w.len(), w[0].kind, w[0].hz), (1, "path", 14.0));
        assert_eq!(p.layer_hz(0, 120.0), 0.0, "not muted");
    }

    #[test]
    fn compound_flashing_counts_for_the_placing_layer_at_its_time_scale() {
        use crate::model::{Clip, Compound, Layer, Blend};
        let mut s = demo();
        let Effect::Strobe { version, mut params } = crate::edit::default_effect("builtin.strobe").unwrap() else { unreachable!() };
        params.mode = crate::model::StrobeMode::Hz.into();
        params.rate_hz = 2.0.into();
        s.presets.push(Preset { id: "st".into(), name: "st".into(), effect: Effect::Strobe { version, params } });
        s.compounds.push(Compound {
            id: "cp".into(),
            name: "cp".into(),
            duration_us: Some(1_000_000),
            layers: vec![Layer { id: "in".into(), name: "in".into(), target_id: String::new(), enabled: true, opacity: 1.0, blend: Blend::Over, content: Content::Continuous { preset_id: "st".into() }, mask: None }],
        });
        if let Content::Timeline { clips } = &mut s.layers[0].content {
            clips.push(Clip { id: "fast".into(), compound_id: Some("cp".into()), start_us: 0, duration_us: 1_000_000, time_scale: Some(2.0), ..Default::default() });
        }
        let p = FlashProfile::of(&s);
        assert_eq!(p.layer_hz(0, 120.0), 4.0, "2 Hz played twice as fast");
    }

    #[test]
    fn limits_defaults_and_ranges() {
        let l = SafetyLimits::default();
        assert_eq!((l.max_brightness, l.max_flash_hz, l.limit_flash), (1.0, 3.0, false));
        assert!(l.issues().is_empty());
        assert_eq!(SafetyLimits { max_brightness: 0.0, max_flash_hz: 0.0, limit_flash: true }.issues().len(), 2);
        let parsed: SafetyLimits = serde_json::from_str(r#"{"max_flash_hz": 5}"#).unwrap();
        assert_eq!((parsed.max_brightness, parsed.max_flash_hz), (1.0, 5.0));
    }
}
