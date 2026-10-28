//! OMOI-IRO desktop shell (M5): owns at most one running `FollowSession`
//! and exposes it to the React UI through typed commands.
//!
//! * Every command validates its input here, in Rust, before touching the
//!   runtime; errors come back as `CommandError { code, message }`.
//! * Ports, the bridge address and Link use come from the terminal settings
//!   (`settings.rs`, the app's config directory), never from the show.
//! * The UI never drives the show clock: it polls `runtime_status` for
//!   display only. Rendering, LED output and the operation route run on the
//!   runtime's own threads whether or not the window is repainting.
//! * Commands run off the main thread (`async`) where they may block.
//! * Leaving the app stops the session (LEDs off, Note Offs for sounding notes).
//!
//! The DTOs are mirrored by hand in `src/ipc/types.ts`.

mod dto;
mod editor;
mod files;
mod library;
mod preview;
mod settings;

use dto::*;
use omoiiro_midi::backend::MidiBackend;
use omoiiro_midi::midir_backend::MidirBackend;
use omoiiro_runtime::follow::SharedBackend;
use omoiiro_runtime::session::{FollowSession, LedOut, LocalTransport, SessionConfig};
use omoiiro_runtime::settings::{check_roles, TerminalSettings};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Manager, RunEvent, State};

pub struct AppState {
    session: Mutex<Option<FollowSession>>,
    backend: SharedBackend,
    show_path: Mutex<Option<PathBuf>>,
    /// The runtime runs the edited new show that has no file yet (show_path
    /// is None); its edits are handed over until it is saved or replaced.
    follows_new: std::sync::atomic::AtomicBool,
    initial_show_path: Option<String>,
    /// Set when closing was stopped because of unsaved changes.
    close_requested: std::sync::atomic::AtomicBool,
    /// Saved terminal settings (what `runtime_start` uses).
    settings: Mutex<TerminalSettings>,
    settings_dir: Mutex<Option<PathBuf>>,
    settings_load_error: Mutex<Option<String>>,
}

impl AppState {
    fn new(initial_show_path: Option<String>) -> AppState {
        AppState {
            initial_show_path,
            close_requested: std::sync::atomic::AtomicBool::new(false),
            session: Mutex::new(None),
            backend: Arc::new(Mutex::new(Box::new(MidirBackend::new("omoiiro")) as Box<dyn MidiBackend>)),
            show_path: Mutex::new(None),
            follows_new: std::sync::atomic::AtomicBool::new(false),
            settings: Mutex::new(TerminalSettings::default()),
            settings_dir: Mutex::new(None),
            settings_load_error: Mutex::new(None),
        }
    }

    fn port_lists(&self) -> CmdResult<(Vec<String>, Vec<String>)> {
        let be = self.backend.lock().unwrap();
        let e = |e: omoiiro_midi::backend::PortError| CommandError::new("ports", e.to_string());
        Ok((be.list_inputs().map_err(e)?, be.list_outputs().map_err(e)?))
    }
}

type CmdResult<T> = Result<T, CommandError>;

#[tauri::command]
fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        name: "OMOI-IRO".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        link_available: cfg!(feature = "link"),
        running: state.session.lock().unwrap().is_some(),
        initial_show_path: state.initial_show_path.clone(),
    }
}

#[tauri::command(async)]
fn list_ports(state: State<'_, AppState>) -> CmdResult<PortsDto> {
    let (inputs, outputs) = state.port_lists()?;
    Ok(PortsDto { backend: state.backend.lock().unwrap().backend_name().into(), inputs, outputs })
}

#[tauri::command(async)]
fn runtime_start(state: State<'_, AppState>, ed: State<'_, editor::EditorState>, req: StartRequest) -> CmdResult<StartResult> {
    start_runtime(&state, &ed, req)
}

