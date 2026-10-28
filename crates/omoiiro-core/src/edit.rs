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

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind { Preset, Compound }

use crate::model::{
    Axis, BeatSource, Blend, ChaseParams, Clip, CrossParams, Mirror, PathKind, PathParams, ShapeKind, ShapeParams, TapGlowParams, Compound, ConstellationParams, Content, CycleParams, CycleScope, Effect, EdgeMode, PadsMode, PadsParams, Recording, MeterChannel, MeterMode, MeterParams, MeterSource, FrameAnchor, FramesParams, Layer, TextFont, TextMode, TextParams, LayerMask, Loss, ParticleMode, ParticlesParams, MtcOutput, NoiseParams, OrbMotion, OrbParams, Preset, RainDirection, RainParams, RegionPattern, RegionsParams, StepMode, StepOrder, RippleFade, RippleParams, RippleTrigger, WaveParams, Preview, PulseParams, RenderSettings, Show, SolidParams,
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
    /// Library metadata on a preset or compound; render output is unaffected.
    SetMaterialMetadata { material_kind: MaterialKind, material_id: String, metadata: crate::model::MaterialMetadata },
    /// Explicit shared edit: every clip and layer using the preset changes.
    SetPreset { preset_id: String, name: Option<String>, effect: Option<Effect> },
    /// Refused while any clip or layer uses the preset (the reason lists them).
    DeletePreset { preset_id: String },
    /// Insert a preset (import). An ID conflict is an error or a rename.
    InsertPreset { preset: Preset, on_conflict: OnConflict },
    /// Library import (A20): the presets the compound uses (an identical
    /// preset already in the show is reused), then the compound with its
    /// preset, layer and clip IDs remapped. An identical compound is reused.
    InsertCompound { compound: Compound, presets: Vec<Preset>, on_conflict: OnConflict },
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
    /// 0.1.3: set or clear a layer's mask (any layer kind).
    SetLayerMask { layer_id: String, mask: Option<LayerMask> },
    /// 0.1.3 stage 6: new compound (one empty inner timeline layer, 4 s).
    AddCompound { name: String },
    /// Rename / set the own length (`duration_us` 0 = the latest inner clip end).
    SetCompound { compound_id: String, name: Option<String>, duration_us: Option<u64> },
    /// Refused while any clip places it (the reason lists them).
    DeleteCompound { compound_id: String },
    /// New inner layer on top: continuous with `preset_id`, else a timeline.
    AddCompoundLayer { compound_id: String, name: String, preset_id: Option<String> },
    /// Place a compound on a timeline layer (`duration_us` absent = its length).
    AddCompoundClip { layer_id: String, compound_id: String, start_us: i64, duration_us: Option<i64> },
    /// Tint ("" = none), time scale (1 = none) and loop of a compound clip.
    SetCompoundClip { clip_id: String, tint: Option<String>, time_scale: Option<f64>, looping: Option<bool> },
    /// 0.1.3 stage 7: a frames preset from an external asset (the store
    /// reads the file). An ID conflict is an error or a rename.
    ImportFrames { asset: crate::assets::FramesAsset, on_conflict: OnConflict },
    /// 0.1.3 stage 8: a new triggered clip holding a recording (its length,
    /// at least 1 µs) drawn with `preset_id`.
    AddRecordingClip { layer_id: String, preset_id: String, start_us: i64, recording: Recording },
    /// Replace (null: remove) the recording of a clip; `looping` null leaves loop as it is.
    SetClipRecording { clip_id: String, recording: Option<Recording>, looping: Option<bool> },
    /// Copy a compound clip out into independent layers above its layer
    /// (refused unless its time scale is 1: effect speeds cannot be scaled).
    DetachCompoundClip { clip_id: String },
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
        presets: vec![Preset { metadata: Default::default(), id: "preset-white".into(), name: "白".into(), effect: default_effect("builtin.solid").expect("solid") }],
        compounds: Vec::new(),
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
                groups: None,
                offset: None,
            },
        },
        // A spot running round the border with a short tail.
        "builtin.path" => Effect::Path {
            version: 1,
            params: PathParams {
                color: "#FFFFFF".to_string().into(),
                path: PathKind::Perimeter.into(),
                points: None,
                from_x: None,
                from_y: None,
                to_x: None,
                to_y: None,
                center_x: None,
                center_y: None,
                orbit_radius: None,
                speed: 0.25.into(),
                size: 1.0.into(),
                tail: 3.0.into(),
                looping: true.into(),
                phase: 0.0.into(),
                mirror: Mirror::None.into(),
            },
        },
        "builtin.particles" => Effect::Particles {
            version: 1,
            params: ParticlesParams {
                mode: ParticleMode::Burst.into(),
                count: Some(16.into()),
                rate: None,
                color: "#FFC040".to_string().into(),
                color_end: Some("#FF2000".to_string().into()),
                speed_min: 2.0.into(),
                speed_max: 6.0.into(),
                angle: 90.0.into(),
                spread: 360.0.into(),
                gravity: (-6.0).into(),
                drag: 1.0.into(),
                lifetime_ms: 1200.into(),
                size: 0.75.into(),
                tail: 1.0.into(),
                edge_mode: EdgeMode::Vanish.into(),
                origin_x: None,
                origin_y: None,
                seed: 1.into(),
                rise_speed: None,
                burst_height: None,
                pause_ms: None,
                implode_radius: None,
            },
        },
        // A white square outline (draw your own frames, or import an asset).
        "builtin.frames" => Effect::Frames {
            version: 1,
            params: FramesParams {
                frames: vec![(0..64).map(|i| if i / 8 == 0 || i / 8 == 7 || i % 8 == 0 || i % 8 == 7 { "FFFFFF" } else { "000000" }).collect()],
                fps: 8.0,
                looping: true,
                hold_last: false,
                hold_ms: None,
                anchor: FrameAnchor::Board,
                color: None,
                opacity_key: None,
            },
        },
        "builtin.text" => Effect::Text {
            version: 1,
            params: TextParams { text: "OMOI-IRO".into(), font: TextFont::F5x7, color: "#FFFFFF".to_string().into(), mode: TextMode::ScrollLeft, speed: 6.0, gap: 1, beat_step: None, looping: true },
        },
        "builtin.pads" => Effect::Pads {
            version: 1,
            params: PadsParams { mode: PadsMode::Heat, color_low: Some("#200040".into()), color_high: Some("#FF4020".into()), color: None, palette: None, gain: Some(0.25), decay_ms: Some(3000), attack_ms: None, sustain_ms: None, release_ms: None, size: None },
        },
        "builtin.cycle" => Effect::Cycle {
            version: 1,
            params: CycleParams { palette: vec!["#FF2060".into(), "#FFC020".into(), "#20E0A0".into(), "#4060FF".into()], scope: CycleScope::Global, lifetime_ms: 600, radius: 0.0 },
        },
        "builtin.constellation" => Effect::Constellation {
            version: 1,
            params: ConstellationParams { color: "#A0C0FF".into(), stroke: 1.0, lifetime_ms: 2000, fade: RippleFade::Linear, max_segments: 6, dots: true },
        },
        "builtin.meter" => Effect::Meter {
            version: 1,
            params: MeterParams {
                source: MeterSource::Master,
                track_name: None,
                channel: MeterChannel::Mix,
                mode: MeterMode::Bar,
                color_low: "#00C060".into(),
                color_high: "#FF2020".into(),
                gain: 1.0,
                floor: 0.05,
                attack_ms: 20,
                release_ms: 300,
                columns: None,
                peak_hold_ms: Some(600),
            },
        },
        "builtin.shape" => Effect::Shape {
            version: 1,
            params: ShapeParams {
                shape: ShapeKind::Circle.into(),
                color: "#FFFFFF".to_string().into(),
                fill: true.into(),
                stroke: 1.0.into(),
                center_x: 3.5.into(),
                center_y: 3.5.into(),
                size: 3.0.into(),
                size_y: None,
                angle: 0.0.into(),
                edge: 0.5.into(),
                invert: None,
            },
        },
        "builtin.tap_glow" => Effect::TapGlow {
            version: 1,
            params: TapGlowParams {
                charge: None,
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
    for l in all_layers(show) {
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
            EditCommand::SetMaterialMetadata { .. } => "素材の分類",
            EditCommand::SetPreset { .. } => "プリセットを編集（共有）",
            EditCommand::DeletePreset { .. } => "プリセットを削除",
            EditCommand::InsertPreset { .. } => "プリセットを読み込み",
            EditCommand::InsertCompound { .. } => "ライブラリーから読み込み",
            EditCommand::SetClipPreset { .. } => "クリップのプリセット",
            EditCommand::EditClipEffect { .. } => "クリップの効果を編集",
            EditCommand::SetSync { .. } => "同期の設定",
            EditCommand::AddTriggeredLayer { .. } => "発火レイヤーを追加",
            EditCommand::SetLayerTrigger { .. } => "発火の設定",
            EditCommand::SetLayerMask { .. } => "マスクの設定",
            EditCommand::AddCompound { .. } => "素材を追加",
            EditCommand::SetCompound { .. } => "素材の設定",
            EditCommand::DeleteCompound { .. } => "素材を削除",
            EditCommand::AddCompoundLayer { .. } => "素材にレイヤーを追加",
            EditCommand::AddCompoundClip { .. } => "素材を置く",
            EditCommand::SetCompoundClip { .. } => "素材の色と速さ",
            EditCommand::DetachCompoundClip { .. } => "素材を展開して独立",
            EditCommand::ImportFrames { .. } => "フレーム列を読み込み",
            EditCommand::AddRecordingClip { .. } => "録音を追加",
            EditCommand::SetClipRecording { .. } => "録音の設定",
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
    pub compound_id: Option<String>,
}

fn non_negative(name: &str, v: i64) -> Result<u64, EditError> {
    u64::try_from(v).map_err(|_| EditError::Invalid(format!("{name} must be >= 0 (got {v})")))
}

/// Every layer: the show's, then the inner layers of the compounds.
fn all_layers(show: &Show) -> impl Iterator<Item = &Layer> {
    show.layers.iter().chain(show.compounds.iter().flat_map(|c| c.layers.iter()))
}

fn all_layer_ids(show: &Show) -> impl Iterator<Item = &str> {
    all_layers(show).map(|l| l.id.as_str())
}

/// The list holding a layer (the show's or a compound's) and its index there.
fn layer_list_mut<'a>(show: &'a mut Show, layer_id: &str) -> Result<(&'a mut Vec<Layer>, usize), EditError> {
    let lists = std::iter::once(&mut show.layers).chain(show.compounds.iter_mut().map(|c| &mut c.layers));
    for list in lists {
        if let Some(i) = list.iter().position(|l| l.id == layer_id) {
            return Ok((list, i));
        }
    }
    Err(EditError::NotFound(format!("layer {layer_id:?}")))
}

fn layer_mut<'a>(show: &'a mut Show, layer_id: &str) -> Result<&'a mut Layer, EditError> {
    let (list, i) = layer_list_mut(show, layer_id)?;
    Ok(&mut list[i])
}

