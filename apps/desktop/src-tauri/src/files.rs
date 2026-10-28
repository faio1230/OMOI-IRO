//! File paths chosen in the UI (dialog or typed) are checked here, in Rust,
//! before anything is read or written (U07).
//!
//! * Shows: `*.show.json`, at most `omoiiro_store::MAX_FILE_BYTES`.
//! * Presets: `*.preset.json`, at most `MAX_PRESET_BYTES`.
//! * Frame assets (`omoiiro.frames` / `omoiiro.bitmap`): `*.json`, at most
//!   `omoiiro_store::assets::MAX_ASSET_BYTES`.
//! * Reading needs an existing regular file; writing needs an existing
//!   parent directory. The dialog plugin only returns a path; it grants no
//!   file access of its own.

use crate::dto::{CommandError, IssueDto};
use omoiiro_store::LoadError;
use std::path::{Path, PathBuf};

pub const SHOW_EXT: &str = ".show.json";
pub const PRESET_EXT: &str = ".preset.json";
pub const MAX_PRESET_BYTES: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Show,
    Preset,
    FramesAsset,
}

impl Kind {
    fn ext(self) -> &'static str {
        match self {
            Kind::Show => SHOW_EXT,
            Kind::Preset => PRESET_EXT,
            Kind::FramesAsset => ".json",
        }
    }

    fn max_bytes(self) -> u64 {
        match self {
            Kind::Show => omoiiro_store::json::MAX_FILE_BYTES as u64,
            Kind::Preset => MAX_PRESET_BYTES,
            Kind::FramesAsset => omoiiro_store::assets::MAX_ASSET_BYTES as u64,
        }
    }
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_lowercase().ends_with(ext) && n.len() > ext.len())
}

/// A path to read: right extension, existing regular file, within the size limit.
pub fn for_read(raw: &str, kind: Kind) -> Result<PathBuf, CommandError> {
    let p = PathBuf::from(raw.trim());
    if p.as_os_str().is_empty() {
        return Err(CommandError::new("invalid", "choose a file"));
    }
    if !has_ext(&p, kind.ext()) {
        return Err(CommandError::new("invalid", format!("{} is not a {} file", p.display(), kind.ext())));
    }
    let meta = std::fs::metadata(&p).map_err(|e| CommandError::new("io", format!("{}: {e}", p.display())))?;
    if !meta.is_file() {
        return Err(CommandError::new("invalid", format!("{} is not a file", p.display())));
    }
    if meta.len() > kind.max_bytes() {
        return Err(CommandError::new("invalid", format!("{} is {} bytes (limit {})", p.display(), meta.len(), kind.max_bytes())));
    }
    Ok(p)
}

/// A path to write: right extension (added if missing), existing parent directory.
pub fn for_write(raw: &str, kind: Kind) -> Result<PathBuf, CommandError> {
    let mut p = PathBuf::from(raw.trim());
    if p.as_os_str().is_empty() {
        return Err(CommandError::new("invalid", "choose a file"));
    }
    if !has_ext(&p, kind.ext()) {
        // "name" or "name.json" from a save dialog becomes "name.show.json".
        let stem = p.file_name().and_then(|n| n.to_str()).unwrap_or_default().trim_end_matches(".json").to_string();
        if stem.is_empty() {
            return Err(CommandError::new("invalid", format!("{} has no file name", p.display())));
        }
        p.set_file_name(format!("{stem}{}", kind.ext()));
    }
    let parent = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    if !parent.is_dir() {
        return Err(CommandError::new("invalid", format!("folder {} does not exist", parent.display())));
    }
    if p.is_dir() {
        return Err(CommandError::new("invalid", format!("{} is a folder", p.display())));
    }
    Ok(p)
}

/// A load error with JSON Pointers for the UI.
pub fn load_error(path: &Path, e: LoadError) -> CommandError {
    let details = match &e {
        LoadError::Schema(v) | LoadError::Semantic(v) => v.iter().map(|i| IssueDto { pointer: i.pointer.clone(), message: i.message.clone() }).collect(),
        _ => Vec::new(),
    };
    let code = match &e {
        LoadError::UnsupportedVersion { .. } => "version",
        LoadError::Schema(_) => "schema",
        LoadError::Semantic(_) => "semantic",
        LoadError::Json { .. } => "json",
        _ => "show",
    };
    let mut err = CommandError::new(code, format!("{} を開けない（今のショーはそのまま）：{e}", path.display()));
    err.details = details;
    err
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_sizes_and_parents() {
        let dir = std::env::temp_dir().join(format!("omoiiro-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let show = dir.join("a.show.json");
        std::fs::write(&show, "{}").unwrap();
        assert_eq!(for_read(show.to_str().unwrap(), Kind::Show).unwrap(), show);
        assert_eq!(for_read(show.to_str().unwrap(), Kind::Preset).unwrap_err().code, "invalid");
        assert_eq!(for_read(dir.join("missing.show.json").to_str().unwrap(), Kind::Show).unwrap_err().code, "io");
        let big = dir.join("big.preset.json");
        std::fs::write(&big, vec![b' '; MAX_PRESET_BYTES as usize + 1]).unwrap();
        assert!(for_read(big.to_str().unwrap(), Kind::Preset).unwrap_err().message.contains("limit"));
        assert_eq!(for_read(".show.json", Kind::Show).unwrap_err().code, "invalid", "no name");

        assert_eq!(for_write(dir.join("b").to_str().unwrap(), Kind::Show).unwrap(), dir.join("b.show.json"));
        assert_eq!(for_write(dir.join("b.json").to_str().unwrap(), Kind::Show).unwrap(), dir.join("b.show.json"));
        assert_eq!(for_write(dir.join("c.PRESET.JSON").to_str().unwrap(), Kind::Preset).unwrap(), dir.join("c.PRESET.JSON"));
        assert!(for_write(dir.join("nope").join("x.show.json").to_str().unwrap(), Kind::Show).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_errors_carry_pointers_and_keep_the_show() {
        let bad = r#"{"schema_version":"0.1.1","id":"x"}"#;
        let e = load_error(Path::new("x.show.json"), omoiiro_store::load_str(bad).unwrap_err());
        assert_eq!(e.code, "schema");
        assert!(!e.details.is_empty() && e.details.iter().all(|d| d.pointer.starts_with('/')));
        assert!(e.message.contains("今のショーはそのまま"));
        let future = r#"{"schema_version":"9.0.0"}"#;
        assert_eq!(load_error(Path::new("x.show.json"), omoiiro_store::load_str(future).unwrap_err()).code, "version");
    }
}
