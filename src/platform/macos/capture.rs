//! AVFoundation capture (docs/spec/v1.md §3.1): lists every video capture device with its live
//! formats, and runs one at a chosen format.

use std::sync::{Arc, OnceLock};

use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send, sel};
use objc2_av_foundation::{
    AVError, AVCaptureConnection, AVCaptureDevice, AVCaptureDeviceDiscoverySession, AVCaptureDeviceFormat,
    AVCaptureDeviceInput, AVCaptureDevicePosition, AVCaptureDeviceTypeBuiltInWideAngleCamera,
    AVCaptureDeviceTypeExternal, AVCaptureOutput, AVCaptureSession, AVCaptureVideoDataOutput,
    AVCaptureVideoDataOutputSampleBufferDelegate, AVMediaTypeAudio, AVMediaTypeVideo,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{CMClock, CMSampleBuffer, CMSyncConvertTime, CMVideoFormatDescriptionGetDimensions};
use objc2_core_video::CVPixelBuffer;
use objc2_foundation::{NSArray, NSDictionary, NSObject, NSObjectProtocol, NSString};
use objc2_quartz_core::CACurrentMediaTime;

use crate::source::{self, NativeFormat, PixelKind, SourceIdentity, SourceInfo};
use crate::video::signal::OpenError;
use crate::video::slot::FrameSlot;
use crate::video::timing::{Stamps, Timed};

/// One captured frame, as handed to the video thread.
pub struct PixelBuffer {
    pub image: CFRetained<CVPixelBuffer>,
    pub stamps: Stamps,
}

// SAFETY: CVPixelBuffer is a reference-counted CoreFoundation object that is safe to retain,
// release and read from any thread; the video thread only reads it.
unsafe impl Send for PixelBuffer {}

impl Timed for PixelBuffer {
    fn stamps(&self) -> Stamps {
        self.stamps
    }
}

/// The session's clock, to convert sample times to host time.
struct SessionClock(CFRetained<CMClock>);

// SAFETY: CMClock is a thread-safe CoreFoundation object; it is only read (converted against).
unsafe impl Send for SessionClock {}
unsafe impl Sync for SessionClock {}

/// Every video capture device macOS lists (docs/spec/v1.md §1: all are offered), cards first.
pub fn list_sources() -> Vec<SourceInfo> {
    unsafe {
        let audio_ids = audio_device_ids();
        let mut sources: Vec<SourceInfo> = video_devices()
            .iter()
            .map(|device| {
                let name = device.localizedName().to_string();
                let external = device.deviceType().isEqual(Some(AVCaptureDeviceTypeExternal));
                let model = source::model_from_mac_model_id(&device.modelID().to_string());
                let serial = audio_ids.iter().find_map(|id| source::serial_from_mac_audio_id(id, &name));
                let identity = SourceIdentity { os_id: device.uniqueID().to_string(), serial, model, name };
                SourceInfo { identity, external, formats: native_formats(&device) }
            })
            .collect();
        sources.sort_by_key(|s| !s.external);
        sources
    }
}

unsafe fn video_devices() -> Retained<NSArray<AVCaptureDevice>> {
    unsafe {
        let types = NSArray::from_slice(&[AVCaptureDeviceTypeExternal, AVCaptureDeviceTypeBuiltInWideAngleCamera]);
        AVCaptureDeviceDiscoverySession::discoverySessionWithDeviceTypes_mediaType_position(
            &types,
            AVMediaTypeVideo,
            AVCaptureDevicePosition::Unspecified,
        )
        .devices()
    }
}

/// Audio input device IDs: a capture card's audio ID carries its USB serial (#25).
unsafe fn audio_device_ids() -> Vec<String> {
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
        return Vec::new();
    };
    #[allow(deprecated, reason = "the replacement discovery types vary by macOS version; this lists every input")]
    let devices = unsafe { AVCaptureDevice::devicesWithMediaType(audio) };
    devices.iter().map(|d| unsafe { d.uniqueID() }.to_string()).collect()
}

