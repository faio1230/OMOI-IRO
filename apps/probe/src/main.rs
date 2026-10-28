use clap::{Parser, Subcommand};
use omoiiro_probe::{bench_cmd, bridge, check_shows_cmd, fuzz_shows_cmd, route_latency_cmd, follow_cmd, import_frames_cmd, make_recording_cmd, q01_cmd, gen, lpx, monitor, mtc_out, ports_cmd, render_cmd, replay_cmd, route, synth, transport_cmd};
use omoiiro_midi::ports;
use omoiiro_runtime::sync::bridge_ingress;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "omoiiro-probe",
    version,
    about = "OMOI-IRO P0 probe: MIDI ports, MTC / MIDI Clock monitor, JSONL record and replay, Launchpad X check, Note/CC routing"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// List MIDI input and output ports. Sends nothing to any device.
    Ports {
        #[arg(long)]
        json: bool,
    },
    /// Observe MTC and/or MIDI Clock on explicit input ports and record JSONL.
    Monitor(monitor::MonitorArgs),
    /// Replay a JSONL recording through the decoders and print events and final state.
    Replay {
        file: PathBuf,
        #[arg(long)]
        json: bool,
        /// Trial (O13): advance from a Full Frame while MIDI Clock is playing, and report the lock error.
        #[arg(long)]
        ff_continue: bool,
    },
    /// Write the synthetic `mtc30-play-seek-gap` fixture (marked synthetic).
    Synth {
        #[arg(long, default_value = "fixtures/synthetic/mtc30-play-seek-gap.jsonl")]
        out: PathBuf,
    },
    /// Send synthetic 30 NDF MTC (and optional MIDI Clock) to a virtual bus for pipeline checks.
    #[command(name = "gen-mtc")]
    GenMtc(gen::GenArgs),
    /// Launchpad X: Device Inquiry, mode, Programmer Mode, one low RGB pad, pad input, restore Live Mode.
    #[command(name = "lpx-check")]
    LpxCheck(lpx::LpxArgs),
    /// Forward Note / CC from an input to the Live control bus (default "OMOI-IRO Control"), Note On vel 0 -> Note Off.
    #[command(name = "route-check")]
    RouteCheck(route::RouteArgs),
    /// Common clock from OmoiIroBridge UDP samples: health, discontinuity, stall, interpolation; records JSONL.
    #[command(name = "bridge-clock")]
    BridgeClock(BridgeListenArgs),
    /// Same as bridge-clock (kept for the P0 commands).
    #[command(name = "bridge-listen", hide = true)]
    BridgeListen(BridgeListenArgs),
    /// Runtime transport (bridge + MIDI Clock or Link) printed once a second.
    Transport(transport_cmd::TransportArgs),
    /// Render a show at any show time without devices (text grid or JSON).
    Render(render_cmd::RenderArgs),
    /// Load, save, reload (byte for byte) and render every *.show.json under the given folders.
    #[command(name = "check-shows")]
    CheckShows(check_shows_cmd::CheckShowsArgs),
    /// Randomly damage valid shows: every load must reject or accept, never panic.
    #[command(name = "fuzz-shows")]
    FuzzShows(fuzz_shows_cmd::FuzzShowsArgs),
    /// Operation route latency (callback → send) while following a show (fake MIDI, synthetic bridge).
    #[command(name = "route-latency")]
    RouteLatency(route_latency_cmd::RouteLatencyArgs),
    /// Add an omoiiro.frames / omoiiro.bitmap asset to a show as a frames preset.
    #[command(name = "import-frames")]
    ImportFrames(import_frames_cmd::ImportFramesArgs),
    /// Turn the pad events of a follow --out record into a recorded triggered clip.
    #[command(name = "make-recording")]
    MakeRecording(make_recording_cmd::MakeRecordingArgs),
    /// Headless follow mode: OmoiIroBridge -> Transport -> render -> virtual LPX (or a port), optional control route.
    Follow(follow_cmd::FollowArgs),
    /// NFR03: render time of 64 pads x 8 layers at 30 fps show times (p95 limit 5 ms).
    #[command(name = "bench-render")]
    BenchRender(bench_cmd::BenchArgs),
    /// Q01: device-less run-through of the show workflow (synthetic; writes results JSON).
    Q01(q01_cmd::Q01Args),
    /// MTC output driven by OmoiIroBridge samples (30 / 25 / 24 NDF).
    #[command(name = "mtc-out")]
    MtcOut(mtc_out::MtcOutArgs),
    /// Analyse an mtc-out recording: QF timing, receiver relock, label / position differences.
    #[command(name = "mtc-out-report")]
    MtcOutReport {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Analyse a bridge-listen recording: intervals, regression, jumps, MTC / Clock offsets.
    #[command(name = "bridge-report")]
    BridgeReport {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Args, Debug)]
