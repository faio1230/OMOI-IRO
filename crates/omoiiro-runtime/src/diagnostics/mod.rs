//! JSONL record format, replay and analyses shared by the probe CLI and the
//! regression tests. Results computed here from recordings are re-analyses,
//! not new device or Live measurements. `perf` holds the Runtime timing
//! histograms (T07); `nfr03` the render load measurement.

pub mod bridge_report;
pub mod mtc_out_report;
pub mod nfr03;
pub mod perf;
pub mod record;
pub mod replay;
pub mod report;
pub mod transport_replay;
