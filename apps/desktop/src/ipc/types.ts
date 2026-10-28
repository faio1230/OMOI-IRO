// IPC types: hand-written mirror of apps/desktop/src-tauri/src/dto.rs.
// A Rust test checks that every StatusDto field appears here.

export interface CommandError {
  code: "invalid" | "io" | "show" | "schema" | "semantic" | "version" | "json" | "start" | "stop" | "busy" | "not_running" | "ports" | "save" | "unsaved" | string;
  message: string;
  /** JSON Pointers and reasons (validation failures). */
  details?: { pointer: string; message: string }[];
}

export interface AppInfo {
  name: string;
  version: string;
  link_available: boolean;
  running: boolean;
  initial_show_path: string | null;
}

export interface PortsDto {
  backend: string;
  inputs: string[];
  outputs: string[];
}

/** Ports, bridge address and Link come from the saved terminal settings. */
export interface StartRequest {
  show_path: string;
}

export interface StartResult {
  revision: number;
  notes: string[];
}

export interface StopResult {
  summary: unknown;
}

export interface BeatDto {
  beat: number;
  tempo_bpm: number;
  phase_valid: boolean;
}

export interface LedDto {
  state: string | null;
  generation: number | null;
  lit: number;
  error: string | null;
}

export interface BindingDto {
  state: string;
  generation: number | null;
  error: string | null;
}

export interface TimingSummary {
  n: number;
  p50_us: number;
  p95_us: number;
  p99_us: number;
  max_us: number;
}

/** Live diagnostics (crates/omoiiro-runtime/src/follow.rs LiveDiag). */
export interface LiveDiag {
  frames: number;
  skipped_frames: number;
  jumps: number;
  jitter: TimingSummary;
  render: TimingSummary;
  led_frames: number;
  led_errors: number;
  led_send: TimingSummary;
  render_to_sent: TimingSummary;
  mailbox_overwritten: number;
  control_sent: number;
  control_errors: number;
  callback_to_send: TimingSummary;
  mtc_sent: number;
  mtc_send_errors: number;
  ripple_spawned: number;
}

export interface ControlDto {
  input: string | null;
  output: string | null;
  fault: string | null;
  pending_note_offs: number;
  input_error: string | null;
  output_error: string | null;
}

export interface DroppedDto {
  bridge: number;
  clock: number;
  control: number;
}

/** crates/omoiiro-core/src/safety.rs */
export interface SafetyLimits {
  max_brightness: number;
  max_flash_hz: number;
  limit_flash: boolean;
}

export interface FlashWarning {
  layer_id: string;
  layer_name: string;
  preset_id: string;
  preset_name: string;
  kind: "pulse" | "chase" | "strobe" | "noise" | "grain" | "wave" | "regions";
  hz: number;
  limit_hz: number;
  tempo_bpm: number;
  clips: string[];
  reason: string;
}

export interface SafetyDto extends SafetyLimits {
  warnings: FlashWarning[];
  muted_layers: string[];
}

export interface StatusDto {
  running: boolean;
  show_path: string | null;
  uptime_ms: number;
  frame: number;
  revision: number;
  show_time_us: number | null;
  timecode: string | null;
  transport: "playing" | "stopped" | "unknown";
  stalled: boolean;
  time_health: "unlocked" | "acquiring" | "locked" | "holdover" | "lost" | "fault";
  output: "follow" | "hold" | "black";
  gain: number;
  dark: string | null;
  beat: BeatDto | null;
  beat_health: string;
  /** MIDI Clock origin taken from the bridge's beats (no Start / SPP yet). */
  beat_seeded: boolean;
  clock_seeded: number;
  blackout: boolean;
  brightness: number;
  led: LedDto;
  control: ControlDto | null;
  timecode_out: BindingDto | null;
  dropped: DroppedDto;
  diag: LiveDiag | null;
  preview_skipped: number;
  safety: SafetyDto;
  document: { path: string; dirty: boolean } | null;
  close_requested: boolean;
  events: string[];
}

// ---- LED preview (apps/desktop/src-tauri/src/preview.rs) ----

export type PreviewSource = "runtime" | "local";

export interface PreviewFrame {
  seq: number;
  source: PreviewSource;
  show_time_us: number | null;
  timecode: string | null;
  playing: boolean;
  dark: string | null;
  /** 64 x RGB 0..255, index = y * 8 + x, y = 0 is the bottom row. */
  rgb: number[];
  skipped: number;
}