/// No show path: the edited document (a new show without a file included).
fn start_runtime(state: &AppState, ed: &editor::EditorState, req: StartRequest) -> CmdResult<StartResult> {
    let mut slot = state.session.lock().unwrap();
    if slot.is_some() {
        return Err(CommandError::new("busy", "既に開始している（先に停止する）"));
    }
    let settings = state.settings.lock().unwrap().clone();
    let (inputs, outputs) = state.port_lists()?;
    let check = check_roles(&settings, &inputs, &outputs);
    if !check.ok {
        return Err(CommandError::new("settings", format!("接続設定に誤りがある：{}", check.errors.join(" / "))));
    }
    let mut path = PathBuf::from(req.show_path.trim());
    if path.as_os_str().is_empty() {
        if let Some(p) = editor::document_path(ed) {
            path = p;
        }
    }
    // The timeline's current (possibly unsaved) show when it edits this file.
    let edited = editor::current_show_for(ed, &path);
    let cfg = req.validate(&settings, edited)?;
    let session = FollowSession::start(cfg, state.backend.clone()).map_err(|e| CommandError::new("start", e))?;
    let mut notes = check.warnings.clone();
    if settings.devices.allow_launchpad_led && settings.ports.led_out.as_deref().is_some_and(omoiiro_midi::ports::looks_like_launchpad) {
        notes.insert(0, omoiiro_runtime::settings::LAUNCHPAD_LED_WARNING.to_string());
    }
    notes.extend(session.notes.iter().cloned());
    let result = StartResult { revision: session.revision, notes };
    *slot = Some(session);
    drop(slot);
    let new_show = path.as_os_str().is_empty();
    *state.show_path.lock().unwrap() = (!new_show).then(|| path.clone());
    state.follows_new.store(new_show, std::sync::atomic::Ordering::SeqCst);
    if !new_show {
        settings::remember_show(state, &path.display().to_string());
    }
    settings::remember_time_source(state, req.time_source);
    Ok(result)
}

#[tauri::command(async)]
fn runtime_stop(state: State<'_, AppState>) -> CmdResult<StopResult> {
    let session = state.session.lock().unwrap().take().ok_or_else(|| CommandError::new("not_running", "開始していない（先に「開始」を押す）"))?;
    state.follows_new.store(false, std::sync::atomic::Ordering::SeqCst);
    let summary = session.stop().map_err(|e| CommandError::new("stop", e))?;
    Ok(StopResult { summary })
}

#[tauri::command]
fn runtime_status(state: State<'_, AppState>, ps: State<'_, Arc<preview::PreviewState>>, ed: State<'_, editor::EditorState>) -> StatusDto {
    let mut st = match state.session.lock().unwrap().as_ref() {
        Some(s) => StatusDto::from_session(s, state.show_path.lock().unwrap().as_deref(), ps.skipped()),
        None => StatusDto::stopped(state.settings.lock().unwrap().safety),
    };
    st.document = editor::document(&ed);
    st.close_requested = state.close_requested.load(std::sync::atomic::Ordering::SeqCst);
    st
}

/// Answer to the close confirmation: "save" (save, then quit), "discard"
/// (quit without saving) or "cancel" (stay).
#[tauri::command(async)]
fn close_decision(app: tauri::AppHandle, state: State<'_, AppState>, ed: State<'_, editor::EditorState>, action: String) -> CmdResult<()> {
    use std::sync::atomic::Ordering;
    match action.as_str() {
        "cancel" => {
            state.close_requested.store(false, Ordering::SeqCst);
            Ok(())
        }
        "save" => {
            // A failed save keeps the app open and shows why.
            editor::save_if_dirty(&ed).inspect_err(|_| state.close_requested.store(false, Ordering::SeqCst))?;
            app.exit(0);
            Ok(())
        }
        "discard" => {
            app.exit(0);
            Ok(())
        }
        other => Err(CommandError::new("invalid", format!("unknown close action {other:?}"))),
    }
}

/// Close and reopen every port (LED, operation route, timecode output) as new
/// connection generations. Sounding notes are released first.
#[tauri::command]
fn reconnect_ports(state: State<'_, AppState>) -> CmdResult<()> {
    with_session(&state, |s| s.reconnect_all())
}

#[tauri::command]
fn set_blackout(state: State<'_, AppState>, on: bool) -> CmdResult<()> {
    with_session(&state, |s| s.set_blackout(on))
}

