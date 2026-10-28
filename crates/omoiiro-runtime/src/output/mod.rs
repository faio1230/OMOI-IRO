//! Output stages driven by the transport: MTC output (M3), LED writer with
//! diff cache and the latest-frame mailbox (M4).

pub mod led;
pub mod identify;
pub mod mailbox;
pub mod mtc;
pub mod reconnect;
