//! macOS: AVFoundation capture → video thread → Metal layer under GPUI's view
//! (docs/spec/v1.md §3.1, §3.3, §3.4; ADR 0001).

mod capture;
mod render;
mod surface;

use std::sync::Arc;
use std::thread::JoinHandle;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::source::{self, LaunchPick, SourceIdentity, SourceInfo, VideoFormat};
use crate::video::picture::PictureSettings;
use crate::video::slot::FrameSlot;
use crate::video::timing::{DelaySettings, Summary};
use crate::video::{VideoShared, lock};
use capture::{Capture, PixelBuffer};

/// The live picture: the video surface and thread live as long as this does; the capture can be
/// switched to another Source or Video format without recreating them.
pub struct VideoPipeline {
    capture: Option<Capture>,
    status: String,
    sources: Vec<SourceInfo>,
    /// The Source and Video format on screen.
    current: Option<(SourceIdentity, VideoFormat)>,
    slot: Arc<FrameSlot<PixelBuffer>>,
    shared: Arc<VideoShared>,
    video_thread: Option<JoinHandle<()>>,
}

impl VideoPipeline {
    /// Puts a video surface under GPUI's content in `window` and starts showing a Source: the
    /// first card at its default Video format. Must be called on the main thread.
    pub fn start(window: &gpui_kit::Window, remembered: Option<(&SourceIdentity, Option<VideoFormat>)>) -> Result<Self, String> {
        let handle = HasWindowHandle::window_handle(window).map_err(|e| format!("no native window: {e}"))?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return Err("expected an AppKit window".into());
        };
        let target = surface::attach_below(appkit.ns_view)?;

        let slot = Arc::new(FrameSlot::new());
        let shared = Arc::new(VideoShared::default());
        let video_thread = {
            let (slot, shared) = (slot.clone(), shared.clone());
            std::thread::Builder::new()
                .name("xreveal-video".into())
                .spawn(move || render::run(target, &slot, shared))
                .map_err(|e| format!("could not start the video thread: {e}"))?
        };
        let mut pipeline = Self {
            capture: None,
            current: None,
            status: "No capture card found.".into(),
            sources: capture::list_sources(),
            slot,
            shared,
            video_thread: Some(video_thread),
        };
        pipeline.show(source::launch_pick(&pipeline.sources, remembered));
        Ok(pipeline)
    }

    fn show(&mut self, pick: LaunchPick) {
        match pick {
            LaunchPick::Show(index, choice) => {
                let os_id = self.sources[index].identity.os_id.clone();
                if let Err(error) = self.switch_to(&os_id, choice) {
                    self.status = error;
                }
            }
            LaunchPick::NotConnected(name) => self.status = format!("{name} isn't connected."),
            LaunchPick::Nothing => {}
        }
    }

    /// The Source and Video format on screen, if any.
    pub fn current(&self) -> Option<&(SourceIdentity, VideoFormat)> {
        self.current.as_ref()
    }

    /// Every capture device and its formats, as listed at start or the last [`refresh_sources`].
    pub fn sources(&self) -> &[SourceInfo] {
        &self.sources
    }

    /// Lists the capture devices again (e.g. after a card is plugged in).
    #[expect(dead_code, reason = "called by the pickers (their own slices)")]
    pub fn refresh_sources(&mut self) {
        self.sources = capture::list_sources();
    }

    /// Shows Source `os_id` at `choice`, replacing the current capture. The video surface stays;
    /// the picture restarts briefly (docs/spec/v1.md §2.3).
    pub fn switch_to(&mut self, os_id: &str, choice: VideoFormat) -> Result<(), String> {
        let info = self.sources.iter().find(|s| s.identity.os_id == os_id).ok_or("That capture card isn't connected.")?;
        let native = source::best_native(&info.formats, choice).ok_or(format!("{} doesn't offer {choice}.", info.identity.name))?;
        let identity = info.identity.clone();
        if let Some(old) = self.capture.take() {
            old.stop();
        }
        let capture = Capture::start(self.slot.clone(), os_id, native)?;
        self.current = Some((identity, choice));
        self.status = capture.description.clone();
        self.capture = Some(capture);
        Ok(())
    }

    /// Changes how the picture is drawn; applies from the next frame.
    pub fn set_picture(&self, settings: PictureSettings) {
        *lock(&self.shared.picture) = settings;
    }

    /// Changes Buffering / the negative A/V sync offset; applies from the next frame.
    pub fn set_delay(&self, delay: DelaySettings) {
        *lock(&self.shared.delay) = delay;
    }

    /// Timing for the Stats overlay.
    pub fn stats(&self) -> Summary {
        lock(&self.shared.stats).summary(self.slot.replaced_total())
    }

    /// What is being shown, e.g. `USB2 Video · 1280x720 @ 60 · 420v`, or why nothing is.
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
