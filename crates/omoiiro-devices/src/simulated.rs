//! Virtual Launchpad X: applies the packets the adapter produces and keeps
//! the resulting pad colors and mode. Used by tests and by headless follow
//! mode instead of a device. Rejects anything a real LPX would not accept as
//! LED data, so golden-byte mistakes show up without hardware.

use crate::adapter::DeviceFrame;
use crate::launchpad_x::{led_index_to_pad, pad_to_led_index, HEADER};
use omoiiro_core::surface::GRID_H;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VirtualLpx {
    pub frame: DeviceFrame,
    /// Last selected mode code (None until a mode select arrives).
    pub mode: Option<u8>,
    pub packets: u64,
    pub led_specs: u64,
}

impl VirtualLpx {
    pub fn new() -> VirtualLpx {
        VirtualLpx::default()
    }

    /// Apply one packet. Returns the number of LED specs applied.
    pub fn apply(&mut self, bytes: &[u8]) -> Result<usize, String> {
        if bytes.len() < 8 || bytes[..6] != HEADER || *bytes.last().unwrap() != 0xF7 {
            return Err(format!("not a Launchpad X SysEx: {bytes:02X?}"));
        }
        let body = &bytes[6..bytes.len() - 1];
        self.packets += 1;
        match body[0] {
            0x0E if body.len() == 2 => {
                self.mode = Some(body[1]);
                Ok(0)
            }
            0x03 => {
                let specs = &body[1..];
                if specs.len() % 5 != 0 {
                    return Err(format!("LED specs are not 5-byte RGB entries: {} bytes", specs.len()));
                }
                if self.mode != Some(1) {
                    return Err("LED data before Programmer Mode was selected".into());
                }
                for s in specs.chunks(5) {
                    if s[0] != 0x03 {
                        return Err(format!("lighting type {} (only RGB = 3 is used)", s[0]));
                    }
                    if s[2..].iter().any(|v| *v > 0x7F) {
                        return Err(format!("RGB component > 127 in {s:02X?}"));
                    }
                    let i = pad_to_led_index(s[1]).ok_or_else(|| format!("pad {} is not on the 8x8 grid", s[1]))?;
                    self.frame.rgb7[i] = [s[2], s[3], s[4]];
                }
                self.led_specs += (specs.len() / 5) as u64;
                Ok(specs.len() / 5)
            }
            other => Err(format!("unexpected Launchpad X command {other:#04X}")),
        }
    }

    /// Text view, top row first: `.` off, `#` lit.
    pub fn ascii(&self) -> String {
        let mut s = String::new();
        for y in (0..GRID_H).rev() {
            for x in 0..8 {
                let i = y * 8 + x;
                debug_assert_eq!(pad_to_led_index(led_index_to_pad(i)), Some(i));
                s.push(if self.frame.rgb7[i] == [0, 0, 0] { '.' } else { '#' });
            }
            s.push('\n');
        }
        s
    }
}
