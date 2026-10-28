//! `check-shows`: every `*.show.json` under a folder is loaded, saved in the
//! canonical form, loaded again (the second save must be byte for byte the
//! first) and rendered at the start, the middle and the last frame. Meant for
//! the fixtures, the examples and delivered show files (Codex `shows/`).
//! A synthetic check of the files: no device, no Live.

use omoiiro_core::render::Renderer;
use omoiiro_runtime::preview::show_length_us;
use std::path::{Path, PathBuf};

#[derive(clap::Args, Debug)]
pub struct CheckShowsArgs {
    /// Folders (searched recursively) or single show files.
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,
}

fn collect(p: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if p.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(p)?.filter_map(Result::ok).map(|e| e.path()).collect();
        entries.sort();
        for e in entries {
            collect(&e, out)?;
        }
    } else if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".show.json")) {
        out.push(p.to_path_buf());
    }
    Ok(())
}

/// One file: Ok(summary) or Err(reason).
fn check(path: &Path) -> Result<String, String> {
    let original = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let show = omoiiro_store::load_str(&original).map_err(|e| format!("load: {e}"))?;
    let saved = omoiiro_store::to_canonical_string(&show);
    let again = omoiiro_store::load_str(&saved).map_err(|e| format!("reload of the saved text: {e}"))?;
    if again != show {
        return Err("the reloaded show differs from the saved one".into());
    }
    let resaved = omoiiro_store::to_canonical_string(&again);
    if resaved != saved {
        return Err("save → load → save is not byte for byte the same".into());
    }
    let mut r = Renderer::new(&show).map_err(|i| format!("renderer: {}", i.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("; ")))?;
    let end = show_length_us(&show).unwrap_or(0).max(1);
    let mut lit = Vec::new();
    for t in [0, end / 2, end - 1] {
        let f = r.render(t, None);
        lit.push(f.leds.iter().filter(|c| c.r + c.g + c.b > 0.0).count());
    }
    // A file not in the canonical layout (or of an older version) must still
    // mean the same: compare the values, the version number aside.
    let canonical = if saved == original {
        "canonical"
    } else {
        let mut a: serde_json::Value = serde_json::from_str(&original).map_err(|e| e.to_string())?;
        let b: serde_json::Value = serde_json::from_str(&saved).map_err(|e| e.to_string())?;
        a["schema_version"] = b["schema_version"].clone();
        if a == b {
            "not canonical (same values once saved)"
        } else {
            "not canonical; a save changes values (numbers written differently or defaults made explicit)"
        }
    };
    Ok(format!("{} presets, {} layers, {} compounds, length {:.1} s, lit at 0 / middle / end {:?}, file {canonical}", show.presets.len(), show.layers.len(), show.compounds.len(), end as f64 / 1e6, lit))
}

pub fn run(args: CheckShowsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    for p in &args.paths {
        collect(p, &mut files)?;
    }
    if files.is_empty() {
        return Err("no *.show.json found".into());
    }
    let mut failed = 0;
    for f in &files {
        match check(f) {
            Ok(s) => println!("[PASS] {}: {s}", f.display()),
            Err(e) => {
                failed += 1;
                println!("[FAIL] {}: {e}", f.display());
            }
        }
    }
    println!("checked {} files, {failed} failed", files.len());
    if failed > 0 {
        return Err(format!("{failed} of {} shows failed", files.len()).into());
    }
    Ok(())
}