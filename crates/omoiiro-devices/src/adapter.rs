//! Device adapter contract (docs/03 "DeviceAdapter") and the device frame.
//!
//! An adapter turns Core frames into device packets; it never owns a port.
//! The Runtime's LED writer sends the packets and keeps the last frame that
//! was sent successfully as the diff base.

use crate::color::quantize_rgb7;
use omoiiro_core::surface::{Frame, GRID_LEDS};

/// Device-side colors after quantization (RGB 0..127 per LED, index = `LedId::index`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceFrame {
    pub rgb7: [[u8; 3]; GRID_LEDS],
}

impl Default for DeviceFrame {
    fn default() -> Self {
        DeviceFrame { rgb7: [[0; 3]; GRID_LEDS] }
    }
}

impl DeviceFrame {
    /// Quantize a Core frame. `gain` (0..=1) is the final output gain
    /// (transport fade, overrides) applied before quantization.
    pub fn from_frame(frame: &Frame, gain: f32) -> DeviceFrame {
        let g = gain.clamp(0.0, 1.0);
        let mut out = DeviceFrame::default();
        for (o, c) in out.rgb7.iter_mut().zip(frame.leds.iter()) {
            *o = quantize_rgb7(c.scale(g));
        }
        out
    }

    pub fn black() -> DeviceFrame {
        DeviceFrame::default()
    }

    /// Indices whose color differs from `other`.
    pub fn changed_from(&self, other: &DeviceFrame) -> Vec<usize> {
        (0..GRID_LEDS).filter(|&i| self.rgb7[i] != other.rgb7[i]).collect()
    }

    pub fn lit(&self) -> usize {
        self.rgb7.iter().filter(|c| **c != [0, 0, 0]).count()
    }
}

/// What a device needs from the LED writer (docs/03).
pub trait DeviceAdapter: Send {
    fn profile(&self) -> &'static str;
    /// Packets sent once after the port opens (e.g. select Programmer Mode).
    fn initialization(&self) -> Vec<Vec<u8>>;
    /// Packets for `frame`. With `last_sent`, only changed LEDs; without it,
    /// every LED (full refresh). Empty when nothing changed.
    fn encode_frame(&self, frame: &DeviceFrame, last_sent: Option<&DeviceFrame>) -> Vec<Vec<u8>>;
    /// Packets before closing (all LEDs off, device mode restored).
    fn shutdown(&self) -> Vec<Vec<u8>>;
}
