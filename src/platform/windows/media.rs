//! Media Foundation capture (docs/spec/v1.md §3.1, ADR 0002).
//!
//! One Source Reader path serves both a capture card (async, frames pushed into the frame slot) and
//! a video file (sync, used by the CI tests with the synthetic clips in `tests/clips/`), so format
//! negotiation and decoding are exercised on CI without a card.
//!
//! Tracer bullet: the first capture device, preferring 1280x720 @ 60 in NV12 or YUY2. Compressed
//! native formats are decoded by Media Foundation for now; xReveal's own MJPEG decoder (zune-jpeg)
//! replaces that in the "Windows MJPEG decode" slice.

use std::sync::{Arc, Mutex};

use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::core::{AsImpl, GUID, Interface, PWSTR, Ref, Result, implement};

use crate::source::{self, NativeFormat, PixelKind, SourceIdentity, SourceInfo};
use crate::video::slot::FrameSlot;
use crate::video::timing::{Stamps, Timed};

const FIRST_VIDEO_STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;


/// What the reader delivers, as the renderer receives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Nv12,
    Yuy2,
    /// Compressed JPEG frames; the video thread decodes them with zune-jpeg (docs/spec/v1.md §3.2).
    Mjpeg,
}

impl PixelFormat {
    fn from_subtype(subtype: GUID) -> Option<Self> {
        match subtype {
            s if s == MFVideoFormat_NV12 => Some(Self::Nv12),
            s if s == MFVideoFormat_YUY2 => Some(Self::Yuy2),
            s if s == MFVideoFormat_MJPG => Some(Self::Mjpeg),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Nv12 => "NV12",
            Self::Yuy2 => "YUY2",
            Self::Mjpeg => "MJPEG",
        }
    }
}

/// What the reader delivers after negotiation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamFormat {
    pub pixel_format: PixelFormat,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// Media Foundation decodes the native format (e.g. MJPEG) into `pixel_format`.
    pub decoded: bool,
}

/// One frame, kept as Media Foundation's own sample until the video thread uploads it.
pub struct Frame {
    pub sample: IMFSample,
    pub format: StreamFormat,
    pub stamps: Stamps,
}

impl Timed for Frame {
    fn stamps(&self) -> Stamps {
        self.stamps
    }
}

/// The host clock, in seconds: QueryPerformanceCounter, the clock DXGI's frame statistics and
/// Media Foundation's device timestamps use.
pub fn host_seconds() -> f64 {
    use std::sync::OnceLock;
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    static FREQUENCY: OnceLock<f64> = OnceLock::new();
    let frequency = *FREQUENCY.get_or_init(|| {
        let mut f = 0i64;
        let _ = unsafe { QueryPerformanceFrequency(&mut f) };
        f.max(1) as f64
    });
    let mut counter = 0i64;
    let _ = unsafe { QueryPerformanceCounter(&mut counter) };
    counter as f64 / frequency
}

/// When the device captured the sample, in host seconds (`MFSampleExtension_DeviceTimestamp` is
/// QPC time in 100 ns units); the arrival time when it's missing or implausible.
fn capture_time(sample: &IMFSample, arrival: f64) -> f64 {
    let seconds = unsafe { sample.GetUINT64(&MFSampleExtension_DeviceTimestamp) }.map(|t| t as f64 / 1e7);
    match seconds {
        Ok(s) if s.is_finite() && s <= arrival && arrival - s < 2.0 => s,
        _ => arrival,
    }
}

// SAFETY: Media Foundation samples and buffers are free-threaded COM objects; the video thread is
// the only one that reads the frame after the capture callback hands it over.
unsafe impl Send for Frame {}

