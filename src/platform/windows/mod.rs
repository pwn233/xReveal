//! Windows: Media Foundation capture → video thread → Direct3D 11 swap chain in a child window over
//! GPUI's window (docs/spec/v1.md §3.1, §3.3, §3.4; ADR 0001, ADR 0002).

pub mod media;
mod render;
mod surface;

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;

use crate::source::{self, LaunchPick, SourceIdentity, SourceInfo, VideoFormat};
use crate::video::picture::PictureSettings;
use crate::video::signal::{Observed, OpenError};
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
    /// The Source and Video format on screen.
    current: Option<(SourceIdentity, VideoFormat)>,
    /// The Source and Video format xReveal is trying to show, even while it can't (busy, unplugged).
    wanted: Option<(SourceIdentity, VideoFormat)>,
    open_error: Option<OpenError>,
    /// When the current capture started.
    started: Option<Instant>,
    /// The video surface's child window, hidden while a screen without a picture shows.
    child: HWND,
    slot: Arc<FrameSlot<Frame>>,
    shared: Arc<VideoShared>,
    video_thread: Option<JoinHandle<()>>,
}

impl VideoPipeline {
    /// Puts a video surface over GPUI's content in `window` and starts showing a Source: the
    /// first device at its default Video format. The surface comes up even without a card (it
    /// shows black). Main thread only.
    pub fn start(window: &gpui_kit::Window, remembered: Option<(&SourceIdentity, Option<VideoFormat>)>, show: bool) -> Result<Self, String> {
        let handle = HasWindowHandle::window_handle(window).map_err(|e| format!("no native window: {e}"))?;
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return Err("expected a Win32 window".into());
        };
        let parent = HWND(win32.hwnd.get() as *mut _);
        let surface = surface::attach_over(parent).map_err(|e| format!("video surface: {e}"))?;
        let child = surface.child;

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
            current: None,
            wanted: None,
            open_error: None,
            started: None,
            child,
            status: "No input found.".into(),
            sources: if started.is_ok() { media::list_sources() } else { Vec::new() },
            slot,
            shared,
            video_thread: Some(video_thread),
        };
        if let Err(error) = started {
            pipeline.status = format!("Media Foundation didn't start: {error}");
        } else if show {
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
        self.sources = media::list_sources();
    }

    /// Shows Source `os_id` at `choice`, replacing the current capture. The video surface stays;
    /// the picture restarts briefly (docs/spec/v1.md §2.3).
    pub fn switch_to(&mut self, os_id: &str, choice: VideoFormat) -> Result<(), String> {
        let info = self.sources.iter().find(|s| s.identity.os_id == os_id).ok_or("That input isn't connected.")?;
        let native = source::best_native(&info.formats, choice).ok_or(format!("{} doesn't offer {choice}.", info.identity.name))?;
        let name = info.identity.name.clone();
        let identity = info.identity.clone();
        self.wanted = Some((identity.clone(), choice));
        self.release();
        let capture = DeviceCapture::start(os_id, name.clone(), native, self.slot.clone()).map_err(|e| {
            let error = if e.code() == windows::Win32::Foundation::E_ACCESSDENIED {
                OpenError::Blocked("Windows is blocking camera access for desktop apps".into())
            } else if media::is_busy(e.code()) {
                OpenError::Busy(format!("{name} is being used by another app (OBS, Discord, a browser…). Close it to continue."))
            } else {
                OpenError::Failed(format!("Couldn't open {name}: {e}"))
            };
            let message = error.message().to_owned();
            self.open_error = Some(error);
            message
        })?;
        self.open_error = None;
        self.current = Some((identity, choice));
        self.started = Some(Instant::now());
        self.status = capture.description();
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
        let failure = self.capture.as_ref().and_then(DeviceCapture::failure);
        Some(Observed {
            connected: !failure.is_some_and(media::is_gone) && media::is_connected(&identity.os_id),
            busy: matches!(self.open_error, Some(OpenError::Busy(_))) || failure.is_some_and(media::is_busy),
            since_start: started.map_or_else(Default::default, |t| t.elapsed()),
            since_frame: self.slot.last_put().filter(|t| started.is_some_and(|s| *t >= s)).map(|t| t.elapsed()),
        })
    }

    /// Windows gives a card to one app at a time, so no other app can change its format under us.
    pub fn taken_over(&self) -> Option<VideoFormat> {
        None
    }

    /// Shows or hides the video surface. It covers everything GPUI draws in the window, so it is
    /// hidden while a screen without a picture (No signal, Source busy…) shows.
    pub fn set_surface_visible(&self, visible: bool) {
        use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNA, ShowWindow};
        unsafe {
            let _ = ShowWindow(self.child, if visible { SW_SHOWNA } else { SW_HIDE });
        }
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
