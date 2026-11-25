//! AVFoundation capture (docs/spec/v1.md §3.1).
//!
//! Tracer bullet: shows the first external capture device, preferring 1280x720 @ 60 (the Reference
//! card's best live mode) in its native pixel format. Source and Video format choice arrive with the
//! "Source, Video format and monitor model" slice.

use std::sync::Arc;

use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_av_foundation::{
    AVCaptureConnection, AVCaptureDevice, AVCaptureDeviceDiscoverySession, AVCaptureDeviceFormat,
    AVCaptureDeviceInput, AVCaptureDevicePosition, AVCaptureDeviceTypeExternal, AVCaptureOutput,
    AVCaptureSession, AVCaptureVideoDataOutput, AVCaptureVideoDataOutputSampleBufferDelegate,
    AVMediaTypeVideo,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{CMSampleBuffer, CMVideoFormatDescriptionGetDimensions};
use objc2_core_video::CVPixelBuffer;
use objc2_foundation::{NSArray, NSDictionary, NSObject, NSObjectProtocol};

use crate::video::slot::FrameSlot;

/// One captured frame, as handed to the video thread.
pub struct PixelBuffer(pub CFRetained<CVPixelBuffer>);

// SAFETY: CVPixelBuffer is a reference-counted CoreFoundation object that is safe to retain,
// release and read from any thread; the video thread only reads it.
unsafe impl Send for PixelBuffer {}

/// The Reference card's best live mode; used when the device offers it.
const PREFERRED: (i32, i32, f64) = (1280, 720, 60.0);

pub struct Capture {
    session: Retained<AVCaptureSession>,
    // Kept alive for as long as the session runs: the output holds its delegate weakly.
    _output: Retained<AVCaptureVideoDataOutput>,
    _sink: Retained<VideoSink>,
    _queue: DispatchRetained<DispatchQueue>,
    pub description: String,
}

impl Capture {
    pub fn start(slot: Arc<FrameSlot<PixelBuffer>>) -> Result<Self, String> {
        unsafe {
            let device = first_capture_device().ok_or("No capture card found.")?;
            let chosen = choose_format(&device);

            let session = AVCaptureSession::new();
            session.beginConfiguration();

            let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device)
                .map_err(|e| format!("Couldn't open {}: {}", device.localizedName(), e.localizedDescription()))?;
            if !session.canAddInput(&input) {
                return Err(format!("{} can't be used right now.", device.localizedName()));
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
            if locked && let Some((format, fps)) = &chosen {
                device.setActiveFormat(format);
                if let Some(duration) = frame_rate_range(format, *fps) {
                    device.setActiveVideoMinFrameDuration(duration);
                    device.setActiveVideoMaxFrameDuration(duration);
                }
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

            Ok(Self { session, _output: output, _sink: sink, _queue: queue, description })
        }
    }

    pub fn stop(&self) {
        unsafe { self.session.stopRunning() };
    }
}

unsafe fn first_capture_device() -> Option<Retained<AVCaptureDevice>> {
    unsafe {
        let types = NSArray::from_slice(&[AVCaptureDeviceTypeExternal]);
        let discovery = AVCaptureDeviceDiscoverySession::discoverySessionWithDeviceTypes_mediaType_position(
            &types,
            AVMediaTypeVideo,
            AVCaptureDevicePosition::Unspecified,
        );
        discovery.devices().firstObject()
    }
}

/// The device format matching [`PREFERRED`], if it exists, with its frame rate.
unsafe fn choose_format(device: &AVCaptureDevice) -> Option<(Retained<AVCaptureDeviceFormat>, f64)> {
    unsafe {
        let (want_w, want_h, want_fps) = PREFERRED;
        device.formats().iter().find_map(|format| {
            let (w, h) = dimensions(&format);
            let has_rate = frame_rate_range(&format, want_fps).is_some();
            (w == want_w && h == want_h && has_rate).then(|| (format.clone(), want_fps))
        })
    }
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
            if let Some(image) = unsafe { sample_buffer.image_buffer() } {
                self.ivars().slot.put(PixelBuffer(image));
            }
        }
    }
);

impl VideoSink {
    fn new(slot: Arc<FrameSlot<PixelBuffer>>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(SinkIvars { slot });
        unsafe { msg_send![super(this), init] }
    }
}