impl Frame {
    /// Calls `f` with the frame's pixel rows: (bytes, pitch in bytes). The bytes stay valid only
    /// inside `f`.
    pub fn with_pixels<R>(&self, f: impl FnOnce(&[u8], usize) -> R) -> Result<R> {
        unsafe {
            let buffer = self.sample.ConvertToContiguousBuffer()?;
            // Compressed frames are a plain byte stream; only uncompressed ones are 2D images.
            let buffer_2d = match self.format.pixel_format {
                PixelFormat::Mjpeg => None,
                _ => buffer.cast::<IMF2DBuffer>().ok(),
            };
            if let Some(buffer_2d) = buffer_2d {
                let mut scanline0 = std::ptr::null_mut();
                let mut pitch = 0i32;
                buffer_2d.Lock2D(&mut scanline0, &mut pitch)?;
                // Negative pitch (bottom-up) doesn't occur for NV12/YUY2 capture; refuse it rather
                // than read out of bounds.
                let result = if pitch > 0 {
                    let rows = self.rows();
                    let bytes = std::slice::from_raw_parts(scanline0, pitch as usize * rows);
                    Ok(f(bytes, pitch as usize))
                } else {
                    Err(windows::core::Error::from(MF_E_INVALIDMEDIATYPE))
                };
                buffer_2d.Unlock2D()?;
                return result;
            }
            let mut data = std::ptr::null_mut();
            let mut length = 0u32;
            buffer.Lock(&mut data, None, Some(&mut length))?;
            let bytes = std::slice::from_raw_parts(data, length as usize);
            let result = f(bytes, self.packed_pitch());
            buffer.Unlock()?;
            Ok(result)
        }
    }

    /// Rows of bytes in the whole frame (NV12 has a half-height chroma plane under the luma).
    fn rows(&self) -> usize {
        let h = self.format.height as usize;
        match self.format.pixel_format {
            PixelFormat::Nv12 => h + h.div_ceil(2),
            PixelFormat::Yuy2 | PixelFormat::Mjpeg => h,
        }
    }

    fn packed_pitch(&self) -> usize {
        let w = self.format.width as usize;
        match self.format.pixel_format {
            PixelFormat::Nv12 => w,
            PixelFormat::Yuy2 => w * 2,
            // Not rows at all: the whole buffer is one JPEG.
            PixelFormat::Mjpeg => 0,
        }
    }
}

/// Starts COM (multithreaded) and Media Foundation for this process. Safe to call repeatedly.
pub fn startup() -> Result<()> {
    unsafe {
        // S_FALSE / RPC_E_CHANGED_MODE just mean COM is already initialised on this thread.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL)
    }
}

/// Every video capture device Media Foundation lists (docs/spec/v1.md §1: all are offered), with
/// the formats each offers right now (read through a short-lived Source Reader).
pub fn list_sources() -> Vec<SourceInfo> {
    let Ok(devices) = devices() else {
        return Vec::new();
    };
    devices
        .into_iter()
        .map(|device| {
            let name = string_attribute(&device, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME).unwrap_or_else(|| "Capture card".into());
            let link = string_attribute(&device, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK).unwrap_or_default();
            let (model, serial) = source::identity_from_windows_link(&link);
            // Every USB device is "external" here; built-in laptop cameras are USB too, so this
            // only orders the list: a card the user plugged in usually lists after the webcam.
            let formats = native_formats(&device).unwrap_or_default();
            SourceInfo { identity: SourceIdentity { os_id: link, serial, model, name }, external: true, formats }
        })
        .collect()
}

/// The device whose symbolic link is `link`.
/// Whether the device with symbolic link `link` is plugged in.
pub fn is_connected(link: &str) -> bool {
    find_device(link).ok().flatten().is_some()
}

/// Whether `code` means another app has the device (docs/spec/v1.md §2.8 Source busy).
pub fn is_busy(code: windows::core::HRESULT) -> bool {
    code == MF_E_HW_MFT_FAILED_START_STREAMING || code == MF_E_VIDEO_RECORDING_DEVICE_PREEMPTED
}

/// Whether `code` means the device went away.
pub fn is_gone(code: windows::core::HRESULT) -> bool {
    code == MF_E_VIDEO_RECORDING_DEVICE_INVALIDATED
}

fn find_device(link: &str) -> Result<Option<IMFActivate>> {
    Ok(devices()?
        .into_iter()
        .find(|d| string_attribute(d, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK).as_deref() == Some(link)))
}

