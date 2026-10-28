//! Show file load and save.

use crate::schema::validate_show;
use serde::Serialize;
use serde_json::Value;
use omoiiro_core::model::{Show, MIGRATABLE_VERSIONS, SCHEMA_VERSION};
use omoiiro_core::validation::{validate, Issue};
use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Largest show file accepted (resource limit; the Schema caps counts, this caps bytes).
pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    TooLarge { bytes: usize },
    Json { line: usize, column: usize, message: String },
    /// `schema_version` other than the supported one. Never auto-converted.
    UnsupportedVersion { found: String },
    Schema(Vec<Issue>),
    /// The Schema accepted the document but the DTO did not (should not happen
    /// while Schema and DTO agree; kept as a guard).
    Model(String),
    Semantic(Vec<Issue>),
}

impl LoadError {
    /// Issues with JSON Pointers, when the error has them.
    pub fn issues(&self) -> &[Issue] {
        match self {
            LoadError::Schema(v) | LoadError::Semantic(v) => v,
            _ => &[],
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "read failed: {e}"),
            LoadError::TooLarge { bytes } => write!(f, "file too large ({bytes} bytes, limit {MAX_FILE_BYTES})"),
            LoadError::Json { line, column, message } => write!(f, "invalid JSON at line {line}, column {column}: {message}"),
            LoadError::UnsupportedVersion { found } => write!(f, "unsupported schema_version {found:?} (this build reads {SCHEMA_VERSION})"),
            LoadError::Schema(v) => write!(f, "schema validation failed: {}", join(v)),
            LoadError::Model(m) => write!(f, "model conversion failed: {m}"),
            LoadError::Semantic(v) => write!(f, "semantic validation failed: {}", join(v)),
        }
    }
}

impl std::error::Error for LoadError {}

fn join(v: &[Issue]) -> String {
    v.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("; ")
}

/// Explicit migration to the current version. 0.1.0 -> 0.1.1 is additive
/// (new optional `mtc_output`, `source` gains `live_bridge`) and so is
/// 0.1.1 -> 0.1.2 (new effect types), so only the version changes;
/// `source: "mtc"` is kept as written. The source file is never rewritten
/// until the user saves.
fn migrate(mut value: Value) -> Value {
    if value.get("schema_version").and_then(Value::as_str).is_some_and(|v| MIGRATABLE_VERSIONS.contains(&v)) {
        value["schema_version"] = Value::String(SCHEMA_VERSION.to_string());
    }
    value
}

/// Parse and fully validate a show document.
pub fn load_str(text: &str) -> Result<Show, LoadError> {
    if text.len() > MAX_FILE_BYTES {
        return Err(LoadError::TooLarge { bytes: text.len() });
    }
    let value: Value = serde_json::from_str(text)
        .map_err(|e| LoadError::Json { line: e.line(), column: e.column(), message: e.to_string() })?;
    match value.get("schema_version").and_then(Value::as_str) {
        Some(v) if v != SCHEMA_VERSION && !MIGRATABLE_VERSIONS.contains(&v) => {
            return Err(LoadError::UnsupportedVersion { found: v.to_string() })
        }
        _ => {}
    }
    // Migrate first, then validate against the current schema (0.1.0 documents
    // are valid 0.1.1 documents once the version is updated).
    let value = migrate(value);
    let schema_issues = validate_show(&value);
    if !schema_issues.is_empty() {
        return Err(LoadError::Schema(schema_issues));
    }
    let show: Show = serde_json::from_value(value).map_err(|e| LoadError::Model(e.to_string()))?;
    let semantic = validate(&show);
    if !semantic.is_empty() {
        return Err(LoadError::Semantic(semantic));
    }
    Ok(show)
}

pub fn load_path(path: &Path) -> Result<Show, LoadError> {
    let bytes = fs::read(path).map_err(LoadError::Io)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(LoadError::TooLarge { bytes: bytes.len() });
    }
    let text = String::from_utf8(bytes).map_err(|e| LoadError::Json { line: 0, column: 0, message: format!("not UTF-8: {e}") })?;
    load_str(&text)
}

/// Canonical text: 2-space indent, LF, trailing newline, Schema key order.
pub fn to_canonical_string<T: Serialize>(value: &T) -> String {
    let mut out = serde_json::to_string_pretty(value).expect("model serializes");
    out.push('\n');
    out
}

fn temp_path_for(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "show".into());
    path.with_file_name(format!(".{name}.tmp-{}", std::process::id()))
}

