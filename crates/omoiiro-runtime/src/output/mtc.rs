//! MTC output stage (docs/10-live-bridge.md "MTC出力"): a dedicated sender
//! thread runs the `MtcGen` state machine against the transport's bridge
//! view, sleeps to each Quarter Frame (coarse sleep, then yield, then spin),
//! sends, and `try_send`s a record of what it sent and when.
//!
//! The application never uses its own MTC output as an input clock.

use crate::sync::bridge::BridgeSnapshot;
use crate::sync::mtc_out::{desired_from, Desired, GenOut, GenStats, MtcGen};
use crate::sync::transport::Transport;
use crate::clock::MonotonicClock;
use crate::diagnostics::record::hex;
use crossbeam_channel::Sender;
use serde_json::{json, Value};
use omoiiro_midi::backend::MidiSend;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Where the sender reads the bridge state from.
pub trait MtcSource: Send + Sync + 'static {
    /// `(bridge snapshot, discontinuity epoch, origin + input offset in us)` at `now_us`.
    fn view(&self, now_us: i64) -> (BridgeSnapshot, u64, f64);
}

impl MtcSource for Mutex<Transport> {
    fn view(&self, now_us: i64) -> (BridgeSnapshot, u64, f64) {
        let t = self.lock().unwrap_or_else(|e| e.into_inner());
        let (s, e) = t.bridge_view(now_us);
        (s, e, t.config().origin_plus_offset_us())
    }
}

/// What the sender thread reports for each message.
pub struct Sent {
    pub t_us: i64,
    pub out: GenOut,
    pub late_us: f64,
    pub src_pos_us: Option<f64>,
}

/// JSONL payload of a sent message (`source = mtc_out`, `kind = send`).
pub fn sent_payload(s: &Sent) -> Value {
    match &s.out {
        GenOut::FullFrame { label, reason, bytes } => json!({
            "type": "ff", "label": label.to_string(), "reason": reason, "bytes": hex(bytes), "src_pos_us": s.src_pos_us,
        }),
        GenOut::Qf { piece, data, label, t_sched_us } => {
            let mut v = json!({ "type": "qf", "piece": piece, "data": format!("{data:02X}"), "label": label.to_string(),
                "t_sched_us": t_sched_us.round() as i64, "late_us": s.late_us.round() as i64 });
            if *piece == 0 {
                v["src_pos_us"] = json!(s.src_pos_us.map(|p| p.round() as i64));
                v["label_us"] = json!(label.to_us());
            }
            v
        }
    }
}

fn sleep_until(clock: &MonotonicClock, t_us: f64, stop: &AtomicBool) {
    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let d = t_us - clock.now_us() as f64;
        if d <= 0.0 {
            return;
        }
        if d > 2_000.0 {
            std::thread::sleep(Duration::from_micros((d - 1_500.0) as u64));
        } else if d > 200.0 {
            std::thread::yield_now();
        } else {
            std::hint::spin_loop();
        }
    }
}

/// Start the sender thread. It stops (sending nothing more) when `stop` is set
/// and returns the generator statistics.
pub fn spawn_sender<O, S>(
    mut out_port: O,
    mut gen: MtcGen,
    source: Arc<S>,
    clock: MonotonicClock,
    tx: Sender<Sent>,
    dropped: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<GenStats>
where
    O: MidiSend + Send + 'static,
    S: MtcSource + ?Sized,
{
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut wake = clock.now_us() as f64;
        while !stop.load(Ordering::Relaxed) {
            sleep_until(&clock, wake, &stop);
            let now = clock.now_us();
            let (snap, epoch, origin_plus_offset_us) = source.view(now);
            let desired = desired_from(&snap, origin_plus_offset_us, now);
            let src_pos_us = match desired {
                Desired::Run { pos_us } | Desired::Hold { pos_us } => Some(pos_us),
                Desired::Off => None,
            };
            out.clear();
            wake = gen.step(now as f64, desired, epoch, &mut out);
            for o in out.drain(..) {
                let (bytes, t_sched): (Vec<u8>, f64) = match &o {
                    GenOut::FullFrame { bytes, .. } => (bytes.to_vec(), now as f64),
                    GenOut::Qf { data, t_sched_us, .. } => (vec![0xF1, *data], *t_sched_us),
                };
                let _ = out_port.send_msg(&bytes);
                let t_sent = clock.now_us();
                let rec = Sent { t_us: t_sent, late_us: t_sent as f64 - t_sched, src_pos_us, out: o };
                if tx.try_send(rec).is_err() {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        gen.stats
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::bridge::BridgeSample;
    use crate::sync::transport::TransportConfig;
    use crossbeam_channel::bounded;
    use omoiiro_core::time::{FrameRate, Timecode};

    struct Sink(Arc<Mutex<Vec<Vec<u8>>>>);
    impl MidiSend for Sink {
        fn send_msg(&mut self, bytes: &[u8]) -> Result<(), String> {
            self.0.lock().unwrap().push(bytes.to_vec());
            Ok(())
        }
    }

    /// Real thread and clock: feed a playing bridge for ~200 ms and check that
    /// a start Full Frame and Quarter Frames come out with the label from the
    /// transport (origin + song + offset).
    #[test]
    fn sender_follows_the_transport() {
        let origin = Timecode::parse("01:00:00:00", FrameRate::Fps30).unwrap();
        let transport = Arc::new(Mutex::new(Transport::new(TransportConfig::defaults(origin, 0, None))));
        let clock = MonotonicClock::new();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = bounded(4096);
        let stop = Arc::new(AtomicBool::new(false));
        let h = spawn_sender(Sink(sent.clone()), MtcGen::new(crate::sync::mtc::FrameRate::Fps30).unwrap(), transport.clone(), clock.clone(), tx, Arc::new(AtomicU64::new(0)), stop.clone());
        let mut ev = Vec::new();
        for k in 0..14 {
            let t = clock.now_us();
            let p = serde_json::json!({"v": 2, "seq": k + 1, "song_ms": 10_000.0 + k as f64 * 16.0, "is_playing": true, "errors": []});
            transport.lock().unwrap().feed_bridge(BridgeSample::from_payload(t, &p), &mut ev);
            std::thread::sleep(Duration::from_millis(16));
        }
        stop.store(true, Ordering::SeqCst);
        let stats = h.join().unwrap();
        let recs: Vec<Sent> = rx.try_iter().collect();
        assert!(matches!(&recs[0].out, GenOut::FullFrame { reason: "start", label, .. } if label.to_string().starts_with("01:00:10:")));
        assert!(stats.quarter_frames >= 8, "{stats:?}");
        assert_eq!(sent.lock().unwrap().len(), recs.len());
        assert_eq!(sent_payload(&recs[0])["type"], "ff");
    }
}