fn native_formats(device: &IMFActivate) -> Result<Vec<NativeFormat>> {
    unsafe {
        let source: IMFMediaSource = device.ActivateObject()?;
        let result = (|| {
            let reader = MFCreateSourceReaderFromMediaSource(&source, None)?;
            let mut out = Vec::new();
            let mut index = 0;
            while let Ok(t) = reader.GetNativeMediaType(FIRST_VIDEO_STREAM, index) {
                index += 1;
                let Some((width, height)) = frame_size(&t) else { continue };
                let pixel = match t.GetGUID(&MF_MT_SUBTYPE).ok().and_then(PixelFormat::from_subtype) {
                    Some(PixelFormat::Nv12) => PixelKind::Nv12,
                    Some(PixelFormat::Yuy2) => PixelKind::Yuy2,
                    Some(PixelFormat::Mjpeg) => PixelKind::Mjpeg,
                    None => PixelKind::Other,
                };
                out.push(NativeFormat { width, height, fps: frame_rate(&t).unwrap_or(0.0), pixel });
            }
            Ok(out)
        })();
        let _ = source.Shutdown();
        let _ = device.ShutdownObject();
        result
    }
}

fn devices() -> Result<Vec<IMFActivate>> {
    unsafe {
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 1)?;
        let attributes = attributes.expect("MFCreateAttributes succeeded");
        attributes.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;

        let mut devices: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        MFEnumDeviceSources(&attributes, &mut devices, &mut count)?;
        if devices.is_null() {
            return Ok(Vec::new());
        }
        let list = std::slice::from_raw_parts_mut(devices, count as usize);
        let found = list.iter_mut().filter_map(Option::take).collect();
        CoTaskMemFree(Some(devices as *const _));
        Ok(found)
    }
}

fn string_attribute(device: &IMFActivate, key: &GUID) -> Option<String> {
    unsafe {
        let mut value = PWSTR::null();
        let mut length = 0u32;
        device.GetAllocatedString(key, &mut value, &mut length).ok()?;
        let name = value.to_string().ok();
        CoTaskMemFree(Some(value.0 as *const _));
        name
    }
}

/// A running capture: frames go into the slot until [`stop`](Self::stop) or drop.
pub struct DeviceCapture {
    callback: IMFSourceReaderCallback,
    _reader: IMFSourceReader,
    _source: IMFMediaSource,
    pub name: String,
    pub format: StreamFormat,
}

impl DeviceCapture {
    /// Opens the device with symbolic link `link` at exactly `native` and starts delivering frames.
    pub fn start(link: &str, name: String, native: NativeFormat, slot: Arc<FrameSlot<Frame>>) -> Result<Self> {
        unsafe {
            let device = find_device(link)?.ok_or_else(|| windows::core::Error::from(MF_E_INVALIDMEDIATYPE))?;
            let source: IMFMediaSource = device.ActivateObject()?;
            let callback_impl = ReaderCallback { slot, state: Mutex::new(CallbackState::default()) };
            let callback: IMFSourceReaderCallback = callback_impl.into();

            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 2)?;
            let attributes = attributes.expect("MFCreateAttributes succeeded");
            attributes.SetUnknown(&MF_SOURCE_READER_ASYNC_CALLBACK, &callback)?;
            attributes.SetUINT32(&MF_LOW_LATENCY, 1)?;

            let reader = MFCreateSourceReaderFromMediaSource(&source, &attributes)?;
            let format = negotiate(&reader, Some(native))?;

            if let Ok(mut state) = callback.as_impl().state.lock() {
                state.reader = Some(reader.clone());
                state.format = Some(format);
            }
            reader.ReadSample(FIRST_VIDEO_STREAM, 0, None, None, None, None)?;
            Ok(Self { callback, _reader: reader, _source: source, name, format })
        }
    }

    pub fn description(&self) -> String {
        let f = &self.format;
        let decoded = if f.decoded { " (decoded by Windows)" } else { "" };
        format!("{} · {}x{} @ {:.0} · {}{}", self.name, f.width, f.height, f.fps, f.pixel_format.name(), decoded)
    }

    /// The error that stopped frames arriving, if one did (unplugged, or taken by another app).
    pub fn failure(&self) -> Option<windows::core::HRESULT> {
        let callback = unsafe { self.callback.as_impl() };
        callback.state.lock().ok().and_then(|s| s.failure)
    }

    /// Stops asking for frames. In-flight callbacks finish on their own.
    pub fn stop(&self) {
        let callback = unsafe { self.callback.as_impl() };
        if let Ok(mut state) = callback.state.lock() {
            state.stopped = true;
            state.reader = None;
        }
    }
}

