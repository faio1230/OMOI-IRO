//! OmoiIroBridge UDP ingress: the receiver thread stamps the session monotonic
//! clock on arrival, copies the datagram and `try_send`s. Parsing and
//! recording happen on the caller's pipeline thread.
//!
//! Moved from tools/p0-probe/src/bridge.rs in M1.

use crate::clock::MonotonicClock;
use crossbeam_channel::Sender;
use serde_json::{json, Value};
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_BIND: &str = "127.0.0.1:9007";

/// One datagram as received.
pub struct RawDatagram {
    pub t_mono_us: i64,
    pub bytes: Vec<u8>,
}

/// Start the UDP receiver thread. It only stamps, copies and `try_send`s.
pub fn spawn_receiver(
    bind: &str,
    clock: MonotonicClock,
    tx: Sender<RawDatagram>,
    dropped: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    let sock = UdpSocket::bind(bind)?;
    sock.set_read_timeout(Some(Duration::from_millis(50)))?;
    Ok(std::thread::spawn(move || {
        let mut buf = vec![0u8; 65536];
        while !stop.load(Ordering::SeqCst) {
            match sock.recv(&mut buf) {
                Ok(n) => {
                    let t_mono_us = clock.now_us();
                    if tx.try_send(RawDatagram { t_mono_us, bytes: buf[..n].to_vec() }).is_err() {
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                // Windows reports ICMP port unreachable on UDP sockets; keep going.
                Err(_) => {}
            }
        }
    }))
}

/// Payload for the record: the datagram JSON, or the raw text if it does not parse.
pub fn datagram_payload(bytes: &[u8]) -> Value {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(v) => v,
        Err(e) => json!({ "raw": String::from_utf8_lossy(bytes), "parse_error": e.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datagram_payload_keeps_unparsable_text() {
        assert_eq!(datagram_payload(br#"{"a":1}"#), json!({"a": 1}));
        let v = datagram_payload(b"not json");
        assert_eq!(v["raw"], "not json");
        assert!(v.get("parse_error").is_some());
    }
}
