//! Windows: Media Foundation capture → video thread → Direct3D 11 swap chain in a child window over
//! GPUI's window (docs/spec/v1.md §3.1, §3.3, §3.4; ADR 0001, ADR 0002).

pub mod media;
mod render;
mod surface;

use std::sync::Arc;
use std::thread::JoinHandle;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;

use crate::video::picture::PictureSettings;
use crate::video::timing::{DelaySettings, Summary};
use crate::video::{VideoShared, lock};
use crate::video::slot::FrameSlot;
use media::{DeviceCapture, Frame};

/// The live picture: keeps capture running and the video thread drawing until dropped.
pub struct VideoPipeline {
    capture: Option<DeviceCapture>,
    status: String,
    slot: Arc<FrameSlot<Frame>>,
    shared: Arc<VideoShared>,
    video_thread: Option<JoinHandle<()>>,
}

impl VideoPipeline {
    /// Puts a video surface over GPUI's content in `window` and starts showing the first capture
    /// device. The surface comes up even without a card (it shows black). Main thread only.
    pub fn start(window: &gpui_kit::Window) -> Result<Self, String> {
        let handle = HasWindowHandle::window_handle(window).map_err(|e| format!("no native window: {e}"))?;
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return Err("expected a Win32 window".into());
        };
        let parent = HWND(win32.hwnd.get() as *mut _);
        let surface = surface::attach_over(parent).map_err(|e| format!("video surface: {e}"))?;

        let slot = Arc::new(FrameSlot::new());
        let shared = Arc::new(VideoShared::default());
        let video_thread = {
            let (slot, shared) = (slot.clone(), shared.clone());
            std::thread::Builder::new()
                .name("xreveal-video".into())
                .spawn(move || render::run(surface, &slot, shared))
                .map_err(|e| format!("could not start the video thread: {e}"))?
        };

        let (capture, status) = match start_capture(slot.clone()) {
            Ok(capture) => {
                let status = capture.description();
                (Some(capture), status)
            }
            Err(status) => (None, status),
        };
        Ok(Self { capture, status, slot, shared, video_thread: Some(video_thread) })
    }

    /// Changes how the picture is drawn; applies from the next frame.
    #[expect(dead_code, reason = "called by the Settings drawer (its own slice)")]
    pub fn set_picture(&self, settings: PictureSettings) {
        *lock(&self.shared.picture) = settings;
    }

    /// Changes Buffering / the negative A/V sync offset; applies from the next frame.
    #[expect(dead_code, reason = "called by the Settings drawer (its own slice)")]
    pub fn set_delay(&self, delay: DelaySettings) {
        *lock(&self.shared.delay) = delay;
    }

    /// Timing for the Stats overlay.
    pub fn stats(&self) -> Summary {
        lock(&self.shared.stats).summary(self.slot.replaced_total())
    }

    /// What is being shown, e.g. `USB2 Video · 1280x720 @ 60 · NV12`, or why nothing is.
    pub fn description(&self) -> &str {
        &self.status
    }
}

fn start_capture(slot: Arc<FrameSlot<Frame>>) -> Result<DeviceCapture, String> {
    media::startup().map_err(|e| format!("Media Foundation didn't start: {e}"))?;
    let (device, name) = media::first_device()
        .map_err(|e| format!("Couldn't list capture devices: {e}"))?
        .ok_or("No capture card found.")?;
    DeviceCapture::start(device, name.clone(), slot).map_err(|e| format!("Couldn't open {name}: {e}"))
}

impl Drop for VideoPipeline {
    fn drop(&mut self) {
        if let Some(capture) = &self.capture {
            capture.stop();
        }
        self.slot.close();
        if let Some(thread) = self.video_thread.take() {
            let _ = thread.join();
        }
    }
}