#[tauri::command]
fn set_brightness(state: State<'_, AppState>, value: f64) -> CmdResult<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(CommandError::new("invalid", format!("brightness must be 0..1, got {value}")));
    }
    with_session(&state, |s| s.set_brightness(value as f32))
}

#[tauri::command]
fn reset_control_fault(state: State<'_, AppState>) -> CmdResult<()> {
    with_session(&state, |s| s.reset_control_fault())
}

/// A21: the local clock of a session started with `time_source: local`
/// (refused while following Live).
fn with_local(state: &State<'_, AppState>, f: impl FnOnce(&FollowSession) -> Result<LocalTransport, String>) -> CmdResult<LocalTransport> {
    match state.session.lock().unwrap().as_ref() {
        Some(s) => f(s).map_err(|e| CommandError::new("time_source", e)),
        None => Err(CommandError::new("not_running", "開始していない（先に「開始」を押す）")),
    }
}

#[tauri::command]
fn runtime_transport_play(state: State<'_, AppState>, playing: bool) -> CmdResult<LocalTransport> {
    with_local(&state, |s| s.local_play(playing))
}

#[tauri::command]
fn runtime_transport_seek(state: State<'_, AppState>, position_us: i64) -> CmdResult<LocalTransport> {
    with_local(&state, |s| s.local_seek(position_us))
}

#[tauri::command]
fn runtime_transport_bpm(state: State<'_, AppState>, bpm: f64) -> CmdResult<LocalTransport> {
    with_local(&state, |s| s.local_set_bpm(bpm))
}

/// Forget every pad press (input history of pads / cycle / constellation).
#[tauri::command]
fn reset_input(state: State<'_, AppState>) -> CmdResult<()> {
    with_session(&state, |s| s.reset_input())
}

/// Start recording pad presses from the current show time.
#[tauri::command]
fn recording_start(state: State<'_, AppState>) -> CmdResult<i64> {
    match state.session.lock().unwrap().as_ref() {
        Some(s) => s.start_recording().map_err(|e| CommandError::new("recording", e)),
        None => Err(CommandError::new("not_running", "開始していない（先に「開始」を押す）")),
    }
}

fn with_session(state: &State<'_, AppState>, f: impl FnOnce(&FollowSession)) -> CmdResult<()> {
    match state.session.lock().unwrap().as_ref() {
        Some(s) => {
            f(s);
            Ok(())
        }
        None => Err(CommandError::new("not_running", "開始していない（先に「開始」を押す）")),
    }
}

impl StartRequest {
    /// Load and check the show; ports and addresses come from the terminal settings.
    fn validate(&self, settings: &TerminalSettings, edited: Option<omoiiro_core::model::Show>) -> CmdResult<SessionConfig> {
        let path = self.show_path.trim();
        let show = match edited {
            Some(s) => s,
            None if path.is_empty() => return Err(CommandError::new("invalid", "ショーファイルを選ぶか、新規ショーを開く")),
            None => {
                let p = files::for_read(path, files::Kind::Show)?;
                omoiiro_store::load_path(&p).map_err(|e| files::load_error(&p, e))?
            }
        };
        let bind = settings.bridge_bind.as_str();
        bind.parse::<std::net::SocketAddr>().map_err(|e| CommandError::new("settings", format!("bridge address {bind:?}: {e}")))?;
        if settings.link && !cfg!(feature = "link") {
            return Err(CommandError::new("settings", "このビルドは Ableton Link に対応していない"));
        }
        let p = &settings.ports;
        let led_out = match &p.led_out {
            None => LedOut::Virtual,
            // Device output is M6: Launchpad-looking ports are refused by the checks and the runtime.
            // A Launchpad-looking port only with the terminal's device permission (M6).
            Some(spec) => LedOut::Port { spec: spec.clone(), allow_device: settings.devices.allow_launchpad_led },
        };
        let control = match (&p.control_in, &p.control_out) {
            (Some(i), Some(o)) => Some((i.clone(), o.clone())),
            _ => None,
        };
        Ok(SessionConfig {
            time_source: self.time_source,
            show,
            bind: bind.to_string(),
            clock_in: p.clock_in.clone(),
            link: settings.link,
            led_out,
            control,
            timecode_out: p.timecode_out.clone(),
            safety: settings.safety,
            // Device output (M6) is identified first; ports that are not a
            // Launchpad (e.g. loopMIDI for tests) are written directly.
            identify_led: p.led_out.as_deref().is_some_and(omoiiro_midi::ports::looks_like_launchpad),
            record: None,
            meta: serde_json::json!({ "command": "desktop" }),
        })
    }
}

