//! Common clock and beat inputs. Decoders take the time as an argument and do
//! no I/O; `bridge_ingress` is the only part that owns a thread.

pub mod bridge;
pub mod bridge_ingress;
pub mod clock;
pub mod mtc;
pub mod mtc_out;
pub mod transport;
