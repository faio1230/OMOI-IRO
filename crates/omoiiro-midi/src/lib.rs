//! OMOI-IRO MIDI layer (docs/03-modules.md).
//!
//! * `ports`: listing and explicit selection (exact name, `#index`, unique
//!   substring; `_exact` variants for anything that sends; duplicate names
//!   need `#index`).
//! * `backend`: the `MidiBackend` contract (M4, I01) and `Ingress` (the
//!   callback only stamps the session monotonic clock, checks the size,
//!   copies and `try_send`s, tagged with role and connection generation);
//!   the `MidiSend` trait.
//! * `midir_backend`: OS ports via midir. `fake`: fake backend for tests.
//! * `routing`: Note / CC forwarding rule (Note On velocity 0 -> Note Off)
//!   and the operation `Router` (whitelist, active notes, fault).
//! * `clock`: the session monotonic clock shared by every connection (and a
//!   fake clock for tests).
//!
//! Moved from tools/p0-probe in M1 (docs/11-implementation-plan.md).

pub mod backend;
pub mod clock;
pub mod fake;
pub mod midir_backend;
pub mod ports;
pub mod routing;
