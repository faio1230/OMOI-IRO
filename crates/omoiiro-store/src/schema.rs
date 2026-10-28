//! JSON Schema validation against the embedded `schemas/show.schema.json`
//! and `schemas/preset.schema.json`. Validators are built once. The preset
//! schema references the show schema through a registry keyed by the show
//! schema's `$id`; nothing is fetched (jsonschema is pinned to 0.60.2
//! without default features).

use jsonschema::{Registry, Validator};
use serde_json::{json, Value};
use omoiiro_core::validation::Issue;
use std::sync::OnceLock;

/// The Schema file this build validates against (the source of truth).
pub const SHOW_SCHEMA: &str = include_str!("../../../schemas/show.schema.json");
pub const PRESET_SCHEMA: &str = include_str!("../../../schemas/preset.schema.json");

fn schema_value() -> &'static Value {
    static V: OnceLock<Value> = OnceLock::new();
    V.get_or_init(|| serde_json::from_str(SHOW_SCHEMA).expect("embedded show schema is valid JSON"))
}

fn show_validator() -> &'static Validator {
    static V: OnceLock<Validator> = OnceLock::new();
    V.get_or_init(|| jsonschema::draft202012::new(schema_value()).expect("embedded show schema compiles"))
}

/// Validator for one definition of the show Schema (e.g. `preset`), sharing its `$defs`.
fn def_validator(def: &str) -> Validator {
    let s = schema_value();
    let sub = json!({
        "$schema": s["$schema"],
        "$defs": s["$defs"],
        "$ref": format!("#/$defs/{def}"),
    });
    jsonschema::draft202012::new(&sub).expect("embedded schema definition compiles")
}

fn issues(v: &Validator, instance: &Value) -> Vec<Issue> {
    v.iter_errors(instance)
        .map(|e| {
            let p = e.instance_path().as_str();
            Issue { pointer: if p.is_empty() { "/".to_string() } else { p.to_string() }, message: e.to_string() }
        })
        .collect()
}

/// Schema issues of a whole show document (empty = valid).
pub fn validate_show(instance: &Value) -> Vec<Issue> {
    issues(show_validator(), instance)
}

/// Schema issues of a single preset object.
pub fn validate_preset(instance: &Value) -> Vec<Issue> {
    static V: OnceLock<Validator> = OnceLock::new();
    issues(V.get_or_init(|| def_validator("preset")), instance)
}

fn preset_file_validator() -> &'static Validator {
    static V: OnceLock<Validator> = OnceLock::new();
    V.get_or_init(|| {
        let show = schema_value();
        let id = show["$id"].as_str().expect("show schema has $id");
        let registry = Registry::new().add(id, show.clone()).expect("register show schema").prepare().expect("prepare registry");
        let preset: Value = serde_json::from_str(PRESET_SCHEMA).expect("embedded preset schema is valid JSON");
        jsonschema::options().with_registry(&registry).build(&preset).expect("embedded preset schema compiles")
    })
}

/// Schema issues of a preset file (`schemas/preset.schema.json`).
pub fn validate_preset_file(instance: &Value) -> Vec<Issue> {
    issues(preset_file_validator(), instance)
}
