//! OMOI-IRO probe CLI (formerly tools/p0-probe). A thin layer over the
//! workspace crates: argument parsing, port opening, threads, printing and
//! JSONL writing. Decoders, codecs and analyses live in `omoiiro-runtime`,
//! `omoiiro-midi` and `omoiiro-devices`.

pub mod bench_cmd;
pub mod bridge;
pub mod follow_cmd;
pub mod gen;
pub mod import_frames_cmd;
pub mod make_recording_cmd;
pub mod lpx;
pub mod monitor;
pub mod mtc_out;
pub mod q01_cmd;
pub mod ports_cmd;
pub mod render_cmd;
pub mod replay_cmd;
pub mod route;
pub mod synth;
pub mod transport_cmd;
