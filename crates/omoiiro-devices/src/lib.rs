//! OMOI-IRO devices (docs/03-modules.md). Device codecs and adapters;
//! no port opening, timers or files.
//!
//! * `launchpad_x`: codec (M1, from tools/p0-probe) and the grid adapter (M4).
//! * `adapter`: `DeviceAdapter` contract and `DeviceFrame` (quantized frame).
//! * `color`: RGB7 quantization.
//! * `simulated`: virtual Launchpad X that applies the adapter's packets.

pub mod adapter;
pub mod color;
pub mod launchpad_x;
pub mod simulated;
