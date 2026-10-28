//! Show editing (M5, U04): edit commands, snapping and Undo / Redo.
//!
//! * `EditCommand` is the only way the UI changes a show. `apply` works on a
//!   copy and runs the semantic validation; a command that would make the
//!   show invalid is rejected and nothing changes.
//! * `History` keeps whole-show snapshots (`Arc<Show>`), not inverse
//!   operations. Undo / Redo therefore do not depend on what a command
//!   touched, and adding parameter automation or graph references to the
//!   model (docs/12) does not change the history format.
//! * Snapping is to whole seconds or to 30 fps frames (bars / beats come
//!   later). Times are show times in microseconds (song start = 0).
//! * IDs of new clips and layers are generated (`clip-N`, `layer-N`) and are
//!   never reused within a show.

use crate::model::{
    Axis, BeatSource, Blend, ChaseParams, Clip, CrossParams, TapGlowParams, Content, Effect, Layer, Loss, MtcOutput, NoiseParams, OrbMotion, OrbParams, Preset, RainDirection, RainParams, RegionPattern, RegionsParams, StepMode, StepOrder, RippleFade, RippleParams, RippleTrigger, WaveParams, Preview, PulseParams, RenderSettings, Show, SolidParams,
    StopBehavior, StrobeMode, StrobeParams, Sync, TriggerOn, TriggerSpec, Target, TimecodeSettings, TimecodeSource, Waveform, SCHEMA_VERSION,
};
use crate::surface::GRID64_PROFILE;
use crate::time::FrameRate;
use crate::validation::{validate, Issue};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// History length (snapshots kept besides the current one).
pub const MAX_HISTORY: usize = 200;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Snap {
    #[default]
    None,
    Second,
    /// 30 fps frame (33 333.3 us), rounded to the nearest frame.
    Frame30,
}

