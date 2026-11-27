//! Capture (docs/spec/v1.md §3.1): AVFoundation lists every video capture device with its live
//! formats and sets the chosen one; CoreMediaIO streams the frames.
//!
//! Frames don't come through an `AVCaptureSession`: a session loads macOS's camera Video Effects
//! into the app (Portrait, Reactions with their 3D scenes, gesture and face detection), ~190 MB
//! and some CPU per frame that an app can't turn off (#118). Reading the device's CoreMediaIO
//! stream directly gets the same frames, already decoded, without any of it.

use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::Arc;

use objc2::rc::Retained;
use objc2_av_foundation::{
    AVCaptureDevice, AVCaptureDeviceDiscoverySession, AVCaptureDeviceFormat, AVCaptureDevicePosition,
    AVCaptureDeviceTypeBuiltInWideAngleCamera, AVCaptureDeviceTypeExternal, AVMediaTypeAudio, AVMediaTypeVideo,
};
use objc2_core_foundation::{CFRetained, CFString};
use objc2_core_media::{CMClock, CMSampleBuffer, CMSimpleQueue, CMSyncConvertTime, CMVideoFormatDescriptionGetDimensions};
use objc2_core_media_io::{
    CMIODeviceStartStream, CMIODeviceStopStream, CMIOObjectGetPropertyData, CMIOObjectGetPropertyDataSize, CMIOObjectID,
    CMIOObjectPropertyAddress, CMIOStreamCopyBufferQueue, CMIOStreamID, kCMIODevicePropertyDeviceUID,
    kCMIODevicePropertyScopeInput, kCMIODevicePropertyStreams, kCMIOHardwarePropertyDevices, kCMIOObjectPropertyElementMain,
    kCMIOObjectPropertyScopeGlobal, kCMIOObjectSystemObject, kCMIOStreamPropertyClock,
};
use objc2_core_video::CVPixelBuffer;
use objc2_foundation::{NSArray, NSObjectProtocol, NSString};
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

/// The stream's clock, to convert sample times to host time.
struct StreamClock(CFRetained<CMClock>);

