//! Platform-independent pieces of the frame pipeline (docs/spec/v1.md §3.3).

pub mod fill;
pub mod picture;
pub mod slot;

use std::sync::atomic::AtomicBool;

/// Set once the video surface has presented its first frame (black is enough). The smoke launch
/// (`XREVEAL_SMOKE_TEST=1`) waits for it to prove the surface works on a machine with no card.
pub static FIRST_PRESENT: AtomicBool = AtomicBool::new(false);