impl Drop for DeviceCapture {
    fn drop(&mut self) {
        self.stop();
        unsafe {
            let _ = self._source.Shutdown();
        }
    }
}

#[derive(Default)]
struct CallbackState {
    reader: Option<IMFSourceReader>,
    format: Option<StreamFormat>,
    stopped: bool,
    failure: Option<windows::core::HRESULT>,
}

#[implement(IMFSourceReaderCallback)]
struct ReaderCallback {
    slot: Arc<FrameSlot<Frame>>,
    state: Mutex<CallbackState>,
}

impl IMFSourceReaderCallback_Impl for ReaderCallback_Impl {
    // Runs on a Media Foundation work queue thread for every frame: hand it over, ask for the next.
    fn OnReadSample(
        &self,
        hrstatus: windows::core::HRESULT,
        _stream_index: u32,
        stream_flags: u32,
        _timestamp: i64,
        sample: Ref<IMFSample>,
    ) -> Result<()> {
        let (reader, mut format) = match self.state.lock() {
            Ok(state) if !state.stopped => (state.reader.clone(), state.format),
            _ => return Ok(()),
        };
        if hrstatus.is_err()
            && let Ok(mut state) = self.state.lock()
        {
            state.failure = Some(hrstatus);
        }
        // The Source changed resolution or refresh rate: adapt silently (docs/spec/v1.md §2.8).
        if stream_flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0
            && let (Some(reader), Some(old)) = (&reader, format)
            && let Ok(new) = unsafe { current_format(reader, old.decoded) }
        {
            format = Some(new);
            if let Ok(mut state) = self.state.lock() {
                state.format = Some(new);
            }
        }
        if let (Some(sample), Some(format)) = (sample.as_ref(), format) {
            let arrival = host_seconds();
            let stamps = Stamps { capture: capture_time(sample, arrival), arrival };
            self.slot.put(Frame { sample: sample.clone(), format, stamps });
        }
        if hrstatus.is_ok()
            && let Some(reader) = reader
        {
            unsafe { reader.ReadSample(FIRST_VIDEO_STREAM, 0, None, None, None, None)? };
        }
        Ok(())
    }

    fn OnFlush(&self, _stream_index: u32) -> Result<()> {
        Ok(())
    }

    fn OnEvent(&self, _stream_index: u32, _event: Ref<IMFMediaEvent>) -> Result<()> {
        Ok(())
    }
}

/// Opens a video file through the same negotiation as a capture card (synchronous reads).
#[cfg(test)]
pub fn open_file(path: &std::path::Path) -> Result<(IMFSourceReader, StreamFormat)> {
    unsafe {
        let reader = MFCreateSourceReaderFromURL(&windows::core::HSTRING::from(path.as_os_str()), None)?;
        let format = negotiate(&reader, None)?;
        Ok((reader, format))
    }
}

/// Reads the next frame synchronously; `None` at the end of the stream.
#[cfg(test)]
pub fn read_frame(reader: &IMFSourceReader, format: StreamFormat) -> Result<Option<Frame>> {
    unsafe {
        loop {
            let mut flags = 0u32;
            let mut sample = None;
            reader.ReadSample(FIRST_VIDEO_STREAM, 0, None, Some(&mut flags), None, Some(&mut sample))?;
            if let Some(sample) = sample {
                let now = host_seconds();
                return Ok(Some(Frame { sample, format, stamps: Stamps { capture: now, arrival: now } }));
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(None);
            }
        }
    }
}

