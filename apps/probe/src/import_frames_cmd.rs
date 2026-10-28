//! `import-frames`: add an external frame asset (`omoiiro.frames` /
//! `omoiiro.bitmap`) to a show as a `builtin.frames` preset, through the
//! same edit command as the GUI, and save the show (to `--out`, or in place).

use omoiiro_core::edit::{apply, EditCommand, OnConflict, Snap};
use std::path::PathBuf;

#[derive(clap::Args, Debug)]
pub struct ImportFramesArgs {
    /// Show file (.show.json).
    #[arg(long)]
    pub show: PathBuf,
    /// Asset file (kind omoiiro.frames or omoiiro.bitmap).
    #[arg(long)]
    pub file: PathBuf,
    /// Give the preset a new ID when the asset's ID is taken (default: refuse).
    #[arg(long)]
    pub rename: bool,
    /// Write here instead of overwriting the show.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

pub fn run(args: ImportFramesArgs) -> Result<(), Box<dyn std::error::Error>> {
    let show = omoiiro_store::load_path(&args.show)?;
    let asset = omoiiro_store::assets::read_frames_asset(&args.file).map_err(|e| format!("{}: {e}", args.file.display()))?;
    let on_conflict = if args.rename { OnConflict::Rename } else { OnConflict::Error };
    let (show, out) = apply(&show, &EditCommand::ImportFrames { asset, on_conflict }, Snap::None)?;
    let target = args.out.unwrap_or(args.show);
    omoiiro_store::save_atomic(&target, &show)?;
    println!("imported preset {} into {}", out.preset_id.unwrap_or_default(), target.display());
    Ok(())
}