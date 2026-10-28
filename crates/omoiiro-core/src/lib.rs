//! OMOI-IRO core (docs/03-modules.md, docs/04-show-data.md).
//!
//! * `model`: the show data model (serde DTO, Schema field order, unknown
//!   fields rejected).
//! * `time`: `ShowTimeUs` (song start = 0), frame rates as rationals,
//!   timecode labels with drop-frame rules, `BeatState`.
//! * `surface`: bottom-left-origin LED IDs, colors, frames.
//! * `timeline`: active clip search, local time, fades.
//! * `render`: effects (`builtin.*`), compositing, the headless
//!   `Renderer`.
//! * `validation`: semantic validation with JSON Pointers.
//! * `safety`: brightness ceiling, flashing-rate analysis and warnings.
//! * `particles`: burst / fountain / rocket / implode particles placed in
//!   closed form from their emission time (seekable).
//! * `shape`: signed distances and alpha of shapes on the grid (shared by
//!   shape, path and ripple).
//! * `edit`: edit commands (validated), snapping, Undo / Redo over snapshots.
//!
//! No midir, Tauri, OS clock, files or threads here. Rendering does no JSON
//! parsing or allocation and is deterministic.

pub mod assets;
pub mod edit;
pub mod font;
pub mod history;
pub mod model;
pub mod particles;
pub mod render;
pub mod safety;
pub mod shape;
pub mod surface;
pub mod time;
pub mod timeline;
pub mod validation;
