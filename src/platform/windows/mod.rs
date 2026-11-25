//! Windows: Media Foundation capture → video thread → Direct3D 11 swap chain in a child window over
//! GPUI's window (docs/spec/v1.md §3.1, §3.3, §3.4; ADR 0001, ADR 0002).

pub mod media;
mod render;
mod surface;

use std::sync::Arc;
use std::thread::JoinHandle;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;

use crate::source::{self, SourceInfo, VideoFormat};
use crate::video::picture::PictureSettings;
use crate::video::slot::FrameSlot;
use crate::video::timing::{DelaySettings, Summary};
use crate::video::{VideoShared, lock};
use media::{DeviceCapture, Frame};

/// The live picture: the video surface and thread live as long as this does; the capture can be
/// switched to another Source or Video format without recreating them.
pub struct VideoPipeline {
    capture: Option<DeviceCapture>,
    status: String,
    sources: Vec<SourceInfo>,
    slot: Arc<FrameSlot<Frame>>,
    shared: Arc<VideoShared>,
    video_thread: Option<JoinHandle<()>>,
}

impl VideoPipeline {
    /// Puts a video surface over GPUI's content in `window` and starts showing a Source: the
    /// first device at its default Video format. The surface comes up even without a card (it
    /// shows black). Main thread only.
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

        let started = media::startup();
        let mut pipeline = Self {
            capture: None,
            status: "No capture card found.".into(),
            sources: if started.is_ok() { media::list_sources() } else { Vec::new() },
            slot,
            shared,
            video_thread: Some(video_thread),
        };
        if let Err(error) = started {
            pipeline.status = format!("Media Foundation didn't start: {error}");
        } else if let Some((index, choice, _)) = source::initial_pick(&pipeline.sources) {
            let os_id = pipeline.sources[index].identity.os_id.clone();
            if let Err(error) = pipeline.switch_to(&os_id, choice) {
                pipeline.status = error;
            }
        }
        Ok(pipeline)
    }

    /// Every capture device and its formats, as listed at start or the last [`refresh_sources`].
    #[expect(dead_code, reason = "listed by the Settings drawer and the pickers (their own slices)")]
    pub fn sources(&self) -> &[SourceInfo] {
        &self.sources
    }

    /// Lists the capture devices again (e.g. after a card is plugged in).
    #[expect(dead_code, reason = "called by the pickers (their own slices)")]
    pub fn refresh_sources(&mut self) {
        self.sources = media::list_sources();
    }

    /// Shows Source `os_id` at `choice`, replacing the current capture. The video surface stays;
    /// the picture restarts briefly (docs/spec/v1.md §2.3).
    pub fn switch_to(&mut self, os_id: &str, choice: VideoFormat) -> Result<(), String> {
        let info = self.sources.iter().find(|s| s.identity.os_id == os_id).ok_or("That capture card isn't connected.")?;
        let native = source::best_native(&info.formats, choice).ok_or(format!("{} doesn't offer {choice}.", info.identity.name))?;
        let name = info.identity.name.clone();
        if let Some(old) = self.capture.take() {
            old.stop();
        }
        let capture = DeviceCapture::start(os_id, name.clone(), native, self.slot.clone()).map_err(|e| format!("Couldn't open {name}: {e}"))?;
        self.status = capture.description();
        self.capture = Some(capture);
        Ok(())
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