unsafe fn native_formats(device: &AVCaptureDevice) -> Vec<NativeFormat> {
    let mut out = Vec::new();
    for format in unsafe { device.formats() }.iter() {
        let (width, height) = unsafe { dimensions(&format) };
        let pixel = pixel_kind(unsafe { format.formatDescription().media_sub_type() });
        for range in unsafe { format.videoSupportedFrameRateRanges() }.iter() {
            // macOS reports e.g. 60.00024; keep the rate a person would read.
            let fps = (unsafe { range.maxFrameRate() } * 100.0).round() / 100.0;
            out.push(NativeFormat { width: width as u32, height: height as u32, fps, pixel });
        }
    }
    out
}

fn pixel_kind(subtype: u32) -> PixelKind {
    match &subtype.to_be_bytes() {
        b"420v" | b"420f" => PixelKind::Nv12,
        b"yuvs" => PixelKind::Yuy2,
        _ => PixelKind::Other,
    }
}

pub struct Capture {
    session: Retained<AVCaptureSession>,
    device: Retained<AVCaptureDevice>,
    // Kept alive for as long as the session runs: the output holds its delegate weakly.
    _output: Retained<AVCaptureVideoDataOutput>,
    _sink: Retained<VideoSink>,
    _queue: DispatchRetained<DispatchQueue>,
    pub description: String,
}

impl Capture {
    /// Opens the device with OS ID `os_id` at exactly `native` and starts delivering frames.
    pub fn start(slot: Arc<FrameSlot<PixelBuffer>>, os_id: &str, native: NativeFormat) -> Result<Self, OpenError> {
        unsafe {
            let device = AVCaptureDevice::deviceWithUniqueID(&NSString::from_str(os_id))
                .ok_or(OpenError::Failed("That input is no longer connected.".into()))?;
            let name = device.localizedName();
            let busy = || OpenError::Busy(format!("{name} is being used by another app (OBS, Discord, a browser…). Close it to continue."));
            let chosen = device.formats().iter().find_map(|format| {
                let (w, h) = dimensions(&format);
                let same_kind = pixel_kind(format.formatDescription().media_sub_type()) == native.pixel;
                let duration = frame_rate_range(&format, native.fps)?;
                (w as u32 == native.width && h as u32 == native.height && same_kind).then(|| (format.clone(), duration))
            });

            let session = AVCaptureSession::new();
            session.beginConfiguration();

            let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device).map_err(|e| {
                let code = e.code();
                if code == AVError::DeviceInUseByAnotherApplication.0 || code == AVError::DeviceAlreadyUsedByAnotherSession.0 {
                    busy()
                } else {
                    OpenError::Failed(format!("Couldn't open {name}: {}", e.localizedDescription()))
                }
            })?;
            if !session.canAddInput(&input) {
                return Err(if device.isInUseByAnotherApplication() { busy() } else { OpenError::Failed(format!("{name} can't be used right now.")) });
            }
            session.addInput(&input);

            let output = AVCaptureVideoDataOutput::new();
            output.setAlwaysDiscardsLateVideoFrames(true);
            // Empty settings = the device's native format, with no conversion (`nil` would add one).
            output.setVideoSettings(Some(&NSDictionary::new()));
            let sink = VideoSink::new(slot);
            let queue = DispatchQueue::new("xreveal.capture", None);
            output.setSampleBufferDelegate_queue(Some(ProtocolObject::from_ref(&*sink)), Some(&queue));
            session.addOutput(&output);
            session.commitConfiguration();

            // The session preset silently overrides `activeFormat` unless the device stays locked
            // across `startRunning` (Reference card finding, #3).
            let locked = device.lockForConfiguration().is_ok();
            if locked && let Some((format, duration)) = &chosen {
                device.setActiveFormat(format);
                device.setActiveVideoMinFrameDuration(*duration);
                device.setActiveVideoMaxFrameDuration(*duration);
            }
            // Sample times are on the session's clock; keep it to convert them to host time.
            if session.respondsToSelector(sel!(synchronizationClock))
                && let Some(clock) = session.synchronizationClock()
            {
                let _ = sink.ivars().clock.set(SessionClock(CFRetained::from(&*clock)));
            }
            session.startRunning();
            if locked {
                device.unlockForConfiguration();
            }

            let active = device.activeFormat();
            let (w, h) = dimensions(&active);
            let fps = 1.0 / device.activeVideoMinFrameDuration().seconds();
            let description = format!(
                "{} · {}x{} @ {:.0} · {}",
                device.localizedName(),
                w,
                h,
                fps,
                fourcc(active.formatDescription().media_sub_type())
            );

            Ok(Self { session, device, _output: output, _sink: sink, _queue: queue, description })
        }
    }

    pub fn stop(&self) {
        unsafe { self.session.stopRunning() };
    }

    /// The format the card is running at now: another app can change it, because macOS shares
    /// cards and the last app to set a format wins (docs/spec/v1.md §2.8).
    pub fn running_format(&self) -> (u32, u32, f64) {
        unsafe {
            let (w, h) = dimensions(&self.device.activeFormat());
            let duration = self.device.activeVideoMinFrameDuration().seconds();
            let fps = if duration > 0.0 { ((1.0 / duration) * 100.0).round() / 100.0 } else { 0.0 };
            (w as u32, h as u32, fps)
        }
    }
}

