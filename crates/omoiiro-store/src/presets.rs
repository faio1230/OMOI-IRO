//! Preset export / import (task C09).
//!
//! Export file (`schemas/preset.schema.json`):
//! `{"schema_version": "0.1.2", "kind": "omoiiro.preset", "preset": {...}}` (0.1.1 files are read).
//! The `preset` object is the show schema's `preset` definition. Import copies the preset into the show (the show keeps its
//! own copy; later library edits do not change it). ID conflicts are an
//! error unless the caller asks for a new ID.

use crate::json::{to_canonical_string, LoadError};
use crate::schema::validate_preset_file;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use omoiiro_core::model::{Preset, Show, SCHEMA_VERSION};
use omoiiro_core::validation::{validate, Issue};

pub const PRESET_KIND: &str = "omoiiro.preset";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetFile {
    pub schema_version: String,
    pub kind: String,
    pub preset: Preset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnConflict {
    /// Refuse when the show already has a preset with the same ID.
    Error,
    /// Give the imported preset a new ID (`<id>-2`, `<id>-3`, ...).
    Rename,
}

/// Export one preset of `show` to file text.
pub fn export_preset(show: &Show, preset_id: &str) -> Option<String> {
    let preset = show.presets.iter().find(|p| p.id == preset_id)?.clone();
    Some(to_canonical_string(&PresetFile { schema_version: SCHEMA_VERSION.into(), kind: PRESET_KIND.into(), preset }))
}

/// Parse and check a preset file.
pub fn parse_preset_file(text: &str) -> Result<Preset, LoadError> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| LoadError::Json { line: e.line(), column: e.column(), message: e.to_string() })?;
    match v.get("schema_version").and_then(Value::as_str) {
        // 0.1.1 / 0.1.2 preset files are valid current ones (additive changes).
        Some(v) if v == SCHEMA_VERSION || v == "0.1.1" || v == "0.1.2" => {}
        Some(other) => return Err(LoadError::UnsupportedVersion { found: other.to_string() }),
        None => return Err(LoadError::Schema(vec![Issue { pointer: "/schema_version".into(), message: "missing".into() }])),
    }
    let issues = validate_preset_file(&v);
    if !issues.is_empty() {
        return Err(LoadError::Schema(issues));
    }
    let file: PresetFile = serde_json::from_value(v).map_err(|e| LoadError::Model(e.to_string()))?;
    Ok(file.preset)
}

/// Import a preset into `show`. Returns the ID it was stored under. The
/// show is left unchanged on any error.
pub fn import_preset(show: &mut Show, text: &str, on_conflict: OnConflict) -> Result<String, LoadError> {
    let mut preset = parse_preset_file(text)?;
    if show.presets.iter().any(|p| p.id == preset.id) {
        match on_conflict {
            OnConflict::Error => {
                return Err(LoadError::Semantic(vec![Issue {
                    pointer: "/preset/id".into(),
                    message: format!("the show already has a preset {:?}", preset.id),
                }]))
            }
            OnConflict::Rename => {
                preset.id = omoiiro_core::edit::rename_on_conflict(&preset.id, show.presets.iter().map(|p| p.id.as_str()));
            }
        }
    }
    let mut next = show.clone();
    let id = preset.id.clone();
    next.presets.push(preset);
    let issues = validate(&next);
    if !issues.is_empty() {
        return Err(LoadError::Semantic(issues));
    }
    *show = next;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::load_str;

    fn demo() -> Show {
        load_str(include_str!("../../../examples/demo.show.json")).unwrap()
    }

    #[test]
    fn export_then_import_with_conflict_policies() {
        let mut show = demo();
        let text = export_preset(&show, "sweep-cyan").unwrap();
        assert!(text.starts_with("{\n  \"schema_version\": \"0.1.3\",\n  \"kind\": \"omoiiro.preset\","));
        // The exported file passes the preset schema.
        assert!(crate::schema::validate_preset_file(&serde_json::from_str(&text).unwrap()).is_empty());
        // Same ID already present.
        assert!(matches!(import_preset(&mut show, &text, OnConflict::Error), Err(LoadError::Semantic(_))));
        assert_eq!(show.presets.len(), 3);
        let id = import_preset(&mut show, &text, OnConflict::Rename).unwrap();
        assert_eq!(id, "sweep-cyan-2");
        let id3 = import_preset(&mut show, &text, OnConflict::Rename).unwrap();
        assert_eq!(id3, "sweep-cyan-3");
        assert_eq!(show.presets.len(), 5);
        // The copy is independent of the original.
        assert_eq!(show.presets[3].effect, show.presets[1].effect);
    }

    #[test]
    fn invalid_preset_files_are_refused_and_the_show_is_unchanged() {
        let mut show = demo();
        let before = show.clone();
        let bad_kind = r##"{"schema_version":"0.1.1","kind":"other","preset":{"id":"x","name":"x","effect":{"type":"builtin.solid","version":1,"params":{"color":"#000000"}}}}"##;
        assert!(import_preset(&mut show, bad_kind, OnConflict::Rename).is_err());
        let bad_color = r##"{"schema_version":"0.1.1","kind":"omoiiro.preset","preset":{"id":"x","name":"x","effect":{"type":"builtin.solid","version":1,"params":{"color":"blue"}}}}"##;
        match import_preset(&mut show, bad_color, OnConflict::Rename) {
            Err(LoadError::Schema(i)) => assert!(i[0].pointer.starts_with("/preset"), "{i:?}"),
            other => panic!("{other:?}"),
        }
        let future = r##"{"schema_version":"9.0.0","kind":"omoiiro.preset","preset":{}}"##;
        assert!(matches!(import_preset(&mut show, future, OnConflict::Rename), Err(LoadError::UnsupportedVersion { .. })));
        assert_eq!(show, before);
    }
}