/// Picks the native type (exactly `want` if given, then NV12 > YUY2 > MJPEG > anything) and makes
/// the reader deliver it; other compressed types are decoded by Media Foundation.
fn negotiate(reader: &IMFSourceReader, want: Option<NativeFormat>) -> Result<StreamFormat> {
    unsafe {
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        reader.SetStreamSelection(FIRST_VIDEO_STREAM, true)?;

        let mut natives = Vec::new();
        let mut index = 0;
        while let Ok(media_type) = reader.GetNativeMediaType(FIRST_VIDEO_STREAM, index) {
            natives.push(media_type);
            index += 1;
        }
        let rank = |t: &IMFMediaType| {
            let (w, h) = frame_size(t).unwrap_or_default();
            let fps = frame_rate(t).unwrap_or_default();
            let kind = t.GetGUID(&MF_MT_SUBTYPE).ok().and_then(PixelFormat::from_subtype);
            let format_rank = match kind {
                Some(PixelFormat::Nv12) => 3,
                Some(PixelFormat::Yuy2) => 2,
                Some(PixelFormat::Mjpeg) => 1,
                None => 0,
            };
            let kind_matches = |want: PixelKind| match kind {
                Some(PixelFormat::Nv12) => want == PixelKind::Nv12,
                Some(PixelFormat::Yuy2) => want == PixelKind::Yuy2,
                Some(PixelFormat::Mjpeg) => want == PixelKind::Mjpeg,
                None => want == PixelKind::Other,
            };
            let matches_want =
                want.is_some_and(|n| (w, h) == (n.width, n.height) && (fps - n.fps).abs() < 0.5 && kind_matches(n.pixel));
            (matches_want, format_rank)
        };
        let native = natives
            .iter()
            .enumerate()
            .max_by_key(|(i, t)| (rank(t), std::cmp::Reverse(*i)))
            .map(|(_, t)| t.clone())
            .ok_or_else(|| windows::core::Error::from(MF_E_INVALIDMEDIATYPE))?;
        reader.SetCurrentMediaType(FIRST_VIDEO_STREAM, None, &native)?;

        let native_subtype = native.GetGUID(&MF_MT_SUBTYPE)?;
        let decoded = PixelFormat::from_subtype(native_subtype).is_none();
        if decoded {
            let mut chosen = Err(windows::core::Error::from(MF_E_INVALIDMEDIATYPE));
            for subtype in [MFVideoFormat_NV12, MFVideoFormat_YUY2] {
                let output = MFCreateMediaType()?;
                output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                output.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
                chosen = reader.SetCurrentMediaType(FIRST_VIDEO_STREAM, None, &output);
                if chosen.is_ok() {
                    break;
                }
            }
            chosen?;
        }

        let mut format = current_format(reader, decoded)?;
        if format.fps == 0.0 {
            format.fps = frame_rate(&native).unwrap_or(0.0);
        }
        Ok(format)
    }
}

/// What the reader delivers now: after negotiating, and again when the Source changes format.
unsafe fn current_format(reader: &IMFSourceReader, decoded: bool) -> Result<StreamFormat> {
    unsafe {
        let current = reader.GetCurrentMediaType(FIRST_VIDEO_STREAM)?;
        let pixel_format = PixelFormat::from_subtype(current.GetGUID(&MF_MT_SUBTYPE)?)
            .ok_or_else(|| windows::core::Error::from(MF_E_INVALIDMEDIATYPE))?;
        let (width, height) = frame_size(&current).ok_or_else(|| windows::core::Error::from(MF_E_INVALIDMEDIATYPE))?;
        let fps = frame_rate(&current).unwrap_or(0.0);
        Ok(StreamFormat { pixel_format, width, height, fps, decoded })
    }
}

fn frame_size(media_type: &IMFMediaType) -> Option<(u32, u32)> {
    let packed = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }.ok()?;
    Some(((packed >> 32) as u32, packed as u32))
}

fn frame_rate(media_type: &IMFMediaType) -> Option<f64> {
    let packed = unsafe { media_type.GetUINT64(&MF_MT_FRAME_RATE) }.ok()?;
    let (numerator, denominator) = ((packed >> 32) as u32, packed as u32);
    (denominator != 0).then(|| numerator as f64 / denominator as f64)
}

#[cfg(test)]
mod tests {
    //! Run the synthetic clips (scripts/make-test-clips.sh) through the capture-card path.
    use super::*;
    use std::path::PathBuf;

    fn clip(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/clips").join(name)
    }