fn clips_mut<'a>(show: &'a mut Show, layer_id: &str) -> Result<&'a mut Vec<Clip>, EditError> {
    let layer = layer_mut(show, layer_id)?;
    match layer.content.clips_mut() {
        Some(clips) => Ok(clips),
        None => Err(EditError::Invalid(format!("layer {layer_id:?} is continuous, not a timeline"))),
    }
}

/// (layer id, clip index) of a clip, in the show's layers or a compound's.
fn find_clip(show: &Show, clip_id: &str) -> Result<(String, usize), EditError> {
    for l in all_layers(show) {
        if let Some(clips) = l.content.clips() {
            if let Some(ci) = clips.iter().position(|c| c.id == clip_id) {
                return Ok((l.id.clone(), ci));
            }
        }
    }
    Err(EditError::NotFound(format!("clip {clip_id:?}")))
}

fn all_clip_ids(show: &Show) -> impl Iterator<Item = &str> {
    all_layers(show).flat_map(|l| match l.content.clips() {
        Some(clips) => clips.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        None => Vec::new(),
    })
}

/// `existing` is `id` or what `rename_on_conflict` made of it (`<id>-N`).
fn same_or_renamed(existing: &str, id: &str) -> bool {
    existing == id || existing.strip_prefix(id).and_then(|r| r.strip_prefix('-')).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Library import (A20): the same preset (ID or its renamed form, name and
/// effect; metadata is not compared), so importing it again adds nothing.
pub fn same_preset(existing: &Preset, imported: &Preset) -> bool {
    same_or_renamed(&existing.id, &imported.id) && existing.name == imported.name && existing.effect == imported.effect
}

/// The preset already in the show that `imported` would duplicate (the exact ID first).
pub fn existing_preset<'a>(show: &'a Show, imported: &Preset) -> Option<&'a Preset> {
    show.presets.iter().find(|x| x.id == imported.id && same_preset(x, imported)).or_else(|| show.presets.iter().find(|x| same_preset(x, imported)))
}

