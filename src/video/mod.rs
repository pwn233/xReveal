//! Platform-independent pieces of the frame pipeline (docs/spec/v1.md §3.3).

pub mod fill;
pub mod flat;
#[cfg(any(target_os = "windows", test))]
pub mod mjpeg;
pub mod picture;
pub mod signal;
pub mod slot;
pub mod timing;

use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// What the UI and the video thread share: how to draw, how much to delay, and what happened.
#[derive(Default)]
pub struct VideoShared {
    pub picture: Mutex<picture::PictureSettings>,
    pub delay: Mutex<timing::DelaySettings>,
    pub stats: Mutex<timing::FrameStats>,
    /// How long the picture has been one flat colour (#108).
    pub flat: Mutex<flat::FlatWatch>,
}

/// Locks a mutex, ignoring poisoning (a panicked holder can't leave these in a broken state).
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// When the process started (set first thing in `main`).
pub static LAUNCH: OnceLock<Instant> = OnceLock::new();

/// Launch → the video surface's first presented frame (black is enough). The ≤ 1 s launch budget
/// (docs/spec/v1.md §4) and the CI smoke launch (`XREVEAL_SMOKE_TEST=1`) both read it.
pub static FIRST_PRESENT: OnceLock<Duration> = OnceLock::new();

/// Records the first present; later calls do nothing.
pub fn mark_first_present() {
    let _ = FIRST_PRESENT.set(LAUNCH.get().map_or(Duration::ZERO, Instant::elapsed));
}
