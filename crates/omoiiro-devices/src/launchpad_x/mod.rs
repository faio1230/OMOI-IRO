//! Launchpad X codec: SysEx builders and reply / input parsers (Programmer's
//! Reference values; the Device Inquiry family code 03 01 / bootloader 03 11
//! is still to be confirmed on the device). Pure, no MIDI I/O. The check
//! session that opens ports lives in `apps/probe` (`lpx-check`).
//!
//! Moved from tools/p0-probe/src/lpx.rs in M1.

pub const DEVICE_INQUIRY: [u8; 6] = [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7];
/// Novation manufacturer ID, Launchpad X device ID.
pub const HEADER: [u8; 6] = [0xF0, 0x00, 0x20, 0x29, 0x02, 0x0C];
const NOVATION: [u8; 3] = [0x00, 0x20, 0x29];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Live,
    Programmer,
}

impl Mode {
    pub fn code(self) -> u8 {
        match self {
            Mode::Live => 0,
            Mode::Programmer => 1,
        }
    }
}

pub fn mode_name(code: u8) -> &'static str {
    match code {
        0 => "live",
        1 => "programmer",
        _ => "unknown",
    }
}

pub fn mode_query() -> Vec<u8> {
    let mut v = HEADER.to_vec();
    v.extend_from_slice(&[0x0E, 0xF7]);
    v
}

pub fn mode_select(mode: Mode) -> Vec<u8> {
    let mut v = HEADER.to_vec();
    v.extend_from_slice(&[0x0E, mode.code(), 0xF7]);
    v
}

/// Programmer Mode pad / button index: rows 1..9, columns 1..9 (11..99).
pub fn valid_index(index: u8) -> bool {
    (1..=9).contains(&(index / 10)) && (1..=9).contains(&(index % 10))
}

/// LED lighting SysEx, lighting type 3 (RGB), components 0..127.
pub fn led_rgb(index: u8, r: u8, g: u8, b: u8) -> Result<Vec<u8>, String> {
    if !valid_index(index) {
        return Err(format!("pad index {index} is not a Programmer Mode index (11..99, no 0 digit)"));
    }
    if r > 0x7F || g > 0x7F || b > 0x7F {
        return Err(format!("RGB components must be 0..127, got {r}/{g}/{b}"));
    }
    let mut v = HEADER.to_vec();
    v.extend_from_slice(&[0x03, 0x03, index, r, g, b, 0xF7]);
    Ok(v)
}

/// Programmer Mode note of a grid LED (`LedId::index`, bottom-left origin):
/// `(y + 1) * 10 + (x + 1)`.
pub fn led_index_to_pad(i: usize) -> u8 {
    debug_assert!(i < 64);
    ((i / 8 + 1) * 10 + (i % 8 + 1)) as u8
}

/// Inverse of `led_index_to_pad` for the 8 x 8 grid (11..88, digits 1..8).
pub fn pad_to_led_index(pad: u8) -> Option<usize> {
    let (row, col) = (pad / 10, pad % 10);
    ((1..=8).contains(&row) && (1..=8).contains(&col)).then(|| (row as usize - 1) * 8 + col as usize - 1)
}

/// One LED SysEx with several RGB specs (lighting type 3). Pads must be
/// valid Programmer Mode indices and components 0..127 (checked by callers).
pub fn led_rgb_many(specs: &[(u8, [u8; 3])]) -> Vec<u8> {
    let mut v = Vec::with_capacity(HEADER.len() + 2 + specs.len() * 5);
    v.extend_from_slice(&HEADER);
    v.push(0x03);
    for (pad, [r, g, b]) in specs {
        v.extend_from_slice(&[0x03, *pad, *r, *g, *b]);
    }
    v.push(0xF7);
    v
}

