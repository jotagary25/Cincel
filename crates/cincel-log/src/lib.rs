//! `cincel-log`: file logging setup and login-capture redaction
//! (`docs/specs/06-etapa4-conexiones-y-cincel.md` §8).
//!
//! Kept as its own crate, separate from the `cincel` binary, so both pieces
//! are unit-testable without GPUI: [`init`] sets up `tracing` to write to
//! stderr (as before) and additionally to a daily-rotating file; [`redact`]
//! is the helper the login-capture pty output of `cincel-connections` goes
//! through before it is logged or shown, so a captured link or one-time code
//! never reaches disk.

mod logging;
mod redact;

pub use logging::init;
pub use redact::redact;
