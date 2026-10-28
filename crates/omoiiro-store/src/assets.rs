//! External frame assets (`omoiiro.frames` / `omoiiro.bitmap`, prompts/codex-assets.md):
//! read and parsed here, turned into a preset by `EditCommand::ImportFrames`.

use crate::json::LoadError;
use omoiiro_core::assets::{FramesAsset, BITMAP_KIND, FRAMES_KIND};
use omoiiro_core::validation::Issue;
use serde_json::Value;
use std::path::Path;

/// Largest asset file read (240 frames of 64 colors fit in well under 1 MB).
pub const MAX_ASSET_BYTES: usize = 8 * 1024 * 1024;

/// Parse an asset file's text.
pub fn parse_frames_asset(text: &str) -> Result<FramesAsset, LoadError> {
    if text.len() > MAX_ASSET_BYTES {
        return Err(LoadError::TooLarge { bytes: text.len() });
    }
    let v: Value = serde_json::from_str(text).map_err(|e| LoadError::Json { line: e.line(), column: e.column(), message: e.to_string() })?;
    match v.get("kind").and_then(Value::as_str) {
        Some(k) if k == FRAMES_KIND || k == BITMAP_KIND => {}
        other => return Err(LoadError::Schema(vec![Issue { pointer: "/kind".into(), message: format!("expected {FRAMES_KIND:?} or {BITMAP_KIND:?} (got {other:?})") }])),
    }
    let asset: FramesAsset = serde_json::from_value(v).map_err(|e| LoadError::Model(e.to_string()))?;
    // Report shape problems (size, colors, rows) here, with the asset's words.
    asset.to_params().map_err(|m| LoadError::Schema(vec![Issue { pointer: String::new(), message: m }]))?;
    Ok(asset)
}

/// Read and parse an asset file (at most `MAX_ASSET_BYTES`).
pub fn read_frames_asset(path: &Path) -> Result<FramesAsset, LoadError> {
    let bytes = std::fs::read(path).map_err(LoadError::Io)?;
    if bytes.len() > MAX_ASSET_BYTES {
        return Err(LoadError::TooLarge { bytes: bytes.len() });
    }
    let text = String::from_utf8(bytes).map_err(|e| LoadError::Json { line: 0, column: 0, message: format!("not UTF-8: {e}") })?;
    parse_frames_asset(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_parse_or_explain() {
        let ok = r#"{"kind": "omoiiro.bitmap", "version": 1, "id": "s", "name": "s", "width": 8, "height": 8,
            "rows": ["X.......", "........", "........", "........", "........", "........", "........", "......X."]}"#;
        assert!(matches!(parse_frames_asset(ok), Ok(FramesAsset::Bitmap { .. })));
        assert!(matches!(parse_frames_asset(r#"{"kind": "omoiiro.preset"}"#), Err(LoadError::Schema(_))));
        let short = ok.replace("\"......X.\"", "\"......X\"");
        assert!(matches!(parse_frames_asset(&short), Err(LoadError::Schema(i)) if i[0].message.contains("8 rows")));
        assert!(matches!(parse_frames_asset(&" ".repeat(MAX_ASSET_BYTES + 1)), Err(LoadError::TooLarge { .. })));
    }
}