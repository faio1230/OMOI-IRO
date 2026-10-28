//! `make-recording`: a recorded triggered clip from the pad events of a
//! `follow --out` record (0.1.3 stage 8), added through the same edit
//! command as the GUI's record button, then the show is saved (to `--out`,
//! or in place).

use omoiiro_core::edit::{apply, EditCommand, Snap};
use omoiiro_core::history::recording_from;
use omoiiro_core::render::PadEvent;
use omoiiro_runtime::diagnostics::record::read_jsonl;
use std::path::PathBuf;

#[derive(clap::Args, Debug)]
pub struct MakeRecordingArgs {
    /// JSONL record of `follow --out`.
    #[arg(long = "in")]
    pub input: PathBuf,
    /// Show file (.show.json).
    #[arg(long)]
    pub show: PathBuf,
    /// Triggered layer that gets the clip.
    #[arg(long)]
    pub layer: String,
    /// Show time the recording starts at (presses before it are left out).
    #[arg(long = "start-us", alias = "start_us", allow_hyphen_values = true)]
    pub start_us: i64,
    /// Preset drawn by the clip (default: the preset of the layer's first clip).
    #[arg(long)]
    pub preset: Option<String>,
    /// Length of one pass (default: the end of the last press).
    #[arg(long = "length-us")]
    pub length_us: Option<u64>,
    /// Repeat the recording over the clip.
    #[arg(long = "loop")]
    pub looping: bool,
    /// Write here instead of overwriting the show.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

pub fn run(args: MakeRecordingArgs) -> Result<(), Box<dyn std::error::Error>> {
    let show = omoiiro_store::load_path(&args.show)?;
    let mut events = Vec::new();
    for r in read_jsonl(&args.input)?.into_iter().filter(|r| r.source == "pad") {
        let Some(show_us) = r.payload["show_us"].as_i64() else { continue };
        let (x, y) = (r.payload["x"].as_u64().unwrap_or(0) as u8, r.payload["y"].as_u64().unwrap_or(0) as u8);
        let ev = match r.kind.as_str() {
            "press" => PadEvent::Press { x, y, velocity: r.payload["velocity"].as_u64().unwrap_or(127) as u8, t_us: r.t_mono_us },
            "release" => PadEvent::Release { x, y, t_us: r.t_mono_us },
            "release_all" => PadEvent::ReleaseAll { t_us: r.t_mono_us },
            "reset" => PadEvent::Reset,
            _ => continue,
        };
        events.push((show_us, ev));
    }
    let mut recording = recording_from(&events, args.start_us);
    recording.length_us = args.length_us;
    if recording.events.is_empty() {
        return Err(format!("{}: no presses at or after show time {} us", args.input.display(), args.start_us).into());
    }
    let preset_id = match args.preset {
        Some(p) => p,
        None => show
            .layers
            .iter()
            .find(|l| l.id == args.layer)
            .and_then(|l| l.content.clips()?.first().map(|c| c.preset_id.clone()))
            .filter(|p| !p.is_empty())
            .ok_or("the layer has no clip to take a preset from: give --preset")?,
    };
    let n = recording.events.len();
    let (show, out) = apply(&show, &EditCommand::AddRecordingClip { layer_id: args.layer, preset_id, start_us: args.start_us, recording: recording.clone() }, Snap::None)?;
    let clip_id = out.clip_id.unwrap_or_default();
    let show = if args.looping {
        apply(&show, &EditCommand::SetClipRecording { clip_id: clip_id.clone(), recording: Some(recording), looping: Some(true) }, Snap::None)?.0
    } else {
        show
    };
    let target = args.out.unwrap_or(args.show);
    omoiiro_store::save_atomic(&target, &show)?;
    println!("recorded clip {clip_id} ({n} presses) written to {}", target.display());
    Ok(())
}