struct BridgeListenArgs {
    /// UDP address to receive on.
    #[arg(long, default_value = bridge_ingress::DEFAULT_BIND)]
    bind: String,
    /// MTC input port, as in `monitor`.
    #[arg(long)]
    mtc_in: Option<String>,
    /// MIDI Clock input port, as in `monitor`.
    #[arg(long)]
    clock_in: Option<String>,
    /// JSONL output path. Default: artifacts/p0/<run-id>.jsonl
    #[arg(long)]
    out: Option<PathBuf>,
    /// Run identifier. Default: monitor-<unix seconds>
    #[arg(long)]
    run_id: Option<String>,
    /// Stop after this many seconds. Default: run until Ctrl+C.
    #[arg(long)]
    duration: Option<u64>,
    /// Status line interval in milliseconds.
    #[arg(long, default_value_t = 1000)]
    interval_ms: u64,
    /// Timecode label of Live's song start for the `tc=` display (show `sync.origin`).
    #[arg(long, default_value = "01:00:00:00")]
    origin: String,
    /// Fixed input offset added to the bridge position, in ms.
    #[arg(long, default_value_t = 0.0)]
    input_offset_ms: f64,
}

fn bridge_monitor(a: BridgeListenArgs) -> Result<(), Box<dyn std::error::Error>> {
    monitor::run(monitor::MonitorArgs {
        mtc_in: a.mtc_in,
        clock_in: a.clock_in,
        link: false,
        link_start_stop_sync: false,
        link_no_start_stop_sync: false,
        bridge_udp: Some(a.bind),
        bridge_origin: Some(a.origin),
        bridge_offset_ms: a.input_offset_ms,
        out: a.out,
        run_id: a.run_id,
        duration: a.duration,
        interval_ms: a.interval_ms,
    })
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Ports { json } => ports::list().map(|l| ports_cmd::print(&l, json)),
        Cmd::Monitor(args) => monitor::run(args),
        Cmd::Replay { file, json, ff_continue } => replay_cmd::run(&file, json, ff_continue),
        Cmd::Synth { out } => synth::write_fixture(&out)
            .map(|n| println!("wrote {} records to {} (synthetic)", n, out.display()))
            .map_err(|e| e.into()),
        Cmd::GenMtc(args) => gen::run(args),
        Cmd::LpxCheck(args) => lpx::run(args),
        Cmd::RouteCheck(args) => route::run(args),
        Cmd::BridgeClock(a) | Cmd::BridgeListen(a) => bridge_monitor(a),
        Cmd::BridgeReport { file, json } => bridge::run_report(&file, json),
        Cmd::Render(args) => render_cmd::run(args),
        Cmd::ImportFrames(args) => import_frames_cmd::run(args),
        Cmd::CheckShows(args) => check_shows_cmd::run(args),
        Cmd::FuzzShows(args) => fuzz_shows_cmd::run(args),
        Cmd::RouteLatency(args) => route_latency_cmd::run(args),
        Cmd::MakeRecording(args) => make_recording_cmd::run(args),
        Cmd::Follow(args) => follow_cmd::run(args),
        Cmd::BenchRender(args) => bench_cmd::run(args),
        Cmd::Q01(args) => q01_cmd::run(args),
        Cmd::Transport(args) => transport_cmd::run(args),
        Cmd::MtcOut(args) => mtc_out::run(args),
        Cmd::MtcOutReport { file, json } => mtc_out::run_report(&file, json),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
