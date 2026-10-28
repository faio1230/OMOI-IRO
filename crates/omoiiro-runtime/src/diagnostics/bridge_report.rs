//! Analysis of a bridge recording (`bridge-report`): arrival intervals of the
//! listener samples, per playing-segment regression of song time (ms)
//! against arrival time and against Live's `perf_ns`, song-time jumps, and,
//! when present, the difference to the MTC position and Clock beats.
//!
//! Moved from tools/p0-probe/src/bridge.rs in M1.

use crate::diagnostics::record::{midi_bytes, Record};
use crate::diagnostics::report::us_to_secs;
use crate::sync::clock::{ClockDecoder, Transport};
use crate::sync::mtc::{MtcDecoder, Params, SyncHealth};
use serde_json::{json, Value};

/// A song-time step that deviates from the elapsed time by more than this is a jump.
pub const JUMP_TOLERANCE_MS: f64 = 20.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub t_mono_us: i64,
    pub kind: String,
    pub event: String,
    pub perf_ns: Option<i64>,
    pub smpte_ms: Option<f64>,
    pub beats: Option<f64>,
    pub tempo: Option<f64>,
    pub is_playing: Option<bool>,
}

impl Sample {
    pub fn from_record(r: &Record) -> Option<Sample> {
        if r.source != "bridge" || r.kind != "sample" {
            return None;
        }
        let p = &r.payload;
        Some(Sample {
            t_mono_us: r.t_mono_us,
            kind: p.get("kind").and_then(Value::as_str).unwrap_or("").to_string(),
            event: p.get("event").and_then(Value::as_str).unwrap_or("").to_string(),
            perf_ns: p.get("perf_ns").and_then(Value::as_i64),
            smpte_ms: p.get("smpte_ms").and_then(Value::as_f64),
            beats: p.get("beats").and_then(Value::as_f64),
            tempo: p.get("tempo").and_then(Value::as_f64),
            is_playing: p.get("is_playing").and_then(Value::as_bool),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dist {
    pub n: usize,
    pub median: f64,
    pub p95: f64,
    pub max: f64,
}

/// Median / p95 (nearest rank) / max. `None` for an empty input.
pub fn dist(values: &[f64]) -> Option<Dist> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let rank = |q: f64| v[((q * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1];
    let median = if v.len() % 2 == 1 { v[v.len() / 2] } else { (v[v.len() / 2 - 1] + v[v.len() / 2]) / 2.0 };
    Some(Dist { n: v.len(), median, p95: rank(0.95), max: *v.last().unwrap() })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub slope: f64,
    pub intercept: f64,
    /// Absolute residuals in ms.
    pub residual: Dist,
}

/// Least squares y = slope * x + intercept, with absolute residual stats.
pub fn fit(x: &[f64], y: &[f64]) -> Option<Fit> {
    let n = x.len();
    if n < 3 || n != y.len() {
        return None;
    }
    let mx = x.iter().sum::<f64>() / n as f64;
    let my = y.iter().sum::<f64>() / n as f64;
    let sxx: f64 = x.iter().map(|a| (a - mx) * (a - mx)).sum();
    if sxx == 0.0 {
        return None;
    }
    let sxy: f64 = x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum();
    let slope = sxy / sxx;
    let intercept = my - slope * mx;
    let res: Vec<f64> = x.iter().zip(y).map(|(a, b)| (b - (slope * a + intercept)).abs()).collect();
    Some(Fit { slope, intercept, residual: dist(&res)? })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Jump {
    pub t_mono_us: i64,
    pub from_ms: f64,
    pub to_ms: f64,
    pub elapsed_ms: f64,
    /// `(to - from) - elapsed`.
    pub deviation_ms: f64,
    pub tempo_before: Option<f64>,
    pub tempo_after: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub start_us: i64,
    pub end_us: i64,
    pub samples: usize,
    /// smpte ms against arrival time (ms).
    pub vs_arrival: Option<Fit>,
    /// smpte ms against Live's perf_counter (ms).
    pub vs_perf: Option<Fit>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Offsets {
    /// smpte ms - MTC position ms, over samples with a locked MTC.
    pub mtc_ms: Option<Dist>,
    pub mtc_median_raw_ms: Option<f64>,
    /// beats - Clock beats, over samples with a playing Clock and a valid position.
    pub clock_beats: Option<Dist>,
    pub clock_median_raw_beats: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BridgeReport {
    pub samples: usize,
    pub listener_samples: usize,
    pub timer_samples: usize,
    pub parse_errors: usize,
    /// Arrival interval of listener `song_time` samples while playing (ms).
    pub listener_interval_ms: Option<Dist>,
    pub listener_per_second: Option<f64>,
    pub timer_interval_ms: Option<Dist>,
    pub segments: Vec<Segment>,
    pub jumps: Vec<Jump>,
    pub offsets: Offsets,
}

fn intervals_ms(ts: &[i64]) -> Vec<f64> {
    ts.windows(2).map(|w| (w[1] - w[0]) as f64 / 1000.0).collect()
}

/// Analyse a recording (bridge samples, optionally with MTC / Clock MIDI).
pub fn analyse(records: &[Record]) -> BridgeReport {
    let samples: Vec<Sample> = records.iter().filter_map(Sample::from_record).collect();
    let parse_errors = records
        .iter()
        .filter(|r| r.source == "bridge" && r.payload.get("parse_error").is_some())
        .count();

    // Listener song_time samples while playing: the stream an app would follow.
    let listener_play: Vec<i64> = samples
        .iter()
        .filter(|s| s.kind == "listener" && s.event == "song_time" && s.is_playing == Some(true))
        .map(|s| s.t_mono_us)
        .collect();
    let listener_interval_ms = dist(&intervals_ms(&listener_play));
    let listener_per_second = match (listener_play.first(), listener_play.last()) {
        (Some(a), Some(b)) if b > a => Some((listener_play.len() - 1) as f64 / ((b - a) as f64 / 1e6)),
        _ => None,
    };
    let timer_ts: Vec<i64> = samples.iter().filter(|s| s.kind == "timer").map(|s| s.t_mono_us).collect();

    // Playing segments over all samples with a song time, split at jumps.
    let timed: Vec<&Sample> = samples.iter().filter(|s| s.smpte_ms.is_some()).collect();
    let mut segments = Vec::new();
    let mut jumps = Vec::new();
    let mut cur: Vec<&Sample> = Vec::new();
    let close = |cur: &mut Vec<&Sample>, segments: &mut Vec<Segment>| {
        if cur.len() >= 2 {
            let ys: Vec<f64> = cur.iter().map(|s| s.smpte_ms.unwrap()).collect();
            let xa: Vec<f64> = cur.iter().map(|s| s.t_mono_us as f64 / 1000.0).collect();
            let with_perf: Vec<&&Sample> = cur.iter().filter(|s| s.perf_ns.is_some()).collect();
            let xp: Vec<f64> = with_perf.iter().map(|s| s.perf_ns.unwrap() as f64 / 1e6).collect();
            let yp: Vec<f64> = with_perf.iter().map(|s| s.smpte_ms.unwrap()).collect();
            segments.push(Segment {
                start_us: cur[0].t_mono_us,
                end_us: cur[cur.len() - 1].t_mono_us,
                samples: cur.len(),
                vs_arrival: fit(&xa, &ys),
                vs_perf: fit(&xp, &yp),
            });
        }
        cur.clear();
    };
    let mut prev: Option<&Sample> = None;
    for s in timed {
        if let Some(p) = prev {
            let elapsed_ms = (s.t_mono_us - p.t_mono_us) as f64 / 1000.0;
            let step = s.smpte_ms.unwrap() - p.smpte_ms.unwrap();
            let deviation = step - elapsed_ms;
            // Only judge jumps while playing; a stopped transport legitimately holds still.
            let playing = p.is_playing == Some(true) && s.is_playing == Some(true);
            let moved_while_stopped = !playing && step.abs() > JUMP_TOLERANCE_MS;
            if (playing && deviation.abs() > JUMP_TOLERANCE_MS) || moved_while_stopped {
                jumps.push(Jump {
                    t_mono_us: s.t_mono_us,
                    from_ms: p.smpte_ms.unwrap(),
                    to_ms: s.smpte_ms.unwrap(),
                    elapsed_ms,
                    deviation_ms: if playing { deviation } else { step },
                    tempo_before: p.tempo,
                    tempo_after: s.tempo,
                });
                close(&mut cur, &mut segments);
            }
        }
        if s.is_playing == Some(true) {
            cur.push(s);
        } else {
            close(&mut cur, &mut segments);
        }
        prev = Some(s);
    }
    close(&mut cur, &mut segments);

    BridgeReport {
        samples: samples.len(),
        listener_samples: samples.iter().filter(|s| s.kind == "listener").count(),
        timer_samples: timer_ts.len(),
        parse_errors,
        listener_interval_ms,
        listener_per_second,
        timer_interval_ms: dist(&intervals_ms(&timer_ts)),
        segments,
        jumps,
        offsets: offsets(records),
    }
}

/// Compare each playing bridge sample with the MTC position and Clock beats
/// decoded from the same recording at the sample's arrival time.
fn offsets(records: &[Record]) -> Offsets {
    let mut mtc = MtcDecoder::new(Params::default());
    let mut clock = ClockDecoder::new();
    let (mut mtc_d, mut clk_d) = (Vec::new(), Vec::new());
    let (mut me, mut ce) = (Vec::new(), Vec::new());
    for r in records {
        if let Some(bytes) = midi_bytes(r) {
            if matches!(r.source.as_str(), "mtc" | "sync") {
                me.clear();
                mtc.feed(r.t_mono_us, &bytes, &mut me);
            }
            if matches!(r.source.as_str(), "clock" | "sync") {
                ce.clear();
                clock.feed(r.t_mono_us, &bytes, &mut ce);
            }
            continue;
        }
        let Some(s) = Sample::from_record(r) else { continue };
        if s.is_playing != Some(true) {
            continue;
        }
        if let Some(ms) = s.smpte_ms {
            let m = mtc.snapshot(s.t_mono_us);
            if m.health == SyncHealth::Locked {
                if let Some(p) = m.position_us {
                    mtc_d.push(ms - p as f64 / 1000.0);
                }
            }
        }
        if let Some(b) = s.beats {
            let c = clock.snapshot(s.t_mono_us);
            if c.transport == Transport::Playing {
                if let Some(cb) = c.beats {
                    clk_d.push(b - cb);
                }
            }
        }
    }
    let spread = |v: &[f64]| -> (Option<Dist>, Option<f64>) {
        let med = dist(v).map(|d| d.median);
        let dev: Vec<f64> = v.iter().map(|x| (x - med.unwrap_or(0.0)).abs()).collect();
        (dist(&dev), med)
    };
    let (mtc_ms, mtc_median_raw_ms) = spread(&mtc_d);
    let (clock_beats, clock_median_raw_beats) = spread(&clk_d);
    Offsets { mtc_ms, mtc_median_raw_ms, clock_beats, clock_median_raw_beats }
}

fn dist_json(d: &Option<Dist>) -> Value {
    match d {
        Some(d) => json!({ "n": d.n, "median": d.median, "p95": d.p95, "max": d.max }),
        None => Value::Null,
    }
}

fn fit_json(f: &Option<Fit>) -> Value {
    match f {
        Some(f) => json!({ "slope": f.slope, "intercept": f.intercept, "residual_abs_ms": dist_json(&Some(f.residual)) }),
        None => Value::Null,
    }
}

pub fn report_json(r: &BridgeReport) -> Value {
    json!({
        "samples": r.samples,
        "listener_samples": r.listener_samples,
        "timer_samples": r.timer_samples,
        "parse_errors": r.parse_errors,
        "listener_song_time_interval_ms_while_playing": dist_json(&r.listener_interval_ms),
        "listener_song_time_per_second_while_playing": r.listener_per_second,
        "timer_interval_ms": dist_json(&r.timer_interval_ms),
        "segments": r.segments.iter().map(|s| json!({
            "start_s": us_to_secs(s.start_us),
            "end_s": us_to_secs(s.end_us),
            "samples": s.samples,
            "smpte_vs_arrival": fit_json(&s.vs_arrival),
            "smpte_vs_live_perf": fit_json(&s.vs_perf),
        })).collect::<Vec<_>>(),
        "jumps": r.jumps.iter().map(|j| json!({
            "t_s": us_to_secs(j.t_mono_us),
            "from_ms": j.from_ms,
            "to_ms": j.to_ms,
            "elapsed_ms": j.elapsed_ms,
            "deviation_ms": j.deviation_ms,
            "tempo_before": j.tempo_before,
            "tempo_after": j.tempo_after,
        })).collect::<Vec<_>>(),
        "offsets": {
            "smpte_minus_mtc_ms": { "median_raw": r.offsets.mtc_median_raw_ms, "abs_dev_from_median": dist_json(&r.offsets.mtc_ms) },
            "beats_minus_clock_beats": { "median_raw": r.offsets.clock_median_raw_beats, "abs_dev_from_median": dist_json(&r.offsets.clock_beats) },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::record::hex;
    use crate::sync::mtc::{encode_full_frame, encode_quarter_frames, FrameRate, Label};

    fn rec(seq: u64, t: i64, source: &str, kind: &str, payload: Value) -> Record {
        Record { run_id: "t".into(), seq, t_mono_us: t, source: source.into(), kind: kind.into(), payload }
    }

    /// `t` = arrival (t_mono_us), `t_live` = Live-side sampling time (perf).
    fn sample(seq: u64, t: i64, t_live: i64, kind: &str, ms: f64, playing: bool, tempo: f64) -> Record {
        rec(
            seq,
            t,
            "bridge",
            "sample",
            json!({
                "kind": kind, "event": if kind == "listener" { "song_time" } else { "tick" },
                "perf_ns": t_live * 1000 + 5_000_000_000i64, "smpte_ms": ms,
                "beats": ms / 500.0, "tempo": tempo, "is_playing": playing,
            }),
        )
    }

    #[test]
    fn dist_and_fit_basics() {
        let d = dist(&[3.0, 1.0, 2.0, 10.0]).unwrap();
        assert_eq!((d.median, d.p95, d.max, d.n), (2.5, 10.0, 10.0, 4));
        let f = fit(&[0.0, 1.0, 2.0, 3.0], &[1.0, 3.0, 5.0, 7.0]).unwrap();
        assert!((f.slope - 2.0).abs() < 1e-12 && (f.intercept - 1.0).abs() < 1e-12);
        assert!(f.residual.max < 1e-9);
        assert_eq!(fit(&[1.0, 1.0, 1.0], &[1.0, 2.0, 3.0]), None);
        assert_eq!(dist(&[]), None);
    }

    /// Synthetic: listener every 16 ms with +-2 ms arrival jitter, a loop
    /// jump (-8000 ms) at 1 s, a stop, then a seek while stopped.
    fn synthetic() -> Vec<Record> {
        let mut v = Vec::new();
        let mut seq = 0;
        let jitter = [0i64, 2000, -2000, 1000, -1000];
        let mut song = 30_000.0;
        let mut t = 0i64;
        for i in 0..125 {
            let tt = t + jitter[i % 5];
            if i == 62 {
                song -= 8000.0; // loop wrap
            }
            v.push(sample(seq, tt, t, "listener", song, true, 120.0));
            seq += 1;
            if i % 6 == 0 {
                v.push(sample(seq, tt + 100, t, "timer", song, true, 120.0));
                seq += 1;
            }
            t += 16_000;
            song += 16.0;
        }
        // Stop, then seek while stopped.
        v.push(sample(seq, t, t, "listener", song, false, 120.0));
        v.push(sample(seq + 1, t + 100_000, t + 100_000, "timer", song, false, 120.0));
        v.push(sample(seq + 2, t + 200_000, t + 200_000, "listener", 60_000.0, false, 120.0));
        v
    }

    #[test]
    fn synthetic_report_measures_intervals_segments_and_jumps() {
        let r = analyse(&synthetic());
        assert_eq!(r.listener_samples, 127);
        let iv = r.listener_interval_ms.unwrap();
        assert!((iv.median - 16.0).abs() <= 1.0, "{iv:?}");
        assert!(iv.max <= 20.0 && iv.max >= 18.0, "{iv:?}");
        assert!((r.listener_per_second.unwrap() - 62.5).abs() < 1.0);
        // Two playing segments split at the loop jump.
        assert_eq!(r.segments.len(), 2, "{:?}", r.segments);
        for s in &r.segments {
            let a = s.vs_arrival.unwrap();
            assert!((a.slope - 1.0).abs() < 0.01, "{a:?}");
            assert!(a.residual.max <= 2.5, "{a:?}");
            // Live-side time has no delivery jitter, so it fits (almost) exactly.
            let p = s.vs_perf.unwrap();
            assert!((p.slope - 1.0).abs() < 1e-9 && p.residual.max < 1e-6, "{p:?}");
        }
        assert_eq!(r.jumps.len(), 2, "{:?}", r.jumps);
        assert!((r.jumps[0].deviation_ms + 8000.0).abs() < 5.0, "{:?}", r.jumps[0]);
        // The seek while stopped is reported with its step.
        assert!(r.jumps[1].to_ms == 60_000.0);
        assert_eq!(r.offsets.mtc_ms, None);
        assert_eq!(r.offsets.clock_beats, None);
    }

    #[test]
    fn offsets_against_locked_mtc_and_playing_clock() {
        let mut v = Vec::new();
        let mut seq = 0;
        let l = |f: u64| Label::from_frame_index(30 * 3600 + f, FrameRate::Fps30);
        v.push(rec(seq, 0, "mtc", "midi", json!({"bytes": hex(&encode_full_frame(l(0))), "backend_ts_us": 0})));
        seq += 1;
        v.push(rec(seq, 0, "clock", "midi", json!({"bytes": "FA", "backend_ts_us": 0})));
        seq += 1;
        let qf = 1_000_000 / 120;
        let tick = 20_833;
        let mut next_tick = 0;
        for k in 0..240i64 {
            let t = k * qf;
            while next_tick * tick <= t {
                v.push(rec(seq, next_tick * tick, "clock", "midi", json!({"bytes": "F8", "backend_ts_us": 0})));
                seq += 1;
                next_tick += 1;
            }
            let label = l((k / 8 * 2) as u64);
            let d = encode_quarter_frames(label)[(k % 8) as usize];
            v.push(rec(seq, t, "mtc", "midi", json!({"bytes": hex(&[0xF1, d]), "backend_ts_us": 0})));
            seq += 1;
            if k % 4 == 2 && k > 40 {
                // Bridge says song time = MTC position - 1 h + 30 ms; beats = clock beats + 0.25.
                let ms = t as f64 / 1000.0 + 30.0;
                v.push(rec(
                    seq,
                    t,
                    "bridge",
                    "sample",
                    json!({"kind": "listener", "event": "song_time", "smpte_ms": ms, "beats": next_tick as f64 / 24.0 + 0.25, "is_playing": true}),
                ));
                seq += 1;
            }
        }
        let r = analyse(&v);
        let mtc_med = r.offsets.mtc_median_raw_ms.unwrap();
        assert!((mtc_med - (30.0 - 3_600_000.0)).abs() < 20.0, "{mtc_med}");
        let clk_med = r.offsets.clock_median_raw_beats.unwrap();
        assert!((clk_med - 0.25).abs() < 0.05, "{clk_med}");
        assert!(r.offsets.mtc_ms.unwrap().n > 40);
    }
}