/// The same compound apart from its inner layer and clip IDs.
fn same_compound(a: &Compound, b: &Compound) -> bool {
    let strip = |c: &Compound| {
        let mut layers = c.layers.clone();
        for l in &mut layers {
            l.id.clear();
            for clip in l.content.clips_mut().into_iter().flatten() {
                clip.id.clear();
            }
        }
        layers
    };
    same_or_renamed(&a.id, &b.id) && a.name == b.name && a.duration_us == b.duration_us && strip(a) == strip(b)
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
            let clip = Clip { id: id.clone(), preset_id: preset_id.clone(), start_us: at(*start_us), duration_us: duration, ..Default::default() };
            clips_mut(&mut s, layer_id)?.push(clip);
            out.clip_id = Some(id);
            out.layer_id = Some(layer_id.clone());
        }
        EditCommand::MoveClip { clip_id, start_us, to_layer_id } => {
            non_negative("start_us", *start_us)?;
            let (from_layer, ci) = find_clip(&s, clip_id)?;
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
            let (lid, ci) = find_clip(&s, clip_id)?;
            let c = &mut clips_mut(&mut s, &lid)?[ci];
            c.duration_us = d;
            fit_fades(c);
            out.clip_id = Some(clip_id.clone());
        }
        EditCommand::SetClipTiming { clip_id, start_us, duration_us, fade_in_us, fade_out_us } => {
            let (start, duration) = (non_negative("start_us", *start_us)?, non_negative("duration_us", *duration_us)?);
            let (fi, fo) = (non_negative("fade_in_us", *fade_in_us)?, non_negative("fade_out_us", *fade_out_us)?);
            let (lid, ci) = find_clip(&s, clip_id)?;
            let c = &mut clips_mut(&mut s, &lid)?[ci];
            // Numeric input: snapped like drags, fades kept as typed (validation reports misfits).
            c.start_us = at(start as i64);
            c.duration_us = at(duration as i64);
            c.fade_in_us = fi;
            c.fade_out_us = fo;
            out.clip_id = Some(clip_id.clone());
        }
        EditCommand::DuplicateClip { clip_id, start_us } => {
            let (lid, ci) = find_clip(&s, clip_id)?;
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
            let (lid, ci) = find_clip(&s, clip_id)?;
            clips_mut(&mut s, &lid)?.remove(ci);
            out.layer_id = Some(lid);
        }
        EditCommand::AddLayer { name, index } => {
            let target_id = s.targets.first().map(|t| t.id.clone()).ok_or_else(|| EditError::Invalid("the show has no target".into()))?;
            let id = fresh_id("layer", all_layer_ids(&s));
            let name = if name.trim().is_empty() { id.clone() } else { name.trim().to_string() };
            let layer = Layer { id: id.clone(), name, target_id, enabled: true, opacity: 1.0, blend: Blend::Over, content: Content::Timeline { clips: Vec::new() }, mask: None };
            let i = index.unwrap_or(s.layers.len()).min(s.layers.len());
            s.layers.insert(i, layer);
            out.layer_id = Some(id);
        }
        EditCommand::AddTriggeredLayer { name, on, index } => {
            let target_id = s.targets.first().map(|t| t.id.clone()).ok_or_else(|| EditError::Invalid("the show has no target".into()))?;
            let id = fresh_id("layer", all_layer_ids(&s));
            let name = if name.trim().is_empty() { id.clone() } else { name.trim().to_string() };
            let interval_ms = (*on == TriggerOn::Interval).then_some(1000);
            let trigger = TriggerSpec { on: *on, interval_ms, origin_x: None, origin_y: None };
            let layer = Layer { id: id.clone(), name, target_id, enabled: true, opacity: 1.0, blend: Blend::Over, content: Content::Triggered { clips: Vec::new(), trigger }, mask: None };
            let i = index.unwrap_or(s.layers.len()).min(s.layers.len());
            s.layers.insert(i, layer);
            out.layer_id = Some(id);
        }
        EditCommand::SetLayerTrigger { layer_id, trigger } => {
            let l = layer_mut(&mut s, layer_id)?;
            match &mut l.content {
                Content::Triggered { trigger: t, .. } => *t = trigger.clone(),
                _ => return Err(EditError::Invalid(format!("layer {layer_id:?} is not a triggered layer"))),
            }
            out.layer_id = Some(layer_id.clone());
        }
        EditCommand::SetLayerMask { layer_id, mask } => {
            let l = layer_mut(&mut s, layer_id)?;
            l.mask = mask.clone();
            out.layer_id = Some(layer_id.clone());
        }
        EditCommand::DeleteLayer { layer_id } => {
            let (list, i) = layer_list_mut(&mut s, layer_id)?;
            list.remove(i);
        }
        EditCommand::MoveLayer { layer_id, to_index } => {
            let (list, i) = layer_list_mut(&mut s, layer_id)?;
            let l = list.remove(i);
            list.insert((*to_index).min(list.len()), l);
            out.layer_id = Some(layer_id.clone());
        }
        EditCommand::SetLayer { layer_id, name, enabled, opacity, blend, target_id } => {
            let l = layer_mut(&mut s, layer_id)?;
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
            let id = fresh_id("layer", all_layer_ids(&s));
            let name = if name.trim().is_empty() { id.clone() } else { name.trim().to_string() };
            let layer = Layer { id: id.clone(), name, target_id, enabled: true, opacity: 1.0, blend: Blend::Over, content: Content::Continuous { preset_id: preset_id.clone() }, mask: None };
            let i = index.unwrap_or(s.layers.len()).min(s.layers.len());
            s.layers.insert(i, layer);
            out.layer_id = Some(id);
        }
        EditCommand::SetLayerPreset { layer_id, preset_id } => {
            let l = layer_mut(&mut s, layer_id)?;
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
            s.presets.push(Preset { metadata: Default::default(), id: id.clone(), name, effect });
            out.preset_id = Some(id);
        }
        EditCommand::SetMaterialMetadata { material_kind, material_id, metadata } => {
            let dest = match material_kind {
                MaterialKind::Preset => &mut s.presets.iter_mut().find(|p| &p.id == material_id).ok_or_else(|| EditError::NotFound(material_id.clone()))?.metadata,
                MaterialKind::Compound => &mut s.compounds.iter_mut().find(|p| &p.id == material_id).ok_or_else(|| EditError::NotFound(material_id.clone()))?.metadata,
            };
            *dest = metadata.clone();
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
        EditCommand::InsertCompound { compound, presets, on_conflict } => {
            let conflict = |what: &str, id: &str| EditError::Invalid(format!("{what} ID {id:?} は既にある（名前を変えて読み込むか、既存を削除する）"));
            let mut preset_ids: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
            for p in presets {
                if preset_ids.contains_key(p.id.as_str()) {
                    continue;
                }
                let id = match existing_preset(&s, p).map(|x| x.id.clone()) {
                    Some(x) => x,
                    None => {
                        let mut q = p.clone();
                        if s.presets.iter().any(|x| x.id == q.id) {
                            match on_conflict {
                                OnConflict::Error => return Err(conflict("プリセット", &q.id)),
                                OnConflict::Rename => q.id = rename_on_conflict(&q.id, s.presets.iter().map(|x| x.id.as_str())),
                            }
                        }
                        let id = q.id.clone();
                        s.presets.push(q);
                        id
                    }
                };
                preset_ids.insert(&p.id, id);
            }
            let remap = |pid: &mut String| {
                if let Some(id) = preset_ids.get(pid.as_str()) {
                    *pid = id.clone();
                }
            };
            let mut cp = compound.clone();
            let mut used_layers: Vec<String> = all_layer_ids(&s).map(str::to_string).collect();
            let mut used_clips: Vec<String> = all_clip_ids(&s).map(str::to_string).collect();
            for l in &mut cp.layers {
                l.id = rename_on_conflict(&l.id, used_layers.iter().map(String::as_str));
                used_layers.push(l.id.clone());
                match &mut l.content {
                    Content::Continuous { preset_id } => remap(preset_id),
                    content => {
                        for c in content.clips_mut().into_iter().flatten() {
                            c.id = rename_on_conflict(&c.id, used_clips.iter().map(String::as_str));
                            used_clips.push(c.id.clone());
                            remap(&mut c.preset_id);
                        }
                    }
                }
            }
            if let Some(x) = s.compounds.iter().find(|x| same_compound(x, &cp)) {
                out.compound_id = Some(x.id.clone());
            } else {
                if s.compounds.iter().any(|x| x.id == cp.id) {
                    match on_conflict {
                        OnConflict::Error => return Err(conflict("素材", &cp.id)),
                        OnConflict::Rename => cp.id = rename_on_conflict(&cp.id, s.compounds.iter().map(|x| x.id.as_str())),
                    }
                }
                out.compound_id = Some(cp.id.clone());
                s.compounds.push(cp);
            }
        }
        EditCommand::SetClipPreset { clip_id, preset_id } => {
            let (lid, ci) = find_clip(&s, clip_id)?;
            clips_mut(&mut s, &lid)?[ci].preset_id = preset_id.clone();
            out.clip_id = Some(clip_id.clone());
            out.preset_id = Some(preset_id.clone());
        }
        EditCommand::EditClipEffect { clip_id, effect } => {
            let (lid, ci) = find_clip(&s, clip_id)?;
            let pid = clips_mut(&mut s, &lid)?[ci].preset_id.clone();
            let (clips, layers) = preset_users(&s, &pid);
            let shared = clips.len() + layers.len() > 1;
            let target = if shared {
                // Copy-on-write: only this clip moves to the new preset.
                let orig = s.presets.iter().find(|p| p.id == pid).ok_or_else(|| EditError::NotFound(format!("preset {pid:?}")))?.clone();
                let id = fresh_id(&format!("{pid}-clip"), s.presets.iter().map(|p| p.id.as_str()));
                s.presets.push(Preset { metadata: orig.metadata.clone(), id: id.clone(), name: format!("{}（{}）", orig.name, clip_id), effect: orig.effect });
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
        EditCommand::AddCompound { name } => {
            let id = fresh_id("compound", s.compounds.iter().map(|c| c.id.as_str()));
            let layer_id = fresh_id("layer", all_layer_ids(&s));
            let name = if name.trim().is_empty() { id.clone() } else { name.trim().to_string() };
            let layer = Layer { id: layer_id.clone(), name: "1".into(), target_id: String::new(), enabled: true, opacity: 1.0, blend: Blend::Over, content: Content::Timeline { clips: Vec::new() }, mask: None };
            s.compounds.push(Compound { metadata: Default::default(), id: id.clone(), name, duration_us: Some(4_000_000), layers: vec![layer] });
            out.compound_id = Some(id);
            out.layer_id = Some(layer_id);
        }
        EditCommand::SetCompound { compound_id, name, duration_us } => {
            let c = s.compounds.iter_mut().find(|c| &c.id == compound_id).ok_or_else(|| EditError::NotFound(format!("compound {compound_id:?}")))?;
            if let Some(n) = name {
                c.name = n.trim().to_string();
            }
            if let Some(d) = duration_us {
                c.duration_us = (*d > 0).then_some(*d);
            }
            out.compound_id = Some(compound_id.clone());
        }
        EditCommand::DeleteCompound { compound_id } => {
            let i = s.compounds.iter().position(|c| &c.id == compound_id).ok_or_else(|| EditError::NotFound(format!("compound {compound_id:?}")))?;
            let users: Vec<&str> = all_layers(&s).flat_map(|l| l.content.clips().into_iter().flatten()).filter(|c| c.compound_id.as_ref() == Some(compound_id)).map(|c| c.id.as_str()).collect();
            if !users.is_empty() {
                return Err(EditError::Invalid(format!("素材 {compound_id:?} は使用中のため削除できない（クリップ {}）", users.join(", "))));
            }
            s.compounds.remove(i);
        }
        EditCommand::AddCompoundLayer { compound_id, name, preset_id } => {
            let id = fresh_id("layer", all_layer_ids(&s));
            let c = s.compounds.iter_mut().find(|c| &c.id == compound_id).ok_or_else(|| EditError::NotFound(format!("compound {compound_id:?}")))?;
            let name = if name.trim().is_empty() { (c.layers.len() + 1).to_string() } else { name.trim().to_string() };
            let content = match preset_id {
                Some(p) => Content::Continuous { preset_id: p.clone() },
                None => Content::Timeline { clips: Vec::new() },
            };
            c.layers.push(Layer { id: id.clone(), name, target_id: String::new(), enabled: true, opacity: 1.0, blend: Blend::Over, content, mask: None });
            out.compound_id = Some(compound_id.clone());
            out.layer_id = Some(id);
        }
        EditCommand::AddCompoundClip { layer_id, compound_id, start_us, duration_us } => {
            non_negative("start_us", *start_us)?;
            let length = s.compounds.iter().find(|c| &c.id == compound_id).ok_or_else(|| EditError::NotFound(format!("compound {compound_id:?}")))?.length_us();
            let duration = match duration_us {
                Some(d) => at(non_negative("duration_us", *d)? as i64),
                None => length,
            };
            if duration == 0 {
                return Err(EditError::Invalid("duration must be > 0".into()));
            }
            let id = fresh_id("clip", all_clip_ids(&s));
            let clip = Clip { id: id.clone(), compound_id: Some(compound_id.clone()), start_us: at(*start_us), duration_us: duration, ..Default::default() };
            clips_mut(&mut s, layer_id)?.push(clip);
            out.clip_id = Some(id);
            out.layer_id = Some(layer_id.clone());
            out.compound_id = Some(compound_id.clone());
        }
        EditCommand::SetCompoundClip { clip_id, tint, time_scale, looping } => {
            let (lid, ci) = find_clip(&s, clip_id)?;
            let c = &mut clips_mut(&mut s, &lid)?[ci];
            if c.compound_id.is_none() {
                return Err(EditError::Invalid(format!("clip {clip_id:?} does not place a compound")));
            }
            if let Some(tint) = tint {
                c.tint = (!tint.is_empty()).then(|| tint.to_uppercase());
            }
            if let Some(k) = time_scale {
                c.time_scale = (*k != 1.0).then_some(*k);
            }
            if let Some(l) = looping {
                c.looping = l.then_some(true);
            }
            out.clip_id = Some(clip_id.clone());
        }
        EditCommand::DetachCompoundClip { clip_id } => {
            let (lid, ci) = find_clip(&s, clip_id)?;
            let li = s.layers.iter().position(|l| l.id == lid).ok_or_else(|| EditError::Invalid(format!("clip {clip_id:?} is inside a compound")))?;
            let clip = clips_mut(&mut s, &lid)?[ci].clone();
            let cid = clip.compound_id.clone().ok_or_else(|| EditError::Invalid(format!("clip {clip_id:?} does not place a compound")))?;
            if clip.time_scale.is_some_and(|k| k != 1.0) {
                return Err(EditError::Invalid("速さ（time_scale）が 1 以外の素材は展開できない。効果そのものの速さ（流れの周期や雨の速さ）は縮尺できず、見た目が変わるため。先に速さを 1 に戻す".into()));
            }
            let cp = s.compounds.iter().find(|c| c.id == cid).cloned().ok_or_else(|| EditError::NotFound(format!("compound {cid:?}")))?;
            let outer = s.layers[li].clone();
            let length = cp.length_us().max(1);
            let reps = if clip.looping.unwrap_or(false) { clip.duration_us.div_ceil(length) } else { 1 };
            // Tinted presets are copied once each.
            let mut copies: std::collections::HashMap<String, String> = std::collections::HashMap::new();
            let mut preset_for = |s: &mut Show, pid: &str| -> String {
                let (Some(tint), Some(rgb)) = (&clip.tint, clip.tint.as_deref().and_then(crate::surface::Rgb::from_hex)) else { return pid.to_string() };
                if let Some(id) = copies.get(pid) {
                    return id.clone();
                }
                let Some(orig) = s.presets.iter().find(|p| p.id == pid).cloned() else { return pid.to_string() };
                let id = fresh_id(&format!("{pid}-tint"), s.presets.iter().map(|p| p.id.as_str()));
                s.presets.push(Preset { metadata: orig.metadata.clone(), id: id.clone(), name: format!("{}（{tint}）", orig.name), effect: crate::render::tinted(&orig.effect, rgb) });
                copies.insert(pid.to_string(), id.clone());
                id
            };
            let mut used_layers: Vec<String> = all_layer_ids(&s).map(str::to_string).collect();
            let mut used_clips: Vec<String> = all_clip_ids(&s).map(str::to_string).collect();
            let mut new_layers = Vec::new();
            for il in &cp.layers {
                let mut clips = Vec::new();
                for r in 0..reps {
                    let base = r * length;
                    // Inner windows (start, end, preset, fades) of this repetition, in clip time.
                    let windows: Vec<(u64, u64, String, u64, u64)> = match &il.content {
                        Content::Continuous { preset_id } => vec![(base, base + length, preset_id.clone(), 0, 0)],
                        _ => il.content.clips().into_iter().flatten().filter(|c| c.start_us < length).map(|c| (base + c.start_us, base + c.end_us().min(length), c.preset_id.clone(), c.fade_in_us, c.fade_out_us)).collect(),
                    };
                    for (a, b, pid, fi, fo) in windows {
                        let b = b.min(clip.duration_us);
                        if a >= b {
                            continue;
                        }
                        let id = fresh_id("clip", used_clips.iter().map(String::as_str));
                        used_clips.push(id.clone());
                        let preset_id = preset_for(&mut s, &pid);
                        let mut c = Clip { id, preset_id, start_us: clip.start_us + a, duration_us: b - a, fade_in_us: fi, fade_out_us: fo, ..Default::default() };
                        // The placing clip's fades go to the pieces at its ends.
                        if a == 0 {
                            c.fade_in_us = c.fade_in_us.max(clip.fade_in_us);
                        }
                        if b == clip.duration_us {
                            c.fade_out_us = c.fade_out_us.max(clip.fade_out_us);
                        }
                        fit_fades(&mut c);
                        clips.push(c);
                    }
                }
                let id = fresh_id("layer", used_layers.iter().map(String::as_str));
                used_layers.push(id.clone());
                new_layers.push(Layer {
                    id,
                    name: format!("{} / {}", cp.name, il.name),
                    target_id: outer.target_id.clone(),
                    enabled: il.enabled && outer.enabled,
                    opacity: il.opacity * outer.opacity,
                    blend: il.blend,
                    content: Content::Timeline { clips },
                    mask: il.mask.clone(),
                });
            }
            clips_mut(&mut s, &lid)?.remove(ci);
            out.layer_id = new_layers.first().map(|l| l.id.clone());
            for (k, l) in new_layers.into_iter().enumerate() {
                s.layers.insert(li + 1 + k, l);
            }
            out.compound_id = Some(cid);
        }
        EditCommand::AddRecordingClip { layer_id, preset_id, start_us, recording } => {
            non_negative("start_us", *start_us)?;
            let id = fresh_id("clip", all_clip_ids(&s));
            let clip = Clip { id: id.clone(), preset_id: preset_id.clone(), start_us: at(*start_us), duration_us: recording.length(), recording: Some(recording.clone()), ..Default::default() };
            clips_mut(&mut s, layer_id)?.push(clip);
            out.clip_id = Some(id);
            out.layer_id = Some(layer_id.clone());
        }
        EditCommand::SetClipRecording { clip_id, recording, looping } => {
            let (lid, ci) = find_clip(&s, clip_id)?;
            let c = &mut clips_mut(&mut s, &lid)?[ci];
            c.recording = recording.clone();
            if let Some(l) = looping {
                c.looping = l.then_some(true);
            }
            if c.recording.is_none() && c.compound_id.is_none() {
                c.looping = None;
            }
            out.clip_id = Some(clip_id.clone());
        }
        EditCommand::ImportFrames { asset, on_conflict } => {
            let params = asset.to_params().map_err(|e| EditError::Invalid(format!("フレーム列 {:?} を読み込めない：{e}", asset.id())))?;
            let mut id = asset.id().to_string();
            if s.presets.iter().any(|p| p.id == id) {
                match on_conflict {
                    OnConflict::Error => return Err(EditError::Invalid(format!("プリセット ID {id:?} は既にある（名前を変えて読み込むか、既存を削除する）"))),
                    OnConflict::Rename => id = rename_on_conflict(&id, s.presets.iter().map(|x| x.id.as_str())),
                }
            }
            let name = if asset.name().trim().is_empty() { id.clone() } else { asset.name().trim().to_string() };
            s.presets.push(Preset { metadata: Default::default(), id: id.clone(), name, effect: Effect::Frames { version: 1, params } });
            out.preset_id = Some(id);
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
        let (lid, ci) = find_clip(s, id).unwrap();
        &all_layers(s).find(|l| l.id == lid).unwrap().content.clips().unwrap()[ci]
    }

    #[test]
    fn material_metadata_history_validation_and_copy() {
        use crate::model::MaterialMetadata;
        let mut h = History::new(new_show("Library"));
        let m = MaterialMetadata { category: "タップ".into(), tags: vec!["暖色".into(), "サビ".into()], favorite: true };
        let cmd = EditCommand::SetMaterialMetadata { material_kind: MaterialKind::Preset, material_id: "preset-white".into(), metadata: m.clone() };
        h.apply(&cmd, Snap::None).unwrap();
        assert_eq!(h.current().presets[0].metadata, m);
        assert!(h.dirty());
        h.undo(); assert!(h.current().presets[0].metadata.is_empty());
        h.redo(); assert_eq!(h.current().presets[0].metadata, m);
        let cp = h.apply(&EditCommand::AddCompound { name: "複合".into() }, Snap::None).unwrap().compound_id.unwrap();
        h.apply(&EditCommand::SetMaterialMetadata { material_kind: MaterialKind::Compound, material_id: cp, metadata: m.clone() }, Snap::None).unwrap();
        assert_eq!(h.current().compounds[0].metadata, m);
        let before = h.current().clone();
        for tags in [vec!["same".into(), "SAME".into()], vec![" ".into()], vec!["a".repeat(33)], vec!["x".into(); 25]] {
            assert!(h.apply(&EditCommand::SetMaterialMetadata { material_kind: MaterialKind::Preset, material_id: "preset-white".into(), metadata: MaterialMetadata { tags, ..m.clone() } }, Snap::None).is_err());
            assert_eq!(h.current(), &before);
        }
        assert!(h.apply(&EditCommand::SetMaterialMetadata { material_kind: MaterialKind::Preset, material_id: "missing".into(), metadata: m.clone() }, Snap::None).is_err());
        let layer = h.apply(&EditCommand::AddLayer { name: "Timeline".into(), index: None }, Snap::None).unwrap().layer_id.unwrap();
        let first = h.apply(&EditCommand::AddClip { layer_id: layer.clone(), preset_id: "preset-white".into(), start_us: 0, duration_us: 1_000_000 }, Snap::None).unwrap().clip_id.unwrap();
        h.apply(&EditCommand::AddClip { layer_id: layer, preset_id: "preset-white".into(), start_us: 1_000_000, duration_us: 1_000_000 }, Snap::None).unwrap();
        let copy = h.apply(&EditCommand::EditClipEffect { clip_id: first, effect: default_effect("builtin.chase").unwrap() }, Snap::None).unwrap().preset_id.unwrap();
        assert_eq!(h.current().presets.iter().find(|p| p.id == copy).unwrap().metadata, m);
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
        assert_eq!(find_clip(&s, &id).unwrap().0, s.layers[1].id, "moved to the motion layer");
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
    fn library_compounds_import_with_their_presets_once() {
        // Library: a compound with a clip of sweep-cyan and a continuous base-blue layer.
        let lib = demo();
        let (lib, o) = apply(&lib, &EditCommand::AddCompound { name: "素材".into() }, Snap::None).unwrap();
        let (cp, inner) = (o.compound_id.unwrap(), o.layer_id.unwrap());
        let (lib, _) = apply(&lib, &EditCommand::AddClip { layer_id: inner, preset_id: "sweep-cyan".into(), start_us: 0, duration_us: 1_000_000 }, Snap::None).unwrap();
        let (lib, _) = apply(&lib, &EditCommand::AddCompoundLayer { compound_id: cp.clone(), name: "".into(), preset_id: Some("base-blue".into()) }, Snap::None).unwrap();
        let compound = lib.compounds.iter().find(|c| c.id == cp).unwrap().clone();
        let presets: Vec<Preset> = lib.presets.iter().filter(|p| p.id == "sweep-cyan" || p.id == "base-blue").cloned().collect();
        let cmd = EditCommand::InsertCompound { compound: compound.clone(), presets: presets.clone(), on_conflict: OnConflict::Rename };
        assert_eq!(cmd.label(), "ライブラリーから読み込み");

        // Into the library's own show: everything is already there, nothing is added.
        let (same, o) = apply(&lib, &cmd, Snap::None).unwrap();
        assert_eq!((same == lib, o.compound_id.as_deref()), (true, Some(cp.as_str())));

        // Into a show whose base-blue is another color and which uses the layer / clip IDs.
        let mut target = new_show("x");
        let mut other = presets.iter().find(|p| p.id == "base-blue").unwrap().clone();
        other.effect = Effect::Solid { version: 1, params: SolidParams { color: "#FF0000".to_string().into() } };
        target.presets.push(other);
        let (target, _) = apply(&target, &EditCommand::AddLayer { name: "".into(), index: None }, Snap::None).unwrap();
        let taken = target.layers.last().unwrap().id.clone();
        assert!(compound.layers.iter().any(|l| l.id == taken), "the test needs a layer ID conflict");
        assert!(matches!(apply(&target, &EditCommand::InsertCompound { compound: compound.clone(), presets: presets.clone(), on_conflict: OnConflict::Error }, Snap::None), Err(EditError::Invalid(_))));
        let (a, o) = apply(&target, &cmd, Snap::None).unwrap();
        let got = a.compounds.iter().find(|c| Some(&c.id) == o.compound_id.as_ref()).unwrap();
        assert!(a.presets.iter().any(|p| p.id == "sweep-cyan") && a.presets.iter().any(|p| p.id == "base-blue-2"));
        assert_eq!(got.layers[1].content, Content::Continuous { preset_id: "base-blue-2".into() }, "renamed preset is used");
        assert_ne!(got.layers[0].id, taken, "layer IDs stay unique show-wide");
        // Again: no new preset or compound.
        let (b, o2) = apply(&a, &cmd, Snap::None).unwrap();
        assert_eq!((b.presets.len(), b.compounds.len(), o2.compound_id), (a.presets.len(), a.compounds.len(), o.compound_id));
        // A single preset from the library matches its renamed copy.
        assert!(existing_preset(&b, presets.iter().find(|p| p.id == "base-blue").unwrap()).is_some_and(|p| p.id == "base-blue-2"));
        assert!(!same_or_renamed("base-blue-x", "base-blue") && same_or_renamed("base-blue-12", "base-blue"));
    }

    #[test]
    fn compounds_are_built_placed_edited_and_detached() {
        let s = demo();
        let (s, o) = apply(&s, &EditCommand::AddCompound { name: "素材".into() }, Snap::None).unwrap();
        let (cp, inner) = (o.compound_id.unwrap(), o.layer_id.unwrap());
        // Inner layers take clips with the usual commands (IDs are show-wide).
        let (s, _) = apply(&s, &EditCommand::AddClip { layer_id: inner.clone(), preset_id: "sweep-cyan".into(), start_us: 500_000, duration_us: 1_000_000 }, Snap::None).unwrap();
        let (s, _) = apply(&s, &EditCommand::AddCompoundLayer { compound_id: cp.clone(), name: "".into(), preset_id: Some("base-blue".into()) }, Snap::None).unwrap();
        let (s, o) = apply(&s, &EditCommand::AddCompoundClip { layer_id: "motion".into(), compound_id: cp.clone(), start_us: 40_000_000, duration_us: None }, Snap::None).unwrap();
        let placed = o.clip_id.unwrap();
        assert_eq!(clip(&s, &placed).duration_us, 4_000_000, "its own length");
        let (s, _) = apply(&s, &EditCommand::SetCompoundClip { clip_id: placed.clone(), tint: Some("#ff0000".into()), time_scale: Some(2.0), looping: Some(true) }, Snap::None).unwrap();
        let c = clip(&s, &placed);
        assert_eq!((c.tint.as_deref(), c.time_scale, c.looping), (Some("#FF0000"), Some(2.0), Some(true)));
        // In use: not deleted. Faster than 1: not detached.
        assert!(matches!(apply(&s, &EditCommand::DeleteCompound { compound_id: cp.clone() }, Snap::None), Err(EditError::Invalid(_))));
        assert!(matches!(apply(&s, &EditCommand::DetachCompoundClip { clip_id: placed.clone() }, Snap::None), Err(EditError::Invalid(m)) if m.contains("time_scale")));
        // At speed 1: two layers above "motion", tinted preset copies, the loop unrolled.
        let (s, _) = apply(&s, &EditCommand::SetCompoundClip { clip_id: placed.clone(), tint: None, time_scale: Some(1.0), looping: None }, Snap::None).unwrap();
        let (s2, _) = apply(&s, &EditCommand::ResizeClip { clip_id: placed.clone(), duration_us: 9_000_000 }, Snap::None).unwrap();
        let (d, _) = apply(&s2, &EditCommand::DetachCompoundClip { clip_id: placed.clone() }, Snap::None).unwrap();
        assert!(find_clip(&d, &placed).is_err());
        let mi = d.layers.iter().position(|l| l.id == "motion").unwrap();
        let (sweep, base) = (&d.layers[mi + 1], &d.layers[mi + 2]);
        assert_eq!(sweep.name, "素材 / 1");
        let starts: Vec<u64> = sweep.content.clips().unwrap().iter().map(|c| c.start_us).collect();
        assert_eq!(starts, vec![40_500_000, 44_500_000, 48_500_000], "looped 3 times in 9 s");
        let base_clips = base.content.clips().unwrap();
        assert_eq!((base_clips.len(), base_clips[2].duration_us), (3, 1_000_000), "the last repeat is cut at the clip end");
        assert!(d.presets.iter().any(|p| p.id.starts_with("sweep-cyan-tint")), "tinted copy");
        // Unused now: the compound can go.
        assert!(apply(&d, &EditCommand::DeleteCompound { compound_id: cp }, Snap::None).is_ok());
    }

    #[test]
    fn recordings_become_triggered_clips() {
        use crate::model::{RecordedPad, Recording};
        let (s, o) = apply(&demo(), &EditCommand::AddTriggeredLayer { name: "rec".into(), on: TriggerOn::Press, index: None }, Snap::None).unwrap();
        let layer = o.layer_id.unwrap();
        let (s, o) = apply(&s, &EditCommand::AddPreset { name: "x".into(), effect_type: "builtin.cross".into() }, Snap::None).unwrap();
        let preset = o.preset_id.unwrap();
        let rec = Recording { events: vec![RecordedPad { t_us: 0, x: 1, y: 1, velocity: 100, duration_us: None }, RecordedPad { t_us: 1_500_000, x: 2, y: 2, velocity: 100, duration_us: Some(500_000) }], length_us: None };
        let (s, o) = apply(&s, &EditCommand::AddRecordingClip { layer_id: layer.clone(), preset_id: preset.clone(), start_us: 8_000_000, recording: rec.clone() }, Snap::None).unwrap();
        let id = o.clip_id.unwrap();
        assert_eq!((clip(&s, &id).start_us, clip(&s, &id).duration_us), (8_000_000, 2_000_000), "as long as the recording");
        let (s, _) = apply(&s, &EditCommand::SetClipRecording { clip_id: id.clone(), recording: Some(rec.clone()), looping: Some(true) }, Snap::None).unwrap();
        assert_eq!(clip(&s, &id).looping, Some(true));
        let mut history = History::new(s.clone());
        let mut longer = rec.clone();
        longer.length_us = Some(4_000_000);
        history.apply(&EditCommand::SetClipRecording { clip_id: id.clone(), recording: Some(longer), looping: None }, Snap::None).unwrap();
        assert_eq!(clip(history.current(), &id).recording.as_ref().unwrap().length(), 4_000_000);
        assert_eq!(clip(history.current(), &id).duration_us, 2_000_000, "loop period editing leaves the placed clip unchanged");
        assert_eq!(clip(history.current(), &id).looping, Some(true));
        assert!(history.undo());
        assert_eq!(clip(history.current(), &id).recording.as_ref().unwrap().length(), 2_000_000);
        assert!(history.redo());
        assert_eq!(clip(history.current(), &id).recording.as_ref().unwrap().length(), 4_000_000);
        let mut invalid = rec.clone(); invalid.length_us = Some(0);
        assert!(history.apply(&EditCommand::SetClipRecording { clip_id: id.clone(), recording: Some(invalid), looping: None }, Snap::None).is_err());
        assert_eq!(clip(history.current(), &id).recording.as_ref().unwrap().length(), 4_000_000);
        let (s, _) = apply(&s, &EditCommand::SetClipRecording { clip_id: id.clone(), recording: None, looping: None }, Snap::None).unwrap();
        assert_eq!((clip(&s, &id).recording.is_none(), clip(&s, &id).looping), (true, None), "loop goes with the recording");
        // Not on a timeline layer.
        assert!(matches!(apply(&s, &EditCommand::AddRecordingClip { layer_id: "motion".into(), preset_id: preset, start_us: 0, recording: rec }, Snap::None), Err(EditError::Rejected(_))));
    }

    #[test]
    fn frame_assets_import_as_presets() {
        use crate::assets::FramesAsset;
        let bitmap = |id: &str| FramesAsset::Bitmap { version: 1, id: id.into(), name: "星".into(), width: 8, height: 8, rows: vec!["...XX...".into(); 8] };
        let (s, o) = apply(&demo(), &EditCommand::ImportFrames { asset: bitmap("star"), on_conflict: OnConflict::Error }, Snap::None).unwrap();
        assert_eq!(o.preset_id.as_deref(), Some("star"));
        let Effect::Frames { params, .. } = &s.presets.iter().find(|p| p.id == "star").unwrap().effect else { panic!() };
        assert_eq!((params.frames.len(), params.hold_last), (1, true));
        // Same ID again: refused, or renamed on request.
        assert!(matches!(apply(&s, &EditCommand::ImportFrames { asset: bitmap("star"), on_conflict: OnConflict::Error }, Snap::None), Err(EditError::Invalid(_))));
        let (_, o) = apply(&s, &EditCommand::ImportFrames { asset: bitmap("star"), on_conflict: OnConflict::Rename }, Snap::None).unwrap();
        assert_eq!(o.preset_id.as_deref(), Some("star-2"));
        // A bad asset says why.
        let bad = FramesAsset::Bitmap { version: 2, id: "x".into(), name: "x".into(), width: 8, height: 8, rows: vec![] };
        assert!(matches!(apply(&s, &EditCommand::ImportFrames { asset: bad, on_conflict: OnConflict::Error }, Snap::None), Err(EditError::Invalid(m)) if m.contains("version")));
    }

    #[test]
    fn any_layer_takes_a_mask_and_drops_it() {
        let mask = LayerMask { shape: ShapeKind::Circle.into(), center_x: 3.5.into(), center_y: 3.5.into(), size: 3.0.into(), size_y: None, angle: None, edge: 0.5.into(), invert: false.into() };
        let (s, o) = apply(&demo(), &EditCommand::SetLayerMask { layer_id: "beat".into(), mask: Some(mask.clone()) }, Snap::None).unwrap();
        assert_eq!(o.layer_id.as_deref(), Some("beat"));
        assert_eq!(s.layers.iter().find(|l| l.id == "beat").unwrap().mask, Some(mask.clone()));
        let bad = LayerMask { shape: ShapeKind::Line.into(), ..mask };
        assert!(matches!(apply(&s, &EditCommand::SetLayerMask { layer_id: "beat".into(), mask: Some(bad) }, Snap::None), Err(EditError::Rejected(_))));
        let (s, _) = apply(&s, &EditCommand::SetLayerMask { layer_id: "beat".into(), mask: None }, Snap::None).unwrap();
        assert_eq!(s, demo());
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