/// Round `us` to the snap grid (nearest), never below 0.
pub fn snap_us(us: i64, snap: Snap) -> u64 {
    let us = us.max(0) as u128;
    match snap {
        Snap::None => us as u64,
        Snap::Second => (((us + 500_000) / 1_000_000) * 1_000_000) as u64,
        // frame k starts at k * 1e6 / 30 (exact, no accumulated rounding)
        Snap::Frame30 => {
            let k = (us * 30 + 500_000) / 1_000_000;
            (k * 1_000_000 / 30) as u64
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum EditCommand {
    /// New clip on a timeline layer (appended, so it composites on top).
    AddClip { layer_id: String, preset_id: String, start_us: i64, duration_us: i64 },
    /// Move a clip in time, optionally to another timeline layer.
    MoveClip { clip_id: String, start_us: i64, to_layer_id: Option<String> },
    /// Change the length (fades are shortened if they no longer fit).
    ResizeClip { clip_id: String, duration_us: i64 },
    /// Numeric input of all timing fields at once.
    SetClipTiming { clip_id: String, start_us: i64, duration_us: i64, fade_in_us: i64, fade_out_us: i64 },
    /// Copy right after the original (or at `start_us`) on the same layer.
    DuplicateClip { clip_id: String, start_us: Option<i64> },
    DeleteClip { clip_id: String },
    /// New timeline layer at `index` (default: on top).
    AddLayer { name: String, index: Option<usize> },
    DeleteLayer { layer_id: String },
    /// Move a layer to `to_index` (array order = compositing order, 0 = bottom).
    MoveLayer { layer_id: String, to_index: usize },
    /// Layer properties; `None` leaves a field unchanged.
    SetLayer {
        layer_id: String,
        name: Option<String>,
        enabled: Option<bool>,
        opacity: Option<f64>,
        blend: Option<Blend>,
        target_id: Option<String>,
    },
    /// New continuous layer showing one preset all the time (default: on top).
    AddContinuousLayer { name: String, preset_id: String, index: Option<usize> },
    /// Change the preset of a continuous layer.
    SetLayerPreset { layer_id: String, preset_id: String },
    /// New built-in preset with default parameters.
    AddPreset { name: String, effect_type: String },
    /// Explicit shared edit: every clip and layer using the preset changes.
    SetPreset { preset_id: String, name: Option<String>, effect: Option<Effect> },
    /// Refused while any clip or layer uses the preset (the reason lists them).
    DeletePreset { preset_id: String },
    /// Insert a preset (import). An ID conflict is an error or a rename.
    InsertPreset { preset: Preset, on_conflict: OnConflict },
    /// Use another preset for one clip.
    SetClipPreset { clip_id: String, preset_id: String },
    /// Edit the effect of one clip only. If the preset is shared with other
    /// clips or layers, it is copied to a new ID first and only this clip
    /// switches to the copy (docs/04); otherwise it is edited in place.
    EditClipEffect { clip_id: String, effect: Effect },
    /// Show sync settings; `None` leaves a field unchanged. `mtc_output_enabled`
    /// writes `sync.mtc_output.enabled` (the frame rate and origin come from
    /// `sync.timecode`).
    SetSync { beat_source: Option<BeatSource>, mtc_output_enabled: Option<bool> },
    /// New triggered layer (no clips yet; add clips as its active windows).
    AddTriggeredLayer { name: String, on: TriggerOn, index: Option<usize> },
    /// Replace the trigger of a triggered layer.
    SetLayerTrigger { layer_id: String, trigger: TriggerSpec },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnConflict {
    #[default]
    Error,
    /// `<id>-2`, `<id>-3`, ... (at most 64 characters).
    Rename,
}

/// A free ID for an imported preset: `id` itself, or `<id>-N` with N >= 2.
pub fn rename_on_conflict<'a>(id: &str, used: impl Iterator<Item = &'a str> + Clone) -> String {
    if !used.clone().any(|u| u == id) {
        return id.to_string();
    }
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            let mut c = id.to_string();
            while c.len() + suffix.len() > crate::validation::MAX_ID_CHARS {
                c.pop();
            }
            c + &suffix
        })
        .find(|c| !used.clone().any(|u| u == c))
        .expect("unbounded")
}

/// A new, empty show: one Launchpad X target, one white Solid preset (the
/// schema needs at least one preset), no layers. Sync follows the bridge at
/// 30 fps from 01:00:00:00 with the default loss rules; the beat source is
/// MIDI Clock and MTC output is off (the show setup of A19; change them in
/// the show settings).
pub fn new_show(name: &str) -> Show {
    let name = if name.trim().is_empty() { "新しいショー".to_string() } else { name.trim().to_string() };
    Show {
        schema: None,
        schema_version: SCHEMA_VERSION.to_string(),
        id: "show-1".into(),
        name,
        seed: 1,
        sync: Sync {
            timecode: TimecodeSettings { source: TimecodeSource::LiveBridge, frame_rate: FrameRate::Fps30, origin: "01:00:00:00".into(), input_offset_us: 0 },
            beat_source: BeatSource::MidiClock,
            stop_behavior: StopBehavior::Hold,
            loss: Loss { holdover_ms: 250, blackout_after_ms: 1000, recovery_fade_ms: 100 },
            mtc_output: Some(MtcOutput { enabled: false }),
        },
        render: RenderSettings { fps: 30, master_brightness: 1.0 },
        preview: Preview { bpm: 120.0 },
        targets: vec![Target { id: "main-pad".into(), name: "Launchpad X".into(), profile: GRID64_PROFILE.into() }],
        presets: vec![Preset { id: "preset-white".into(), name: "白".into(), effect: default_effect("builtin.solid").expect("solid") }],
        layers: Vec::new(),
    }
}

/// Default effect for a new built-in preset.
pub fn default_effect(effect_type: &str) -> Option<Effect> {
    Some(match effect_type {
        "builtin.solid" => Effect::Solid { version: 1, params: SolidParams { color: "#FFFFFF".to_string().into() } },
        "builtin.chase" => Effect::Chase {
            version: 1,
            params: ChaseParams { color: "#00DCC8".to_string().into(), period_us: 2_000_000.into(), width: 2.into(), axis: Axis::X.into(), reverse: None },
        },
        "builtin.pulse" => Effect::Pulse {
            version: 1,
            params: PulseParams { color: "#FFFFFF".to_string().into(), beats_per_cycle: 1.0.into(), depth: 1.0.into(), waveform: Waveform::Sine.into() },
        },
        // Beat mode at 1 beat per cycle: 2 Hz at 120 bpm, under the default 3 Hz limit.
        "builtin.strobe" => Effect::Strobe {
            version: 1,
            params: StrobeParams {
                color: "#FFFFFF".to_string().into(),
                mode: StrobeMode::Beat.into(),
                rate_hz: 2.0.into(),
                beats_per_cycle: 1.0.into(),
                duty: 0.5.into(),
                phase_offset: 0.0.into(),
            },
        },
        // Green digital rain with a bright head.
        "builtin.rain" => Effect::Rain {
            version: 1,
            params: RainParams {
                color: "#00C040".to_string().into(),
                head_color: Some("#C8FFD8".to_string().into()),
                direction: RainDirection::Down.into(),
                speed: 8.0.into(),
                density: 0.4.into(),
                tail: 2.0.into(),
                seed: 1.into(),
                flicker: 0.0.into(),
            },
        },
        "builtin.wave" => Effect::Wave {
            version: 1,
            params: WaveParams {
                color: "#00DCC8".to_string().into(),
                axis: Axis::X.into(),
                wavelength: 8.0.into(),
                amplitude: 2.5.into(),
                center: 3.5.into(),
                speed: 1.0.into(),
                thickness: 1.0.into(),
                beat_amplitude: false.into(),
                phase: None,
            },
        },
        "builtin.orb" => Effect::Orb {
            version: 1,
            params: OrbParams {
                color: "#FFB040".to_string().into(),
                center_x: 3.5.into(),
                center_y: 3.5.into(),
                radius: 3.0.into(),
                edge: 0.7.into(),
                light_angle: 90.0.into(),
                shading: 0.7.into(),
                dither: 0.0.into(),
                dither_rate_hz: 4.0.into(),
                seed: 1.into(),
                motion: OrbMotion::None.into(),
                speed: 1.0.into(),
                orbit_radius: 1.5.into(),
                bounce_angle: 35.0.into(),
                orbit_phase: None,
            },
        },
        // Left / right halves alternating on the beat (2 Hz at 120 bpm).
        "builtin.regions" => Effect::Regions {
            version: 1,
            params: RegionsParams {
                pattern: RegionPattern::HalvesLr.into(),
                color: "#FFFFFF".to_string().into(),
                color_b: None,
                step_mode: StepMode::Beat.into(),
                step_beats: 1.0.into(),
                step_ms: 500.into(),
                order: StepOrder::Forward.into(),
                soft: 0.0.into(),
                duty: 1.0.into(),
            },
        },
        "builtin.tap_glow" => Effect::TapGlow {
            version: 1,
            params: TapGlowParams {
                color_start: "#FFFFFF".to_string().into(),
                color_mid: "#FFB040".to_string().into(),
                color_end: "#400000".to_string().into(),
                lifetime_ms: 1200.into(),
                radius: 1.5.into(),
                repeats: 1.into(),
                repeat_interval_ms: 300.into(),
                repeat_decay: 0.6.into(),
            },
        },
        "builtin.cross" => Effect::Cross {
            version: 1,
            params: CrossParams { color: "#00DCC8".to_string().into(), speed: 12.0.into(), length: 2.0.into(), lifetime_ms: 800.into() },
        },
        "builtin.ripple" => Effect::Ripple {
            version: 1,
            params: RippleParams {
                color: "#00DCC8".to_string().into(),
                speed: 6.0.into(),
                ring_width: 1.0.into(),
                lifetime_ms: 1200.into(),
                fade: RippleFade::Linear.into(),
                max_concurrent: 8.into(),
                trigger: RippleTrigger::AnyPad.into(),
                shape: None,
                origin_x: None,
                origin_y: None,
                interval_ms: None,
            },
        },
        // The reference look: light-blue to white grains on deep blue. 6 Hz pattern = 3 Hz blinking.
        "builtin.noise" => Effect::Noise {
            version: 1,
            params: NoiseParams {
                base_color: "#0D2346".to_string().into(),
                highlight_color: "#9FE8FF".to_string().into(),
                density: 0.3.into(),
                rate_hz: 6.0.into(),
                seed: 1.into(),
                smooth: false.into(),
                depth: 0.5.into(),
            },
        },
        _ => return None,
    })
}

/// Where a preset is used: clip IDs and continuous layer IDs.
pub fn preset_users(show: &Show, preset_id: &str) -> (Vec<String>, Vec<String>) {
    let mut clips = Vec::new();
    let mut layers = Vec::new();
    for l in &show.layers {
        match &l.content {
            Content::Timeline { clips: cs } | Content::Triggered { clips: cs, .. } => clips.extend(cs.iter().filter(|c| c.preset_id == preset_id).map(|c| c.id.clone())),
            Content::Continuous { preset_id: p } if p == preset_id => layers.push(l.id.clone()),
            Content::Continuous { .. } => {}
        }
    }
    (clips, layers)
}

impl EditCommand {
    /// Short Japanese label for the history list.
    pub fn label(&self) -> &'static str {
        match self {
            EditCommand::AddClip { .. } => "クリップを追加",
            EditCommand::MoveClip { .. } => "クリップを移動",
            EditCommand::ResizeClip { .. } => "クリップの長さ",
            EditCommand::SetClipTiming { .. } => "クリップの時間を入力",
            EditCommand::DuplicateClip { .. } => "クリップを複製",
            EditCommand::DeleteClip { .. } => "クリップを削除",
            EditCommand::AddLayer { .. } => "レイヤーを追加",
            EditCommand::DeleteLayer { .. } => "レイヤーを削除",
            EditCommand::MoveLayer { .. } => "レイヤーの順序",
            EditCommand::SetLayer { .. } => "レイヤーの設定",
            EditCommand::AddContinuousLayer { .. } => "常時レイヤーを追加",
            EditCommand::SetLayerPreset { .. } => "レイヤーのプリセット",
            EditCommand::AddPreset { .. } => "プリセットを追加",
            EditCommand::SetPreset { .. } => "プリセットを編集（共有）",
            EditCommand::DeletePreset { .. } => "プリセットを削除",
            EditCommand::InsertPreset { .. } => "プリセットを読み込み",
            EditCommand::SetClipPreset { .. } => "クリップのプリセット",
            EditCommand::EditClipEffect { .. } => "クリップの効果を編集",
            EditCommand::SetSync { .. } => "同期の設定",
            EditCommand::AddTriggeredLayer { .. } => "発火レイヤーを追加",
            EditCommand::SetLayerTrigger { .. } => "発火の設定",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditError {
    NotFound(String),
    Invalid(String),
    /// The edited show failed semantic validation (nothing was changed).
    Rejected(Vec<Issue>),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::NotFound(s) => write!(f, "not found: {s}"),
            EditError::Invalid(s) => write!(f, "invalid edit: {s}"),
            EditError::Rejected(i) => {
                write!(f, "the edit would make the show invalid:")?;
                for x in i {
                    write!(f, " {} {};", x.pointer, x.message)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for EditError {}

/// What an applied command produced (for selecting it in the UI).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct EditOutcome {
    pub clip_id: Option<String>,
    pub layer_id: Option<String>,
    pub preset_id: Option<String>,
}

fn non_negative(name: &str, v: i64) -> Result<u64, EditError> {
    u64::try_from(v).map_err(|_| EditError::Invalid(format!("{name} must be >= 0 (got {v})")))
}

fn clips_mut<'a>(show: &'a mut Show, layer_id: &str) -> Result<&'a mut Vec<Clip>, EditError> {
    let layer = show.layers.iter_mut().find(|l| l.id == layer_id).ok_or_else(|| EditError::NotFound(format!("layer {layer_id:?}")))?;
    match layer.content.clips_mut() {
        Some(clips) => Ok(clips),
        None => Err(EditError::Invalid(format!("layer {layer_id:?} is continuous, not a timeline"))),
    }
}

/// (layer index, clip index) of a clip.
fn find_clip(show: &Show, clip_id: &str) -> Result<(usize, usize), EditError> {
    for (li, l) in show.layers.iter().enumerate() {
        if let Some(clips) = l.content.clips() {
            if let Some(ci) = clips.iter().position(|c| c.id == clip_id) {
                return Ok((li, ci));
            }
        }
    }
    Err(EditError::NotFound(format!("clip {clip_id:?}")))
}

fn all_clip_ids(show: &Show) -> impl Iterator<Item = &str> {
    show.layers.iter().flat_map(|l| match l.content.clips() {
        Some(clips) => clips.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        None => Vec::new(),
    })
}

/// First free `prefix-N` (N >= 1) not used by `used`.
fn fresh_id<'a>(prefix: &str, used: impl Iterator<Item = &'a str>) -> String {
    let used: std::collections::HashSet<&str> = used.collect();
    (1..).map(|n| format!("{prefix}-{n}")).find(|id| !used.contains(id.as_str())).expect("unbounded")
}

/// Fit fades into a (possibly shorter) clip: fade in first, then fade out.
fn fit_fades(c: &mut Clip) {
    c.fade_in_us = c.fade_in_us.min(c.duration_us);
    c.fade_out_us = c.fade_out_us.min(c.duration_us - c.fade_in_us);
}

/// Apply `cmd` to a copy of `show`, snapping times, then validate.
pub fn apply(show: &Show, cmd: &EditCommand, snap: Snap) -> Result<(Show, EditOutcome), EditError> {
    let mut s = show.clone();
    let mut out = EditOutcome::default();
    let at = |v: i64| snap_us(v, snap);
    match cmd {
        EditCommand::AddClip { layer_id, preset_id, start_us, duration_us } => {
            non_negative("start_us", *start_us)?;
            let duration = at(non_negative("duration_us", *duration_us)? as i64);
            if duration == 0 {
                return Err(EditError::Invalid("duration must be > 0".into()));
            }
            let id = fresh_id("clip", all_clip_ids(&s));
            let clip = Clip { id: id.clone(), preset_id: preset_id.clone(), start_us: at(*start_us), duration_us: duration, fade_in_us: 0, fade_out_us: 0 };
            clips_mut(&mut s, layer_id)?.push(clip);
            out.clip_id = Some(id);
            out.layer_id = Some(layer_id.clone());
        }
        EditCommand::MoveClip { clip_id, start_us, to_layer_id } => {
            non_negative("start_us", *start_us)?;
            let (li, ci) = find_clip(&s, clip_id)?;
            let from_layer = s.layers[li].id.clone();
            let target = to_layer_id.clone().unwrap_or(from_layer.clone());
            let mut clip = clips_mut(&mut s, &from_layer)?.remove(ci);
            clip.start_us = at(*start_us);
            if target == from_layer {
                clips_mut(&mut s, &from_layer)?.insert(ci, clip);
            } else {
                clips_mut(&mut s, &target)?.push(clip);
            }
            out.clip_id = Some(clip_id.clone());
            out.layer_id = Some(target);
        }
        EditCommand::ResizeClip { clip_id, duration_us } => {
            let d = at(non_negative("duration_us", *duration_us)? as i64);
            if d == 0 {
                return Err(EditError::Invalid("duration must be > 0".into()));
            }
            let (li, ci) = find_clip(&s, clip_id)?;
            let lid = s.layers[li].id.clone();
            let c = &mut clips_mut(&mut s, &lid)?[ci];
            c.duration_us = d;
            fit_fades(c);
            out.clip_id = Some(clip_id.clone());
        }
        EditCommand::SetClipTiming { clip_id, start_us, duration_us, fade_in_us, fade_out_us } => {
            let (start, duration) = (non_negative("start_us", *start_us)?, non_negative("duration_us", *duration_us)?);
            let (fi, fo) = (non_negative("fade_in_us", *fade_in_us)?, non_negative("fade_out_us", *fade_out_us)?);
            let (li, ci) = find_clip(&s, clip_id)?;
            let lid = s.layers[li].id.clone();
            let c = &mut clips_mut(&mut s, &lid)?[ci];
            // Numeric input: snapped like drags, fades kept as typed (validation reports misfits).
            c.start_us = at(start as i64);
            c.duration_us = at(duration as i64);
            c.fade_in_us = fi;
            c.fade_out_us = fo;
            out.clip_id = Some(clip_id.clone());
        }
        EditCommand::DuplicateClip { clip_id, start_us } => {
            let (li, ci) = find_clip(&s, clip_id)?;
            let lid = s.layers[li].id.clone();
            let id = fresh_id("clip", all_clip_ids(&s));
            let clips = clips_mut(&mut s, &lid)?;
            let mut copy = clips[ci].clone();
            copy.id = id.clone();
            copy.start_us = match start_us {
                Some(t) => at(non_negative("start_us", *t)? as i64),
                None => copy.end_us(),
            };
            clips.insert(ci + 1, copy);
            out.clip_id = Some(id);
            out.layer_id = Some(lid);
        }
        EditCommand::DeleteClip { clip_id } => {
            let (li, ci) = find_clip(&s, clip_id)?;
            let lid = s.layers[li].id.clone();
            clips_mut(&mut s, &lid)?.remove(ci);
            out.layer_id = Some(lid);
        }
        EditCommand::AddLayer { name, index } => {
            let target_id = s.targets.first().map(|t| t.id.clone()).ok_or_else(|| EditError::Invalid("the show has no target".into()))?;
            let id = fresh_id("layer", s.layers.iter().map(|l| l.id.as_str()));
            let name = if name.trim().is_empty() { id.clone() } else { name.trim().to_string() };
            let layer = Layer { id: id.clone(), name, target_id, enabled: true, opacity: 1.0, blend: Blend::Over, content: Content::Timeline { clips: Vec::new() } };
            let i = index.unwrap_or(s.layers.len()).min(s.layers.len());
            s.layers.insert(i, layer);
            out.layer_id = Some(id);
        }
        EditCommand::AddTriggeredLayer { name, on, index } => {
            let target_id = s.targets.first().map(|t| t.id.clone()).ok_or_else(|| EditError::Invalid("the show has no target".into()))?;
            let id = fresh_id("layer", s.layers.iter().map(|l| l.id.as_str()));
            let name = if name.trim().is_empty() { id.clone() } else { name.trim().to_string() };
            let interval_ms = (*on == TriggerOn::Interval).then_some(1000);
            let trigger = TriggerSpec { on: *on, interval_ms, origin_x: None, origin_y: None };
            let layer = Layer { id: id.clone(), name, target_id, enabled: true, opacity: 1.0, blend: Blend::Over, content: Content::Triggered { clips: Vec::new(), trigger } };
            let i = index.unwrap_or(s.layers.len()).min(s.layers.len());
            s.layers.insert(i, layer);
            out.layer_id = Some(id);
        }
        EditCommand::SetLayerTrigger { layer_id, trigger } => {
            let l = s.layers.iter_mut().find(|l| &l.id == layer_id).ok_or_else(|| EditError::NotFound(format!("layer {layer_id:?}")))?;
            match &mut l.content {
                Content::Triggered { trigger: t, .. } => *t = trigger.clone(),
                _ => return Err(EditError::Invalid(format!("layer {layer_id:?} is not a triggered layer"))),
            }
            out.layer_id = Some(layer_id.clone());
        }
        EditCommand::DeleteLayer { layer_id } => {
            let i = s.layers.iter().position(|l| &l.id == layer_id).ok_or_else(|| EditError::NotFound(format!("layer {layer_id:?}")))?;
            s.layers.remove(i);
        }
        EditCommand::MoveLayer { layer_id, to_index } => {
            let i = s.layers.iter().position(|l| &l.id == layer_id).ok_or_else(|| EditError::NotFound(format!("layer {layer_id:?}")))?;
            let l = s.layers.remove(i);
            s.layers.insert((*to_index).min(s.layers.len()), l);
            out.layer_id = Some(layer_id.clone());
        }
        EditCommand::SetLayer { layer_id, name, enabled, opacity, blend, target_id } => {
            let l = s.layers.iter_mut().find(|l| &l.id == layer_id).ok_or_else(|| EditError::NotFound(format!("layer {layer_id:?}")))?;
            if let Some(n) = name {
                l.name = n.trim().to_string();
            }
            if let Some(e) = enabled {
                l.enabled = *e;
            }
            if let Some(o) = opacity {
                l.opacity = *o;
            }
            if let Some(b) = blend {
                l.blend = *b;
            }
            if let Some(t) = target_id {
                l.target_id = t.clone();
            }
            out.layer_id = Some(layer_id.clone());
        }
        EditCommand::AddContinuousLayer { name, preset_id, index } => {
            let target_id = s.targets.first().map(|t| t.id.clone()).ok_or_else(|| EditError::Invalid("the show has no target".into()))?;
            let id = fresh_id("layer", s.layers.iter().map(|l| l.id.as_str()));
            let name = if name.trim().is_empty() { id.clone() } else { name.trim().to_string() };
            let layer = Layer { id: id.clone(), name, target_id, enabled: true, opacity: 1.0, blend: Blend::Over, content: Content::Continuous { preset_id: preset_id.clone() } };
            let i = index.unwrap_or(s.layers.len()).min(s.layers.len());
            s.layers.insert(i, layer);
            out.layer_id = Some(id);
        }
        EditCommand::SetLayerPreset { layer_id, preset_id } => {
            let l = s.layers.iter_mut().find(|l| &l.id == layer_id).ok_or_else(|| EditError::NotFound(format!("layer {layer_id:?}")))?;
            match &mut l.content {
                Content::Continuous { preset_id: p } => *p = preset_id.clone(),
                Content::Timeline { .. } | Content::Triggered { .. } => return Err(EditError::Invalid(format!("layer {layer_id:?} has clips; set the preset per clip"))),
            }
            out.layer_id = Some(layer_id.clone());
        }
        EditCommand::AddPreset { name, effect_type } => {
            let effect = default_effect(effect_type).ok_or_else(|| EditError::Invalid(format!("unknown effect type {effect_type:?}")))?;
            let id = fresh_id("preset", s.presets.iter().map(|p| p.id.as_str()));
            let name = if name.trim().is_empty() { id.clone() } else { name.trim().to_string() };
            s.presets.push(Preset { id: id.clone(), name, effect });
            out.preset_id = Some(id);
        }
        EditCommand::SetPreset { preset_id, name, effect } => {
            let p = s.presets.iter_mut().find(|p| &p.id == preset_id).ok_or_else(|| EditError::NotFound(format!("preset {preset_id:?}")))?;
            if let Some(n) = name {
                p.name = n.trim().to_string();
            }
            if let Some(e) = effect {
                p.effect = e.clone();
            }
            out.preset_id = Some(preset_id.clone());
        }
        EditCommand::DeletePreset { preset_id } => {
            let i = s.presets.iter().position(|p| &p.id == preset_id).ok_or_else(|| EditError::NotFound(format!("preset {preset_id:?}")))?;
            let (clips, layers) = preset_users(&s, preset_id);
            if !clips.is_empty() || !layers.is_empty() {
                let mut who = Vec::new();
                if !clips.is_empty() {
                    who.push(format!("クリップ {}", clips.join(", ")));
                }
                if !layers.is_empty() {
                    who.push(format!("常時レイヤー {}", layers.join(", ")));
                }
                return Err(EditError::Invalid(format!("プリセット {preset_id:?} は使用中のため削除できない（{}）", who.join("、"))));
            }
            s.presets.remove(i);
        }
        EditCommand::InsertPreset { preset, on_conflict } => {
            let mut p = preset.clone();
            if s.presets.iter().any(|x| x.id == p.id) {
                match on_conflict {
                    OnConflict::Error => return Err(EditError::Invalid(format!("プリセット ID {:?} は既にある（名前を変えて読み込むか、既存を削除する）", p.id))),
                    OnConflict::Rename => p.id = rename_on_conflict(&p.id, s.presets.iter().map(|x| x.id.as_str())),
                }
            }
            out.preset_id = Some(p.id.clone());
            s.presets.push(p);
        }
        EditCommand::SetClipPreset { clip_id, preset_id } => {
            let (li, ci) = find_clip(&s, clip_id)?;
            let lid = s.layers[li].id.clone();
            clips_mut(&mut s, &lid)?[ci].preset_id = preset_id.clone();
            out.clip_id = Some(clip_id.clone());
            out.preset_id = Some(preset_id.clone());
        }
        EditCommand::EditClipEffect { clip_id, effect } => {
            let (li, ci) = find_clip(&s, clip_id)?;
            let lid = s.layers[li].id.clone();
            let pid = clips_mut(&mut s, &lid)?[ci].preset_id.clone();
            let (clips, layers) = preset_users(&s, &pid);
            let shared = clips.len() + layers.len() > 1;
            let target = if shared {
                // Copy-on-write: only this clip moves to the new preset.
                let orig = s.presets.iter().find(|p| p.id == pid).ok_or_else(|| EditError::NotFound(format!("preset {pid:?}")))?.clone();
                let id = fresh_id(&format!("{pid}-clip"), s.presets.iter().map(|p| p.id.as_str()));
                s.presets.push(Preset { id: id.clone(), name: format!("{}（{}）", orig.name, clip_id), effect: orig.effect });
                clips_mut(&mut s, &lid)?[ci].preset_id = id.clone();
                id
            } else {
                pid
            };
            let p = s.presets.iter_mut().find(|p| p.id == target).expect("preset exists");
            p.effect = effect.clone();
            out.clip_id = Some(clip_id.clone());
            out.preset_id = Some(target);
        }
        EditCommand::SetSync { beat_source, mtc_output_enabled } => {
            if let Some(b) = beat_source {
                s.sync.beat_source = *b;
            }
            if let Some(on) = mtc_output_enabled {
                s.sync.mtc_output = Some(MtcOutput { enabled: *on });
            }
        }
    }
    let issues = validate(&s);
    if !issues.is_empty() {
        return Err(EditError::Rejected(issues));
    }
    Ok((s, out))
}

#[derive(Clone, Debug)]
struct Entry {
    show: Arc<Show>,
    label: &'static str,
}

/// Undo / Redo over whole-show snapshots.
#[derive(Clone, Debug)]
pub struct History {
    entries: Vec<Entry>,
    cursor: usize,
    /// Cursor position of the last save (None: never saved in this history).
    saved: Option<usize>,
    dropped: usize,
}

impl History {
    /// Start from a loaded (valid) show, which counts as saved.
    pub fn new(show: Show) -> History {
        History { entries: vec![Entry { show: Arc::new(show), label: "読み込み" }], cursor: 0, saved: Some(0), dropped: 0 }
    }

    /// Start from a new show that has never been saved (dirty from the start).
    pub fn new_unsaved(show: Show) -> History {
        History { entries: vec![Entry { show: Arc::new(show), label: "新規" }], cursor: 0, saved: None, dropped: 0 }
    }

    pub fn current(&self) -> &Arc<Show> {
        &self.entries[self.cursor].show
    }

    /// Apply an edit; on success it becomes the current state and the redo
    /// branch is discarded.
    pub fn apply(&mut self, cmd: &EditCommand, snap: Snap) -> Result<EditOutcome, EditError> {
        let (show, out) = apply(self.current(), cmd, snap)?;
        self.entries.truncate(self.cursor + 1);
        if self.saved.is_some_and(|s| s > self.cursor) {
            self.saved = None;
        }
        self.entries.push(Entry { show: Arc::new(show), label: cmd.label() });
        self.cursor += 1;
        if self.entries.len() > MAX_HISTORY + 1 {
            self.entries.remove(0);
            self.cursor -= 1;
            self.dropped += 1;
            self.saved = self.saved.and_then(|s| s.checked_sub(1));
        }
        Ok(out)
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    /// Label of the edit that `undo` would revert.
    pub fn undo_label(&self) -> Option<&'static str> {
        self.can_undo().then(|| self.entries[self.cursor].label)
    }

    pub fn redo_label(&self) -> Option<&'static str> {
        self.can_redo().then(|| self.entries[self.cursor + 1].label)
    }

    pub fn undo(&mut self) -> bool {
        if self.can_undo() {
            self.cursor -= 1;
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if self.can_redo() {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    /// True when the current state differs from the last saved one.
    pub fn dirty(&self) -> bool {
        self.saved != Some(self.cursor)
    }

    pub fn mark_saved(&mut self) {
        self.saved = Some(self.cursor);
    }

    /// Number of edits that can be undone.
    pub fn depth(&self) -> usize {
        self.cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Effect, Param};

    fn demo() -> Show {
        crate::model::demo_for_tests()
    }

    fn clip<'a>(s: &'a Show, id: &str) -> &'a Clip {
        let (li, ci) = find_clip(s, id).unwrap();
        match &s.layers[li].content {
            Content::Timeline { clips } => &clips[ci],
            _ => unreachable!(),
        }
    }

    #[test]
    fn snapping_to_seconds_and_frames() {
        assert_eq!(snap_us(1_499_999, Snap::Second), 1_000_000);
        assert_eq!(snap_us(1_500_000, Snap::Second), 2_000_000);
        assert_eq!(snap_us(-5, Snap::Second), 0);
        assert_eq!(snap_us(16_000, Snap::Frame30), 0);
        assert_eq!(snap_us(17_000, Snap::Frame30), 33_333);
        assert_eq!(snap_us(1_000_000, Snap::Frame30), 1_000_000);
        assert_eq!(snap_us(3_600_000_000 + 20_000, Snap::Frame30), 3_600_033_333);
        assert_eq!(snap_us(123, Snap::None), 123);
    }

    #[test]
    fn clip_edits_and_rejections() {
        let s = demo();
        let (s, o) = apply(&s, &EditCommand::AddClip { layer_id: "base".into(), preset_id: "sweep-cyan".into(), start_us: 61_400_000, duration_us: 4_200_000 }, Snap::Second).unwrap();
        let id = o.clip_id.unwrap();
        assert_eq!(id, "clip-1");
        assert_eq!((clip(&s, &id).start_us, clip(&s, &id).duration_us), (61_000_000, 4_000_000));

        let (s, _) = apply(&s, &EditCommand::MoveClip { clip_id: id.clone(), start_us: 70_000_000, to_layer_id: Some("motion".into()) }, Snap::None).unwrap();
        assert_eq!(find_clip(&s, &id).unwrap().0, 1, "moved to the motion layer");
        let (s, _) = apply(&s, &EditCommand::SetClipTiming { clip_id: id.clone(), start_us: 70_000_000, duration_us: 2_000_000, fade_in_us: 1_500_000, fade_out_us: 0 }, Snap::None).unwrap();
        let (s, _) = apply(&s, &EditCommand::ResizeClip { clip_id: id.clone(), duration_us: 1_000_000 }, Snap::None).unwrap();
        assert_eq!(clip(&s, &id).fade_in_us, 1_000_000, "fades are fitted into a shorter clip");
        let (s, o) = apply(&s, &EditCommand::DuplicateClip { clip_id: id.clone(), start_us: None }, Snap::None).unwrap();
        assert_eq!(clip(&s, o.clip_id.as_deref().unwrap()).start_us, 71_000_000, "copy starts where the original ends");
        let (s, _) = apply(&s, &EditCommand::DeleteClip { clip_id: id.clone() }, Snap::None).unwrap();
        assert!(find_clip(&s, &id).is_err());

        // Rejected: numeric fades longer than the clip, unknown preset, continuous layer, negative time.
        let bad = EditCommand::SetClipTiming { clip_id: "intro-base".into(), start_us: 0, duration_us: 1_000_000, fade_in_us: 800_000, fade_out_us: 800_000 };
        assert!(matches!(apply(&s, &bad, Snap::None), Err(EditError::Rejected(_))));
        let bad = EditCommand::AddClip { layer_id: "base".into(), preset_id: "nope".into(), start_us: 0, duration_us: 1 };
        assert!(matches!(apply(&s, &bad, Snap::None), Err(EditError::Rejected(_))));
        let cont = s.layers.iter().find(|l| matches!(l.content, Content::Continuous { .. })).map(|l| l.id.clone());
        if let Some(c) = cont {
            assert!(matches!(apply(&s, &EditCommand::AddClip { layer_id: c, preset_id: "base-blue".into(), start_us: 0, duration_us: 1 }, Snap::None), Err(EditError::Invalid(_))));
        }
        assert!(matches!(apply(&s, &EditCommand::MoveClip { clip_id: "intro-base".into(), start_us: -1, to_layer_id: None }, Snap::None), Err(EditError::Invalid(_))));
    }

    #[test]
    fn layer_edits_respect_the_limits() {
        let s = demo();
        let n = s.layers.len();
        let (s, o) = apply(&s, &EditCommand::AddLayer { name: "  ".into(), index: Some(0) }, Snap::None).unwrap();
        let id = o.layer_id.unwrap();
        assert_eq!((s.layers[0].id.as_str(), s.layers[0].name.as_str()), (id.as_str(), id.as_str()));
        let (s, _) = apply(&s, &EditCommand::MoveLayer { layer_id: id.clone(), to_index: 99 }, Snap::None).unwrap();
        assert_eq!(s.layers.last().unwrap().id, id, "moved on top (clamped)");
        let (mut s, _) = apply(&s, &EditCommand::DeleteLayer { layer_id: id }, Snap::None).unwrap();
        assert_eq!(s.layers.len(), n);
        while s.layers.len() < crate::validation::MAX_LAYERS {
            s = apply(&s, &EditCommand::AddLayer { name: "x".into(), index: None }, Snap::None).unwrap().0;
        }
        assert!(matches!(apply(&s, &EditCommand::AddLayer { name: "x".into(), index: None }, Snap::None), Err(EditError::Rejected(_))), "max 8 layers");
    }

    #[test]
    fn history_undo_redo_and_dirty() {
        let mut h = History::new(demo());
        assert!(!h.dirty() && !h.can_undo());
        let add = EditCommand::AddLayer { name: "a".into(), index: None };
        h.apply(&add, Snap::None).unwrap();
        h.apply(&EditCommand::AddLayer { name: "b".into(), index: None }, Snap::None).unwrap();
        assert!(h.dirty());
        assert_eq!(h.undo_label(), Some("レイヤーを追加"));
        assert!(h.undo());
        assert_eq!(h.current().layers.last().unwrap().name, "a");
        assert!(h.redo());
        assert_eq!(h.current().layers.last().unwrap().name, "b");
        h.undo();
        h.undo();
        assert!(!h.dirty(), "back at the loaded state");
        h.apply(&EditCommand::AddLayer { name: "c".into(), index: None }, Snap::None).unwrap();
        assert!(!h.can_redo(), "a new edit drops the redo branch");
        h.mark_saved();
        assert!(!h.dirty());
        assert!(h.apply(&EditCommand::DeleteLayer { layer_id: "nope".into() }, Snap::None).is_err());
        assert_eq!(h.depth(), 1, "a rejected edit leaves no history entry");
        for k in 0..(MAX_HISTORY + 10) {
            let name = format!("n{k}");
            let id = h.current().layers.last().unwrap().id.clone();
            h.apply(&EditCommand::MoveLayer { layer_id: id, to_index: 0 }, Snap::None).unwrap();
            let _ = name;
        }
        assert_eq!(h.depth(), MAX_HISTORY);
        assert!(h.dirty());
    }

    #[test]
    fn layer_properties_and_continuous_layers() {
        let s = demo();
        let set = EditCommand::SetLayer { layer_id: "motion".into(), name: Some(" 動き2 ".into()), enabled: Some(false), opacity: Some(0.25), blend: Some(Blend::Add), target_id: None };
        let (s, _) = apply(&s, &set, Snap::None).unwrap();
        let l = s.layers.iter().find(|l| l.id == "motion").unwrap();
        assert_eq!((l.name.as_str(), l.enabled, l.opacity, l.blend), ("動き2", false, 0.25, Blend::Add));
        let bad = EditCommand::SetLayer { layer_id: "motion".into(), name: None, enabled: None, opacity: Some(1.5), blend: None, target_id: None };
        assert!(matches!(apply(&s, &bad, Snap::None), Err(EditError::Rejected(_))), "opacity is 0..1");
        let bad = EditCommand::SetLayer { layer_id: "motion".into(), name: None, enabled: None, opacity: None, blend: None, target_id: Some("nope".into()) };
        assert!(matches!(apply(&s, &bad, Snap::None), Err(EditError::Rejected(_))), "unknown target");
        let (s, o) = apply(&s, &EditCommand::AddContinuousLayer { name: "常時".into(), preset_id: "base-blue".into(), index: None }, Snap::None).unwrap();
        let id = o.layer_id.unwrap();
        let (s, _) = apply(&s, &EditCommand::SetLayerPreset { layer_id: id.clone(), preset_id: "sweep-cyan".into() }, Snap::None).unwrap();
        assert!(matches!(&s.layers.last().unwrap().content, Content::Continuous { preset_id } if preset_id == "sweep-cyan"));
        assert!(matches!(apply(&s, &EditCommand::SetLayerPreset { layer_id: "base".into(), preset_id: "sweep-cyan".into() }, Snap::None), Err(EditError::Invalid(_))));
    }

    #[test]
    fn presets_add_edit_delete_and_import() {
        let s = demo();
        let (s, o) = apply(&s, &EditCommand::AddPreset { name: "赤".into(), effect_type: "builtin.solid".into() }, Snap::None).unwrap();
        let pid = o.preset_id.unwrap();
        assert!(matches!(apply(&s, &EditCommand::AddPreset { name: "x".into(), effect_type: "graph".into() }, Snap::None), Err(EditError::Invalid(_))));
        let red = Effect::Solid { version: 1, params: SolidParams { color: "#FF0000".to_string().into() } };
        let (s, _) = apply(&s, &EditCommand::SetPreset { preset_id: pid.clone(), name: None, effect: Some(red.clone()) }, Snap::None).unwrap();
        let bad = Effect::Chase { version: 1, params: ChaseParams { color: "#FF0000".to_string().into(), period_us: 10.into(), width: 2.into(), axis: Axis::X.into(), reverse: None } };
        assert!(matches!(apply(&s, &EditCommand::SetPreset { preset_id: pid.clone(), name: None, effect: Some(bad) }, Snap::None), Err(EditError::Rejected(_))), "period below the schema range");
        // In use: refused with the users listed.
        let Err(EditError::Invalid(msg)) = apply(&s, &EditCommand::DeletePreset { preset_id: "base-blue".into() }, Snap::None) else { panic!() };
        assert!(msg.contains("intro-base"), "{msg}");
        let (s, _) = apply(&s, &EditCommand::DeletePreset { preset_id: pid.clone() }, Snap::None).unwrap();
        // Import with a conflicting ID.
        let p = s.presets[0].clone();
        assert!(matches!(apply(&s, &EditCommand::InsertPreset { preset: p.clone(), on_conflict: OnConflict::Error }, Snap::None), Err(EditError::Invalid(_))));
        let (s, o) = apply(&s, &EditCommand::InsertPreset { preset: p.clone(), on_conflict: OnConflict::Rename }, Snap::None).unwrap();
        assert_eq!(o.preset_id.as_deref(), Some(format!("{}-2", p.id).as_str()));
        assert_eq!(rename_on_conflict("a", ["a", "a-2"].into_iter()), "a-3");
        assert_eq!(rename_on_conflict(&"x".repeat(64), ["x".repeat(64).as_str()].into_iter()).len(), 64);
        let _ = s;
    }

    #[test]
    fn clip_preset_swap_and_copy_on_write() {
        let s = demo();
        // Two clips share base-blue.
        let (s, o) = apply(&s, &EditCommand::AddClip { layer_id: "motion".into(), preset_id: "base-blue".into(), start_us: 40_000_000, duration_us: 2_000_000 }, Snap::None).unwrap();
        let second = o.clip_id.unwrap();
        let red = Effect::Solid { version: 1, params: SolidParams { color: "#FF0000".to_string().into() } };
        let (s, o) = apply(&s, &EditCommand::EditClipEffect { clip_id: second.clone(), effect: red.clone() }, Snap::None).unwrap();
        let copy = o.preset_id.unwrap();
        assert_ne!(copy, "base-blue", "a shared preset is copied for the single clip");
        assert_eq!(clip(&s, &second).preset_id, copy);
        assert_eq!(clip(&s, "intro-base").preset_id, "base-blue", "the other clip is unchanged");
        let base = s.presets.iter().find(|p| p.id == "base-blue").unwrap();
        assert_ne!(base.effect, red);
        // Now the copy is used once: edited in place.
        let green = Effect::Solid { version: 1, params: SolidParams { color: "#00FF00".to_string().into() } };
        let n = s.presets.len();
        let (s, o) = apply(&s, &EditCommand::EditClipEffect { clip_id: second.clone(), effect: green }, Snap::None).unwrap();
        assert_eq!((o.preset_id.as_deref(), s.presets.len()), (Some(copy.as_str()), n));
        let (s, _) = apply(&s, &EditCommand::SetClipPreset { clip_id: second.clone(), preset_id: "sweep-cyan".into() }, Snap::None).unwrap();
        assert_eq!(clip(&s, &second).preset_id, "sweep-cyan");
        assert!(matches!(apply(&s, &EditCommand::SetClipPreset { clip_id: second, preset_id: "nope".into() }, Snap::None), Err(EditError::Rejected(_))));
    }

    #[test]
    fn new_show_is_valid_empty_and_unsaved() {
        let s = new_show("  ");
        assert!(validate(&s).is_empty(), "{:?}", validate(&s));
        assert_eq!((s.name.as_str(), s.layers.len(), s.presets.len()), ("新しいショー", 0, 1));
        // The show setup of A19: MIDI Clock beats, no MTC output.
        assert_eq!(s.sync.beat_source, BeatSource::MidiClock);
        assert_eq!(s.sync.mtc_output, Some(MtcOutput { enabled: false }));
        let h = History::new_unsaved(s);
        assert!(h.dirty() && !h.can_undo());
    }

    #[test]
    fn triggered_layers_take_clips_and_a_trigger() {
        let s = demo();
        let (s, o) = apply(&s, &EditCommand::AddPreset { name: "r".into(), effect_type: "builtin.ripple".into() }, Snap::None).unwrap();
        let ripple = o.preset_id.unwrap();
        let (s, o) = apply(&s, &EditCommand::AddTriggeredLayer { name: "発火".into(), on: TriggerOn::Press, index: None }, Snap::None).unwrap();
        let layer = o.layer_id.unwrap();
        // Clips are its active windows, added and edited like timeline clips.
        let (s, o) = apply(&s, &EditCommand::AddClip { layer_id: layer.clone(), preset_id: ripple, start_us: 0, duration_us: 8_000_000 }, Snap::None).unwrap();
        let clip = o.clip_id.unwrap();
        let (s, _) = apply(&s, &EditCommand::ResizeClip { clip_id: clip, duration_us: 4_000_000 }, Snap::None).unwrap();
        let trigger = TriggerSpec { on: TriggerOn::Interval, interval_ms: Some(700), origin_x: Some(1.0), origin_y: None };
        let (s, _) = apply(&s, &EditCommand::SetLayerTrigger { layer_id: layer.clone(), trigger: trigger.clone() }, Snap::None).unwrap();
        let l = s.layers.iter().find(|l| l.id == layer).unwrap();
        assert!(matches!(&l.content, Content::Triggered { clips, trigger: t } if clips[0].duration_us == 4_000_000 && *t == trigger));
        // Interval without its interval is refused; other layers have no trigger.
        let bad = TriggerSpec { on: TriggerOn::Interval, interval_ms: None, origin_x: None, origin_y: None };
        assert!(matches!(apply(&s, &EditCommand::SetLayerTrigger { layer_id: layer, trigger: bad.clone() }, Snap::None), Err(EditError::Rejected(_))));
        assert!(matches!(apply(&s, &EditCommand::SetLayerTrigger { layer_id: "base".into(), trigger: bad }, Snap::None), Err(EditError::Invalid(_))));
    }

    #[test]
    fn set_sync_changes_only_the_given_fields() {
        let s = demo();
        assert_eq!(s.sync.beat_source, BeatSource::AbletonLink);
        let cmd: EditCommand = serde_json::from_str(r#"{"op":"set_sync","beat_source":"midi_clock","mtc_output_enabled":null}"#).unwrap();
        let (a, _) = apply(&s, &cmd, Snap::None).unwrap();
        assert_eq!(a.sync.beat_source, BeatSource::MidiClock);
        assert_eq!(a.sync.mtc_output, s.sync.mtc_output, "MTC output unchanged");
        let (b, _) = apply(&a, &EditCommand::SetSync { beat_source: None, mtc_output_enabled: Some(false) }, Snap::None).unwrap();
        assert_eq!((b.sync.beat_source, b.sync.mtc_output.clone()), (BeatSource::MidiClock, Some(MtcOutput { enabled: false })));
        assert_eq!(cmd.label(), "同期の設定");
        // Saved as the schema's snake_case values.
        let j = serde_json::to_value(&b.sync).unwrap();
        assert_eq!((j["beat_source"].as_str(), j["mtc_output"]["enabled"].as_bool()), (Some("midi_clock"), Some(false)));
    }

    #[test]
    fn provisions_for_automation_and_graphs_keep_the_0_1_format() {
        // Params are tagged internally but saved as plain values.
        let s = demo();
        let Effect::Chase { params, .. } = &s.presets[1].effect else { panic!() };
        assert!(params.period_us.is_constant());
        assert_eq!(serde_json::to_value(&params.width).unwrap(), serde_json::json!(2));
        assert_eq!(Param::from(0.5f64), Param::Value(0.5));
        assert!(s.presets.iter().all(|p| p.effect.is_builtin()));
        // A graph reference is not part of Schema 0.1.x.
        let graph = r#"{"id":"g","name":"g","effect":{"type":"graph","version":1,"graph_id":"x"}}"#;
        assert!(serde_json::from_str::<crate::model::Preset>(graph).is_err());
    }
}
