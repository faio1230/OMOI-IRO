//! IPC data types (mirrored in `src/ipc/types.ts`; field names are the JSON
//! keys, snake_case like every other OMOI-IRO JSON).

use omoiiro_core::safety::{FlashWarning, SafetyLimits};
use omoiiro_runtime::follow::LiveDiag;
use omoiiro_runtime::session::FollowSession;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct CommandError {
    /// Stable machine code: invalid, io, show, schema, semantic, version, json, start, stop, busy,
    /// not_running, ports, settings, save, following, no_preview, no_document, edit, history.
    pub code: String,
    pub message: String,
    /// JSON Pointers and reasons (validation failures).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<IssueDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IssueDto {
    pub pointer: String,
    pub message: String,
}

impl CommandError {
    pub fn new(code: &str, message: impl Into<String>) -> CommandError {
        CommandError { code: code.into(), message: message.into(), details: Vec::new() }
    }
}

/// The open timeline document (for the header and the close confirmation).
#[derive(Debug, Serialize)]
pub struct DocumentDto {
    pub path: String,
    pub dirty: bool,
}

#[derive(Debug, Serialize)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub link_available: bool,
    pub running: bool,
    /// Show file given on the command line (`omoiiro <file>`), to prefill the form.
    pub initial_show_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PortsDto {
    pub backend: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

/// Start the runtime with this show. Ports, bridge address and Link come
/// from the saved terminal settings.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub show_path: String,
}