/// Save atomically: temp file in the same directory, flush + sync, rename
/// over `path`. On failure the original file is left untouched.
pub fn save_atomic(path: &Path, show: &Show) -> std::io::Result<()> {
    let text = to_canonical_string(show);
    let tmp = temp_path_for(path);
    let result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::validate_show;
    use serde_json::json;

    const DEMO: &str = include_str!("../../../examples/demo.show.json");

    /// The 0.1.1 demo as the current version writes it (migration changes only the version).
    fn demo_current() -> String {
        DEMO.replace("\"schema_version\": \"0.1.1\"", &format!("\"schema_version\": \"{SCHEMA_VERSION}\""))
    }

    fn demo_value() -> Value {
        serde_json::from_str(DEMO).unwrap()
    }

    fn schema_pointers(v: &Value) -> Vec<String> {
        match load_str(&v.to_string()) {
            Err(LoadError::Schema(issues)) => issues.into_iter().map(|i| i.pointer).collect(),
            other => panic!("expected a schema error, got {other:?}"),
        }
    }

    #[test]
    fn demo_loads_and_unchanged_save_is_byte_identical() {
        let show = load_str(DEMO).unwrap();
        assert_eq!(to_canonical_string(&show), demo_current());
        // A current-version file saves byte for byte.
        assert_eq!(to_canonical_string(&load_str(&demo_current()).unwrap()), demo_current());
    }

    #[test]
    fn version_0_1_2_fixtures_load_unchanged_as_0_1_3() {
        // Every 0.1.2 fixture loads, saves as the current version with nothing
        // else changed, and renders the same before and after.
        for (name, text) in [
            ("effects", include_str!("../../../fixtures/shows/effects-0.1.2.show.json")),
            ("ripple", include_str!("../../../fixtures/shows/ripple-0.1.2.show.json")),
            ("rain-wave-orb", include_str!("../../../fixtures/shows/rain-wave-orb-0.1.2.show.json")),
            ("d-effects", include_str!("../../../fixtures/shows/d-effects-0.1.2.show.json")),
        ] {
            let show = load_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(show.schema_version, SCHEMA_VERSION, "{name}");
            // Same values (some fixtures are not in the canonical layout).
            let saved = to_canonical_string(&show);
            let mut expected: Value = serde_json::from_str(text).unwrap();
            expected["schema_version"] = json!(SCHEMA_VERSION);
            assert_eq!(serde_json::from_str::<Value>(&saved).unwrap(), expected, "{name}");
            let again = load_str(&saved).unwrap();
            let mut a = omoiiro_core::render::Renderer::new(&show).unwrap();
            let mut b = omoiiro_core::render::Renderer::new(&again).unwrap();
            for t in [0, 1_500_000, 9_300_000, 23_000_000] {
                assert_eq!(a.render(t, None), b.render(t, None), "{name} at {t}");
            }
        }
    }

    #[test]
    fn triggered_layer_round_trips_and_needs_its_interval() {
        let mut v = demo_value();
        v["layers"][0]["content"] = json!({"kind": "triggered", "clips": [], "trigger": {"on": "interval"}});
        assert!(schema_pointers(&v).iter().any(|p| p.starts_with("/layers/0/content")), "interval without interval_ms");
        v["layers"][0]["content"]["trigger"]["interval_ms"] = json!(800);
        v["layers"][0]["content"]["trigger"]["origin_x"] = json!(2.5);
        let show = load_str(&v.to_string()).unwrap();
        let back: Value = serde_json::to_value(&show).unwrap();
        assert_eq!(back["layers"][0]["content"], v["layers"][0]["content"]);
    }

    #[test]
    fn serialized_model_passes_the_schema() {
        let show = load_str(DEMO).unwrap();
        let v: Value = serde_json::to_value(&show).unwrap();
        assert!(validate_show(&v).is_empty());
    }

    #[test]
    fn new_0_1_2_presets_round_trip() {
        let mut show = load_str(DEMO).unwrap();
        for (id, t) in [("tg", "builtin.tap_glow"), ("x", "builtin.cross"), ("rg", "builtin.regions"), ("ra", "builtin.rain"), ("w", "builtin.wave"), ("o", "builtin.orb"), ("r", "builtin.ripple"), ("s", "builtin.strobe"), ("n", "builtin.noise")] {
            let effect = omoiiro_core::edit::default_effect(t).unwrap();
            show.presets.push(omoiiro_core::model::Preset { id: id.into(), name: id.into(), effect });
        }
        let text = to_canonical_string(&show);
        assert!(validate_show(&serde_json::from_str(&text).unwrap()).is_empty());
        assert_eq!(load_str(&text).unwrap(), show);
        // Out of range values are caught by the schema with a pointer.
        let mut v: Value = serde_json::from_str(&text).unwrap();
        let n = v["presets"].as_array().unwrap().len();
        v["presets"][n - 2]["effect"]["params"]["duty"] = json!(0.99);
        v["presets"][n - 1]["effect"]["params"]["rate_hz"] = json!(31);
        let p = schema_pointers(&v);
        assert!(p.contains(&format!("/presets/{}/effect/params/duty", n - 2)), "{p:?}");
        assert!(p.contains(&format!("/presets/{}/effect/params/rate_hz", n - 1)), "{p:?}");
        // An interval ripple needs its origin and interval (schema), and round-trips with them.
        let ripple = v["presets"].as_array().unwrap().iter().position(|p| p["effect"]["type"] == "builtin.ripple").unwrap();
        let mut w: Value = serde_json::from_str(&text).unwrap();
        w["presets"][ripple]["effect"]["params"]["trigger"] = json!("interval");
        assert!(!schema_pointers(&w).is_empty(), "interval without origin / interval_ms");
        let p = &mut w["presets"][ripple]["effect"]["params"];
        p["origin_x"] = json!(3.5);
        p["origin_y"] = json!(3.5);
        p["interval_ms"] = json!(800);
        p["shape"] = json!("diamond");
        let text = w.to_string();
        let show = load_str(&text).unwrap();
        assert_eq!(serde_json::to_value(&show).unwrap()["presets"][ripple], w["presets"][ripple]);
    }

    /// The eight invalid cases of the design review (README "設計成果物の確認").
    #[test]
    fn eight_invalid_kinds_are_rejected_by_the_schema() {
        let mut v = demo_value();
        v["layers"][0]["extra"] = json!(1); // unknown key
        assert!(schema_pointers(&v).contains(&"/layers/0".to_string()));

        let mut v = demo_value();
        v["presets"][0]["effect"]["params"]["color"] = json!("#12345"); // bad color
        assert!(!schema_pointers(&v).is_empty());

        let mut v = demo_value();
        v["layers"][0]["content"]["clips"][0]["duration_us"] = json!(0); // zero duration
        assert!(!schema_pointers(&v).is_empty());

        let mut v = demo_value();
        v["presets"][0]["effect"]["type"] = json!("builtin.sparkle"); // unknown effect
        assert!(schema_pointers(&v).iter().any(|p| p.starts_with("/presets/0/effect")));

        let mut v = demo_value();
        v["layers"][1]["opacity"] = json!(1.5); // opacity out of range
        assert!(schema_pointers(&v).contains(&"/layers/1/opacity".to_string()));

        let mut v = demo_value();
        v["layers"][2]["content"]["clips"] = json!([]); // ambiguous content (both shapes)
        assert!(schema_pointers(&v).iter().any(|p| p.starts_with("/layers/2/content")));

        let mut v = demo_value();
        v["layers"][0]["content"]["clips"][0]["start_us"] = json!(9_007_199_254_740_992u64); // beyond safe integer
        // 0.1.1 uses if/then instead of oneOf, so the pointer reaches the clip field.
        assert!(schema_pointers(&v).contains(&"/layers/0/content/clips/0/start_us".to_string()));

        let mut v = demo_value();
        v["presets"][1]["effect"]["params"]["width"] = json!(9); // deep pointer inside an effect
        assert!(schema_pointers(&v).contains(&"/presets/1/effect/params/width".to_string()));

        let mut v = demo_value();
        v["schema_version"] = json!("0.2.0"); // unsupported version
        assert!(matches!(load_str(&v.to_string()), Err(LoadError::UnsupportedVersion { .. })));
    }

    #[test]
    fn version_0_1_0_is_migrated_on_load() {
        let old = include_str!("../../../fixtures/shows/demo-0.1.0.show.json");
        let show = load_str(old).unwrap();
        assert_eq!(show.schema_version, SCHEMA_VERSION);
        assert_eq!(show.sync.timecode.source, omoiiro_core::model::TimecodeSource::Mtc);
        assert_eq!(show.sync.mtc_output, None);
        // Saving writes the current version; nothing else changes.
        let saved = to_canonical_string(&show);
        assert_eq!(saved, old.replace("\"schema_version\": \"0.1.0\"", &format!("\"schema_version\": \"{SCHEMA_VERSION}\"")));
    }

    #[test]
    fn semantic_errors_after_the_schema() {
        let mut v = demo_value();
        v["layers"][1]["content"]["clips"][0]["preset_id"] = json!("missing");
        match load_str(&v.to_string()) {
            Err(LoadError::Semantic(i)) => assert_eq!(i[0].pointer, "/layers/1/content/clips/0/preset_id"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn json_syntax_errors_have_line_and_column() {
        match load_str("{\n  \"id\": ,\n}") {
            Err(LoadError::Json { line, column, .. }) => assert_eq!((line, column), (2, 9)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn too_large_is_refused_before_parsing() {
        let big = " ".repeat(MAX_FILE_BYTES + 1);
        assert!(matches!(load_str(&big), Err(LoadError::TooLarge { .. })));
    }

    #[test]
    fn atomic_save_round_trips_and_failure_keeps_the_original() {
        let dir = std::env::temp_dir().join(format!("omoiiro-store-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.show.json");
        let mut show = load_str(DEMO).unwrap();
        save_atomic(&path, &show).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), demo_current());
        // Save again unchanged: identical bytes, no temp file left behind.
        save_atomic(&path, &show).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), demo_current());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        // Edit, save, reload.
        show.name = "変更後".into();
        save_atomic(&path, &show).unwrap();
        assert_eq!(load_path(&path).unwrap().name, "変更後");
        // A save into a missing directory fails and leaves nothing.
        let bad = dir.join("missing").join("b.show.json");
        assert!(save_atomic(&bad, &show).is_err());
        assert_eq!(load_path(&path).unwrap().name, "変更後");
        fs::remove_dir_all(&dir).ok();
    }
}
