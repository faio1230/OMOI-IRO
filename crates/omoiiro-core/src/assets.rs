//! External frame assets (0.1.3, docs/14 §4-7; the format is defined in
//! prompts/codex-assets.md): `omoiiro.frames` (8 × 8 color frames) and
//! `omoiiro.bitmap` (one 8 × 8 on / off picture). Reading the file is the
//! store's job; this turns the parsed body into `builtin.frames` parameters.
//! Unknown extra fields are ignored so a newer writer can add some.

use crate::model::{FrameAnchor, FramesParams};
use crate::validation::MAX_FRAMES;
use serde::{Deserialize, Serialize};

pub const FRAMES_KIND: &str = "omoiiro.frames";
pub const BITMAP_KIND: &str = "omoiiro.bitmap";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum FramesAsset {
    /// Frames of 64 "#RRGGBB" colors, top row first, left to right.
    #[serde(rename = "omoiiro.frames")]
    Frames {
        version: u32,
        id: String,
        name: String,
        width: u32,
        height: u32,
        fps: f64,
        #[serde(rename = "loop")]
        looping: bool,
        frames: Vec<Vec<String>>,
    },
    /// Eight rows of eight characters, top first: `X` lit, `.` dark.
    #[serde(rename = "omoiiro.bitmap")]
    Bitmap { version: u32, id: String, name: String, width: u32, height: u32, rows: Vec<String> },
}

impl FramesAsset {
    pub fn id(&self) -> &str {
        match self {
            FramesAsset::Frames { id, .. } | FramesAsset::Bitmap { id, .. } => id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            FramesAsset::Frames { name, .. } | FramesAsset::Bitmap { name, .. } => name,
        }
    }

    /// The `builtin.frames` parameters, or what is wrong with the asset.
    /// A bitmap is one white-on-black frame kept on screen (`hold_last`).
    pub fn to_params(&self) -> Result<FramesParams, String> {
        let (version, width, height) = match self {
            FramesAsset::Frames { version, width, height, .. } | FramesAsset::Bitmap { version, width, height, .. } => (*version, *width, *height),
        };
        if version != 1 {
            return Err(format!("version {version} is not supported (1)"));
        }
        if (width, height) != (8, 8) {
            return Err(format!("{width} × {height}: only 8 × 8 is supported"));
        }
        match self {
            FramesAsset::Frames { fps, looping, frames, .. } => {
                if !(1..=MAX_FRAMES).contains(&frames.len()) {
                    return Err(format!("1..={MAX_FRAMES} frames (got {})", frames.len()));
                }
                if !(1.0..=30.0).contains(fps) {
                    return Err(format!("fps must be within 1..=30 (got {fps})"));
                }
                let mut out = Vec::with_capacity(frames.len());
                for (i, f) in frames.iter().enumerate() {
                    if f.len() != 64 {
                        return Err(format!("frame {i}: 64 colors expected (got {})", f.len()));
                    }
                    let mut hex = String::with_capacity(384);
                    for (j, c) in f.iter().enumerate() {
                        let ok = c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit());
                        if !ok {
                            return Err(format!("frame {i}, cell {j}: {c:?} is not #RRGGBB"));
                        }
                        hex.push_str(&c[1..].to_ascii_uppercase());
                    }
                    out.push(hex);
                }
                Ok(FramesParams { frames: out, fps: *fps, looping: *looping, hold_last: false, hold_ms: None, anchor: FrameAnchor::Board, color: None, opacity_key: None })
            }
            FramesAsset::Bitmap { rows, .. } => {
                if rows.len() != 8 || rows.iter().any(|r| r.chars().count() != 8 || !r.chars().all(|c| c == 'X' || c == '.')) {
                    return Err("8 rows of 8 characters, X or . each".into());
                }
                let hex = rows.iter().flat_map(|r| r.chars()).map(|c| if c == 'X' { "FFFFFF" } else { "000000" }).collect();
                Ok(FramesParams { frames: vec![hex], fps: 1.0, looping: false, hold_last: true, hold_ms: None, anchor: FrameAnchor::Board, color: None, opacity_key: None })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmaps_and_frames_become_frame_params() {
        let star: FramesAsset = serde_json::from_value(serde_json::json!({
            "kind": "omoiiro.bitmap", "version": 1, "id": "star", "name": "星", "width": 8, "height": 8, "extra": "ignored",
            "rows": ["...X....", "...X....", "XXXXXXX.", ".XXXXX..", "..XXX...", ".XX.XX..", "XX...XX.", "........"]
        }))
        .unwrap();
        let p = star.to_params().unwrap();
        assert_eq!((p.frames.len(), p.hold_last, p.looping), (1, true, false));
        assert_eq!(&p.frames[0][..12], "000000000000", "top row, left first");
        assert_eq!(&p.frames[0][18..24], "FFFFFF", "top row, 4th cell");
        let red = vec!["#ff0000".to_string(); 64];
        let anim = FramesAsset::Frames { version: 1, id: "a".into(), name: "a".into(), width: 8, height: 8, fps: 12.0, looping: true, frames: vec![red.clone(), red] };
        let p = anim.to_params().unwrap();
        assert_eq!((p.frames.len(), p.fps, p.looping, &p.frames[1][..6]), (2, 12.0, true, "FF0000"));
        let big = FramesAsset::Frames { version: 1, id: "b".into(), name: "b".into(), width: 16, height: 8, fps: 12.0, looping: true, frames: vec![] };
        assert!(big.to_params().unwrap_err().contains("8 × 8"));
    }
}