// SAFETY: CMClock is a thread-safe CoreFoundation object; it is only read (converted against).
unsafe impl Send for StreamClock {}
unsafe impl Sync for StreamClock {}

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
    device: Retained<AVCaptureDevice>,
    stream: Stream,
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
            // The format is set on the device, which is shared: the stream runs at it. With no
            // `AVCaptureSession` there is no session preset to override it (#3).
            if let Some((format, duration)) = &chosen
                && device.lockForConfiguration().is_ok()
            {
                device.setActiveFormat(format);
                device.setActiveVideoMinFrameDuration(*duration);
                device.setActiveVideoMaxFrameDuration(*duration);
                device.unlockForConfiguration();
            }

            let stream = Stream::start(os_id, slot).map_err(|status| {
                if device.isInUseByAnotherApplication() {
                    busy()
                } else {
                    OpenError::Failed(format!("Couldn't open {name} (CoreMediaIO error {status})."))
                }
            })?;

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

            Ok(Self { device, stream, description })
        }
    }

    pub fn stop(&self) {
        self.stream.stop();
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

/// A device's CoreMediaIO video stream, running: macOS hands each frame to a queue and calls
/// [`queue_altered`], which passes it to the frame slot.
struct Stream {
    device: CMIOObjectID,
    stream: CMIOStreamID,
    _queue: CFRetained<CMSimpleQueue>,
    /// What [`queue_altered`] reads; freed only after the stream has stopped calling it.
    sink: *mut Sink,
}

struct Sink {
    queue: *const CMSimpleQueue,
    slot: Arc<FrameSlot<PixelBuffer>>,
    clock: Option<StreamClock>,
}

impl Stream {
    /// Starts the first input stream of the CoreMediaIO device whose UID is `os_id` (the same UID
    /// AVFoundation calls `uniqueID`). The error is CoreMediaIO's status.
    unsafe fn start(os_id: &str, slot: Arc<FrameSlot<PixelBuffer>>) -> Result<Self, i32> {
        unsafe {
            let devices = object_ids(kCMIOObjectSystemObject, address(kCMIOHardwarePropertyDevices, kCMIOObjectPropertyScopeGlobal));
            let device = devices.into_iter().find(|&d| uid(d).as_deref() == Some(os_id)).ok_or(-1)?;
            let stream = *object_ids(device, address(kCMIODevicePropertyStreams, kCMIODevicePropertyScopeInput)).first().ok_or(-2)?;
            let clock = clock(stream).map(StreamClock);
            let sink = Box::into_raw(Box::new(Sink { queue: ptr::null(), slot, clock }));
            let mut queue: *mut CMSimpleQueue = ptr::null_mut();
            let status = CMIOStreamCopyBufferQueue(stream, Some(queue_altered), sink.cast(), &mut queue);
            let Some(queue) = NonNull::new(queue).filter(|_| status == 0) else {
                drop(Box::from_raw(sink));
                return Err(status);
            };
            // SAFETY: "Copy": the queue comes retained.
            let queue = CFRetained::from_raw(queue);
            (*sink).queue = &*queue;
            let status = CMIODeviceStartStream(device, stream);
            let this = Self { device, stream, _queue: queue, sink };
            if status != 0 {
                return Err(status);
            }
            Ok(this)
        }
    }

    fn stop(&self) {
        unsafe { CMIODeviceStopStream(self.device, self.stream) };
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe {
            CMIODeviceStopStream(self.device, self.stream);
            // Take the callback back from the stream before freeing what it reads.
            let mut queue: *mut CMSimpleQueue = ptr::null_mut();
            if CMIOStreamCopyBufferQueue(self.stream, None, ptr::null_mut(), &mut queue) == 0
                && let Some(queue) = NonNull::new(queue)
            {
                drop(CFRetained::from_raw(queue));
            }
            drop(Box::from_raw(self.sink));
        }
    }
}

// SAFETY: the stream is driven by IDs and a CoreFoundation queue, both usable from any thread;
// `sink` is only read by CoreMediaIO's callback until `Drop` takes it back.
unsafe impl Send for Stream {}

/// Runs on CoreMediaIO's thread whenever a frame is queued: only hands the frames over.
unsafe extern "C-unwind" fn queue_altered(_stream: CMIOStreamID, _token: *mut c_void, sink: *mut c_void) {
    // SAFETY: `sink` is the `Sink` given to `CMIOStreamCopyBufferQueue`, alive until the
    // callback is taken back.
    let sink = unsafe { &*(sink as *const Sink) };
    let Some(queue) = (unsafe { sink.queue.as_ref() }) else { return };
    while let Some(sample) = NonNull::new(unsafe { queue.dequeue() } as *mut CMSampleBuffer) {
        // SAFETY: a dequeued sample buffer belongs to the caller, retained.
        let sample = unsafe { CFRetained::from_raw(sample) };
        let arrival = CACurrentMediaTime();
        if let Some(image) = unsafe { sample.image_buffer() } {
            let capture = capture_time(sink.clock.as_ref(), &sample, arrival);
            sink.slot.put(PixelBuffer { image, stamps: Stamps { capture, arrival } });
        }
    }
}

/// The sample's capture time in host seconds; the arrival time when it can't be trusted.
fn capture_time(clock: Option<&StreamClock>, sample: &CMSampleBuffer, arrival: f64) -> f64 {
    let pts = unsafe { sample.presentation_time_stamp() };
    let seconds = match clock {
        Some(StreamClock(clock)) => unsafe {
            let host = CMClock::host_time_clock();
            CMSyncConvertTime(pts, clock, &host).seconds()
        },
        None => unsafe { pts.seconds() },
    };
    // A capture time in the future, or seconds in the past, means a clock we don't understand.
    if seconds.is_finite() && seconds <= arrival && arrival - seconds < 2.0 { seconds } else { arrival }
}

fn address(selector: u32, scope: u32) -> CMIOObjectPropertyAddress {
    CMIOObjectPropertyAddress { mSelector: selector, mScope: scope, mElement: kCMIOObjectPropertyElementMain }
}

/// A property that is a list of CoreMediaIO object IDs (devices, streams).
unsafe fn object_ids(object: CMIOObjectID, address: CMIOObjectPropertyAddress) -> Vec<CMIOObjectID> {
    unsafe {
        let mut size = 0u32;
        if CMIOObjectGetPropertyDataSize(object, &address, 0, ptr::null(), &mut size) != 0 {
            return Vec::new();
        }
        let mut ids = vec![0 as CMIOObjectID; size as usize / size_of::<CMIOObjectID>()];
        let mut used = 0u32;
        if CMIOObjectGetPropertyData(object, &address, 0, ptr::null(), size, &mut used, ids.as_mut_ptr().cast()) != 0 {
            return Vec::new();
        }
        ids.truncate(used as usize / size_of::<CMIOObjectID>());
        ids
    }
}

/// A CoreFoundation-object property, which comes retained ("Copy").
unsafe fn cf_property(object: CMIOObjectID, selector: u32) -> Option<NonNull<c_void>> {
    unsafe {
        let address = address(selector, kCMIOObjectPropertyScopeGlobal);
        let mut value: *mut c_void = ptr::null_mut();
        let mut used = 0u32;
        let status = CMIOObjectGetPropertyData(object, &address, 0, ptr::null(), size_of::<*mut c_void>() as u32, &mut used, (&raw mut value).cast());
        if status == 0 { NonNull::new(value) } else { None }
    }
}

/// A device's UID.
unsafe fn uid(device: CMIOObjectID) -> Option<String> {
    let value = unsafe { cf_property(device, kCMIODevicePropertyDeviceUID) }?;
    // SAFETY: the UID property is a CFString, retained.
    Some(unsafe { CFRetained::from_raw(value.cast::<CFString>()) }.to_string())
}

/// A stream's clock, if it has one.
unsafe fn clock(stream: CMIOStreamID) -> Option<CFRetained<CMClock>> {
    let value = unsafe { cf_property(stream, kCMIOStreamPropertyClock) }?;
    // SAFETY: the clock property is a CMClock, retained.
    Some(unsafe { CFRetained::from_raw(value.cast::<CMClock>()) })
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
