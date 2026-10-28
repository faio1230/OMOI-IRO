//! OMOI-IRO store (docs/04-show-data.md, tasks C02, C08, C09).
//!
//! Load: size limit -> JSON parse -> version check -> migration (0.1.0 ->
//! 0.1.1: version only) -> JSON Schema (Draft 2020-12,
//! `schemas/show.schema.json` embedded) -> serde DTO (unknown fields
//! rejected) -> semantic validation
//! (`omoiiro_core::validation`). Every rejection carries a JSON Pointer or a
//! line / column. A failed load never replaces the caller's current show.
//!
//! Save: canonical form (2-space indent, LF, trailing newline, Schema key
//! order, integral numbers as integers), written to a temporary file in the
//! same directory, flushed, then renamed over the target. An unchanged show
//! saves byte for byte.
//!
//! Presets: a single preset exports to its own file
//! (`schemas/preset.schema.json`: `{"schema_version", "kind": "omoiiro.preset", "preset"}`) and imports into a
//! show by copy, with an explicit policy for ID conflicts.
//!
//! Assets: external `omoiiro.frames` / `omoiiro.bitmap` files (0.1.3) are
//! read (at most 8 MB) and parsed for `EditCommand::ImportFrames`.

pub mod assets;
pub mod json;
pub mod presets;
pub mod schema;

pub use json::{load_path, load_str, save_atomic, to_canonical_string, LoadError, MAX_FILE_BYTES};