export interface PreviewInfo {
  show_path: string;
  length_us: number | null;
  bpm: number;
  show_bpm: number;
  position_us: number;
  playing: boolean;
  mode: PreviewSource;
}

// ---- Terminal settings (crates/omoiiro-runtime/src/settings.rs, src-tauri/src/settings.rs) ----

export type PortRole = "control_in" | "control_out" | "clock_in" | "led_out" | "timecode_out";

export interface PortRoles {
  control_in: string | null;
  control_out: string | null;
  clock_in: string | null;
  /** null = in-process virtual Launchpad X. */
  led_out: string | null;
  timecode_out: string | null;
}

export interface TerminalSettings {
  version: number;
  backend: string;
  bridge_bind: string;
  ports: PortRoles;
  link: boolean;
  recent_shows: string[];
  safety: SafetyLimits;
  devices: { allow_launchpad_led: boolean };
}

export interface SettingsView {
  settings: TerminalSettings;
  path: string | null;
  load_error: string | null;
  link_available: boolean;
  running: boolean;
}

export type RoleState = "unused" | "virtual" | "ok" | "missing" | "fault" | "invalid";

export interface RoleReport {
  role: PortRole;
  direction: "input" | "output";
  spec: string | null;
  resolved: string | null;
  state: RoleState;
  message: string | null;
}

export interface CheckReport {
  roles: RoleReport[];
  ok: boolean;
  errors: string[];
  warnings: string[];
}

export interface SettingsCheck {
  backend: string;
  inputs: string[];
  outputs: string[];
  report: CheckReport;
}

export interface ListenCounts {
  total: number;
  midi_clock: number;
  transport: number;
  mtc_quarter_frame: number;
  sysex: number;
  note: number;
  control_change: number;
  other: number;
}

export interface ListenResult {
  role: string;
  port: string;
  duration_ms: number;
  counts: ListenCounts;
  samples: string[];
  dropped: number;
}

// ---- Timeline editing (crates/omoiiro-core/src/edit.rs, src-tauri/src/editor.rs) ----

export type Snap = "none" | "second" | "frame30";

export type EditCommand =
  | { op: "add_clip"; layer_id: string; preset_id: string; start_us: number; duration_us: number }
  | { op: "move_clip"; clip_id: string; start_us: number; to_layer_id: string | null }
  | { op: "resize_clip"; clip_id: string; duration_us: number }
  | { op: "set_clip_timing"; clip_id: string; start_us: number; duration_us: number; fade_in_us: number; fade_out_us: number }
  | { op: "duplicate_clip"; clip_id: string; start_us: number | null }
  | { op: "delete_clip"; clip_id: string }
  | { op: "add_layer"; name: string; index: number | null }
  | { op: "delete_layer"; layer_id: string }
  | { op: "move_layer"; layer_id: string; to_index: number }
  | { op: "set_layer"; layer_id: string; name: string | null; enabled: boolean | null; opacity: number | null; blend: Blend | null; target_id: string | null }
  | { op: "add_continuous_layer"; name: string; preset_id: string; index: number | null }
  | { op: "set_layer_preset"; layer_id: string; preset_id: string }
  | { op: "add_preset"; name: string; effect_type: EffectType }
  | { op: "set_preset"; preset_id: string; name: string | null; effect: Effect | null }
  | { op: "delete_preset"; preset_id: string }
  | { op: "set_clip_preset"; clip_id: string; preset_id: string }
  | { op: "edit_clip_effect"; clip_id: string; effect: Effect }
  | { op: "set_sync"; beat_source: BeatSource | null; mtc_output_enabled: boolean | null }
  | { op: "add_triggered_layer"; name: string; on: TriggerOn; index: number | null }
  | { op: "set_layer_trigger"; layer_id: string; trigger: TriggerSpec }
  | { op: "set_layer_mask"; layer_id: string; mask: LayerMask | null }
  | { op: "add_compound"; name: string }
  | { op: "set_compound"; compound_id: string; name: string | null; duration_us: number | null }
  | { op: "delete_compound"; compound_id: string }
  | { op: "add_compound_layer"; compound_id: string; name: string; preset_id: string | null }
  | { op: "add_compound_clip"; layer_id: string; compound_id: string; start_us: number; duration_us: number | null }
  | { op: "set_compound_clip"; clip_id: string; tint: string | null; time_scale: number | null; looping: boolean | null }
  | { op: "detach_compound_clip"; clip_id: string };

