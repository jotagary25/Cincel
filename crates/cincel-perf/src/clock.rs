//! The single time base of the bench: `CLOCK_MONOTONIC`, the clock sway
//! stamps screencopy frames with (§3.2).

use rustix::time::{ClockId, clock_gettime};

/// Monotonic time, nanoseconds.
pub fn now_ns() -> u64 {
    let now = clock_gettime(ClockId::Monotonic);
    now.tv_sec as u64 * 1_000_000_000 + now.tv_nsec as u64
}

/// Milliseconds between two marks (`later` - `earlier`), two decimals.
pub fn ms_between(earlier: u64, later: u64) -> f64 {
    crate::stats::round2((later as f64 - earlier as f64) / 1e6)
}