#[derive(Debug, Serialize)]
pub struct StartResult {
    pub revision: u64,
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct StopResult {
    pub summary: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct BeatDto {
    pub beat: f64,
    pub tempo_bpm: f64,
    pub phase_valid: bool,
}

#[derive(Debug, Serialize)]
pub struct LedDto {
    pub state: Option<&'static str>,
    pub generation: Option<u64>,
    pub lit: usize,
    pub error: Option<String>,
}

/// One output binding (timecode output).
#[derive(Debug, Serialize)]
pub struct BindingDto {
    pub state: &'static str,
    pub generation: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ControlDto {
    pub input: Option<&'static str>,
    pub output: Option<&'static str>,
    /// Fault cause: input queue overflow / oversize input / control send failed / control output lost.
    pub fault: Option<String>,
    pub pending_note_offs: usize,
    pub input_error: Option<String>,
    pub output_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DroppedDto {
    pub bridge: u64,
    pub clock: u64,
    pub control: u64,
}

/// Safety limits in effect and flashing warnings (docs/12).
#[derive(Debug, Serialize)]
pub struct SafetyDto {
    pub max_brightness: f64,
    pub max_flash_hz: f64,
    pub limit_flash: bool,
    /// At the measured tempo while running; empty while stopped.
    pub warnings: Vec<FlashWarning>,
    /// Layers muted by the flashing limit (only with `limit_flash`).
    pub muted_layers: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AudioDto {
    pub age_ms: i64,
    /// Older than 500 ms: meters draw nothing.
    pub lost: bool,
    pub master: [f32; 2],
    pub tracks: Vec<(String, [f32; 2])>,
}

#[derive(Debug, Serialize)]
pub struct StatusDto {
    pub running: bool,
    pub show_path: Option<String>,
    pub uptime_ms: i64,
    pub frame: u64,
    pub revision: u64,
    pub show_time_us: Option<i64>,
    pub timecode: Option<String>,
    pub transport: &'static str,
    pub stalled: bool,
    pub time_health: &'static str,
    pub output: &'static str,
    pub gain: f32,
    pub dark: Option<&'static str>,
    pub beat: Option<BeatDto>,
    pub beat_health: &'static str,
    /// MIDI Clock origin taken from the bridge's beats (no Start / SPP yet).
    pub beat_seeded: bool,
    /// Times the clock origin was seeded from the bridge (diagnostics).
    pub clock_seeded: u64,
    pub blackout: bool,
    pub brightness: f32,
    pub led: LedDto,
    pub control: Option<ControlDto>,
    pub timecode_out: Option<BindingDto>,
    pub dropped: DroppedDto,
    /// Live diagnostics (T07); `None` while stopped.
    pub diag: Option<LiveDiag>,
    /// Preview frames replaced before the UI acknowledged the previous one.
    pub preview_skipped: u64,
    /// 0.1.3: show time the running pad recording began at (None: not recording).
    pub recording_since_us: Option<i64>,
    /// 0.1.3: bridge meters (None: none received in this run).
    pub audio: Option<AudioDto>,
    /// Frames whose rendering panicked (the previous frame was held) and the last message.
    pub render_panics: u64,
    pub render_fault: Option<String>,
    pub safety: SafetyDto,
    pub document: Option<DocumentDto>,
    /// The window was asked to close while the document has unsaved changes.
    pub close_requested: bool,
    /// Event lines since the previous status call (bridge lock, jumps, stop ...).
    pub events: Vec<String>,
}

impl StatusDto {
    pub fn stopped(limits: SafetyLimits) -> StatusDto {
        StatusDto {
            running: false,
            show_path: None,
            uptime_ms: 0,
            frame: 0,
            revision: 0,
            show_time_us: None,
            timecode: None,
            transport: "stopped",
            stalled: false,
            time_health: "unlocked",
            output: "black",
            gain: 0.0,
            dark: None,
            beat: None,
            beat_health: "unselected",
            beat_seeded: false,
            clock_seeded: 0,
            blackout: false,
            brightness: 1.0,
            led: LedDto { state: None, generation: None, lit: 0, error: None },
            control: None,
            timecode_out: None,
            dropped: DroppedDto { bridge: 0, clock: 0, control: 0 },
            diag: None,
            preview_skipped: 0,
            recording_since_us: None,
            audio: None,
            render_panics: 0,
            render_fault: None,
            safety: SafetyDto { max_brightness: limits.max_brightness, max_flash_hz: limits.max_flash_hz, limit_flash: limits.limit_flash, warnings: Vec::new(), muted_layers: Vec::new() },
            document: None,
            close_requested: false,
            events: Vec::new(),
        }
    }

    pub fn from_session(s: &FollowSession, show_path: Option<&Path>, preview_skipped: u64) -> StatusDto {
        let st = s.status();
        let t = &st.transport;
        let f = &st.follow;
        StatusDto {
            running: true,
            show_path: show_path.map(|p| p.display().to_string()),
            uptime_ms: st.uptime_us / 1000,
            frame: f.frame_index,
            revision: f.revision,
            show_time_us: f.show_time_us,
            timecode: t.mtc_label.clone(),
            transport: t.transport.name(),
            stalled: t.stalled,
            time_health: t.time_health.name(),
            output: t.output.name(),
            gain: f.gain,
            dark: f.dark,
            beat: t.beat.map(|b| BeatDto { beat: b.beat, tempo_bpm: b.tempo_bpm, phase_valid: b.phase_valid }),
            beat_health: t.beat_health.name(),
            beat_seeded: t.beat_seeded,
            clock_seeded: t.clock_seeded,
            blackout: st.blackout,
            brightness: st.brightness,
            led: LedDto { state: f.led_state, generation: f.led_generation, lit: f.led_lit, error: f.led_error.clone() },
            control: f.control_out.map(|out| ControlDto {
                input: f.control_in,
                output: Some(out),
                fault: st.control_fault.clone(),
                pending_note_offs: f.control_pending_note_offs,
                input_error: f.control_in_error.clone(),
                output_error: f.control_out_error.clone(),
            }),
            timecode_out: f.timecode_out.map(|state| BindingDto { state, generation: f.timecode_generation, error: f.timecode_error.clone() }),
            dropped: DroppedDto { bridge: st.dropped_bridge, clock: st.dropped_clock, control: st.dropped_control },
            diag: Some(f.diag),
            preview_skipped,
            recording_since_us: s.recording_since(),
            render_panics: f.render_panics,
            render_fault: f.render_fault.clone(),
            audio: s.audio().map(|(age, a)| AudioDto { age_ms: age / 1000, lost: age > omoiiro_core::render::AudioInput::LOST_AFTER_US, master: a.master, tracks: a.tracks }),
            safety: {
                let l = s.shared().overrides.limits();
                SafetyDto { max_brightness: l.max_brightness, max_flash_hz: l.max_flash_hz, limit_flash: l.limit_flash, warnings: f.safety_warnings.clone(), muted_layers: f.muted_layers.clone() }
            },
            document: None,
            close_requested: false,
            events: s.drain_events(),
        }
    }
}
