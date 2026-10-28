//! JSONL record format shared by `monitor`, `replay` and `synth`.
//!
//! One record per line:
//! `{"run_id":..., "seq":..., "t_mono_us":..., "source":..., "kind":..., "payload":{...}}`
//!
//! * `source`: `mtc`, `clock`, `sync`, `lpx_in`, `lpx_out`, `route_in`,
//!   `control_out`, `link`, `meta`
//! * `kind`: `midi` (payload has `bytes` as hex and `backend_ts_us`),
//!   `start`, `end`, `synthetic`, `drop`, `note`

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Record {
    pub run_id: String,
    pub seq: u64,
    pub t_mono_us: i64,
    pub source: String,
    pub kind: String,
    pub payload: serde_json::Value,
}

pub struct JsonlWriter {
    out: BufWriter<File>,
    run_id: String,
    seq: u64,
}

impl JsonlWriter {
    pub fn create(path: &Path, run_id: &str) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        Ok(Self {
            out: BufWriter::new(File::create(path)?),
            run_id: run_id.to_string(),
            seq: 0,
        })
    }

    pub fn write(
        &mut self,
        t_mono_us: i64,
        source: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> std::io::Result<()> {
        let rec = Record {
            run_id: self.run_id.clone(),
            seq: self.seq,
            t_mono_us,
            source: source.to_string(),
            kind: kind.to_string(),
            payload,
        };
        self.seq += 1;
        serde_json::to_writer(&mut self.out, &rec)?;
        self.out.write_all(b"\n")
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }

    pub fn written(&self) -> u64 {
        self.seq
    }
}

pub fn read_jsonl(path: &Path) -> Result<Vec<Record>, Box<dyn std::error::Error>> {
    let f = File::open(path)?;
    let mut out = Vec::new();
    for (i, line) in BufReader::new(f).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: Record = serde_json::from_str(&line)
            .map_err(|e| format!("{}:{}: {}", path.display(), i + 1, e))?;
        out.push(rec);
    }
    Ok(out)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    s.split_whitespace()
        .map(|t| u8::from_str_radix(t, 16).map_err(|e| format!("bad hex byte {:?}: {}", t, e)))
        .collect()
}

pub fn midi_payload(bytes: &[u8], backend_ts_us: u64) -> serde_json::Value {
    serde_json::json!({ "bytes": hex(bytes), "backend_ts_us": backend_ts_us })
}

/// Extract the MIDI bytes of a `kind == "midi"` record.
pub fn midi_bytes(rec: &Record) -> Option<Vec<u8>> {
    if rec.kind != "midi" {
        return None;
    }
    rec.payload
        .get("bytes")
        .and_then(|v| v.as_str())
        .and_then(|s| parse_hex(s).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let b = [0xF0u8, 0x7F, 0x00, 0x0A, 0xF7];
        assert_eq!(hex(&b), "F0 7F 00 0A F7");
        assert_eq!(parse_hex("F0 7F 00 0A F7").unwrap(), b.to_vec());
    }
}
