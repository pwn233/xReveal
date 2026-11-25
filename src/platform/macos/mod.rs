//! macOS: AVFoundation capture → video thread → Metal layer under GPUI's view
//! (docs/spec/v1.md §3.1, §3.3, §3.4; ADR 0001).

mod capture;
mod render;
mod surface;

use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::video::picture::PictureSettings;
use crate::video::slot::FrameSlot;
use capture::{Capture, PixelBuffer};

/// The live picture: keeps capture running and the video thread drawing until dropped.
pub struct VideoPipeline {
    capture: Capture,
    slot: Arc<FrameSlot<PixelBuffer>>,
    settings: Arc<Mutex<PictureSettings>>,
    video_thread: Option<JoinHandle<()>>,
}

impl VideoPipeline {
    /// Puts a video surface under GPUI's content in `window` and starts showing the first capture
    /// device. Must be called on the main thread.
    pub fn start(window: &gpui_kit::Window) -> Result<Self, String> {
        let handle = HasWindowHandle::window_handle(window).map_err(|e| format!("no native window: {e}"))?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return Err("expected an AppKit window".into());
        };
        let target = surface::attach_below(appkit.ns_view)?;

        let slot = Arc::new(FrameSlot::new());
        let settings = Arc::new(Mutex::new(PictureSettings::default()));
        let video_thread = {
            let (slot, settings) = (slot.clone(), settings.clone());
            std::thread::Builder::new()
                .name("xreveal-video".into())
                .spawn(move || render::run(target, &slot, settings))
                .map_err(|e| format!("could not start the video thread: {e}"))?
        };
        let capture = Capture::start(slot.clone())?;
        Ok(Self { capture, slot, settings, video_thread: Some(video_thread) })
    }

    /// Changes how the picture is drawn; applies from the next frame.
    #[expect(dead_code, reason = "called by the Settings drawer (its own slice)")]
    pub fn set_picture(&self, settings: PictureSettings) {
        *self.settings.lock().unwrap_or_else(PoisonError::into_inner) = settings;
    }

    /// A one-line description of what is being shown, e.g. `USB2 Video · 1280x720 @ 60 · 420v`.
    pub fn description(&self) -> &str {
        &self.capture.description
    }
}

impl Drop for VideoPipeline {
    fn drop(&mut self) {
        self.capture.stop();
        self.slot.close();
        if let Some(thread) = self.video_thread.take() {
            let _ = thread.join();
        }
    }
}
