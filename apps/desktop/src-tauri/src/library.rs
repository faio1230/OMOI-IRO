//! Material library (A20): ordinary show files registered in the terminal
//! settings (`library_shows`). Their presets and compounds are listed and
//! copied into the edited show on use.
//!
//! * Library files are only read (store loading with migration and the size
//!   limit), never written. Listing and previews touch neither the edited
//!   show nor the runtime.
//! * Import goes through the edit history like any edit (`InsertPreset` /
//!   `InsertCompound`), so it also works while the runtime follows the show.
//!   Importing something the show already has adds nothing.

use crate::dto::CommandError;
use crate::editor::{self, EditorState, EditorView};
use crate::files::{self, Kind};
use crate::preview::PreviewState;
use crate::AppState;
use omoiiro_core::edit::{self, EditCommand, EditOutcome, MaterialKind, OnConflict, Snap};
use omoiiro_core::model::{Content, MaterialMetadata, Preset, Show};
use omoiiro_runtime::settings as ts;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tauri::State;

type CmdResult<T> = Result<T, CommandError>;

/// Library files kept parsed (path, modification stamp).
const CACHE_FILES: usize = 4;

type Stamp = (Option<SystemTime>, u64);

#[derive(Default)]
pub struct LibraryState {
    cache: Mutex<Vec<(PathBuf, Stamp, Arc<Show>)>>,
}

#[derive(Debug, Serialize)]
pub struct LibraryPreset {
    pub id: String,
    pub name: String,
    pub effect_type: &'static str,
    pub metadata: MaterialMetadata,
    pub effect: omoiiro_core::model::Effect,
}