    /// Y, Cb, Cr at the centre of each of the five bars (white, red, green, blue, black).
    fn bar_samples(frame: &Frame) -> Vec<(u8, u8, u8)> {
        let f = frame.format;
        let (w, h) = (f.width as usize, f.height as usize);
        frame
            .with_pixels(|bytes, pitch| {
                let row = h / 2;
                (0..5)
                    .map(|bar| {
                        let x = (w * (2 * bar + 1) / 10) & !1;
                        match f.pixel_format {
                            PixelFormat::Nv12 => {
                                let y = bytes[row * pitch + x];
                                let chroma = (h + row / 2) * pitch + x;
                                (y, bytes[chroma], bytes[chroma + 1])
                            }
                            PixelFormat::Yuy2 => {
                                let p = row * pitch + x * 2; // x is even: Y0 U Y1 V
                                (bytes[p], bytes[p + 1], bytes[p + 3])
                            }
                            PixelFormat::Mjpeg => unreachable!("MJPEG frames are decoded before sampling"),
                        }
                    })
                    .collect()
            })
            .expect("frame pixels are readable")
    }

    fn assert_bars(samples: &[(u8, u8, u8)], expected: &[(u8, u8, u8)], tolerance: i32, what: &str) {
        for (bar, (got, want)) in samples.iter().zip(expected).enumerate() {
            let close = |a: u8, b: u8| (a as i32 - b as i32).abs() <= tolerance;
            assert!(
                close(got.0, want.0) && close(got.1, want.1) && close(got.2, want.2),
                "{what}: bar {bar} is {got:?}, expected about {want:?} (all bars: {samples:?})"
            );
        }
    }

    /// BT.709 limited range: white, red, green, blue, black.
    const LIMITED: [(u8, u8, u8); 5] = [(235, 128, 128), (63, 102, 240), (173, 42, 26), (32, 240, 118), (16, 128, 128)];
    /// BT.709 full range.
    const FULL: [(u8, u8, u8); 5] = [(255, 128, 128), (54, 98, 255), (182, 30, 12), (18, 255, 116), (0, 128, 128)];

    fn first_frame(name: &str) -> Frame {
        startup().expect("Media Foundation starts");
        let (reader, format) = open_file(&clip(name)).expect("clip opens");
        read_frame(&reader, format).expect("frame reads").expect("clip has a frame")
    }

    #[test]
    fn raw_yuy2_passes_through_unchanged() {
        let frame = first_frame("bars-640x360p60-yuy2-limited.avi");
        assert_eq!(frame.format.pixel_format, PixelFormat::Yuy2);
        assert!(!frame.format.decoded);
        assert_eq!((frame.format.width, frame.format.height), (640, 360));
        assert_bars(&bar_samples(&frame), &LIMITED, 2, "YUY2");
    }

    #[test]
    fn h264_is_decoded_to_nv12() {
        let frame = first_frame("bars-1280x720p60-h264-limited.mp4");
        assert_eq!(frame.format.pixel_format, PixelFormat::Nv12);
        assert!(frame.format.decoded);
        assert_eq!((frame.format.width, frame.format.height), (1280, 720));
        assert_bars(&bar_samples(&frame), &LIMITED, 6, "H.264 → NV12");
    }

    #[test]
    fn mjpeg_arrives_compressed_and_xreveal_decodes_it() {
        let frame = first_frame("bars-1280x720p60-mjpeg-full.avi");
        assert_eq!(frame.format.pixel_format, PixelFormat::Mjpeg);
        assert!(!frame.format.decoded, "Media Foundation must not decode MJPEG (spec §3.2)");
        let mut decoder = crate::video::mjpeg::MjpegDecoder::default();
        let samples = frame
            .with_pixels(|jpeg, _| {
                let decoded = decoder.decode(jpeg).expect("zune-jpeg decodes the card's frame");
                assert_eq!((decoded.width, decoded.height), (1280, 720));
                (0..5)
                    .map(|bar| {
                        let p = (360 * 1280 + 1280 * (2 * bar + 1) / 10) * 3;
                        (decoded.ycbcr[p], decoded.ycbcr[p + 1], decoded.ycbcr[p + 2])
                    })
                    .collect::<Vec<_>>()
            })
            .expect("frame bytes");
        assert_bars(&samples, &FULL, 8, "MJPEG via zune-jpeg");
    }
}
