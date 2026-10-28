//! OMOI-IRO runtime (docs/03-modules.md, docs/11-implementation-plan.md).
//!
//! * `sync`: time-taking pure state machines for the common clock and beat
//!   inputs: the OmoiIroBridge decoder (first source of the common clock), the
//!   MTC decoder (external MTC and self-check), the MIDI Clock decoder, the
//!   MTC generator, the bridge UDP ingress and `transport` (M3).
//! * `link`: Ableton Link observation (feature `link`; never enables Start
//!   Stop Sync, never commits).
//! * `scheduler`: render frame timing on the monotonic clock (M4, T05).
//! * `engine`: show replacement at frame boundaries and overrides (T06).
//! * `output`: MTC sender (M3), LED writer with diff cache (I05) and the
//!   latest-frame mailbox (T05).
//! * `connections`: port bindings, reconnect and fault states (I06).
//! * `control`: fixed Trigger / Action and the operation route (I03, I07).
//! * `follow`: headless follow mode (bridge -> Transport -> Core -> LEDs).
//! * `preview`: the UI's local preview clock and renderer (never external).
//! * `settings`: terminal settings (port roles, bridge address) and their checks.
//! * `session`: a running follow session (start / status / stop) for the
//!   CLI and the desktop shell (M5).
//! * `diagnostics`: JSONL record format, replay, report formatting, the
//!   bridge / mtc-out analyses, timing histograms (T07) and NFR03.
//! * `clock`: the session monotonic clock (re-exported from `omoiiro-midi`).
//!
//! Moved from tools/p0-probe in M1; M3 added the transport, M4 the rest.

pub use omoiiro_midi::clock;

pub mod connections;
pub mod control;
pub mod diagnostics;
pub mod engine;
pub mod follow;
pub mod link;
pub mod output;
pub mod preview;
pub mod scheduler;
pub mod session;
pub mod settings;
pub mod sync;
