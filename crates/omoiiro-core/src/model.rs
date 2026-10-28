//! Show data model (docs/04-show-data.md, schemas/show.schema.json 0.1.3).
//!
//! These types are the serde DTO and the domain model at once. Field order
//! follows the Schema so that serialization gives the canonical key order;
//! unknown fields are rejected. Integral numbers stored as `f64` (opacity,
//! depth, bpm, ...) serialize as JSON integers, so an unchanged show saves
//! byte for byte. Structural checks are the JSON Schema's job (omoiiro-store);
//! cross-references and intervals are `validation`'s.

use crate::time::FrameRate;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const SCHEMA_VERSION: &str = "0.1.3";
/// Older versions the store migrates on load (additive changes: only the
/// version is rewritten; 0.1.2 adds effect types, 0.1.3 the triggered layer).
pub const MIGRATABLE_VERSIONS: &[&str] = &["0.1.0", "0.1.1", "0.1.2"];

/// Serialize an `f64` as an integer when it is one (1.0 -> `1`, 0.6 -> `0.6`).
pub fn ser_num<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
    if v.fract() == 0.0 && v.abs() < 9_007_199_254_740_992.0 {
        s.serialize_i64(*v as i64)
    } else {
        s.serialize_f64(*v)
    }
}

/// An effect parameter (U04 provision for automation, docs/12).
///
/// `Value` serializes as exactly that value, so files without automation are
/// unchanged. `Keyframes` (0.1.3, docs/14 §4-2) is `{"keyframes": [{"t_us",
/// "v", "ease"}]}`: `t_us` ascending, 1..=64 points; before the first point
/// the first value holds, after the last the last value; `ease` shapes the
/// way from a point to the next. The time axis is chosen by the renderer:
/// the clip's local time, the show time for continuous layers, an
/// instance's age for input-driven effects.
///
/// Numbers (and integers, rounded) and colors interpolate; enums and bools
/// only step (other eases are refused when reading).
#[derive(Clone, Debug, PartialEq)]
pub enum Param<T> {
    /// A constant value.
    Value(T),
    Keyframes(Vec<Keyframe<T>>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyframe<T> {
    pub t_us: u64,
    pub v: T,
    #[serde(default)]
    pub ease: Ease,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ease {
    #[default]
    Linear,
    Step,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl Ease {
    /// Shape the fraction 0..=1 of a segment.
    pub fn apply(self, f: f64) -> f64 {
        let f = f.clamp(0.0, 1.0);
        match self {
            Ease::Linear => f,
            Ease::Step => 0.0,
            Ease::EaseIn => f * f,
            Ease::EaseOut => 1.0 - (1.0 - f) * (1.0 - f),
            Ease::EaseInOut => f * f * (3.0 - 2.0 * f),
        }
    }
}

/// Most points in one keyframe list.
pub const MAX_KEYFRAMES: usize = 64;

/// How a parameter type follows keyframes.
pub trait Sample: Clone {
    /// Interpolates between points (numbers, colors); otherwise only `step`.
    const INTERPOLATES: bool;
    /// Value at fraction `f` (already eased) from `a` to `b`.
    fn mix(a: &Self, b: &Self, f: f64) -> Self {
        if f >= 1.0 {
            b.clone()
        } else {
            a.clone()
        }
    }
}

impl Sample for f64 {
    const INTERPOLATES: bool = true;
    fn mix(a: &f64, b: &f64, f: f64) -> f64 {
        a + (b - a) * f
    }
}

impl Sample for u32 {
    const INTERPOLATES: bool = true;
    fn mix(a: &u32, b: &u32, f: f64) -> u32 {
        (*a as f64 + (*b as f64 - *a as f64) * f).round() as u32
    }
}

impl Sample for u64 {
    const INTERPOLATES: bool = true;
    fn mix(a: &u64, b: &u64, f: f64) -> u64 {
        (*a as f64 + (*b as f64 - *a as f64) * f).round() as u64
    }
}

/// Colors: interpolated by the renderer in sRGB (`Param::segment`); `at` steps.
impl Sample for String {
    const INTERPOLATES: bool = true;
}

impl Sample for bool {
    const INTERPOLATES: bool = false;
}

macro_rules! step_only {
    ($($t:ty),*) => { $(impl Sample for $t { const INTERPOLATES: bool = false; })* };
}

impl<T> Param<T> {
    /// The constant value, or the first keyframe's value.
    pub fn value(&self) -> &T {
        match self {
            Param::Value(v) => v,
            Param::Keyframes(k) => &k[0].v,
        }
    }

    /// Every value the parameter can take (for range checks and worst cases).
    pub fn values(&self) -> impl Iterator<Item = &T> {
        let (one, many) = match self {
            Param::Value(v) => (Some(v), None),
            Param::Keyframes(k) => (None, Some(k.iter().map(|k| &k.v))),
        };
        one.into_iter().chain(many.into_iter().flatten())
    }

    /// True when the parameter does not change over time.
    pub fn is_constant(&self) -> bool {
        matches!(self, Param::Value(_))
    }

    /// The point in force at `t_us` and, inside a segment, the next point
    /// with the eased fraction towards it.
    pub fn segment(&self, t_us: u64) -> (&T, Option<(&T, f64)>) {
        match self {
            Param::Value(v) => (v, None),
            Param::Keyframes(k) => {
                let i = k.partition_point(|p| p.t_us <= t_us);
                if i == 0 {
                    return (&k[0].v, None);
                }
                let a = &k[i - 1];
                match k.get(i) {
                    Some(b) if b.t_us > a.t_us => (&a.v, Some((&b.v, a.ease.apply((t_us - a.t_us) as f64 / (b.t_us - a.t_us) as f64)))),
                    _ => (&a.v, None),
                }
            }
        }
    }
}

impl<T: Sample> Param<T> {
    /// Value at `t_us` on the parameter's time axis.
    pub fn at(&self, t_us: u64) -> T {
        match self.segment(t_us) {
            (a, None) => a.clone(),
            (a, Some((b, f))) => T::mix(a, b, f),
        }
    }
}

impl<T> From<T> for Param<T> {
    fn from(v: T) -> Param<T> {
        Param::Value(v)
    }
}

#[derive(Serialize)]
struct KeyframesRef<'a, T> {
    keyframes: &'a [Keyframe<T>],
}

impl<T: Serialize> Serialize for Param<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Param::Value(v) => v.serialize(s),
            Param::Keyframes(k) => KeyframesRef { keyframes: k }.serialize(s),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ParamRepr<T> {
    Keyframes(KeyframesOwned<T>),
    Value(T),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyframesOwned<T> {
    keyframes: Vec<Keyframe<T>>,
}

impl<'de, T: Deserialize<'de> + Sample> Deserialize<'de> for Param<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Param<T>, D::Error> {
        use serde::de::Error;
        match ParamRepr::<T>::deserialize(d)? {
            ParamRepr::Value(v) => Ok(Param::Value(v)),
            ParamRepr::Keyframes(KeyframesOwned { keyframes: k }) => {
                if k.is_empty() || k.len() > MAX_KEYFRAMES {
                    return Err(D::Error::custom(format!("keyframes must have 1..={MAX_KEYFRAMES} points (got {})", k.len())));
                }
                if k.windows(2).any(|w| w[1].t_us < w[0].t_us) {
                    return Err(D::Error::custom("keyframes must be in ascending t_us"));
                }
                if !T::INTERPOLATES && k.iter().any(|p| p.ease != Ease::Step) {
                    return Err(D::Error::custom("this parameter only steps: every keyframe ease must be \"step\""));
                }
                Ok(Param::Keyframes(k))
            }
        }
    }
}

/// `ser_num` for a numeric parameter (keyframe values too).
pub fn ser_param_num<S: Serializer>(v: &Param<f64>, s: S) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct Num(#[serde(serialize_with = "ser_num")] f64);
    #[derive(Serialize)]
    struct NumKey {
        t_us: u64,
        v: Num,
        ease: Ease,
    }
    match v {
        Param::Value(x) => ser_num(x, s),
        Param::Keyframes(k) => {
            let k: Vec<NumKey> = k.iter().map(|p| NumKey { t_us: p.t_us, v: Num(p.v), ease: p.ease }).collect();
            #[derive(Serialize)]
            struct Keys {
                keyframes: Vec<NumKey>,
            }
            Keys { keyframes: k }.serialize(s)
        }
    }
}

/// `ser_num` for an optional numeric parameter (skipped when absent).
pub fn ser_opt_param_num<S: Serializer>(v: &Option<Param<f64>>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(p) => ser_param_num(p, s),
        None => s.serialize_none(),
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Show {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub schema_version: String,
    pub id: String,
    pub name: String,
    pub seed: u32,
    pub sync: Sync,
    pub render: RenderSettings,
    pub preview: Preview,
    pub targets: Vec<Target>,
    pub presets: Vec<Preset>,
    pub layers: Vec<Layer>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sync {
    pub timecode: TimecodeSettings,
    pub beat_source: BeatSource,
    pub stop_behavior: StopBehavior,
    pub loss: Loss,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtc_output: Option<MtcOutput>,
}

/// MTC generated by the app for external gear (frame rate and origin from `timecode`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MtcOutput {
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimecodeSettings {
    pub source: TimecodeSource,
    pub frame_rate: FrameRate,
    /// `HH:MM:SS:FF` label of the song start.
    pub origin: String,
    pub input_offset_us: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimecodeSource {
    /// The Live Remote Script's song time is the common clock (default).
    #[serde(rename = "live_bridge")]
    LiveBridge,
    /// An external MTC input.
    #[serde(rename = "mtc")]
    Mtc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeatSource {
    AbletonLink,
    MidiClock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopBehavior {
    Hold,
    Blackout,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Loss {
    pub holdover_ms: u32,
    pub blackout_after_ms: u32,
    pub recovery_fade_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderSettings {
    pub fps: u32,
    #[serde(serialize_with = "ser_num")]
    pub master_brightness: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    #[serde(serialize_with = "ser_num")]
    pub bpm: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub profile: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub effect: Effect,
}

/// What a preset renders. The JSON `type` tag is the extension point for
/// effect sources: `builtin.*` are compiled-in effects (type + version +
/// params); a node-graph reference becomes another tagged variant (e.g.
/// `graph` with a graph ID) in Schema 0.2, compiled by the same
/// `CompiledEffect::compile` entry point. Unknown types are rejected today.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Effect {
    #[serde(rename = "builtin.solid")]
    Solid { version: u32, params: SolidParams },
    #[serde(rename = "builtin.chase")]
    Chase { version: u32, params: ChaseParams },
    #[serde(rename = "builtin.pulse")]
    Pulse { version: u32, params: PulseParams },
    /// 0.1.2
    #[serde(rename = "builtin.strobe")]
    Strobe { version: u32, params: StrobeParams },
    /// 0.1.2
    #[serde(rename = "builtin.noise")]
    Noise { version: u32, params: NoiseParams },
    /// 0.1.2, input driven: rings spread from pressed pads.
    #[serde(rename = "builtin.ripple")]
    Ripple { version: u32, params: RippleParams },
    /// 0.1.2: falling drops with tails (digital rain).
    #[serde(rename = "builtin.rain")]
    Rain { version: u32, params: RainParams },
    /// 0.1.2: a moving sine curve.
    #[serde(rename = "builtin.wave")]
    Wave { version: u32, params: WaveParams },
    /// 0.1.2: a shaded disc, still, orbiting or bouncing.
    #[serde(rename = "builtin.orb")]
    Orb { version: u32, params: OrbParams },
    /// 0.1.2: groups of cells lit in turn, by beat or by time.
    #[serde(rename = "builtin.regions")]
    Regions { version: u32, params: RegionsParams },
    /// 0.1.2, input driven: a glow that cools down where a pad was pressed.
    #[serde(rename = "builtin.tap_glow")]
    TapGlow { version: u32, params: TapGlowParams },
    /// 0.1.2, input driven: heads run up, down, left and right from a pressed pad.
    #[serde(rename = "builtin.cross")]
    Cross { version: u32, params: CrossParams },
}

/// Glow at a pressed pad: start → mid → end color over the lifetime, repeated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TapGlowParams {
    pub color_start: Param<String>,
    pub color_mid: Param<String>,
    pub color_end: Param<String>,
    /// 100..=5000 ms.
    pub lifetime_ms: Param<u32>,
    /// Cells, 0..=3 (1 at the pad, 0 at this distance; 0 = the pad only).
    #[serde(serialize_with = "ser_param_num")]
    pub radius: Param<f64>,
    /// 1..=8 glows per press.
    pub repeats: Param<u32>,
    /// 50..=2000 ms between repeats.
    pub repeat_interval_ms: Param<u32>,
    /// Brightness factor per repeat, 0..=1.
    #[serde(serialize_with = "ser_param_num")]
    pub repeat_decay: Param<f64>,
}

/// Four heads running from a pressed pad with linear tails.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrossParams {
    pub color: Param<String>,
    /// Cells per second, 1..=32.
    #[serde(serialize_with = "ser_param_num")]
    pub speed: Param<f64>,
    /// Tail length in cells, 0.5..=8.
    #[serde(serialize_with = "ser_param_num")]
    pub length: Param<f64>,
    /// 100..=3000 ms.
    pub lifetime_ms: Param<u32>,
}

/// Groups of cells (`pattern`) lit one after another.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionsParams {
    pub pattern: Param<RegionPattern>,
    pub color: Param<String>,
    /// Groups that are not lit show this color (alternating look); absent = transparent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_b: Option<Param<String>>,
    pub step_mode: Param<StepMode>,
    /// Beats per step (mode `beat`): 0.25, 0.5, 1, 2 or 4.
    #[serde(serialize_with = "ser_param_num")]
    pub step_beats: Param<f64>,
    /// Milliseconds per step (mode `seconds`), 50..=5000.
    pub step_ms: Param<u32>,
    pub order: Param<StepOrder>,
    /// Cross-fade width to the neighbouring steps, 0..=1.
    #[serde(serialize_with = "ser_param_num")]
    pub soft: Param<f64>,
    /// Lit fraction of each step, 0.05..=1.
    #[serde(serialize_with = "ser_param_num")]
    pub duty: Param<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionPattern {
    HalvesLr,
    HalvesTb,
    Checker,
    Quadrants,
    Columns,
    Rows,
    Perimeter,
    Fan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepMode {
    Beat,
    Seconds,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOrder {
    Forward,
    Reverse,
    Pingpong,
}

impl Effect {
    pub fn type_name(&self) -> &'static str {
        match self {
            Effect::Solid { .. } => "builtin.solid",
            Effect::Chase { .. } => "builtin.chase",
            Effect::Pulse { .. } => "builtin.pulse",
            Effect::Strobe { .. } => "builtin.strobe",
            Effect::Noise { .. } => "builtin.noise",
            Effect::Ripple { .. } => "builtin.ripple",
            Effect::Rain { .. } => "builtin.rain",
            Effect::Wave { .. } => "builtin.wave",
            Effect::Orb { .. } => "builtin.orb",
            Effect::Regions { .. } => "builtin.regions",
            Effect::TapGlow { .. } => "builtin.tap_glow",
            Effect::Cross { .. } => "builtin.cross",
        }
    }

    pub fn version(&self) -> u32 {
        match self {
            Effect::Solid { version, .. }
            | Effect::Chase { version, .. }
            | Effect::Pulse { version, .. }
            | Effect::Strobe { version, .. }
            | Effect::Noise { version, .. }
            | Effect::Ripple { version, .. }
            | Effect::Rain { version, .. }
            | Effect::Wave { version, .. }
            | Effect::Orb { version, .. }
            | Effect::Regions { version, .. }
            | Effect::TapGlow { version, .. }
            | Effect::Cross { version, .. } => *version,
        }
    }

    /// Draws instances (position and age) handed to it: ripple, tap_glow, cross.
    pub fn takes_instances(&self) -> bool {
        matches!(self, Effect::Ripple { .. } | Effect::TapGlow { .. } | Effect::Cross { .. })
    }

    /// Compiled-in effect (`builtin.*`), as opposed to a future graph reference.
    pub fn is_builtin(&self) -> bool {
        self.type_name().starts_with("builtin.")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolidParams {
    pub color: Param<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChaseParams {
    pub color: Param<String>,
    pub period_us: Param<u64>,
    pub width: Param<u32>,
    pub axis: Param<Axis>,
    /// 0.1.2, optional: move right to left (x) or top to bottom (y). Absent
    /// in older files and not written when absent, so they stay byte for byte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse: Option<Param<bool>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    X,
    Y,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PulseParams {
    pub color: Param<String>,
    #[serde(serialize_with = "ser_param_num")]
    pub beats_per_cycle: Param<f64>,
    #[serde(serialize_with = "ser_param_num")]
    pub depth: Param<f64>,
    pub waveform: Param<Waveform>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Waveform {
    Sine,
    Triangle,
}

/// Square-wave flashing: on for `duty` of each cycle, off otherwise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrobeParams {
    pub color: Param<String>,
    pub mode: Param<StrobeMode>,
    /// Cycles per second (mode `hz`), 0.5..=30.
    #[serde(serialize_with = "ser_param_num")]
    pub rate_hz: Param<f64>,
    /// Beats per cycle (mode `beat`): 0.25, 0.5, 1 or 2.
    #[serde(serialize_with = "ser_param_num")]
    pub beats_per_cycle: Param<f64>,
    /// On fraction of a cycle, 0.05..=0.95.
    #[serde(serialize_with = "ser_param_num")]
    pub duty: Param<f64>,
    /// Shift of the cycle start, 0..=1 cycle.
    #[serde(serialize_with = "ser_param_num")]
    pub phase_offset: Param<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrobeMode {
    /// Fixed rate from the clip's local time.
    Hz,
    /// Follows the beat phase like Pulse (transparent while it is unknown).
    Beat,
}

/// Deterministic sparkle: a hashed pattern that changes `rate_hz` times a second.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoiseParams {
    pub base_color: Param<String>,
    pub highlight_color: Param<String>,
    /// Fraction of lit cells, 0..=1.
    #[serde(serialize_with = "ser_param_num")]
    pub density: Param<f64>,
    /// Pattern changes per second, 0.5..=30.
    #[serde(serialize_with = "ser_param_num")]
    pub rate_hz: Param<f64>,
    pub seed: Param<u32>,
    /// Cross-fade to the next pattern instead of switching.
    pub smooth: Param<bool>,
    /// Brightness spread of lit cells, 0..=1 (0 = all at the highlight color).
    #[serde(serialize_with = "ser_param_num")]
    pub depth: Param<f64>,
}

/// Rings spreading from pressed pads (operation input; nothing without it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RippleParams {
    pub color: Param<String>,
    /// Cells per second, 0.5..=16.
    #[serde(serialize_with = "ser_param_num")]
    pub speed: Param<f64>,
    /// Ring width in cells, 0.5..=4.
    #[serde(serialize_with = "ser_param_num")]
    pub ring_width: Param<f64>,
    /// 200..=4000 ms.
    pub lifetime_ms: Param<u32>,
    pub fade: Param<RippleFade>,
    /// Rings drawn at once, 1..=16 (the oldest go first).
    pub max_concurrent: Param<u32>,
    pub trigger: Param<RippleTrigger>,
    /// Ring shape (distance metric); absent = circle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Param<RippleShape>>,
    /// `interval` only (required there): ring centre, 0..=7 cells.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_param_num")]
    pub origin_x: Option<Param<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_param_num")]
    pub origin_y: Option<Param<f64>>,
    /// `interval` only (required there): a ring every 100..=10000 ms of local time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<Param<u32>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RippleFade {
    Linear,
    Exp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RippleTrigger {
    /// Any grid pad Note On.
    AnyPad,
    /// A ring every `interval_ms` of the clip's local time at `origin_x/y` (no input).
    Interval,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RippleShape {
    /// Euclidean distance.
    Circle,
    /// Manhattan distance.
    Diamond,
    /// Chebyshev distance.
    Square,
}
/// Drops run along `direction`, one lane per row / column, with a tail behind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RainParams {
    pub color: Param<String>,
    /// Color of the leading cell; absent = `color`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_color: Option<Param<String>>,
    pub direction: Param<RainDirection>,
    /// Cells per second, 0.5..=16.
    #[serde(serialize_with = "ser_param_num")]
    pub speed: Param<f64>,
    /// Chance of a drop per lane and half second, 0..=1.
    #[serde(serialize_with = "ser_param_num")]
    pub density: Param<f64>,
    /// Tail length in cells, 0..=8 (linear fade).
    #[serde(serialize_with = "ser_param_num")]
    pub tail: Param<f64>,
    pub seed: Param<u32>,
    /// Brightness flicker of tail cells, 0..=1 (8 Hz steps).
    #[serde(serialize_with = "ser_param_num")]
    pub flicker: Param<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RainDirection {
    Down,
    Up,
    Left,
    Right,
}

/// `center + amplitude * sin(2π(pos / wavelength - speed * t))`, drawn `thickness` thick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaveParams {
    pub color: Param<String>,
    /// `x`: the wave runs horizontally (curve over x); `y`: vertically.
    pub axis: Param<Axis>,
    /// Cells, 2..=16.
    #[serde(serialize_with = "ser_param_num")]
    pub wavelength: Param<f64>,
    /// Cells, 0.5..=3.5.
    #[serde(serialize_with = "ser_param_num")]
    pub amplitude: Param<f64>,
    /// Cells, 0..=7.
    #[serde(serialize_with = "ser_param_num")]
    pub center: Param<f64>,
    /// Cycles per second, -4..=4 (negative = the other way).
    #[serde(serialize_with = "ser_param_num")]
    pub speed: Param<f64>,
    /// Cells, 0.5..=3.
    #[serde(serialize_with = "ser_param_num")]
    pub thickness: Param<f64>,
    /// Scale the amplitude 0.3..1 with the beat phase (fixed while it is unknown).
    pub beat_amplitude: Param<bool>,
    /// Phase offset of the sine, 0..=1 cycle; absent = 0.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_param_num")]
    pub phase: Option<Param<f64>>,
}

/// A shaded disc (light from `light_angle`), optionally dithered and moving.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrbParams {
    pub color: Param<String>,
    /// Cells, 0..=7.
    #[serde(serialize_with = "ser_param_num")]
    pub center_x: Param<f64>,
    #[serde(serialize_with = "ser_param_num")]
    pub center_y: Param<f64>,
    /// Cells, 0.5..=6.
    #[serde(serialize_with = "ser_param_num")]
    pub radius: Param<f64>,
    /// Soft edge width in cells, 0..=2.
    #[serde(serialize_with = "ser_param_num")]
    pub edge: Param<f64>,
    /// Degrees, 0..=360 (90 = light from above).
    #[serde(serialize_with = "ser_param_num")]
    pub light_angle: Param<f64>,
    /// 0..=1: lit side 1, far side 1 - shading.
    #[serde(serialize_with = "ser_param_num")]
    pub shading: Param<f64>,
    /// 0..=1 grain from a hash per cell and step.
    #[serde(serialize_with = "ser_param_num")]
    pub dither: Param<f64>,
    /// Grain steps per second, 0.5..=15.
    #[serde(serialize_with = "ser_param_num")]
    pub dither_rate_hz: Param<f64>,
    pub seed: Param<u32>,
    pub motion: Param<OrbMotion>,
    /// orbit: turns per second (0.05..=2); bounce: cells per second (0.5..=12).
    #[serde(serialize_with = "ser_param_num")]
    pub speed: Param<f64>,
    /// Cells, 0..=4 (orbit).
    #[serde(serialize_with = "ser_param_num")]
    pub orbit_radius: Param<f64>,
    /// Degrees (bounce direction).
    #[serde(serialize_with = "ser_param_num")]
    pub bounce_angle: Param<f64>,
    /// Start angle of the orbit, 0..=1 turn; absent = 0.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_param_num")]
    pub orbit_phase: Option<Param<f64>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrbMotion {
    None,
    Orbit,
    Bounce,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub id: String,
    pub name: String,
    pub target_id: String,
    pub enabled: bool,
    #[serde(serialize_with = "ser_num")]
    pub opacity: f64,
    pub blend: Blend,
    pub content: Content,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blend {
    Over,
    Add,
    Max,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Content {
    Timeline { clips: Vec<Clip> },
    Continuous { preset_id: String },
    /// 0.1.3: clips are the windows where `trigger` fires instances of the
    /// clip's preset (positions and times come from the trigger).
    Triggered { clips: Vec<Clip>, trigger: TriggerSpec },
}

impl Content {
    /// Clips of a timeline or triggered layer.
    pub fn clips(&self) -> Option<&Vec<Clip>> {
        match self {
            Content::Timeline { clips } | Content::Triggered { clips, .. } => Some(clips),
            Content::Continuous { .. } => None,
        }
    }

    pub fn clips_mut(&mut self) -> Option<&mut Vec<Clip>> {
        match self {
            Content::Timeline { clips } | Content::Triggered { clips, .. } => Some(clips),
            Content::Continuous { .. } => None,
        }
    }
}

/// When a triggered layer fires (docs/14 §4-1).
///
/// * `press` / `release` / `hold`: pad input, timed by its arrival
///   (monotonic): instances are not replayed by a seek and keep ageing
///   through Stop. Only input that arrives while a clip is active counts.
///   `hold` lasts from press to release (at most 10 s).
/// * `clip_start` / `interval`: the clip's local time (deterministic, seek
///   safe), at `origin_x` / `origin_y` (default the centre).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerSpec {
    pub on: TriggerOn,
    /// `interval` only (required there): 100..=10000 ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u32>,
    /// `clip_start` / `interval`: 0..=7 cells; absent = 3.5.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_num")]
    pub origin_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_num")]
    pub origin_y: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerOn {
    Press,
    Release,
    Hold,
    ClipStart,
    Interval,
}

/// `ser_num` for an optional plain number (skipped when absent).
pub fn ser_opt_num<S: Serializer>(v: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(x) => ser_num(x, s),
        None => s.serialize_none(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub id: String,
    pub preset_id: String,
    pub start_us: u64,
    pub duration_us: u64,
    pub fade_in_us: u64,
    pub fade_out_us: u64,
}

impl Clip {
    /// End (exclusive) in show time; saturates instead of overflowing.
    pub fn end_us(&self) -> u64 {
        self.start_us.saturating_add(self.duration_us)
    }
}

/// The 0.1.1 demo show, at the current version (what the store's migration gives).
#[cfg(test)]
pub(crate) fn demo_for_tests() -> Show {
    let mut s: Show = serde_json::from_str(include_str!("../../../examples/demo.show.json")).unwrap();
    s.schema_version = SCHEMA_VERSION.to_string();
    s
}

step_only!(Axis, Waveform, StrobeMode, RippleFade, RippleTrigger, RippleShape, RainDirection, OrbMotion, RegionPattern, StepMode, StepOrder);

#[cfg(test)]
mod keyframe_tests {
    use super::*;

    fn kf(json: &str) -> Result<Param<f64>, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn plain_values_are_unchanged_and_keyframes_round_trip() {
        let p: Param<f64> = serde_json::from_str("0.5").unwrap();
        assert_eq!(p, Param::Value(0.5));
        #[derive(Serialize)]
        struct W(#[serde(serialize_with = "ser_param_num")] Param<f64>);
        assert_eq!(serde_json::to_string(&W(Param::Value(6.0))).unwrap(), "6");
        let text = r#"{"keyframes":[{"t_us":0,"v":4,"ease":"linear"},{"t_us":1000000,"v":16.5,"ease":"step"}]}"#;
        let k = kf(text).unwrap();
        assert_eq!(serde_json::to_string(&W(k.clone())).unwrap(), text, "integral values stay integers");
        // ease defaults to linear when absent.
        assert_eq!(kf(r#"{"keyframes":[{"t_us":0,"v":1}]}"#).unwrap(), Param::Keyframes(vec![Keyframe { t_us: 0, v: 1.0, ease: Ease::Linear }]));
    }

    #[test]
    fn bad_keyframe_lists_are_refused_when_reading() {
        assert!(kf(r#"{"keyframes":[]}"#).is_err(), "empty");
        assert!(kf(r#"{"keyframes":[{"t_us":5,"v":1},{"t_us":2,"v":2}]}"#).is_err(), "descending");
        let many = (0..65).map(|i| format!(r#"{{"t_us":{i},"v":1}}"#)).collect::<Vec<_>>().join(",");
        assert!(kf(&format!(r#"{{"keyframes":[{many}]}}"#)).is_err(), "more than 64");
        // Enums and bools only step.
        assert!(serde_json::from_str::<Param<Axis>>(r#"{"keyframes":[{"t_us":0,"v":"x","ease":"linear"}]}"#).is_err());
        assert!(serde_json::from_str::<Param<bool>>(r#"{"keyframes":[{"t_us":0,"v":true,"ease":"step"},{"t_us":9,"v":false,"ease":"step"}]}"#).is_ok());
    }

    #[test]
    fn sampling_holds_the_ends_and_follows_the_ease() {
        let p = kf(r#"{"keyframes":[{"t_us":1000,"v":10},{"t_us":3000,"v":20,"ease":"step"},{"t_us":5000,"v":40}]}"#).unwrap();
        assert_eq!(p.at(0), 10.0, "before the first point");
        assert_eq!(p.at(2000), 15.0, "linear midpoint");
        assert_eq!(p.at(4000), 20.0, "step holds until the next point");
        assert_eq!(p.at(5000), 40.0);
        assert_eq!(p.at(99_000), 40.0, "after the last point");
        let e = |ease: Ease| Param::Keyframes(vec![Keyframe { t_us: 0, v: 0.0, ease }, Keyframe { t_us: 100, v: 1.0, ease: Ease::Linear }]).at(50);
        assert_eq!(e(Ease::EaseIn), 0.25);
        assert_eq!(e(Ease::EaseOut), 0.75);
        assert_eq!(e(Ease::EaseInOut), 0.5);
        // Integers interpolate rounded; enums step.
        let n: Param<u32> = Param::Keyframes(vec![Keyframe { t_us: 0, v: 100, ease: Ease::Linear }, Keyframe { t_us: 10, v: 200, ease: Ease::Linear }]);
        assert_eq!(n.at(3), 130);
        let a: Param<Axis> = Param::Keyframes(vec![Keyframe { t_us: 0, v: Axis::X, ease: Ease::Step }, Keyframe { t_us: 10, v: Axis::Y, ease: Ease::Step }]);
        assert_eq!((a.at(9), a.at(10)), (Axis::X, Axis::Y));
        assert_eq!(p.values().copied().collect::<Vec<_>>(), vec![10.0, 20.0, 40.0]);
    }
}