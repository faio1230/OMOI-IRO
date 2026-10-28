//! LED writer with a diff cache (I05).
//!
//! * The diff base is the last frame that was sent successfully, never a
//!   frame that was rendered but not sent (docs/02).
//! * A send failure, a reconnect (`reconnect`), a mode change or an output
//!   resume (`invalidate`) clears the base; the next frame is a full
//!   refresh. After a reconnect the adapter's initialization is sent again.
//! * The Launchpad X adapter puts one frame in one SysEx, so a frame is
//!   never split and old / new generations never mix.

use omoiiro_devices::adapter::{DeviceAdapter, DeviceFrame};
use omoiiro_midi::backend::MidiSend;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LedStats {
    pub frames: u64,
    pub unchanged: u64,
    pub full_refreshes: u64,
    pub packets: u64,
    pub bytes: u64,
    pub send_errors: u64,
    pub initializations: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteReport {
    pub packets: usize,
    pub bytes: usize,
    pub full: bool,
}

pub struct LedWriter<A: DeviceAdapter> {
    adapter: A,
    last_sent: Option<DeviceFrame>,
    initialized: bool,
    generation: u64,
    pub stats: LedStats,
}

impl<A: DeviceAdapter> LedWriter<A> {
    pub fn new(adapter: A) -> LedWriter<A> {
        LedWriter { adapter, last_sent: None, initialized: false, generation: 0, stats: LedStats::default() }
    }

    pub fn last_sent(&self) -> Option<&DeviceFrame> {
        self.last_sent.as_ref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Next frame is a full refresh (mode change, output resume).
    pub fn invalidate(&mut self) {
        self.last_sent = None;
    }

    /// A new connection: initialize again, then full refresh.
    pub fn reconnect(&mut self, generation: u64) {
        self.generation = generation;
        self.initialized = false;
        self.last_sent = None;
    }

    /// Send `frame`. On any failure the cache is cleared and the error returned.
    pub fn write<O: MidiSend + ?Sized>(&mut self, out: &mut O, frame: &DeviceFrame) -> Result<WriteReport, String> {
        let mut report = WriteReport { packets: 0, bytes: 0, full: false };
        if !self.initialized {
            for p in self.adapter.initialization() {
                self.send(out, &p, &mut report)?;
            }
            self.initialized = true;
            self.stats.initializations += 1;
        }
        report.full = self.last_sent.is_none();
        let packets = self.adapter.encode_frame(frame, self.last_sent.as_ref());
        for p in &packets {
            self.send(out, p, &mut report)?;
        }
        self.stats.frames += 1;
        if packets.is_empty() {
            self.stats.unchanged += 1;
        }
        if report.full {
            self.stats.full_refreshes += 1;
        }
        self.last_sent = Some(*frame);
        Ok(report)
    }

    fn send<O: MidiSend + ?Sized>(&mut self, out: &mut O, p: &[u8], report: &mut WriteReport) -> Result<(), String> {
        match out.send_msg(p) {
            Ok(()) => {
                report.packets += 1;
                report.bytes += p.len();
                self.stats.packets += 1;
                self.stats.bytes += p.len() as u64;
                Ok(())
            }
            Err(e) => {
                self.stats.send_errors += 1;
                self.last_sent = None;
                self.initialized = false;
                Err(e)
            }
        }
    }

    /// All LEDs off and the device mode restored (best effort).
    pub fn shutdown<O: MidiSend + ?Sized>(&mut self, out: &mut O) -> Result<(), String> {
        for p in self.adapter.shutdown() {
            out.send_msg(&p)?;
        }
        self.last_sent = None;
        self.initialized = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omoiiro_devices::launchpad_x::LaunchpadXAdapter;
    use omoiiro_devices::simulated::VirtualLpx;

    struct Out {
        lpx: VirtualLpx,
        fail: bool,
        sent: Vec<Vec<u8>>,
    }

    impl MidiSend for Out {
        fn send_msg(&mut self, b: &[u8]) -> Result<(), String> {
            if self.fail {
                return Err("unplugged".into());
            }
            self.lpx.apply(b)?;
            self.sent.push(b.to_vec());
            Ok(())
        }
    }

    fn frame(i: usize, c: [u8; 3]) -> DeviceFrame {
        let mut f = DeviceFrame::black();
        f.rgb7[i] = c;
        f
    }

    #[test]
    fn diff_against_the_last_successful_send_and_full_refresh_on_failure() {
        let mut out = Out { lpx: VirtualLpx::new(), fail: false, sent: Vec::new() };
        let mut w = LedWriter::new(LaunchpadXAdapter);
        let a = frame(0, [10, 0, 0]);
        let r = w.write(&mut out, &a).unwrap();
        assert_eq!((r.packets, r.full), (2, true), "mode select + full frame");
        let r = w.write(&mut out, &a).unwrap();
        assert_eq!(r.packets, 0, "unchanged frame sends nothing");

        // Frame b fails to send: the base is cleared, not set to b.
        out.fail = true;
        let b = frame(1, [0, 20, 0]);
        assert!(w.write(&mut out, &b).is_err());
        assert_eq!(w.last_sent(), None, "unsent frame is never the diff base");
        out.fail = false;
        out.lpx = VirtualLpx::new(); // device came back blank
        let r = w.write(&mut out, &b).unwrap();
        assert_eq!((r.packets, r.full), (2, true), "re-initialize + full refresh");
        assert_eq!(out.lpx.frame, b);

        // Small change: one packet with one spec.
        let c = frame(1, [0, 21, 0]);
        let r = w.write(&mut out, &c).unwrap();
        assert_eq!((r.packets, r.bytes, r.full), (1, 6 + 1 + 5 + 1, false));
        assert_eq!(out.lpx.frame, c);

        w.reconnect(9);
        let r = w.write(&mut out, &c).unwrap();
        assert_eq!((r.packets, r.full, w.generation()), (2, true, 9));
        w.shutdown(&mut out).unwrap();
        assert_eq!((out.lpx.frame, out.lpx.mode), (DeviceFrame::black(), Some(0)));
        assert_eq!((w.stats.send_errors, w.stats.full_refreshes, w.stats.unchanged), (1, 3, 1));
    }
}