pub fn run() {
    // `omoiiro <show file>` prefills the form; nothing starts by itself.
    let initial = std::env::args().nth(1).filter(|a| !a.starts_with('-'));
    tauri::Builder::default()
        .manage(AppState::new(initial))
        .manage(Arc::new(preview::PreviewState::new()))
        .manage(editor::EditorState::default())
        .manage(library::LibraryState::default())
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            // Unsaved timeline edits: keep the window and ask in the UI (status poll).
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                if editor::is_dirty(&app.state::<editor::EditorState>()) {
                    api.prevent_close();
                    app.state::<AppState>().close_requested.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
        })
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            settings::load_into(&app.state::<AppState>(), dir);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            list_ports,
            runtime_start,
            runtime_stop,
            runtime_status,
            set_blackout,
            set_brightness,
            reset_control_fault,
            reconnect_ports,
            preview::preview_subscribe,
            preview::preview_ack,
            preview::preview_open,
            preview::preview_mode,
            preview::preview_play,
            preview::preview_seek,
            preview::preview_set_bpm,
            preview::preview_pad,
            settings::settings_get,
            settings::settings_check,
            settings::settings_save,
            settings::settings_listen,
            editor::editor_open,
            editor::editor_view,
            editor::material_preview,
            editor::editor_apply,
            editor::editor_undo,
            editor::editor_redo,
            editor::preset_import,
            editor::frames_import,
            editor::recording_stop,
            reset_input,
            runtime_transport_play,
            runtime_transport_seek,
            runtime_transport_bpm,
            recording_start,
            editor::preset_export,
            editor::editor_save,
            editor::editor_save_as,
            editor::editor_new,
            library::library_list,
            library::library_preview,
            library::library_import,
            library::library_add,
            library::library_remove,
            close_decision
        ])
        .build(tauri::generate_context!())
        .expect("error while building OMOI-IRO")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                if let Some(s) = app.state::<AppState>().session.lock().unwrap().take() {
                    let _ = s.stop();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> String {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../../examples/demo.show.json").to_string()
    }

    /// A new show without a file starts as it is (local clock), takes edits,
    /// and keeps taking them after "save as" gives it a file.
    #[test]
    fn a_new_show_starts_without_a_file_and_keeps_following_after_save_as() {
        use omoiiro_core::edit::{EditCommand, Snap};
        let (be, ports) = omoiiro_midi::fake::fake_backend();
        ports.add_output(omoiiro_midi::ports::names::CONTROL_PORT);
        let state = AppState::new(None);
        *state.backend.lock().unwrap() = Box::new(be);
        let ed = editor::EditorState::default();
        let ps = preview::PreviewState::new();
        let req = || StartRequest { show_path: String::new(), time_source: omoiiro_runtime::session::TimeSource::Local };
        // Nothing open: refused in Japanese.
        assert!(start_runtime(&state, &ed, req()).unwrap_err().message.contains("新規ショー"));

        // The new show starts; the runtime has no show path but follows the document.
        let mut doc = editor::EditorDoc { path: PathBuf::new(), history: omoiiro_core::edit::History::new_unsaved(omoiiro_core::edit::new_show("x")) };
        doc.history.apply(&EditCommand::AddPreset { name: "白".into(), effect_type: "builtin.solid".into() }, Snap::None).unwrap();
        *ed.doc.lock().unwrap() = Some(doc);
        let r = start_runtime(&state, &ed, req()).unwrap();
        assert_eq!(r.revision, 1);
        assert!(state.show_path.lock().unwrap().is_none());
        let v = editor::apply_command(&ed, &state, &ps, EditCommand::AddLayer { name: "a".into(), index: None }, Snap::None).unwrap();
        assert_eq!(v.runtime_revision, Some(2), "the edit reached the runtime");

        // Save as: the runtime now follows the file.
        let dir = std::env::temp_dir().join(format!("omoiiro-newstart-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("new.show.json");
        let v = editor::save_as(&ed, &state, file.to_str().unwrap()).unwrap();
        assert_eq!(v.runtime_revision, Some(2));
        assert_eq!(state.show_path.lock().unwrap().as_deref(), Some(file.as_path()));
        let v = editor::apply_command(&ed, &state, &ps, EditCommand::AddLayer { name: "b".into(), index: None }, Snap::None).unwrap();
        assert_eq!(v.runtime_revision, Some(3), "still following after the save");
        std::thread::sleep(std::time::Duration::from_millis(200)); // adopted at the next frame
        let st = state.session.lock().unwrap().as_ref().unwrap().status();
        assert_eq!((st.follow.revision, st.time_source), (3, omoiiro_runtime::session::TimeSource::Local));
        state.session.lock().unwrap().take().unwrap().stop().unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn start_request_validation_uses_terminal_settings() {
        let mut s = TerminalSettings::default();
        let cfg = StartRequest { show_path: demo(), time_source: Default::default() }.validate(&s, None).unwrap();
        assert_eq!(cfg.led_out, LedOut::Virtual);
        assert!(cfg.control.is_none(), "no control input: no route");
        assert_eq!(StartRequest { show_path: "".into(), time_source: Default::default() }.validate(&s, None).unwrap_err().code, "invalid");
        assert_eq!(StartRequest { show_path: "nope.show.json".into(), time_source: Default::default() }.validate(&s, None).unwrap_err().code, "io");
        assert_eq!(StartRequest { show_path: "demo.json".into(), time_source: Default::default() }.validate(&s, None).unwrap_err().code, "invalid", "only *.show.json");
        s.ports.control_in = Some("Pads".into());
        s.ports.timecode_out = Some("OMOI-IRO Timecode".into());
        let cfg = StartRequest { show_path: demo(), time_source: Default::default() }.validate(&s, None).unwrap();
        assert_eq!(cfg.time_source, omoiiro_runtime::session::TimeSource::Live, "existing calls follow Live");
        let req: StartRequest = serde_json::from_str(&format!("{{\"show_path\": {:?}, \"time_source\": \"local\"}}", demo())).unwrap();
        assert_eq!(req.validate(&s, None).unwrap().time_source, omoiiro_runtime::session::TimeSource::Local);
        assert_eq!(cfg.control, Some(("Pads".into(), omoiiro_midi::ports::names::CONTROL_PORT.into())));
        assert_eq!(cfg.timecode_out.as_deref(), Some("OMOI-IRO Timecode"));
        // Device LED output follows the terminal permission.
        s.ports.led_out = Some("LPX MIDI".into());
        assert_eq!(StartRequest { show_path: demo(), time_source: Default::default() }.validate(&s, None).unwrap().led_out, LedOut::Port { spec: "LPX MIDI".into(), allow_device: false });
        s.devices.allow_launchpad_led = true;
        let cfg = StartRequest { show_path: demo(), time_source: Default::default() }.validate(&s, None).unwrap();
        assert_eq!(cfg.led_out, LedOut::Port { spec: "LPX MIDI".into(), allow_device: true });
        assert!(cfg.identify_led, "a device is identified first");
        s.bridge_bind = "localhost".into();
        assert_eq!(StartRequest { show_path: demo(), time_source: Default::default() }.validate(&s, None).unwrap_err().code, "settings");
    }

    #[test]
    fn status_dto_keys_match_the_typescript_mirror() {
        let v = serde_json::to_value(StatusDto::stopped(Default::default())).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        let ts = include_str!("../../src/ipc/types.ts");
        for k in keys {
            assert!(ts.contains(&format!("  {k}:")), "StatusDto field {k} missing in types.ts");
        }
    }
}