export type Blend = "over" | "add" | "max";
export type EffectType = "builtin.solid" | "builtin.chase" | "builtin.pulse" | "builtin.strobe" | "builtin.noise" | "builtin.ripple" | "builtin.rain" | "builtin.wave" | "builtin.orb" | "builtin.regions" | "builtin.tap_glow" | "builtin.cross" | "builtin.path" | "builtin.shape" | "builtin.particles";
export type OnConflict = "error" | "rename";

export type Ease = "linear" | "step" | "ease_in" | "ease_out" | "ease_in_out";
export interface Keyframe<T> { t_us: number; v: T; ease?: Ease }
/** A parameter value or its keyframes (0.1.3). */
export type K<T> = T | { keyframes: Keyframe<T>[] };

/** Effect as saved in the show (schemas/show.schema.json `effect`). */
export type Effect =
  | { type: "builtin.solid"; version: number; params: { color: K<string> } }
  | { type: "builtin.chase"; version: number; params: { color: K<string>; period_us: K<number>; width: K<number>; axis: "x" | "y"; reverse?: boolean } }
  | { type: "builtin.pulse"; version: number; params: { color: K<string>; beats_per_cycle: K<number>; depth: K<number>; waveform: "sine" | "triangle" } }
  | { type: "builtin.strobe"; version: number; params: { color: K<string>; mode: "hz" | "beat"; rate_hz: K<number>; beats_per_cycle: K<number>; duty: K<number>; phase_offset: K<number> } }
  | { type: "builtin.noise"; version: number; params: { base_color: K<string>; highlight_color: K<string>; density: K<number>; rate_hz: K<number>; seed: K<number>; smooth: boolean; depth: K<number> } }
  | { type: "builtin.ripple"; version: number; params: { color: K<string>; speed: K<number>; ring_width: K<number>; lifetime_ms: K<number>; fade: "linear" | "exp"; max_concurrent: K<number>; trigger: "any_pad" | "interval"; shape?: "circle" | "diamond" | "square"; origin_x?: K<number>; origin_y?: K<number>; interval_ms?: K<number> } }
  | { type: "builtin.rain"; version: number; params: { color: K<string>; head_color?: K<string>; direction: "down" | "up" | "left" | "right"; speed: K<number>; density: K<number>; tail: K<number>; seed: K<number>; flicker: K<number> } }
  | { type: "builtin.wave"; version: number; params: { color: K<string>; axis: "x" | "y"; wavelength: K<number>; amplitude: K<number>; center: K<number>; speed: K<number>; thickness: K<number>; beat_amplitude: boolean; phase?: K<number> } }
  | { type: "builtin.orb"; version: number; params: { color: K<string>; center_x: K<number>; center_y: K<number>; radius: K<number>; edge: K<number>; light_angle: K<number>; shading: K<number>; dither: K<number>; dither_rate_hz: K<number>; seed: K<number>; motion: "none" | "orbit" | "bounce"; speed: K<number>; orbit_radius: K<number>; bounce_angle: K<number>; orbit_phase?: K<number> } }
  | { type: "builtin.regions"; version: number; params: { pattern: RegionPattern; color: K<string>; color_b?: K<string>; step_mode: "beat" | "seconds"; step_beats: K<number>; step_ms: K<number>; order: "forward" | "reverse" | "pingpong"; soft: K<number>; duty: K<number>; groups?: number[][]; offset?: number[] } }
  | { type: "builtin.tap_glow"; version: number; params: { color_start: K<string>; color_mid: K<string>; color_end: K<string>; lifetime_ms: K<number>; radius: K<number>; repeats: K<number>; repeat_interval_ms: K<number>; repeat_decay: K<number> } }
  | { type: "builtin.cross"; version: number; params: { color: K<string>; speed: K<number>; length: K<number>; lifetime_ms: K<number> } }
  | { type: "builtin.path"; version: number; params: { color: K<string>; path: PathKind; points?: [number, number][]; from_x?: K<number>; from_y?: K<number>; to_x?: K<number>; to_y?: K<number>; center_x?: K<number>; center_y?: K<number>; orbit_radius?: K<number>; speed: K<number>; size: K<number>; tail: K<number>; loop: boolean; phase: K<number>; mirror: Mirror } }
  | { type: "builtin.shape"; version: number; params: { shape: ShapeKind; color: K<string>; fill: boolean; stroke: K<number>; center_x: K<number>; center_y: K<number>; size: K<number>; size_y?: K<number>; angle: K<number>; edge: K<number>; invert?: K<boolean> } }
  | { type: "builtin.particles"; version: number; params: ParticlesParams };

