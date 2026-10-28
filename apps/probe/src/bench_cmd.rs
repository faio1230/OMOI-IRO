//! `bench-render`: NFR03 render-time measurement on this machine and build
//! (`omoiiro_runtime::diagnostics::nfr03`). Not a device test.

use omoiiro_runtime::diagnostics::nfr03;

#[derive(clap::Args, Debug)]
pub struct BenchArgs {
    /// Number of consecutive 30 fps frames to evaluate.
    #[arg(long, default_value_t = 9000)]
    pub frames: u64,
    /// Print JSON only.
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: BenchArgs) -> Result<(), Box<dyn std::error::Error>> {
    let minutes = (args.frames / (30 * 60) + 1) as u32;
    let show = nfr03::load_show(minutes);
    let h = nfr03::measure(&show, args.frames);
    let build = if cfg!(debug_assertions) { "debug" } else { "release" };
    let report = nfr03::report(&h, build);
    if args.json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }
    Ok(())
}