/// Launchpad X 8 x 8 grid adapter (`novation.launchpad_x.grid64.v1`).
/// A frame is one SysEx carrying only the changed pads (at most 64 specs,
/// 328 bytes), so a frame is never split and generations never mix.
#[derive(Clone, Copy, Debug, Default)]
pub struct LaunchpadXAdapter;

impl crate::adapter::DeviceAdapter for LaunchpadXAdapter {
    fn profile(&self) -> &'static str {
        omoiiro_core::surface::GRID64_PROFILE
    }

    fn initialization(&self) -> Vec<Vec<u8>> {
        vec![mode_select(Mode::Programmer)]
    }

    fn encode_frame(&self, frame: &crate::adapter::DeviceFrame, last_sent: Option<&crate::adapter::DeviceFrame>) -> Vec<Vec<u8>> {
        let idx: Vec<usize> = match last_sent {
            Some(last) => frame.changed_from(last),
            None => (0..64).collect(),
        };
        if idx.is_empty() {
            return Vec::new();
        }
        let specs: Vec<(u8, [u8; 3])> = idx.iter().map(|&i| (led_index_to_pad(i), frame.rgb7[i].map(|v| v.min(0x7F)))).collect();
        vec![led_rgb_many(&specs)]
    }

    fn shutdown(&self) -> Vec<Vec<u8>> {
        let mut v = self.encode_frame(&crate::adapter::DeviceFrame::black(), None);
        v.push(mode_select(Mode::Live));
        v
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityKind {
    LaunchpadX,
    LaunchpadXBootloader,
    OtherNovation,
    OtherManufacturer,
}

impl IdentityKind {
    pub fn name(self) -> &'static str {
        match self {
            IdentityKind::LaunchpadX => "launchpad_x",
            IdentityKind::LaunchpadXBootloader => "launchpad_x_bootloader",
            IdentityKind::OtherNovation => "other_novation",
            IdentityKind::OtherManufacturer => "other_manufacturer",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub device_id: u8,
    pub manufacturer: Vec<u8>,
    pub family: [u8; 2],
    pub model: [u8; 2],
    /// Firmware version bytes as sent (4 bytes on Launchpad X).
    pub version: Vec<u8>,
    pub kind: IdentityKind,
}

impl Identity {
    /// Version as text: each byte as a decimal digit group, e.g. `0.4.0.7`.
    pub fn version_text(&self) -> String {
        self.version.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(".")
    }
}

/// Parse a Universal Device Inquiry reply (F0 7E dd 06 02 mm.. ff ff pp pp vv.. F7).
/// Family 03 01 is the Launchpad X application, 03 11 its bootloader (from
/// the Launchpad X Programmer's Reference; to be confirmed on the device).
pub fn parse_identity(bytes: &[u8]) -> Option<Identity> {
    if bytes.len() < 6 || bytes[0] != 0xF0 || bytes[1] != 0x7E || bytes[3] != 0x06 || bytes[4] != 0x02 {
        return None;
    }
    if *bytes.last()? != 0xF7 {
        return None;
    }
    let body = &bytes[5..bytes.len() - 1];
    let (manufacturer, rest) = if body.first() == Some(&0x00) {
        if body.len() < 3 {
            return None;
        }
        (body[..3].to_vec(), &body[3..])
    } else {
        (body.get(..1)?.to_vec(), body.get(1..)?)
    };
    if rest.len() < 4 {
        return None;
    }
    let family = [rest[0], rest[1]];
    let model = [rest[2], rest[3]];
    let version = rest[4..].to_vec();
    let kind = if manufacturer != NOVATION {
        IdentityKind::OtherManufacturer
    } else {
        match family {
            [0x03, 0x01] => IdentityKind::LaunchpadX,
            [0x03, 0x11] => IdentityKind::LaunchpadXBootloader,
            _ => IdentityKind::OtherNovation,
        }
    };
    Some(Identity { device_id: bytes[2], manufacturer, family, model, version, kind })
}

/// Parse a mode reply (F0 00 20 29 02 0C 0E mm F7).
pub fn parse_mode(bytes: &[u8]) -> Option<u8> {
    if bytes.len() == 9 && bytes[..6] == HEADER && bytes[6] == 0x0E && bytes[8] == 0xF7 && bytes[7] <= 0x7F {
        Some(bytes[7])
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadEvent {
    Press { note: bool, index: u8, value: u8 },
    Release { note: bool, index: u8 },
}

/// Pad (Note) and button (CC) press / release. Note On velocity 0 is a release.
pub fn parse_pad(bytes: &[u8]) -> Option<PadEvent> {
    if bytes.len() != 3 {
        return None;
    }
    let (index, value) = (bytes[1], bytes[2]);
    match bytes[0] & 0xF0 {
        0x90 if value > 0 => Some(PadEvent::Press { note: true, index, value }),
        0x90 | 0x80 => Some(PadEvent::Release { note: true, index }),
        0xB0 if value > 0 => Some(PadEvent::Press { note: false, index, value }),
        0xB0 => Some(PadEvent::Release { note: false, index }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sysex_builders_match_the_runbook() {
        assert_eq!(DEVICE_INQUIRY, [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]);
        assert_eq!(mode_query(), vec![0xF0, 0x00, 0x20, 0x29, 0x02, 0x0C, 0x0E, 0xF7]);
        assert_eq!(mode_select(Mode::Programmer), vec![0xF0, 0x00, 0x20, 0x29, 0x02, 0x0C, 0x0E, 0x01, 0xF7]);
        assert_eq!(mode_select(Mode::Live), vec![0xF0, 0x00, 0x20, 0x29, 0x02, 0x0C, 0x0E, 0x00, 0xF7]);
        assert_eq!(
            led_rgb(11, 10, 0, 0).unwrap(),
            vec![0xF0, 0x00, 0x20, 0x29, 0x02, 0x0C, 0x03, 0x03, 11, 10, 0, 0, 0xF7]
        );
    }

    #[test]
    fn grid_index_and_pad_notes() {
        assert_eq!(led_index_to_pad(0), 11); // bottom-left
        assert_eq!(led_index_to_pad(7), 18);
        assert_eq!(led_index_to_pad(56), 81); // top-left
        assert_eq!(led_index_to_pad(63), 88);
        for i in 0..64 {
            assert_eq!(pad_to_led_index(led_index_to_pad(i)), Some(i));
        }
        assert_eq!(pad_to_led_index(19), None); // right-hand CC column
        assert_eq!(pad_to_led_index(91), None); // top row buttons
    }

    #[test]
    fn frame_golden_bytes_full_diff_and_shutdown() {
        use crate::adapter::{DeviceAdapter, DeviceFrame};
        use crate::simulated::VirtualLpx;
        let a = LaunchpadXAdapter;
        assert_eq!(a.initialization(), vec![mode_select(Mode::Programmer)]);
        let mut f = DeviceFrame::black();
        f.rgb7[0] = [127, 0, 0];
        f.rgb7[63] = [0, 10, 127];
        let full = a.encode_frame(&f, None);
        assert_eq!(full.len(), 1);
        assert_eq!(full[0].len(), 6 + 1 + 64 * 5 + 1);
        assert_eq!(&full[0][..12], &[0xF0, 0x00, 0x20, 0x29, 0x02, 0x0C, 0x03, 0x03, 11, 127, 0, 0]);
        let mut g = f;
        g.rgb7[63] = [0, 0, 0];
        assert_eq!(a.encode_frame(&g, Some(&f)), vec![vec![0xF0, 0x00, 0x20, 0x29, 0x02, 0x0C, 0x03, 0x03, 88, 0, 0, 0, 0xF7]]);
        assert!(a.encode_frame(&g, Some(&g)).is_empty());

        let mut v = VirtualLpx::new();
        assert!(v.apply(&full[0]).is_err(), "LED data before Programmer Mode");
        for p in a.initialization().iter().chain(full.iter()) {
            v.apply(p).unwrap();
        }
        assert_eq!(v.frame, f);
        for p in a.shutdown() {
            v.apply(&p).unwrap();
        }
        assert_eq!((v.frame, v.mode), (DeviceFrame::black(), Some(0)));
        assert!(v.apply(&led_rgb(19, 1, 1, 1).unwrap()).is_err(), "only grid pads are frame LEDs");
    }

    #[test]
    fn led_rgb_rejects_bad_index_and_values() {
        assert!(led_rgb(10, 1, 1, 1).is_err());
        assert!(led_rgb(0, 1, 1, 1).is_err());
        assert!(led_rgb(100, 1, 1, 1).is_err());
        assert!(led_rgb(99, 1, 1, 1).is_ok()); // logo LED
        assert!(led_rgb(11, 128, 0, 0).is_err());
    }

    fn lpx_reply(version: [u8; 4]) -> Vec<u8> {
        let mut v = vec![0xF0, 0x7E, 0x00, 0x06, 0x02, 0x00, 0x20, 0x29, 0x03, 0x01, 0x00, 0x00];
        v.extend_from_slice(&version);
        v.push(0xF7);
        v
    }

    #[test]
    fn identity_reply_is_parsed() {
        let id = parse_identity(&lpx_reply([0, 4, 0, 7])).unwrap();
        assert_eq!(id.kind, IdentityKind::LaunchpadX);
        assert_eq!(id.version_text(), "0.4.0.7");
        assert_eq!(id.family, [0x03, 0x01]);
        let mut boot = lpx_reply([0, 0, 0, 1]);
        boot[9] = 0x11;
        assert_eq!(parse_identity(&boot).unwrap().kind, IdentityKind::LaunchpadXBootloader);
        let mut mini = lpx_reply([0, 0, 0, 1]);
        mini[8] = 0x13;
        assert_eq!(parse_identity(&mini).unwrap().kind, IdentityKind::OtherNovation);
        let other = [0xF0, 0x7E, 0x10, 0x06, 0x02, 0x41, 0x01, 0x02, 0x03, 0x04, 0x00, 0xF7];
        assert_eq!(parse_identity(&other).unwrap().kind, IdentityKind::OtherManufacturer);
        assert_eq!(parse_identity(&DEVICE_INQUIRY), None); // our own request
        assert_eq!(parse_identity(&[0xF0, 0x7E, 0x00, 0x06, 0x02, 0xF7]), None);
        assert_eq!(parse_identity(&[0x90, 11, 1]), None);
    }

    #[test]
    fn mode_reply_is_parsed() {
        assert_eq!(parse_mode(&mode_select(Mode::Live)), Some(0));
        assert_eq!(parse_mode(&mode_select(Mode::Programmer)), Some(1));
        assert_eq!(parse_mode(&mode_query()), None);
        assert_eq!(parse_mode(&[0xF0, 0x00, 0x20, 0x29, 0x02, 0x0D, 0x0E, 0x01, 0xF7]), None);
    }

    #[test]
    fn pad_messages_are_parsed_with_note_on_zero_as_release() {
        assert_eq!(parse_pad(&[0x90, 11, 64]), Some(PadEvent::Press { note: true, index: 11, value: 64 }));
        assert_eq!(parse_pad(&[0x90, 11, 0]), Some(PadEvent::Release { note: true, index: 11 }));
        assert_eq!(parse_pad(&[0x80, 11, 64]), Some(PadEvent::Release { note: true, index: 11 }));
        assert_eq!(parse_pad(&[0xB0, 91, 127]), Some(PadEvent::Press { note: false, index: 91, value: 127 }));
        assert_eq!(parse_pad(&[0xB0, 91, 0]), Some(PadEvent::Release { note: false, index: 91 }));
        assert_eq!(parse_pad(&[0xA0, 11, 30]), None);
    }
}
