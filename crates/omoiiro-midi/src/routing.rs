//! Note / CC forwarding rule used by the control route (Launchpad X or any
//! input -> the Live control bus).
//!
//! Only Note Off (8n), Note On (9n) and Control Change (Bn) are forwarded.
//! Note On with velocity 0 is normalized to Note Off (8n kk 00). Everything
//! else (aftertouch, SysEx, Real Time, ...) is not forwarded.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Routed {
    /// Forward these bytes. `normalized` is true when Note On vel 0 became Note Off.
    Forward { bytes: [u8; 3], normalized: bool },
    /// Not a Note / CC message; record only.
    Ignore,
}

/// Decide what to forward for one complete MIDI message.
pub fn route(bytes: &[u8]) -> Routed {
    if bytes.len() != 3 || bytes[1] > 0x7F || bytes[2] > 0x7F {
        return Routed::Ignore;
    }
    let status = bytes[0];
    let ch = status & 0x0F;
    match status & 0xF0 {
        0x90 if bytes[2] == 0 => Routed::Forward { bytes: [0x80 | ch, bytes[1], 0x00], normalized: true },
        0x80 | 0x90 | 0xB0 => Routed::Forward { bytes: [status, bytes[1], bytes[2]], normalized: false },
        _ => Routed::Ignore,
    }
}

/// Whitelist of the operation route (I03). Fixed rules: Note and/or CC on
/// the channels in `channels` (bit n = MIDI channel n + 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteRules {
    pub notes: bool,
    pub cc: bool,
    pub channels: u16,
}

impl Default for RouteRules {
    fn default() -> Self {
        RouteRules { notes: true, cc: true, channels: 0xFFFF }
    }
}

/// What the router decided for one input message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteOutcome {
    /// Send these bytes to the control output.
    Send { bytes: [u8; 3], normalized: bool },
    /// Not on the whitelist (record only).
    Ignored,
    /// Duplicate on the same route: Note On for a sounding note, or Note Off
    /// for a note this route never turned on.
    Suppressed,
    /// The route is in fault and forwards nothing until `reset`.
    Blocked,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RouterStats {
    pub received: u64,
    pub sent: u64,
    pub normalized: u64,
    pub ignored: u64,
    pub suppressed: u64,
    pub blocked: u64,
    pub faults: u64,
    pub released: u64,
}

/// Operation router with active-note tracking (I03).
///
/// * Forwards whitelisted Note / CC (Note On vel 0 -> Note Off).
/// * Tracks notes it turned on, per channel. A second Note On for a sounding
///   note and a Note Off for a note it never turned on are suppressed.
/// * `fault` stops the route and returns the Note Offs for every sounding
///   note (queue overflow, send failure, disconnect). Dropped input is never
///   silent: the caller reports it here.
/// * `release_all` returns the Note Offs without faulting (settings change,
///   shutdown). Never an All Notes Off to whole channels.
#[derive(Clone, Debug)]
pub struct Router {
    rules: RouteRules,
    active: [u128; 16],
    fault: Option<String>,
    pub stats: RouterStats,
}

impl Router {
    pub fn new(rules: RouteRules) -> Router {
        Router { rules, active: [0; 16], fault: None, stats: RouterStats::default() }
    }

    pub fn rules(&self) -> RouteRules {
        self.rules
    }

    pub fn fault_reason(&self) -> Option<&str> {
        self.fault.as_deref()
    }

    pub fn handle(&mut self, bytes: &[u8]) -> RouteOutcome {
        self.stats.received += 1;
        if self.fault.is_some() {
            self.stats.blocked += 1;
            return RouteOutcome::Blocked;
        }
        let (out, normalized) = match route(bytes) {
            Routed::Forward { bytes, normalized } => (bytes, normalized),
            Routed::Ignore => {
                self.stats.ignored += 1;
                return RouteOutcome::Ignored;
            }
        };
        let ch = (out[0] & 0x0F) as usize;
        let kind = out[0] & 0xF0;
        let allowed = self.rules.channels & (1 << ch) != 0 && if kind == 0xB0 { self.rules.cc } else { self.rules.notes };
        if !allowed {
            self.stats.ignored += 1;
            return RouteOutcome::Ignored;
        }
        let bit = 1u128 << out[1];
        match kind {
            0x90 if self.active[ch] & bit != 0 => {
                self.stats.suppressed += 1;
                return RouteOutcome::Suppressed;
            }
            0x90 => self.active[ch] |= bit,
            0x80 if self.active[ch] & bit == 0 => {
                self.stats.suppressed += 1;
                return RouteOutcome::Suppressed;
            }
            0x80 => self.active[ch] &= !bit,
            _ => {}
        }
        self.stats.sent += 1;
        if normalized {
            self.stats.normalized += 1;
        }
        RouteOutcome::Send { bytes: out, normalized }
    }

    /// Sounding notes as (channel 0..15, note).
    pub fn active_notes(&self) -> Vec<(u8, u8)> {
        let mut v = Vec::new();
        for (ch, bits) in self.active.iter().enumerate() {
            for n in 0..128u8 {
                if bits & (1u128 << n) != 0 {
                    v.push((ch as u8, n));
                }
            }
        }
        v
    }