/// Whether the device with OS ID `os_id` is plugged in.
pub fn is_connected(os_id: &str) -> bool {
    unsafe { AVCaptureDevice::deviceWithUniqueID(&NSString::from_str(os_id)).is_some_and(|d| d.isConnected()) }
}

/// The frame duration for `fps` if `format` supports that rate.
unsafe fn frame_rate_range(format: &AVCaptureDeviceFormat, fps: f64) -> Option<objc2_core_media::CMTime> {
    unsafe {
        format
            .videoSupportedFrameRateRanges()
            .iter()
            .find(|range| (range.maxFrameRate() - fps).abs() < 0.01)
            .map(|range| range.minFrameDuration())
    }
}

unsafe fn dimensions(format: &AVCaptureDeviceFormat) -> (i32, i32) {
    let description = unsafe { format.formatDescription() };
    let d = unsafe { CMVideoFormatDescriptionGetDimensions(&description) };
    (d.width, d.height)
}

pub fn fourcc(code: u32) -> String {
    code.to_be_bytes().iter().map(|&b| if b.is_ascii_graphic() { b as char } else { '?' }).collect()
}

struct SinkIvars {
    slot: Arc<FrameSlot<PixelBuffer>>,
    clock: OnceLock<SessionClock>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; VideoSink doesn't implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "XRevealVideoSink"]
    #[ivars = SinkIvars]
    struct VideoSink;

    unsafe impl NSObjectProtocol for VideoSink {}

    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for VideoSink {
        // Runs on the capture queue for every frame: only hand the frame over.
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        fn capture_output(&self, _output: &AVCaptureOutput, sample_buffer: &CMSampleBuffer, _connection: &AVCaptureConnection) {
            let arrival = CACurrentMediaTime();
            if let Some(image) = unsafe { sample_buffer.image_buffer() } {
                let capture = self.capture_time(sample_buffer, arrival);
                self.ivars().slot.put(PixelBuffer { image, stamps: Stamps { capture, arrival } });
            }
        }
    }
);

impl VideoSink {
    fn new(slot: Arc<FrameSlot<PixelBuffer>>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(SinkIvars { slot, clock: OnceLock::new() });
        unsafe { msg_send![super(this), init] }
    }

    /// The sample's capture time in host seconds; the arrival time when it can't be trusted.
    fn capture_time(&self, sample_buffer: &CMSampleBuffer, arrival: f64) -> f64 {
        let pts = unsafe { sample_buffer.presentation_time_stamp() };
        let seconds = match self.ivars().clock.get() {
            Some(SessionClock(clock)) => unsafe {
                let host = CMClock::host_time_clock();
                CMSyncConvertTime(pts, clock, &host).seconds()
            },
            None => unsafe { pts.seconds() },
        };
        // A capture time in the future, or seconds in the past, means a clock we don't understand.
        if seconds.is_finite() && seconds <= arrival && arrival - seconds < 2.0 { seconds } else { arrival }
    }
}