#[derive(Debug, Serialize)]
pub struct LibraryCompound {
    pub id: String,
    pub name: String,
    pub length_us: u64,
    pub metadata: MaterialMetadata,
    pub preset_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LibraryView {
    pub path: String,
    pub name: String,
    /// Changes when the file changes (for preview caches).
    pub stamp: String,
    pub presets: Vec<LibraryPreset>,
    pub compounds: Vec<LibraryCompound>,
}

fn stamp_of(path: &Path) -> CmdResult<Stamp> {
    let meta = std::fs::metadata(path).map_err(|e| CommandError::new("io", format!("{}: {e}", path.display())))?;
    Ok((meta.modified().ok(), meta.len()))
}

/// Read (or reuse) a library file. Never writes it.
fn load(lib: &LibraryState, raw: &str) -> CmdResult<(PathBuf, Stamp, Arc<Show>)> {
    let path = files::for_read(raw, Kind::Show)?;
    let stamp = stamp_of(&path)?;
    if let Some((_, _, show)) = lib.cache.lock().unwrap().iter().find(|(p, s, _)| *p == path && *s == stamp) {
        return Ok((path, stamp, show.clone()));
    }
    let show = Arc::new(omoiiro_store::load_path(&path).map_err(|e| files::load_error(&path, e))?);
    let mut cache = lib.cache.lock().unwrap();
    cache.retain(|(p, _, _)| *p != path);
    if cache.len() >= CACHE_FILES {
        cache.remove(0);
    }
    cache.push((path.clone(), stamp, show.clone()));
    Ok((path, stamp, show))
}

/// Presets used by a compound's inner layers, in first-use order.
fn compound_preset_ids(c: &omoiiro_core::model::Compound) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for l in &c.layers {
        let used: Vec<&str> = match &l.content {
            Content::Continuous { preset_id } => vec![preset_id.as_str()],
            content => content.clips().into_iter().flatten().filter(|c| c.compound_id.is_none()).map(|c| c.preset_id.as_str()).collect(),
        };
        for id in used {
            if !id.is_empty() && !ids.iter().any(|x| x == id) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

fn library_view(path: &Path, stamp: Stamp, show: &Show) -> LibraryView {
    let modified = stamp.0.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
    LibraryView {
        path: path.display().to_string(),
        name: show.name.clone(),
        stamp: format!("{modified}:{}", stamp.1),
        presets: show.presets.iter().map(|p| LibraryPreset { id: p.id.clone(), name: p.name.clone(), effect_type: p.effect.type_name(), metadata: p.metadata.clone(), effect: p.effect.clone() }).collect(),
        compounds: show
            .compounds
            .iter()
            .map(|c| LibraryCompound { id: c.id.clone(), name: c.name.clone(), length_us: c.length_us(), metadata: c.metadata.clone(), preset_ids: compound_preset_ids(c) })
            .collect(),
    }
}

/// The edit that copies one library material into a show.
fn import_command(show: &Show, kind: MaterialKind, id: &str) -> CmdResult<EditCommand> {
    let missing = || CommandError::new("not_found", format!("ライブラリーに素材 {id:?} がない（ファイルが変わった可能性）"));
    Ok(match kind {
        MaterialKind::Preset => EditCommand::InsertPreset { preset: show.presets.iter().find(|p| p.id == id).ok_or_else(missing)?.clone(), on_conflict: OnConflict::Rename },
        MaterialKind::Compound => {
            let compound = show.compounds.iter().find(|c| c.id == id).ok_or_else(missing)?.clone();
            let ids = compound_preset_ids(&compound);
            let presets: Vec<Preset> = ids.iter().filter_map(|i| show.presets.iter().find(|p| &p.id == i).cloned()).collect();
            EditCommand::InsertCompound { compound, presets, on_conflict: OnConflict::Rename }
        }
    })
}

/// What an import would reuse instead of adding (the show already has it).
fn reused(current: &Show, command: &EditCommand) -> CmdResult<Option<EditOutcome>> {
    Ok(match command {
        EditCommand::InsertPreset { preset, .. } => edit::existing_preset(current, preset).map(|p| EditOutcome { preset_id: Some(p.id.clone()), ..Default::default() }),
        EditCommand::InsertCompound { .. } => {
            let (after, out) = edit::apply(current, command, Snap::None).map_err(|e| CommandError::new("edit", e.to_string()))?;
            (after == *current).then_some(out)
        }
        _ => None,
    })
}

#[tauri::command(async)]
pub fn library_list(lib: State<'_, LibraryState>, path: String) -> CmdResult<LibraryView> {
    let (p, stamp, show) = load(&lib, &path)?;
    Ok(library_view(&p, stamp, &show))
}

/// 24 sampled frames, as `material_preview` but from a library file.
#[tauri::command]
pub async fn library_preview(lib: State<'_, LibraryState>, path: String, material_kind: MaterialKind, material_id: String) -> CmdResult<Vec<Vec<u8>>> {
    let (_, _, show) = load(&lib, &path)?;
    tauri::async_runtime::spawn_blocking(move || omoiiro_runtime::material_preview::sample(&show, material_kind, &material_id))
        .await
        .map_err(|e| CommandError::new("preview", e.to_string()))?
        .map_err(|e| CommandError::new("preview", e))
}

/// Copy a library material into the edited show (one history entry). The
/// view's `outcome` names the material in the edited show.
#[tauri::command(async)]
pub fn library_import(
    ed: State<'_, EditorState>,
    app: State<'_, AppState>,
    ps: State<'_, Arc<PreviewState>>,
    lib: State<'_, LibraryState>,
    path: String,
    material_kind: MaterialKind,
    material_id: String,
) -> CmdResult<EditorView> {
    let (_, _, show) = load(&lib, &path)?;
    let command = import_command(&show, material_kind, &material_id)?;
    let reuse = {
        let g = ed.doc.lock().unwrap();
        let doc = g.as_ref().ok_or_else(|| CommandError::new("no_document", "open a show in the timeline first"))?;
        reused(doc.history.current(), &command)?
    };
    match reuse {
        Some(outcome) => editor::current_view(&ed, &app, Some(outcome)),
        None => editor::editor_apply(ed, app, ps, command, Snap::None),
    }
}

/// Change the registered list and save the terminal settings.
fn update_library(app: &AppState, change: impl FnOnce(&mut Vec<String>) -> CmdResult<()>) -> CmdResult<Vec<String>> {
    if app.settings_load_error.lock().unwrap().is_some() {
        return Err(CommandError::new("settings", "端末設定ファイルを読めなかったため、登録を保存できない（接続設定を保存し直してから登録する）"));
    }
    let dir = app.settings_dir.lock().unwrap().clone().ok_or_else(|| CommandError::new("settings", "settings directory is not ready"))?;
    let mut current = app.settings.lock().unwrap();
    let mut next = current.clone();
    change(&mut next.library_shows)?;
    let next = next.normalized();
    ts::save(&dir, &next).map_err(|e| CommandError::new("save", e))?;
    *current = next;
    Ok(current.library_shows.clone())
}

/// Register a show file as a library (it must load). Returns the list.
#[tauri::command(async)]
pub fn library_add(app: State<'_, AppState>, lib: State<'_, LibraryState>, path: String) -> CmdResult<Vec<String>> {
    let (p, _, _) = load(&lib, &path)?;
    let p = p.display().to_string();
    update_library(&app, |list| {
        if list.contains(&p) {
            return Ok(());
        }
        if list.len() >= ts::MAX_LIBRARY {
            return Err(CommandError::new("invalid", format!("ライブラリーは {} 件まで（先にどれかを外す）", ts::MAX_LIBRARY)));
        }
        list.push(p);
        Ok(())
    })
}

/// Unregister (the file itself is untouched). Returns the list.
#[tauri::command(async)]
pub fn library_remove(app: State<'_, AppState>, path: String) -> CmdResult<Vec<String>> {
    update_library(&app, |list| {
        list.retain(|p| *p != path);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        format!("{}/../../../{name}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn lists_old_files_and_reports_broken_ones() {
        let lib = LibraryState::default();
        let (p, stamp, show) = load(&lib, &fixture("fixtures/shows/compounds-0.1.3.show.json")).unwrap();
        let v = library_view(&p, stamp, &show);
        assert!(!v.compounds.is_empty() && !v.presets.is_empty(), "a 0.1.3 file loads through the migration");
        for c in &v.compounds {
            assert!(c.preset_ids.iter().all(|id| v.presets.iter().any(|p| &p.id == id)));
        }
        // Read again: the cached copy, the file is unchanged.
        let before = std::fs::read(&p).unwrap();
        let (_, _, again) = load(&lib, &fixture("fixtures/shows/compounds-0.1.3.show.json")).unwrap();
        assert!(Arc::ptr_eq(&show, &again));
        assert_eq!(std::fs::read(&p).unwrap(), before, "library files are never written");

        let dir = std::env::temp_dir().join(format!("omoiiro-library-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.show.json");
        std::fs::write(&bad, r#"{"schema_version":"0.1.1","id":"x"}"#).unwrap();
        let e = load(&lib, bad.to_str().unwrap()).unwrap_err();
        assert_eq!(e.code, "schema");
        assert!(!e.details.is_empty() && e.details.iter().all(|d| d.pointer.starts_with('/')));
        assert_eq!(load(&lib, dir.join("missing.show.json").to_str().unwrap()).unwrap_err().code, "io");
        assert_eq!(load(&lib, dir.join("x.preset.json").to_str().unwrap()).unwrap_err().code, "invalid");
        std::fs::remove_dir_all(&dir).ok();
        // At most four files stay parsed.
        for f in ["compounds-0.1.3", "frames-text-0.1.3", "particles-0.1.3", "triggered-0.1.3", "keyframes-0.1.3"] {
            load(&lib, &fixture(&format!("fixtures/shows/{f}.show.json"))).unwrap();
        }
        assert_eq!(lib.cache.lock().unwrap().len(), CACHE_FILES);
    }

    #[test]
    fn template_materials_import_once_into_a_new_show() {
        let lib = LibraryState::default();
        let (_, _, show) = load(&lib, &fixture("shows/omoi-iro-live-template.show.json")).unwrap();
        let mut h = edit::History::new_unsaved(edit::new_show("x"));
        for c in &show.compounds {
            let cmd = import_command(&show, MaterialKind::Compound, &c.id).unwrap();
            assert!(reused(h.current(), &cmd).unwrap().is_none());
            h.apply(&cmd, Snap::None).unwrap();
            assert!(reused(h.current(), &cmd).unwrap().is_some(), "{}: a second import adds nothing", c.id);
        }
        for p in &show.presets {
            let cmd = import_command(&show, MaterialKind::Preset, &p.id).unwrap();
            if reused(h.current(), &cmd).unwrap().is_none() {
                h.apply(&cmd, Snap::None).unwrap();
            }
            assert!(reused(h.current(), &cmd).unwrap().is_some(), "{}", p.id);
        }
        assert_eq!(h.current().compounds.len(), show.compounds.len());
        assert!(import_command(&show, MaterialKind::Preset, "missing").is_err());
    }
}
