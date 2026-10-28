//! Timeline editing (U04): the edit history lives here, in Rust; the UI only
//! sends `EditCommand`s and draws the returned view.
//!
//! * Every command is applied to a copy and validated (`omoiiro_core::edit`);
//!   a rejected edit changes nothing and returns the reasons.
//! * Each accepted state (edit, undo, redo) is handed to a running runtime
//!   that follows the same show file (`FollowSession::submit_show`, adopted
//!   at the next frame boundary) and to the local preview (clock kept).
//! * Nothing here moves the show clock. Saving is U07.

use crate::dto::{CommandError, DocumentDto};
use crate::files::{self, Kind};
use crate::preview::PreviewState;
use crate::AppState;
use omoiiro_core::edit::{preset_users, EditCommand, EditOutcome, History, OnConflict, Snap};
use omoiiro_core::model::{Content, Effect, Show};
use omoiiro_core::safety::{FlashProfile, FlashWarning, SafetyLimits};
use omoiiro_runtime::preview::show_length_us;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::State;

type CmdResult<T> = Result<T, CommandError>;

pub struct EditorDoc {
    /// Empty for a new show that has not been saved yet.
    pub path: PathBuf,
    pub history: History,
}

#[derive(Default)]
pub struct EditorState {
    pub doc: Mutex<Option<EditorDoc>>,
}

#[derive(Debug, Serialize)]
pub struct ClipDto {
    pub id: String,
    pub preset_id: String,
    pub start_us: u64,
    pub duration_us: u64,
    pub fade_in_us: u64,
    pub fade_out_us: u64,
}

#[derive(Debug, Serialize)]
pub struct LayerDto {
    pub id: String,
    pub name: String,
    pub target_id: String,
    /// "timeline", "continuous" or "triggered".
    pub kind: &'static str,
    pub enabled: bool,
    pub opacity: f64,
    pub blend: &'static str,
    /// Continuous layers: their preset.
    pub preset_id: Option<String>,
    pub clips: Vec<ClipDto>,
    /// Triggered layers: when they fire.
    pub trigger: Option<omoiiro_core::model::TriggerSpec>,
}