export type ParticleMode = "burst" | "fountain" | "rocket" | "implode";
export interface ParticlesParams {
  mode: ParticleMode; count?: K<number>; rate?: K<number>; color: K<string>; color_end?: K<string>;
  speed_min: K<number>; speed_max: K<number>; angle: K<number>; spread: K<number>; gravity: K<number>; drag: K<number>;
  lifetime_ms: K<number>; size: K<number>; tail: K<number>; edge_mode: "vanish" | "bounce";
  origin_x?: K<number>; origin_y?: K<number>; seed: K<number>;
  rise_speed?: K<number>; burst_height?: K<number>; pause_ms?: K<number>; implode_radius?: K<number>;
}

export type PathKind = "line" | "perimeter" | "orbit" | "figure8" | "points";
export type Mirror = "none" | "x" | "y" | "xy";
export type ShapeKind = "line" | "circle" | "rect" | "diamond" | "wipe";

export type RegionPattern = "halves_lr" | "halves_tb" | "checker" | "quadrants" | "columns" | "rows" | "perimeter" | "fan" | "custom";

export interface ClipDto {
  id: string;
  preset_id: string;
  start_us: number;
  duration_us: number;
  fade_in_us: number;
  fade_out_us: number;
  /** 0.1.3 compound clips (preset_id is then ""). */
  compound_id: string | null;
  tint: string | null;
  time_scale: number | null;
  looping: boolean | null;
}

/** A compound: inner layers (no target) and the clips placing it. */
export interface CompoundDto {
  id: string;
  name: string;
  duration_us: number | null;
  length_us: number;
  layers: LayerDto[];
  used_by_clips: string[];
}

export interface LayerDto {
  id: string;
  name: string;
  target_id: string;
  kind: "timeline" | "continuous" | "triggered";
  enabled: boolean;
  opacity: number;
  blend: Blend;
  preset_id: string | null;
  clips: ClipDto[];
  /** Triggered layers: when they fire. */
  trigger: TriggerSpec | null;
  /** 0.1.3: cut-out of the layer's own drawing. */
  mask: LayerMask | null;
}

/** Layer mask (0.1.3): time axis = the layer's (clip local time, or show time on a continuous layer). */
export interface LayerMask { shape: K<Exclude<ShapeKind, "line">>; center_x: K<number>; center_y: K<number>; size: K<number>; size_y?: K<number>; angle?: K<number>; edge: K<number>; invert: K<boolean> }

export type TriggerOn = "press" | "release" | "hold" | "clip_start" | "interval";

export interface TriggerSpec {
  on: TriggerOn;
  interval_ms?: number;
  origin_x?: number;
  origin_y?: number;
}

export interface PresetDto {
  id: string;
  name: string;
  effect_type: EffectType;
  color: string;
  effect: Effect;
  used_by_clips: string[];
  used_by_layers: string[];
}

export interface EditOutcome {
  clip_id: string | null;
  layer_id: string | null;
  preset_id: string | null;
  compound_id: string | null;
}

export interface EditorView {
  path: string;
  name: string;
  targets: { id: string; name: string }[];
  length_us: number | null;
  layers: LayerDto[];
  compounds: CompoundDto[];
  presets: PresetDto[];
  dirty: boolean;
  can_undo: boolean;
  can_redo: boolean;
  undo_label: string | null;
  redo_label: string | null;
  history_depth: number;
  runtime_revision: number | null;
  outcome: EditOutcome | null;
  flash_warnings: FlashWarning[];
  max_flash_hz: number;
  preview_bpm: number;
  beat_source: BeatSource;
  mtc_output_enabled: boolean;
}

export type BeatSource = "ableton_link" | "midi_clock";
