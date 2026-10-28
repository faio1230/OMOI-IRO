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
    /// The particle load instead (8 layers of capped fountains, 256 particles each).
    #[arg(long)]
    pub particles: bool,
    /// The compound load instead: 8 layers × 4 inner layers (with
    /// `--particles`: every inner layer a capped fountain).
    #[arg(long)]
    pub compounds: bool,
}

pub fn run(args: BenchArgs) -> Result<(), Box<dyn std::error::Error>> {
    let build = if cfg!(debug_assertions) { "debug" } else { "release" };
    let report = if args.compounds {
        let h = nfr03::measure(&nfr03::load_show_compounds(args.particles), args.frames);
        let load = if args.particles {
            "64 pads, 8 layers x 4 inner compound layers, every inner layer a capped fountain (256 per layer, 768 per show drawn), loop, tint on 4"
        } else {
            "64 pads, 8 layers x 4 inner compound layers, overlapping 2 s clips with fades over chase/pulse/noise/rain/orb, over/add/max, loop, tint on 4"
        };
        nfr03::report_load(&h, build, load)
    } else if args.particles {
        let h = nfr03::measure(&nfr03::load_show_particles(), args.frames);
        nfr03::report_load(&h, build, "64 pads, 8 continuous particle layers, fountain 64/s x 6 s (256 per layer, 768 per show drawn), size 2, tail 4, bounce, over/add/max")
    } else {
        let minutes = (args.frames / (30 * 60) + 1) as u32;
        let h = nfr03::measure(&nfr03::load_show(minutes), args.frames);
        nfr03::report(&h, build)
    };
    if args.json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }
    Ok(())
}