    /// Note Offs for every sounding note; the notes are forgotten.
    pub fn release_all(&mut self) -> Vec<[u8; 3]> {
        let offs: Vec<[u8; 3]> = self.active_notes().into_iter().map(|(ch, n)| [0x80 | ch, n, 0]).collect();
        self.active = [0; 16];
        self.stats.released += offs.len() as u64;
        offs
    }

    /// Stop the route; returns the Note Offs to send (best effort).
    pub fn fault(&mut self, reason: &str) -> Vec<[u8; 3]> {
        if self.fault.is_none() {
            self.stats.faults += 1;
            self.fault = Some(reason.to_string());
        }
        self.release_all()
    }

    /// Leave the fault state (after the cause was handled, e.g. reconnect).
    pub fn reset(&mut self) {
        self.fault = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_tracks_notes_and_suppresses_duplicates() {
        let mut r = Router::new(RouteRules::default());
        assert_eq!(r.handle(&[0x90, 11, 100]), RouteOutcome::Send { bytes: [0x90, 11, 100], normalized: false });
        assert_eq!(r.handle(&[0x90, 11, 90]), RouteOutcome::Suppressed);
        assert_eq!(r.handle(&[0x91, 11, 90]), RouteOutcome::Send { bytes: [0x91, 11, 90], normalized: false });
        assert_eq!(r.active_notes(), vec![(0, 11), (1, 11)]);
        assert_eq!(r.handle(&[0x90, 11, 0]), RouteOutcome::Send { bytes: [0x80, 11, 0], normalized: true });
        assert_eq!(r.handle(&[0x80, 11, 0]), RouteOutcome::Suppressed);
        assert_eq!(r.handle(&[0x80, 50, 0]), RouteOutcome::Suppressed, "never turned on");
        assert_eq!(r.handle(&[0xB0, 1, 2]), RouteOutcome::Send { bytes: [0xB0, 1, 2], normalized: false });
        assert_eq!(r.handle(&[0xA0, 1, 2]), RouteOutcome::Ignored);
        assert_eq!(r.active_notes(), vec![(1, 11)]);
    }

    #[test]
    fn router_whitelist_by_kind_and_channel() {
        let mut r = Router::new(RouteRules { notes: true, cc: false, channels: 0b1 });
        assert_eq!(r.handle(&[0xB0, 1, 2]), RouteOutcome::Ignored);
        assert_eq!(r.handle(&[0x91, 1, 2]), RouteOutcome::Ignored);
        assert!(matches!(r.handle(&[0x90, 1, 2]), RouteOutcome::Send { .. }));
    }

    #[test]
    fn fault_releases_notes_and_blocks_until_reset() {
        let mut r = Router::new(RouteRules::default());
        r.handle(&[0x90, 11, 100]);
        r.handle(&[0x92, 64, 100]);
        assert_eq!(r.fault("input queue overflow"), vec![[0x80, 11, 0], [0x82, 64, 0]]);
        assert_eq!(r.fault_reason(), Some("input queue overflow"));
        assert_eq!(r.handle(&[0x90, 12, 100]), RouteOutcome::Blocked);
        assert!(r.active_notes().is_empty());
        r.reset();
        assert!(matches!(r.handle(&[0x90, 12, 100]), RouteOutcome::Send { .. }));
        assert_eq!(r.release_all(), vec![[0x80, 12, 0]]);
        assert_eq!((r.stats.faults, r.stats.blocked, r.stats.released), (1, 1, 3));
    }

    #[test]
    fn note_on_velocity_zero_becomes_note_off_on_the_same_channel() {
        assert_eq!(route(&[0x90, 60, 0]), Routed::Forward { bytes: [0x80, 60, 0], normalized: true });
        assert_eq!(route(&[0x93, 11, 0]), Routed::Forward { bytes: [0x83, 11, 0], normalized: true });
    }

    #[test]
    fn note_and_cc_pass_unchanged() {
        assert_eq!(route(&[0x90, 11, 100]), Routed::Forward { bytes: [0x90, 11, 100], normalized: false });
        assert_eq!(route(&[0x80, 11, 64]), Routed::Forward { bytes: [0x80, 11, 64], normalized: false });
        assert_eq!(route(&[0xB0, 91, 127]), Routed::Forward { bytes: [0xB0, 91, 127], normalized: false });
        assert_eq!(route(&[0xBF, 19, 0]), Routed::Forward { bytes: [0xBF, 19, 0], normalized: false });
    }

    #[test]
    fn other_messages_are_not_forwarded() {
        assert_eq!(route(&[0xA0, 11, 40]), Routed::Ignore); // poly aftertouch
        assert_eq!(route(&[0xD0, 40]), Routed::Ignore); // channel pressure
        assert_eq!(route(&[0xE0, 0, 64]), Routed::Ignore);
        assert_eq!(route(&[0xC0, 1]), Routed::Ignore);
        assert_eq!(route(&[0xF8]), Routed::Ignore);
        assert_eq!(route(&[0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]), Routed::Ignore);
        assert_eq!(route(&[0x90, 0x80, 1]), Routed::Ignore); // bad data byte
    }
}
