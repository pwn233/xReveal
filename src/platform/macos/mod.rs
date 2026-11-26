//! macOS: AVFoundation capture → video thread → Metal layer under GPUI's view
//! (docs/spec/v1.md §3.1, §3.3, §3.4; ADR 0001).

pub mod access;
mod capture;
mod render;
pub mod screens;
mod surface;

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::source::{self, LaunchPick, SourceIdentity, SourceInfo, VideoFormat};
use crate::video::picture::PictureSettings;
use crate::video::signal::{Observed, OpenError};
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
    /// The Source and Video format xReveal is trying to show, even while it can't (busy, unplugged).
    wanted: Option<(SourceIdentity, VideoFormat)>,
    open_error: Option<OpenError>,
    /// When the current capture started.
    started: Option<Instant>,
    slot: Arc<FrameSlot<PixelBuffer>>,
    shared: Arc<VideoShared>,
    video_thread: Option<JoinHandle<()>>,
    /// The video layer, to keep its pixel density in step with the window's screen.
    layer: objc2::rc::Retained<objc2_quartz_core::CAMetalLayer>,
}

impl VideoPipeline {
    /// Puts a video surface under GPUI's content in `window` and starts showing a Source: the
    /// first card at its default Video format. Must be called on the main thread.
    pub fn start(window: &gpui_kit::Window, remembered: Option<(&SourceIdentity, Option<VideoFormat>)>, show: bool) -> Result<Self, String> {
        let handle = HasWindowHandle::window_handle(window).map_err(|e| format!("no native window: {e}"))?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return Err("expected an AppKit window".into());
        };
        let target = surface::attach_below(appkit.ns_view)?;
        let layer = target.layer.clone();

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
            wanted: None,
            open_error: None,
            started: None,
            status: "No input found.".into(),
            sources: capture::list_sources(),
            slot,
            shared,
            video_thread: Some(video_thread),
            layer,
        };
        if show {
            pipeline.show_launch(remembered);
        }
        Ok(pipeline)
    }

    /// Shows the remembered Source, as at launch (docs/spec/v1.md §2.1). With nothing remembered,
    /// the first card; when the remembered one isn't plugged in, nothing (the picker waits).
    pub fn show_launch(&mut self, remembered: Option<(&SourceIdentity, Option<VideoFormat>)>) {
        match source::launch_pick(&self.sources, remembered) {
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
    pub fn refresh_sources(&mut self) {
        self.sources = capture::list_sources();
    }

    /// Shows Source `os_id` at `choice`, replacing the current capture. The video surface stays;
    /// the picture restarts briefly (docs/spec/v1.md §2.3).
    pub fn switch_to(&mut self, os_id: &str, choice: VideoFormat) -> Result<(), String> {
        let info = self.sources.iter().find(|s| s.identity.os_id == os_id).ok_or("That input isn't connected.")?;
        let native = source::best_native(&info.formats, choice).ok_or(format!("{} doesn't offer {choice}.", info.identity.name))?;
        let identity = info.identity.clone();
        self.wanted = Some((identity.clone(), choice));
        self.release();
        let capture = Capture::start(self.slot.clone(), os_id, native).inspect_err(|e| self.open_error = Some(e.clone())).map_err(|e| e.message().to_owned())?;
        self.open_error = None;
        self.current = Some((identity, choice));
        self.started = Some(Instant::now());
        self.status = capture.description.clone();
        self.capture = Some(capture);
        Ok(())
    }

    /// Stops showing anything and forgets the wanted Source (Reset all: back to the picker).
    pub fn forget(&mut self) {
        self.release();
        self.wanted = None;
        self.open_error = None;
    }

    /// Why the wanted Source didn't open, if it didn't.
    pub fn open_error(&self) -> Option<&OpenError> {
        self.open_error.as_ref()
    }

    /// Stops the capture (the card is gone); the Source stays wanted.
    pub fn release(&mut self) {
        if let Some(old) = self.capture.take() {
            old.stop();
        }
        self.current = None;
        self.started = None;
    }

    /// Opens the wanted Source again (it came back, or is no longer busy).
    pub fn retry(&mut self) -> Result<(), String> {
        let (identity, choice) = self.wanted.clone().ok_or("Nothing to show.")?;
        self.refresh_sources();
        self.switch_to(&identity.os_id, choice)
    }

    /// The Source xReveal is trying to show, even while it can't.
    pub fn wanted(&self) -> Option<&(SourceIdentity, VideoFormat)> {
        self.wanted.as_ref()
    }

    /// What can be known for certain about the wanted Source (docs/spec/v1.md §2.8).
    pub fn observe(&self) -> Option<Observed> {
        let (identity, _) = self.wanted.as_ref()?;
        let started = self.started;
        Some(Observed {
            connected: capture::is_connected(&identity.os_id),
            busy: matches!(self.open_error, Some(OpenError::Busy(_))),
            since_start: started.map_or_else(Default::default, |t| t.elapsed()),
            since_frame: self.slot.last_put().filter(|t| started.is_some_and(|s| *t >= s)).map(|t| t.elapsed()),
        })
    }

    /// The Video format the card is running at, when another app changed it from xReveal's choice.
    pub fn taken_over(&self) -> Option<VideoFormat> {
        let (_, choice) = self.current.as_ref()?;
        let (width, height, fps) = self.capture.as_ref()?.running_format();
        let running = VideoFormat { width, height, fps };
        ((width, height) != (choice.width, choice.height) || (fps - choice.fps).abs() > 0.5).then_some(running)
    }

    /// macOS offers no driver settings windows for capture devices.
    pub fn has_device_settings(&self) -> bool {
        false
    }

    pub fn open_device_settings(&mut self, _window: &gpui_kit::Window) {}

    /// The stream is closed while the device's settings window is open.
    pub fn settings_paused(&self) -> bool {
        false
    }

    pub fn poll_device_settings(&mut self) -> bool {
        false
    }

    /// The window's pixel density changed (it moved between a Retina and a 1× screen): the video
    /// thread sizes its drawable from the layer's scale, so keep that in step (#82).
    pub fn set_scale(&self, scale: f32) {
        if (self.layer.contentsScale() - scale as f64).abs() > f64::EPSILON {
            self.layer.setContentsScale(scale as f64);
        }
    }

    /// The video surface is under GPUI's view here, so GPUI's screens already cover it.
    pub fn set_surface_visible(&self, _visible: bool) {}

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

impl VideoPipeline {
    /// Every capture device and its formats (for `--report`).
    pub fn list_sources() -> Vec<SourceInfo> {
        capture::list_sources()
    }

    /// Captures from `os_id` at `native` for `seconds` without drawing anything, for `--report`:
    /// what the capture says it delivers, and the gaps between frames as they arrive, in ms.
    pub fn measure(os_id: &str, native: source::NativeFormat, seconds: f64) -> Result<(String, Vec<f64>), String> {
        use crate::video::slot::Taken;
        use std::time::Duration;
        let slot = Arc::new(FrameSlot::<PixelBuffer>::new());
        let capture = Capture::start(slot.clone(), os_id, native).map_err(|e| e.message().to_owned())?;
        let started = Instant::now();
        let mut last = None::<Instant>;
        let mut gaps = Vec::new();
        while started.elapsed().as_secs_f64() < seconds {
            if let Taken::Frame(_) = slot.take_timeout(Duration::from_millis(250)) {
                let now = Instant::now();
                if let Some(previous) = last {
                    gaps.push(now.duration_since(previous).as_secs_f64() * 1000.0);
                }
                last = Some(now);
            }
        }
        capture.stop();
        Ok((capture.description.clone(), gaps))
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
