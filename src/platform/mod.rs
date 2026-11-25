//! Platform-specific capture and video surfaces. Each OS exposes the same `VideoPipeline`.

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub use macos::VideoPipeline;
#[cfg(target_os = "windows")]
pub use windows::VideoPipeline;