#[derive(Debug, Serialize)]
pub struct PresetDto {
    pub id: String,
    pub name: String,
    pub effect_type: &'static str,
    /// `#RRGGBB` of the effect's color (for drawing clips).
    pub color: String,
    /// The effect as saved in the show (type, version, params).
    pub effect: Effect,
    /// Clips and continuous layers using this preset.
    pub used_by_clips: Vec<String>,
    pub used_by_layers: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TargetDto {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct EditorView {
    pub path: String,
    pub name: String,
    pub targets: Vec<TargetDto>,
    pub length_us: Option<i64>,
    pub layers: Vec<LayerDto>,
    pub presets: Vec<PresetDto>,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<&'static str>,
    pub redo_label: Option<&'static str>,
    pub history_depth: usize,
    /// Revision adopted by the running runtime for this file, if it follows it.
    pub runtime_revision: Option<u64>,
    /// The last command's result (new clip / layer ID to select).
    pub outcome: Option<EditOutcome>,
    /// Flashing above this terminal's limit, at the show's preview tempo.
    pub flash_warnings: Vec<FlashWarning>,
    pub max_flash_hz: f64,
    pub preview_bpm: f64,
    /// Show sync settings editable in the GUI (`set_sync`).
    pub beat_source: omoiiro_core::model::BeatSource,
    pub mtc_output_enabled: bool,
}

fn limits(app: &AppState) -> SafetyLimits {
    app.settings.lock().unwrap().safety
}

fn color_of(e: &Effect) -> String {
    match e {
        Effect::Solid { params, .. } => params.color.value().clone(),
        Effect::Chase { params, .. } => params.color.value().clone(),
        Effect::Pulse { params, .. } => params.color.value().clone(),
        Effect::Strobe { params, .. } => params.color.value().clone(),
        Effect::Noise { params, .. } => params.highlight_color.value().clone(),
        Effect::Ripple { params, .. } => params.color.value().clone(),
        Effect::Rain { params, .. } => params.color.value().clone(),
        Effect::Wave { params, .. } => params.color.value().clone(),
        Effect::Orb { params, .. } => params.color.value().clone(),
        Effect::Regions { params, .. } => params.color.value().clone(),
        Effect::Path { params, .. } => params.color.value().clone(),
        Effect::Shape { params, .. } => params.color.value().clone(),
        Effect::TapGlow { params, .. } => params.color_mid.value().clone(),
        Effect::Cross { params, .. } => params.color.value().clone(),
    }
}

fn view(doc: &EditorDoc, runtime_revision: Option<u64>, outcome: Option<EditOutcome>, limits: SafetyLimits) -> EditorView {
    let s = doc.history.current();
    let flash_warnings = FlashProfile::of(s).warnings(s.preview.bpm, limits.max_flash_hz);
    EditorView {
        path: doc.path.display().to_string(),
        name: s.name.clone(),
        targets: s.targets.iter().map(|t| TargetDto { id: t.id.clone(), name: t.name.clone() }).collect(),
        length_us: show_length_us(s),
        layers: s
            .layers
            .iter()
            .map(|l| LayerDto {
                id: l.id.clone(),
                name: l.name.clone(),
                target_id: l.target_id.clone(),
                kind: match l.content {
                    Content::Timeline { .. } => "timeline",
                    Content::Continuous { .. } => "continuous",
                    Content::Triggered { .. } => "triggered",
                },
                enabled: l.enabled,
                opacity: l.opacity,
                blend: match l.blend {
                    omoiiro_core::model::Blend::Over => "over",
                    omoiiro_core::model::Blend::Add => "add",
                    omoiiro_core::model::Blend::Max => "max",
                },
                preset_id: match &l.content {
                    Content::Continuous { preset_id } => Some(preset_id.clone()),
                    Content::Timeline { .. } | Content::Triggered { .. } => None,
                },
                clips: match l.content.clips() {
                    Some(clips) => clips
                        .iter()
                        .map(|c| ClipDto { id: c.id.clone(), preset_id: c.preset_id.clone(), start_us: c.start_us, duration_us: c.duration_us, fade_in_us: c.fade_in_us, fade_out_us: c.fade_out_us })
                        .collect(),
                    None => Vec::new(),
                },
                trigger: match &l.content {
                    Content::Triggered { trigger, .. } => Some(trigger.clone()),
                    _ => None,
                },
            })
            .collect(),
        presets: s
            .presets
            .iter()
            .map(|p| {
                let (used_by_clips, used_by_layers) = preset_users(s, &p.id);
                PresetDto { id: p.id.clone(), name: p.name.clone(), effect_type: p.effect.type_name(), color: color_of(&p.effect), effect: p.effect.clone(), used_by_clips, used_by_layers }
            })
            .collect(),
        dirty: doc.history.dirty(),
        can_undo: doc.history.can_undo(),
        can_redo: doc.history.can_redo(),
        undo_label: doc.history.undo_label(),
        redo_label: doc.history.redo_label(),
        history_depth: doc.history.depth(),
        runtime_revision,
        outcome,
        flash_warnings,
        max_flash_hz: limits.max_flash_hz,
        preview_bpm: s.preview.bpm,
        beat_source: s.sync.beat_source,
        mtc_output_enabled: s.sync.mtc_output.as_ref().is_some_and(|m| m.enabled),
    }
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Hand the current state to the runtime (if it follows this file) and to
/// the local preview (if it shows this file). Never holds two locks at once.
fn propagate(app: &AppState, ps: &PreviewState, path: &std::path::Path, show: &Show) -> Result<Option<u64>, CommandError> {
    let runtime_path = app.show_path.lock().unwrap().clone();
    let mut revision = None;
    if runtime_path.as_deref().is_some_and(|p| same_file(p, path)) {
        if let Some(s) = app.session.lock().unwrap().as_mut() {
            revision = Some(s.submit_show(show).map_err(|i| CommandError::new("edit", format!("runtime rejected the show: {i:?}")))?);
        }
    }
    ps.replace_local_if(path, show);
    Ok(revision)
}

/// The current editor show, if it is the given file (used by `runtime_start`).
pub fn current_show_for(ed: &EditorState, path: &std::path::Path) -> Option<Show> {
    let doc = ed.doc.lock().unwrap();
    doc.as_ref().filter(|d| same_file(&d.path, path)).map(|d| (**d.history.current()).clone())
}

#[tauri::command(async)]
/// `discard`: open even with unsaved changes (the user chose 「保存せずに開く」).
pub fn editor_open(ed: State<'_, EditorState>, app: State<'_, AppState>, show_path: String, discard: Option<bool>) -> CmdResult<EditorView> {
    let path = files::for_read(&show_path, Kind::Show)?;
    // A file that fails to load never replaces the open document.
    let show = omoiiro_store::load_path(&path).map_err(|e| files::load_error(&path, e))?;
    {
        let g = ed.doc.lock().unwrap();
        if !discard.unwrap_or(false) && g.as_ref().is_some_and(|d| d.history.dirty()) {
            return Err(CommandError::new("unsaved", "編集中のショーに未保存の変更がある。保存してから開く"));
        }
    }
    let doc = EditorDoc { path: path.clone(), history: History::new(show) };
    let v = view(&doc, None, None, limits(&app));
    *ed.doc.lock().unwrap() = Some(doc);
    crate::settings::remember_show(&app, &path.display().to_string());
    Ok(v)
}

/// A new, empty show (unsaved; "save as" gives it a file). `discard` as in `editor_open`.
#[tauri::command]
pub fn editor_new(ed: State<'_, EditorState>, app: State<'_, AppState>, name: String, discard: Option<bool>) -> CmdResult<EditorView> {
    {
        let g = ed.doc.lock().unwrap();
        if !discard.unwrap_or(false) && g.as_ref().is_some_and(|d| d.history.dirty()) {
            return Err(CommandError::new("unsaved", "編集中のショーに未保存の変更がある。保存してから新規を作る"));
        }
    }
    let doc = EditorDoc { path: PathBuf::new(), history: History::new_unsaved(omoiiro_core::edit::new_show(&name)) };
    let v = view(&doc, None, None, limits(&app));
    *ed.doc.lock().unwrap() = Some(doc);
    Ok(v)
}

/// The open document, for the status poll.
pub fn document(ed: &EditorState) -> Option<DocumentDto> {
    ed.doc.lock().unwrap().as_ref().map(|d| DocumentDto { path: d.path.display().to_string(), dirty: d.history.dirty() })
}

/// Save to `path` (temporary file, flush, replace). The original stays when this fails.
fn save_to(doc: &mut EditorDoc, path: &std::path::Path) -> CmdResult<()> {
    omoiiro_store::save_atomic(path, doc.history.current())
        .map_err(|e| CommandError::new("save", format!("{} に保存できなかった（元のファイルはそのまま）：{e}", path.display())))?;
    doc.history.mark_saved();
    Ok(())
}

#[tauri::command(async)]
pub fn editor_save(ed: State<'_, EditorState>, app: State<'_, AppState>) -> CmdResult<EditorView> {
    let l = limits(&app);
    let mut g = ed.doc.lock().unwrap();
    let doc = g.as_mut().ok_or_else(|| CommandError::new("no_document", "open a show in the timeline first"))?;
    if doc.path.as_os_str().is_empty() {
        return Err(CommandError::new("save_as", "新しいショーは「名前を付けて保存」で保存する"));
    }
    let path = doc.path.clone();
    save_to(doc, &path)?;
    Ok(view(doc, None, None, l))
}

/// Save under a new name (`*.show.json`); the document then refers to it.
#[tauri::command(async)]
pub fn editor_save_as(ed: State<'_, EditorState>, app: State<'_, AppState>, path: String) -> CmdResult<EditorView> {
    let target = files::for_write(&path, Kind::Show)?;
    let l = limits(&app);
    let v = {
        let mut g = ed.doc.lock().unwrap();
        let doc = g.as_mut().ok_or_else(|| CommandError::new("no_document", "open a show in the timeline first"))?;
        save_to(doc, &target)?;
        doc.path = target.clone();
        view(doc, None, None, l)
    };
    crate::settings::remember_show(&app, &target.display().to_string());
    Ok(v)
}

/// Save the open document if it has changes (used when closing).
pub fn save_if_dirty(ed: &EditorState) -> CmdResult<()> {
    let mut g = ed.doc.lock().unwrap();
    if let Some(doc) = g.as_mut() {
        if doc.history.dirty() {
            if doc.path.as_os_str().is_empty() {
                return Err(CommandError::new("save_as", "新しいショーは「名前を付けて保存」で保存してから閉じる"));
            }
            let path = doc.path.clone();
            save_to(doc, &path)?;
        }
    }
    Ok(())
}

pub fn is_dirty(ed: &EditorState) -> bool {
    ed.doc.lock().unwrap().as_ref().is_some_and(|d| d.history.dirty())
}

#[tauri::command]
pub fn editor_view(ed: State<'_, EditorState>, app: State<'_, AppState>) -> Option<EditorView> {
    let l = limits(&app);
    let path = ed.doc.lock().unwrap().as_ref().map(|d| d.path.clone())?;
    // Not holding the document lock while reading the session (one lock at a time).
    let rev = runtime_revision_for(&app, &path);
    ed.doc.lock().unwrap().as_ref().map(|d| view(d, rev, None, l))
}

/// Revision the running runtime uses, when it follows this file (else None).
fn runtime_revision_for(app: &AppState, path: &std::path::Path) -> Option<u64> {
    let runtime_path = app.show_path.lock().unwrap().clone();
    if !runtime_path.as_deref().is_some_and(|p| same_file(p, path)) {
        return None;
    }
    app.session.lock().unwrap().as_ref().map(|s| s.revision)
}

/// Read a preset file (0.1.1, kind omoiiro.preset) and insert it through the history.
#[tauri::command(async)]
pub fn preset_import(ed: State<'_, EditorState>, app: State<'_, AppState>, ps: State<'_, Arc<PreviewState>>, path: String, on_conflict: OnConflict) -> CmdResult<EditorView> {
    let p = files::for_read(&path, Kind::Preset)?;
    let text = std::fs::read_to_string(&p).map_err(|e| CommandError::new("import", format!("{}: {e}", p.display())))?;
    let preset = omoiiro_store::presets::parse_preset_file(&text).map_err(|e| files::load_error(&p, e))?;
    editor_apply(ed, app, ps, EditCommand::InsertPreset { preset, on_conflict }, Snap::None)
}

/// Write one preset of the edited show to a preset file (temporary file, then replace).
#[tauri::command(async)]
pub fn preset_export(ed: State<'_, EditorState>, preset_id: String, path: String, overwrite: bool) -> CmdResult<String> {
    let text = {
        let g = ed.doc.lock().unwrap();
        let doc = g.as_ref().ok_or_else(|| CommandError::new("no_document", "open a show in the timeline first"))?;
        omoiiro_store::presets::export_preset(doc.history.current(), &preset_id).ok_or_else(|| CommandError::new("export", format!("no preset {preset_id:?}")))?
    };
    let p = files::for_write(&path, Kind::Preset)?;
    if p.exists() && !overwrite {
        return Err(CommandError::new("exists", format!("{} already exists", p.display())));
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| CommandError::new("export", format!("{}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, &p).map_err(|e| CommandError::new("export", format!("{}: {e}", p.display())))?;
    Ok(p.display().to_string())
}

#[tauri::command(async)]
pub fn editor_apply(ed: State<'_, EditorState>, app: State<'_, AppState>, ps: State<'_, Arc<PreviewState>>, command: EditCommand, snap: Snap) -> CmdResult<EditorView> {
    let (path, show, outcome) = {
        let mut g = ed.doc.lock().unwrap();
        let doc = g.as_mut().ok_or_else(|| CommandError::new("no_document", "open a show in the timeline first"))?;
        let outcome = doc.history.apply(&command, snap).map_err(|e| CommandError::new("edit", e.to_string()))?;
        (doc.path.clone(), doc.history.current().clone(), outcome)
    };
    let rev = propagate(&app, &ps, &path, &show)?;
    let l = limits(&app);
    let g = ed.doc.lock().unwrap();
    Ok(view(g.as_ref().expect("document"), rev, Some(outcome), l))
}

fn step(ed: &EditorState, app: &AppState, ps: &PreviewState, undo: bool) -> CmdResult<EditorView> {
    let (path, show) = {
        let mut g = ed.doc.lock().unwrap();
        let doc = g.as_mut().ok_or_else(|| CommandError::new("no_document", "open a show in the timeline first"))?;
        let moved = if undo { doc.history.undo() } else { doc.history.redo() };
        if !moved {
            return Err(CommandError::new("history", if undo { "nothing to undo" } else { "nothing to redo" }));
        }
        (doc.path.clone(), doc.history.current().clone())
    };
    let rev = propagate(app, ps, &path, &show)?;
    let l = limits(app);
    let g = ed.doc.lock().unwrap();
    Ok(view(g.as_ref().expect("document"), rev, None, l))
}

#[tauri::command(async)]
pub fn editor_undo(ed: State<'_, EditorState>, app: State<'_, AppState>, ps: State<'_, Arc<PreviewState>>) -> CmdResult<EditorView> {
    step(&ed, &app, &ps, true)
}

#[tauri::command(async)]
pub fn editor_redo(ed: State<'_, EditorState>, app: State<'_, AppState>, ps: State<'_, Arc<PreviewState>>) -> CmdResult<EditorView> {
    step(&ed, &app, &ps, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_and_start_hand_off_use_the_edited_show() {
        let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../examples/demo.show.json"));
        let show = omoiiro_store::load_path(&path).unwrap();
        let mut doc = EditorDoc { path: path.clone(), history: History::new(show) };
        let out = doc.history.apply(&EditCommand::AddLayer { name: "pads".into(), index: None }, Snap::None).unwrap();
        let v = view(&doc, Some(2), Some(out), SafetyLimits::default());
        assert!(v.flash_warnings.is_empty(), "demo pulse at 120 bpm is 2 Hz");
        let strict = view(&doc, None, None, SafetyLimits { max_flash_hz: 1.5, ..SafetyLimits::default() });
        assert_eq!(strict.flash_warnings.len(), 1);
        assert_eq!(strict.flash_warnings[0].preset_id, "beat-white");
        assert!(v.dirty && v.can_undo && !v.can_redo);
        assert_eq!(v.layers.last().unwrap().name, "pads");
        assert_eq!(v.presets[0].color, "#163B75");
        assert_eq!(v.outcome.as_ref().unwrap().layer_id.as_deref(), Some(v.layers.last().unwrap().id.as_str()));
        let ed = EditorState::default();
        *ed.doc.lock().unwrap() = Some(doc);
        let started = current_show_for(&ed, &path).expect("same file");
        assert_eq!(started.layers.last().unwrap().name, "pads", "runtime_start takes the unsaved edit");
        assert!(current_show_for(&ed, std::path::Path::new("other.show.json")).is_none());
    }

    #[test]
    fn save_is_atomic_stable_and_clears_dirty() {
        let src = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../examples/demo.show.json"));
        let dir = std::env::temp_dir().join(format!("omoiiro-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("copy.show.json");
        std::fs::copy(&src, &path).unwrap();
        let original = std::fs::read(&path).unwrap();
        let mut doc = EditorDoc { path: path.clone(), history: History::new(omoiiro_store::load_path(&path).unwrap()) };
        // Unchanged save: the same file except the migrated version (0.1.1 -> current).
        save_to(&mut doc, &path).unwrap();
        let migrated = String::from_utf8(original.clone()).unwrap().replace("\"schema_version\": \"0.1.1\"", &format!("\"schema_version\": \"{}\"", omoiiro_core::model::SCHEMA_VERSION));
        assert_eq!(String::from_utf8(std::fs::read(&path).unwrap()).unwrap(), migrated);
        doc.history.apply(&EditCommand::AddLayer { name: "x".into(), index: None }, Snap::None).unwrap();
        assert!(doc.history.dirty());
        save_to(&mut doc, &path).unwrap();
        assert!(!doc.history.dirty());
        assert_ne!(std::fs::read(&path).unwrap(), original);
        assert!(!dir.join("copy.show.json.tmp").exists() && std::fs::read_dir(&dir).unwrap().count() == 1, "no temporary file left");
        // A failed save keeps the original file and reports why.
        doc.history.apply(&EditCommand::AddLayer { name: "y".into(), index: None }, Snap::None).unwrap();
        let before = std::fs::read(&path).unwrap();
        let blocked = dir.join("blocked.show.json");
        std::fs::create_dir_all(&blocked).unwrap(); // a folder where the file should go
        let e = save_to(&mut doc, &blocked).unwrap_err();
        assert_eq!(e.code, "save");
        assert!(e.message.contains("元のファイルはそのまま"));
        assert!(doc.history.dirty(), "still unsaved");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preset_export_then_import_through_the_history() {
        let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../examples/demo.show.json"));
        let mut h = History::new(omoiiro_store::load_path(&path).unwrap());
        let text = omoiiro_store::presets::export_preset(h.current(), "sweep-cyan").unwrap();
        let preset = omoiiro_store::presets::parse_preset_file(&text).unwrap();
        assert!(h.apply(&EditCommand::InsertPreset { preset: preset.clone(), on_conflict: OnConflict::Error }, Snap::None).is_err());
        let out = h.apply(&EditCommand::InsertPreset { preset, on_conflict: OnConflict::Rename }, Snap::None).unwrap();
        assert_eq!(out.preset_id.as_deref(), Some("sweep-cyan-2"));
        let v = view(&EditorDoc { path, history: h }, None, None, SafetyLimits::default());
        let p = v.presets.iter().find(|p| p.id == "sweep-cyan-2").unwrap();
        assert!(p.used_by_clips.is_empty() && p.used_by_layers.is_empty());
        assert_eq!(v.presets.iter().find(|p| p.id == "sweep-cyan").unwrap().used_by_clips, vec!["intro-sweep".to_string()]);
    }
}
