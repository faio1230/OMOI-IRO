//! Device identification before LED output (M6 preparation; docs/02 "MIDI接続
//! のライフサイクル": Connecting -> Identifying -> Configuring -> Ready).
//!
//! A pure state machine driven by `now`: the caller sends the packets it
//! returns and feeds the SysEx replies that arrive on the device's input.
//!
//! * Identifying: Universal Device Inquiry. A Launchpad X reply moves on; a
//!   Launchpad X bootloader, another Novation device or another maker is a
//!   fault with the reason (nothing else is sent to it).
//! * Configuring: select Programmer Mode and query the mode; a reply of 1
//!   (programmer) is Ready.
//! * No reply within `reply_timeout_us`: the request is repeated up to
//!   `max_tries` times, then fault ("no reply").
//!
//! LED frames are written only in Ready (the LED loop checks `ready()`).

use omoiiro_devices::launchpad_x::{mode_query, mode_select, parse_identity, parse_mode, Identity, IdentityKind, Mode, DEVICE_INQUIRY};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentifyState {
    Identifying,
    Configuring,
    Ready,
    Fault,
}

impl IdentifyState {
    pub fn name(self) -> &'static str {
        match self {
            IdentifyState::Identifying => "identifying",
            IdentifyState::Configuring => "configuring",
            IdentifyState::Ready => "ready",
            IdentifyState::Fault => "fault",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct IdentifyConfig {
    pub reply_timeout_us: i64,
    pub max_tries: u32,
}

impl Default for IdentifyConfig {
    fn default() -> Self {
        IdentifyConfig { reply_timeout_us: 1_000_000, max_tries: 3 }
    }
}

#[derive(Clone, Debug)]
pub struct DeviceIdentify {
    cfg: IdentifyConfig,
    state: IdentifyState,
    sent_at: i64,
    tries: u32,
    pub identity: Option<Identity>,
    pub fault: Option<String>,
}

impl DeviceIdentify {
    pub fn new(cfg: IdentifyConfig) -> DeviceIdentify {
        DeviceIdentify { cfg, state: IdentifyState::Identifying, sent_at: i64::MIN, tries: 0, identity: None, fault: None }
    }

    pub fn state(&self) -> IdentifyState {
        self.state
    }

    pub fn ready(&self) -> bool {
        self.state == IdentifyState::Ready
    }

    /// A (new) connection: start over. Returns the packets to send.
    pub fn start(&mut self, now: i64) -> Vec<Vec<u8>> {
        self.state = IdentifyState::Identifying;
        self.identity = None;
        self.fault = None;
        self.tries = 1;
        self.sent_at = now;
        vec![DEVICE_INQUIRY.to_vec()]
    }

    fn request(&self) -> Vec<Vec<u8>> {
        match self.state {
            IdentifyState::Identifying => vec![DEVICE_INQUIRY.to_vec()],
            IdentifyState::Configuring => vec![mode_select(Mode::Programmer), mode_query()],
            _ => Vec::new(),
        }
    }

    fn fail(&mut self, reason: String) -> Vec<Vec<u8>> {
        self.state = IdentifyState::Fault;
        self.fault = Some(reason);
        Vec::new()
    }

    /// A message from the device's input (only SysEx matters).
    pub fn on_input(&mut self, bytes: &[u8], now: i64) -> Vec<Vec<u8>> {
        match self.state {
            IdentifyState::Identifying => match parse_identity(bytes) {
                Some(id) => {
                    let kind = id.kind;
                    let text = format!("{} {}", kind.name(), id.version_text());
                    self.identity = Some(id);
                    match kind {
                        IdentityKind::LaunchpadX => {
                            self.state = IdentifyState::Configuring;
                            self.tries = 1;
                            self.sent_at = now;
                            self.request()
                        }
                        IdentityKind::LaunchpadXBootloader => self.fail(format!("Launchpad X がブートローダーで起動している（{text}）。通常起動し直す")),
                        _ => self.fail(format!("Launchpad X ではない機器が応答した（{text}）。ポートの選択を確かめる")),
                    }
                }
                None => Vec::new(),
            },
            IdentifyState::Configuring => match parse_mode(bytes) {
                Some(1) => {
                    self.state = IdentifyState::Ready;
                    Vec::new()
                }
                // A mode reply that is not Programmer: wait for the retry.
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// Timeouts and retries. Returns the packets to send (if any).
    pub fn poll(&mut self, now: i64) -> Vec<Vec<u8>> {
        if !matches!(self.state, IdentifyState::Identifying | IdentifyState::Configuring) || now - self.sent_at < self.cfg.reply_timeout_us {
            return Vec::new();
        }
        if self.tries >= self.cfg.max_tries {
            let what = if self.state == IdentifyState::Identifying { "Device Inquiry" } else { "Programmer Mode の確認" };
            return self.fail(format!("{what} に {} 回応答がない（機器の電源・ポート・モードを確かめる）", self.tries));
        }
        self.tries += 1;
        self.sent_at = now;
        self.request()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lpx_reply(family_lo: u8) -> Vec<u8> {
        vec![0xF0, 0x7E, 0x00, 0x06, 0x02, 0x00, 0x20, 0x29, 0x03, family_lo, 0x00, 0x00, 0, 4, 0, 7, 0xF7]
    }

    #[test]
    fn launchpad_x_goes_to_ready() {
        let mut d = DeviceIdentify::new(IdentifyConfig::default());
        assert_eq!(d.start(0), vec![DEVICE_INQUIRY.to_vec()]);
        assert!(d.on_input(&[0x90, 11, 1], 10).is_empty(), "pad input is not a reply");
        assert_eq!(d.on_input(&lpx_reply(0x01), 100), vec![mode_select(Mode::Programmer), mode_query()]);
        assert_eq!(d.state(), IdentifyState::Configuring);
        assert!(d.on_input(&mode_select(Mode::Live), 200).is_empty());
        assert_eq!(d.state(), IdentifyState::Configuring, "live mode reply: not yet");
        d.on_input(&mode_select(Mode::Programmer), 300);
        assert!(d.ready());
        assert_eq!(d.identity.as_ref().unwrap().version_text(), "0.4.0.7");
    }

    #[test]
    fn wrong_devices_and_bootloader_fault_with_reasons() {
        let mut d = DeviceIdentify::new(IdentifyConfig::default());
        d.start(0);
        assert!(d.on_input(&lpx_reply(0x11), 1).is_empty());
        assert_eq!(d.state(), IdentifyState::Fault);
        assert!(d.fault.as_ref().unwrap().contains("ブートローダー"));
        d.start(10);
        let mut mini = lpx_reply(0x01);
        mini[8] = 0x13;
        d.on_input(&mini, 11);
        assert!(d.fault.as_ref().unwrap().contains("Launchpad X ではない"));
        d.start(20);
        d.on_input(&[0xF0, 0x7E, 0x10, 0x06, 0x02, 0x41, 0x01, 0x02, 0x03, 0x04, 0x00, 0xF7], 21);
        assert_eq!(d.state(), IdentifyState::Fault);
    }

    #[test]
    fn no_reply_retries_then_faults() {
        let mut d = DeviceIdentify::new(IdentifyConfig { reply_timeout_us: 100, max_tries: 3 });
        d.start(0);
        assert!(d.poll(50).is_empty());
        assert_eq!(d.poll(100), vec![DEVICE_INQUIRY.to_vec()]);
        assert_eq!(d.poll(200), vec![DEVICE_INQUIRY.to_vec()]);
        assert!(d.poll(300).is_empty());
        assert_eq!(d.state(), IdentifyState::Fault);
        assert!(d.fault.as_ref().unwrap().contains("3 回応答がない"));
        // Configuring retries resend both packets.
        d.start(1000);
        d.on_input(&lpx_reply(0x01), 1000);
        assert_eq!(d.poll(1100), vec![mode_select(Mode::Programmer), mode_query()]);
    }
}
