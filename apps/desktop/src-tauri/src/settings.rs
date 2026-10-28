//! Connection and sync settings (U02): terminal settings in the app's config
//! directory (`%APPDATA%\jp.faio.omoiiro\terminal-settings.json` on
//! Windows), checked against the current port lists before they are saved
//! or used.
//!
//! * `settings_check` lists ports and checks the roles. It sends nothing.
//! * `settings_listen` opens one input role for a few seconds and counts
//!   what arrives (receive check). It sends nothing and is refused while the
//!   runtime holds the ports.
//! * `settings_save` refuses settings with errors (reverse flow, double
//!   assignment, Launchpad-looking outputs, same-name ports, bad address)
//!   and returns the reasons. A missing port is only a warning.
//! * Saved settings take effect at the next `runtime_start`.

use crate::dto::CommandError;
use crate::AppState;
use crossbeam_channel::bounded;
use omoiiro_midi::backend::{Ingress, IngressStats, RawIn};
use omoiiro_midi::clock::MonotonicClock;
use omoiiro_runtime::diagnostics::record::hex;
use omoiiro_runtime::settings::{self as ts, check_roles, CheckReport, ListenCounts, TerminalSettings};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::State;

type CmdResult<T> = Result<T, CommandError>;

#[derive(Debug, Serialize)]
pub struct SettingsView {
    pub settings: TerminalSettings,
    /// Where the file lives (None before the app finished starting).
    pub path: Option<String>,
    /// The saved file could not be read; defaults are shown until a save.
    pub load_error: Option<String>,
    pub link_available: bool,
    pub running: bool,
}

#[derive(Debug, Serialize)]
pub struct SettingsCheck {
    pub backend: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub report: CheckReport,
}

#[derive(Debug, Serialize)]
pub struct ListenResult {
    pub role: String,
    pub port: String,
    pub duration_ms: u64,
    pub counts: ListenCounts,
    /// First messages as hex (at most 8).
    pub samples: Vec<String>,
    pub dropped: u64,
}

pub fn load_into(state: &AppState, dir: PathBuf) {
    match ts::load(&dir) {
        Ok(s) => *state.settings.lock().unwrap() = s,
        Err(e) => *state.settings_load_error.lock().unwrap() = Some(e),
    }
    *state.settings_dir.lock().unwrap() = Some(dir);
}

/// Add a show to the recent list and save (best effort; errors are ignored
/// because the list is a convenience).
pub fn remember_show(state: &AppState, path: &str) {
    let mut s = state.settings.lock().unwrap();
    s.remember_show(path);
    if let Some(dir) = state.settings_dir.lock().unwrap().as_ref() {
        if state.settings_load_error.lock().unwrap().is_none() {
            let _ = ts::save(dir, &s);
        }
    }
}

fn view(state: &AppState) -> SettingsView {
    SettingsView {
        settings: state.settings.lock().unwrap().clone(),
        path: state.settings_dir.lock().unwrap().as_ref().map(|d| d.join(ts::SETTINGS_FILE).display().to_string()),
        load_error: state.settings_load_error.lock().unwrap().clone(),
        link_available: cfg!(feature = "link"),
        running: state.session.lock().unwrap().is_some(),
    }
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> SettingsView {
    view(&state)
}

#[tauri::command(async)]
pub fn settings_check(state: State<'_, AppState>, settings: TerminalSettings) -> CmdResult<SettingsCheck> {
    let settings = settings.normalized();
    let (inputs, outputs) = state.port_lists()?;
    let report = check_roles(&settings, &inputs, &outputs);
    Ok(SettingsCheck { backend: state.backend.lock().unwrap().backend_name().into(), inputs, outputs, report })
}

#[tauri::command(async)]
pub fn settings_save(state: State<'_, AppState>, settings: TerminalSettings) -> CmdResult<SettingsView> {
    let mut settings = settings.normalized();
    if settings.version != ts::SETTINGS_VERSION {
        return Err(CommandError::new("invalid", format!("settings version {} is not {}", settings.version, ts::SETTINGS_VERSION)));
    }
    if settings.link && !cfg!(feature = "link") {
        return Err(CommandError::new("invalid", "this build has no Ableton Link support"));
    }
    let (inputs, outputs) = state.port_lists()?;
    let report = check_roles(&settings, &inputs, &outputs);
    if !report.ok {
        return Err(CommandError::new("settings", report.errors.join(" / ")));
    }
    // The recent and library lists are kept by the app, not edited by the form.
    {
        let current = state.settings.lock().unwrap();
        settings.recent_shows = current.recent_shows.clone();
        settings.library_shows = current.library_shows.clone();
    }
    let dir = state.settings_dir.lock().unwrap().clone().ok_or_else(|| CommandError::new("settings", "settings directory is not ready"))?;
    ts::save(&dir, &settings).map_err(|e| CommandError::new("save", e))?;
    *state.settings.lock().unwrap() = settings;
    *state.settings_load_error.lock().unwrap() = None;
    Ok(view(&state))
}

/// Receive check of an input role: open, count for `duration_ms`, close. Sends nothing.
#[tauri::command(async)]
pub fn settings_listen(state: State<'_, AppState>, settings: TerminalSettings, role: String, duration_ms: u64) -> CmdResult<ListenResult> {
    if state.session.lock().unwrap().is_some() {
        return Err(CommandError::new("busy", "stop the runtime first: it holds the input ports"));
    }
    if !(500..=10_000).contains(&duration_ms) {
        return Err(CommandError::new("invalid", "duration must be 500..10000 ms"));
    }
    let settings = settings.normalized();
    let spec = match role.as_str() {
        "control_in" => settings.ports.control_in.clone(),
        "clock_in" => settings.ports.clock_in.clone(),
        _ => return Err(CommandError::new("invalid", format!("{role:?} is not an input role"))),
    }
    .ok_or_else(|| CommandError::new("invalid", "the role has no port"))?;
    let (tx, rx) = bounded::<RawIn>(4096);
    let stats = Arc::new(IngressStats::default());
    let conn = state
        .backend
        .lock()
        .unwrap()
        .open_input(&spec, true, Ingress::new(MonotonicClock::new(), tx, stats.clone()))
        .map_err(|e| CommandError::new("ports", e.to_string()))?;
    std::thread::sleep(Duration::from_millis(duration_ms));
    let port = conn.name.clone();
    drop(conn);
    let mut counts = ListenCounts::default();
    let mut samples = Vec::new();
    for ev in rx.try_iter() {
        counts.add(&ev.bytes);
        if samples.len() < 8 {
            samples.push(hex(&ev.bytes));
        }
    }
    Ok(ListenResult { role, port, duration_ms, counts, samples, dropped: stats.dropped() })
}
