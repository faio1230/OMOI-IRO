//! Synthetic MTC / MIDI Clock sender for pipeline checks without Live.
//!
//! Sends 30 NDF Quarter Frames (and optionally MIDI Clock) to an explicit
//! virtual bus so that `monitor` can be exercised end to end through the OS
//! MIDI stack. It refuses ports that look like a Launchpad. Timing uses
//! `thread::sleep`, so jitter of a few milliseconds is expected; this is a
//! source for plumbing tests, not a timing reference.

use omoiiro_midi::clock::MonotonicClock;
use omoiiro_midi::ports;
use omoiiro_runtime::sync::mtc::{encode_full_frame, encode_quarter_frames, FrameRate, Label};
use std::thread;
use std::time::Duration;

#[derive(clap::Args, Debug)]
pub struct GenArgs {
    /// Output port: exact name or `#index`. Use a virtual bus, never a device.
    #[arg(long)]
    pub out: String,
    /// Start label HH:MM:SS:FF at 30 NDF.
    #[arg(long, default_value = "01:00:00:00")]
    pub start: String,
    /// Seconds of Quarter Frames to send.
    #[arg(long, default_value_t = 10)]
    pub seconds: u64,
    /// Skip the initial Full Frame.
    #[arg(long)]
    pub no_full_frame: bool,
    /// Also send MIDI Clock at this BPM (Start first, Stop at the end).
    #[arg(long)]
    pub clock_bpm: Option<u32>,
    /// Pause Quarter Frames for this many ms at the midpoint (the label keeps running).
    #[arg(long)]
    pub gap_ms: Option<u64>,
}

pub fn parse_label(s: &str) -> Result<Label, String> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 4 {
        return Err(format!("label {s:?} must be HH:MM:SS:FF"));
    }
    let p = |i: usize| parts[i].parse::<u8>().map_err(|e| format!("label {s:?}: {e}"));
    let label = Label::new(p(0)?, p(1)?, p(2)?, p(3)?, FrameRate::Fps30);
    label.validate().map_err(|e| format!("label {s:?}: {e:?}"))?;
    Ok(label)
}

fn sleep_until(clock: &MonotonicClock, t_us: i64) {
    loop {
        let now = clock.now_us();
        if now >= t_us {
            return;
        }
        let wait = (t_us - now) as u64;
        thread::sleep(Duration::from_micros(wait.min(2_000)));
    }
}

pub fn run(args: GenArgs) -> Result<(), Box<dyn std::error::Error>> {
    let start = parse_label(&args.start)?;
    let (output, port, name) = ports::open_output("omoiiro-probe-gen", &args.out)?;
    if ports::looks_like_launchpad(&name) {
        return Err(format!("refusing to send synthetic sync to a device-looking port {name:?}").into());
    }
    let mut conn = output
        .connect(&port, "omoiiro-probe-gen")
        .map_err(|e| format!("open output {name:?}: {e}"))?;
    println!("output      : {name}");
    println!("start label : {start} (30 NDF), {} s{}", args.seconds, args.clock_bpm.map(|b| format!(", clock {b} bpm")).unwrap_or_default());

    let rate = FrameRate::Fps30;
    let clock = MonotonicClock::new();
    let t0 = clock.now_us() + 50_000;
    if !args.no_full_frame {
        conn.send(&encode_full_frame(start))?;
        println!("sent full frame {start}");
    }
    if args.clock_bpm.is_some() {
        conn.send(&[0xFA])?;
    }
    let total_qf = (args.seconds * 120) as i64;
    let gap = args.gap_ms.map(|ms| {
        let mid = total_qf / 2;
        let gap_qf = (ms as i64 * 120 / 1000).max(1);
        (mid, mid + gap_qf)
    });
    let tick_us = args.clock_bpm.map(|b| 60_000_000 / (b as i64 * 24));
    let mut next_tick = 0i64;
    let mut sent_qf = 0u64;
    let mut sent_ticks = 0u64;
    let mut skipped = 0u64;
    let mut k = 0i64;
    while k < total_qf {
        let t_qf = t0 + k * 1_000_000 / 120;
        // Emit clock ticks that are due before this quarter frame.
        if let Some(tu) = tick_us {
            while t0 + next_tick * tu <= t_qf {
                sleep_until(&clock, t0 + next_tick * tu);
                conn.send(&[0xF8])?;
                sent_ticks += 1;
                next_tick += 1;
            }
        }
        sleep_until(&clock, t_qf);
        let in_gap = gap.map(|(a, b)| k >= a && k < b).unwrap_or(false);
        if in_gap {
            skipped += 1;
        } else {
            let frame = start.frame_index() + (k / 8) as u64 * 2;
            let label = Label::from_frame_index(frame, rate);
            let pieces = encode_quarter_frames(label);
            conn.send(&[0xF1, pieces[(k % 8) as usize]])?;
            sent_qf += 1;
        }
        k += 1;
    }
    if args.clock_bpm.is_some() {
        conn.send(&[0xFC])?;
    }
    let last = Label::from_frame_index(start.frame_index() + ((total_qf - 1) / 8) as u64 * 2, rate);
    println!("done: quarter frames sent={sent_qf} skipped(gap)={skipped} clock ticks={sent_ticks} last label={last}");
    Ok(())
}
