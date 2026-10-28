//! `render`: headless render of a show at any show time (task C10). Loads and
//! validates the show with `omoiiro-store`, renders with `omoiiro_core::render`.

use serde_json::json;
use omoiiro_core::render::Renderer;
use omoiiro_core::time::{BeatState, Timecode};
use std::path::PathBuf;

#[derive(clap::Args, Debug)]
pub struct RenderArgs {
    /// Show file (.show.json).
    #[arg(long)]
    pub show: PathBuf,
    /// Show time in microseconds (song start = 0).
    #[arg(long, allow_hyphen_values = true)]
    pub at: i64,
    /// Quarter-note beat position for beat-driven effects (Pulse). Without it the beat is unknown.
    #[arg(long, allow_hyphen_values = true)]
    pub beat: Option<f64>,
    /// Tempo in BPM reported with the beat (informational).
    #[arg(long, default_value_t = 120.0)]
    pub tempo: f64,
    /// Print JSON instead of the text grid.
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: RenderArgs) -> Result<(), Box<dyn std::error::Error>> {
    let show = omoiiro_store::load_path(&args.show)?;
    let mut renderer = Renderer::new(&show).map_err(|issues| {
        issues.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("; ")
    })?;
    let beat = args.beat.map(|b| BeatState::new(b, args.tempo));
    let frame = renderer.render(args.at, beat);
    let tc = &show.sync.timecode;
    let label = Timecode::parse(&tc.origin, tc.frame_rate)?.add_us(args.at).to_string();
    let rows = frame.rows_top_down();
    if args.json {
        let leds: Vec<String> = frame.leds.iter().map(|c| c.to_hex()).collect();
        let v = json!({
            "show_id": show.id,
            "t_us": args.at,
            "mtc_label": label,
            "frame_rate": tc.frame_rate.name(),
            "beat": args.beat,
            "tempo_bpm": args.beat.map(|_| args.tempo),
            "rows_top_down": rows,
            "leds_bottom_left": leds,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    println!("show: {} ({})", show.id, args.show.display());
    println!(
        "t = {} us  label = {label} ({} fps)  beat = {}",
        args.at,
        tc.frame_rate.name(),
        args.beat.map(|b| format!("{b} @ {} bpm", args.tempo)).unwrap_or_else(|| "unknown".into())
    );
    println!("rows top to bottom, x = 0..7 left to right (LED index = y * 8 + x, bottom-left origin):");
    for (i, row) in rows.iter().enumerate() {
        println!("  y={} {}", 7 - i, row.join(" "));
    }
    Ok(())
}
