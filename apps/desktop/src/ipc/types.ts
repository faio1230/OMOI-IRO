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
  | { op: "set_sync"; beat_source: BeatSource | null; mtc_output_enabled: boolean | null };

export type Blend = "over" | "add" | "max";
export type EffectType = "builtin.solid" | "builtin.chase" | "builtin.pulse" | "builtin.strobe" | "builtin.noise" | "builtin.ripple" | "builtin.rain" | "builtin.wave" | "builtin.orb" | "builtin.regions" | "builtin.tap_glow" | "builtin.cross";
export type OnConflict = "error" | "rename";

/** Effect as saved in the show (schemas/show.schema.json `effect`). */
export type Effect =
  | { type: "builtin.solid"; version: number; params: { color: string } }
  | { type: "builtin.chase"; version: number; params: { color: string; period_us: number; width: number; axis: "x" | "y"; reverse?: boolean } }
  | { type: "builtin.pulse"; version: number; params: { color: string; beats_per_cycle: number; depth: number; waveform: "sine" | "triangle" } }
  | { type: "builtin.strobe"; version: number; params: { color: string; mode: "hz" | "beat"; rate_hz: number; beats_per_cycle: number; duty: number; phase_offset: number } }
  | { type: "builtin.noise"; version: number; params: { base_color: string; highlight_color: string; density: number; rate_hz: number; seed: number; smooth: boolean; depth: number } }
  | { type: "builtin.ripple"; version: number; params: { color: string; speed: number; ring_width: number; lifetime_ms: number; fade: "linear" | "exp"; max_concurrent: number; trigger: "any_pad" | "interval"; shape?: "circle" | "diamond" | "square"; origin_x?: number; origin_y?: number; interval_ms?: number } }
  | { type: "builtin.rain"; version: number; params: { color: string; head_color?: string; direction: "down" | "up" | "left" | "right"; speed: number; density: number; tail: number; seed: number; flicker: number } }
  | { type: "builtin.wave"; version: number; params: { color: string; axis: "x" | "y"; wavelength: number; amplitude: number; center: number; speed: number; thickness: number; beat_amplitude: boolean; phase?: number } }
  | { type: "builtin.orb"; version: number; params: { color: string; center_x: number; center_y: number; radius: number; edge: number; light_angle: number; shading: number; dither: number; dither_rate_hz: number; seed: number; motion: "none" | "orbit" | "bounce"; speed: number; orbit_radius: number; bounce_angle: number; orbit_phase?: number } }
  | { type: "builtin.regions"; version: number; params: { pattern: RegionPattern; color: string; color_b?: string; step_mode: "beat" | "seconds"; step_beats: number; step_ms: number; order: "forward" | "reverse" | "pingpong"; soft: number; duty: number } }
  | { type: "builtin.tap_glow"; version: number; params: { color_start: string; color_mid: string; color_end: string; lifetime_ms: number; radius: number; repeats: number; repeat_interval_ms: number; repeat_decay: number } }
  | { type: "builtin.cross"; version: number; params: { color: string; speed: number; length: number; lifetime_ms: number } };

export type RegionPattern = "halves_lr" | "halves_tb" | "checker" | "quadrants" | "columns" | "rows" | "perimeter" | "fan";

export interface ClipDto {
  id: string;
  preset_id: string;
  start_us: number;
  duration_us: number;
  fade_in_us: number;
  fade_out_us: number;
}

export interface LayerDto {
  id: string;
  name: string;
  target_id: string;
  kind: "timeline" | "continuous";
  enabled: boolean;
  opacity: number;
  blend: Blend;
  preset_id: string | null;
  clips: ClipDto[];
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
}

export interface EditorView {
  path: string;
  name: string;
  targets: { id: string; name: string }[];
  length_us: number | null;
  layers: LayerDto[];
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
