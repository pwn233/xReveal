//! The sound chain (docs/spec/v1.md §3.6): `cpal` input → lock-free ring → `rubato` resampler
//! steered by the [`Pacer`] → volume and mute → `cpal` output.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, InterfaceType, StreamConfig, SupportedBufferSize};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Adjustable, Async, FixedAsync, PolynomialDegree, Resampler};

use super::pacer::{Pacer, Step};
use super::ring::{Reader, Writer, ring};

/// The buffer macOS is asked for (docs/spec/v1.md §3.6). Windows takes WASAPI's default period.
pub const MAC_BUFFER: u32 = 128;
/// Doubling stops here.
pub const MAX_BUFFER: u32 = 2048;
/// Frames the resampler produces at a time.
const CHUNK: usize = 64;
/// The ring holds this much sound at most (the A/V sync offset can ask for half a second).
const RING_SECONDS: f64 = 1.5;

/// An audio device as xReveal offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDevice {
    /// cpal's device ID (on macOS, CoreAudio's UID), stable across runs.
    pub id: String,
    pub name: String,
}

pub fn inputs() -> Vec<AudioDevice> {
    devices(true)
}

pub fn outputs() -> Vec<AudioDevice> {
    devices(false)
}

fn devices(input: bool) -> Vec<AudioDevice> {
    let host = cpal::default_host();
    let Ok(devices) = (if input { host.input_devices() } else { host.output_devices() }) else { return Vec::new() };
    devices
        .filter_map(|d| {
            let id = d.id().ok()?.id().to_owned();
            let name = d.description().map(|desc| desc.name().to_owned()).unwrap_or_else(|_| d.to_string());
            Some(AudioDevice { id, name })
        })
        .collect()
}

/// The input that carries the same physical card's sound (docs/spec/v1.md §2.1), never matched by
/// name: on Windows by container ID, on macOS by the serial in the audio device's UID.
pub fn paired_input(card: &crate::source::SourceIdentity, inputs: &[AudioDevice]) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use crate::platform::windows::container;
        let found = container::of_video(&card.os_id)?;
        let containers: Vec<(String, Option<_>)> = inputs.iter().map(|d| (d.id.clone(), container::of_audio_endpoint(&d.id))).collect();
        same_container(found, &containers)
    }
    #[cfg(not(target_os = "windows"))]
    by_serial(&card.name, card.serial.as_deref(), inputs)
}

/// The first input in `inputs` (ID, container) in the card's container.
#[cfg_attr(not(any(target_os = "windows", test)), expect(dead_code, reason = "Windows pairs by container"))]
pub fn same_container<T: PartialEq>(card: T, inputs: &[(String, Option<T>)]) -> Option<String> {
    inputs.iter().find(|(_, c)| c.as_ref() == Some(&card)).map(|(id, _)| id.clone())
}

/// The input whose macOS UID carries the card's serial.
#[cfg_attr(all(target_os = "windows", not(test)), expect(dead_code, reason = "macOS pairs by serial"))]
pub fn by_serial(card_name: &str, card_serial: Option<&str>, inputs: &[AudioDevice]) -> Option<String> {
    let serial = card_serial?;
    inputs
        .iter()
        .find(|d| crate::source::serial_from_mac_audio_id(&d.id, card_name).as_deref() == Some(serial))
        .map(|d| d.id.clone())
}

/// What the input or output with cpal ID `id` is, for the echo guard (#112). `None` is the system
/// output.
pub fn kind(id: Option<&str>, input: bool) -> super::echo::DeviceKind {
    let id = match id {
        Some(id) => id.to_owned(),
        None => match find(None, input).and_then(|d| d.id().ok()) {
            Some(id) => id.id().to_owned(),
            None => return super::echo::DeviceKind::Other,
        },
    };
    #[cfg(target_os = "macos")]
    return crate::platform::macos::sound_devices::kind(&id, input);
    #[cfg(target_os = "windows")]
    return crate::platform::windows::container::kind_of_endpoint(&id, input);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (id, input);
        super::echo::DeviceKind::Other
    }
}

fn find(id: Option<&str>, input: bool) -> Option<cpal::Device> {
    let host = cpal::default_host();
    match id {
        None if input => host.default_input_device(),
        None => host.default_output_device(),
        Some(id) => {
            let mut devices = if input { host.input_devices().ok()? } else { host.output_devices().ok()? };
            devices.find(|d| d.id().is_ok_and(|i| i.id() == id))
        }
    }
}

/// What the UI and the two callbacks share.
#[derive(Default)]
struct Shared {
    volume_bits: AtomicU32,
    muted: AtomicBool,
    offset_ms: AtomicI32,
    /// The ring ran dry while sound was still arriving: a dropout of our own making.
    underruns: AtomicU64,
    /// Frames of resampled sound played (not silence).
    played: AtomicU64,
    /// The loudest sample per channel (left, right) since the UI last read it, as `f32` bits:
    /// the Input level meter.
    peak_left: AtomicU32,
    peak_right: AtomicU32,
    /// Microseconds since `started` when the input last delivered sound.
    input_seen_us: AtomicU64,
    /// The output's own delay (its buffer, latency and safety offset), in microseconds.
    output_latency_us: AtomicU64,
    failed: AtomicBool,
    /// What the OS said when it stopped a stream.
    error: std::sync::Mutex<Option<String>>,
}

impl Shared {
    fn fail(&self, side: &str, error: cpal::Error) {
        self.failed.store(true, Ordering::Relaxed);
        if let Ok(mut e) = self.error.lock() {
            *e = Some(format!("{side}: {error}"));
        }
    }
}

/// Volume, mute and the A/V sync offset.
#[derive(Clone, Copy, Debug)]
pub struct Mix {
    pub volume: f32,
    pub muted: bool,
    pub offset_ms: i32,
}

/// Sound playing from one input to one output; stops when dropped.
pub struct AudioPipeline {
    _input: cpal::Stream,
    _output: cpal::Stream,
    shared: Arc<Shared>,
    pub input_id: String,
    /// The output's ID, or `None` for the system output.
    pub output_id: Option<String>,
    pub bluetooth: bool,
    /// The output buffer asked for, in frames (`None`: the OS default).
    pub buffer: Option<u32>,
}

impl AudioPipeline {
    /// Plays input `input_id` on output `output_id` (`None`: the system output). `buffer` is the
    /// output buffer remembered for that device; macOS otherwise asks for [`MAC_BUFFER`].
    pub fn start(input_id: &str, output_id: Option<&str>, buffer: Option<u32>, mix: Mix) -> Result<Self, String> {
        let input = find(Some(input_id), true).ok_or("The sound input isn't connected.")?;
        let output = find(output_id, false).ok_or("The sound output isn't connected.")?;
        let in_default = input.default_input_config().map_err(|e| format!("Sound input: {e}"))?;
        // Open the output at the device's current rate: otherwise macOS changes it system-wide.
        let out_default = output.default_output_config().map_err(|e| format!("Sound output: {e}"))?;
        let mac = cfg!(target_os = "macos");
        let in_buffer = mac.then_some(MAC_BUFFER);
        let out_buffer = buffer.or(mac.then_some(MAC_BUFFER));
        let in_config = stream_config(in_default.config(), in_default.buffer_size(), in_buffer);
        let out_config = stream_config(out_default.config(), out_default.buffer_size(), out_buffer);
        let (in_rate, out_rate) = (in_config.sample_rate, out_config.sample_rate);
        let (in_channels, out_channels) = (in_config.channels as usize, out_config.channels as usize);

        let shared = Arc::new(Shared::default());
        shared.volume_bits.store(mix.volume.to_bits(), Ordering::Relaxed);
        shared.muted.store(mix.muted, Ordering::Relaxed);
        shared.offset_ms.store(mix.offset_ms, Ordering::Relaxed);
        let started = Instant::now();

        let (writer, reader) = ring((in_rate as f64 * RING_SECONDS) as usize * 2);
        let period_ms = |frames: Option<u32>, rate: u32, fallback_ms: f64| frames.map_or(fallback_ms, |f| f as f64 * 1000.0 / rate as f64);
        // Enough cushion for one period of each side plus scheduling jitter.
        let cushion_ms = 1.5 * (period_ms(in_buffer, in_rate, 10.0) + period_ms(out_buffer, out_rate, 10.0)) + 2.0;

        let input_stream = {
            let shared = shared.clone();
            let failed = shared.clone();
            let mut writer = writer;
            let mut stereo = Vec::with_capacity(16_384);
            input
                .build_input_stream::<f32, _, _>(
                    in_config,
                    move |data, _| {
                        to_stereo(data, in_channels, &mut stereo);
                        let (left, right) = peaks(&stereo);
                        raise(&shared.peak_left, left);
                        raise(&shared.peak_right, right);
                        writer_push(&mut writer, &stereo);
                        shared.input_seen_us.store(started.elapsed().as_micros() as u64, Ordering::Relaxed);
                    },
                    move |e| failed.fail("Sound input", e),
                    None,
                )
                .map_err(|e| format!("Couldn't open the sound input: {e}"))?
        };

        let mut player = Player::new(reader, in_rate, out_rate, out_channels, cushion_ms, shared.clone(), started)?;
        let output_stream = {
            let failed = shared.clone();
            output
                .build_output_stream::<f32, _, _>(out_config, move |data, info| player.fill(data, info), move |e| failed.fail("Sound output", e), None)
                .map_err(|e| format!("Couldn't open the sound output: {e}"))?
        };
        input_stream.play().map_err(|e| format!("Sound input: {e}"))?;
        output_stream.play().map_err(|e| format!("Sound output: {e}"))?;

        let description = output.description().ok();
        Ok(Self {
            _input: input_stream,
            _output: output_stream,
            shared,
            input_id: input_id.to_owned(),
            output_id: output_id.map(str::to_owned),
            bluetooth: description.is_some_and(|d| d.interface_type() == InterfaceType::Bluetooth),
            buffer: out_buffer,
        })
    }

    pub fn set_mix(&self, mix: Mix) {
        self.shared.volume_bits.store(mix.volume.to_bits(), Ordering::Relaxed);
        self.shared.muted.store(mix.muted, Ordering::Relaxed);
        self.shared.offset_ms.store(mix.offset_ms, Ordering::Relaxed);
    }

    /// The loudest sample per channel (0 to 1) since the last call, for the level meter.
    pub fn take_peaks(&self) -> (f32, f32) {
        let take = |a: &AtomicU32| f32::from_bits(a.swap(0, Ordering::Relaxed));
        (take(&self.shared.peak_left), take(&self.shared.peak_right))
    }

    /// Dropouts so far (the ring ran dry while sound was arriving).
    pub fn underruns(&self) -> u64 {
        self.shared.underruns.load(Ordering::Relaxed)
    }

    /// Frames of sound played so far.
    #[cfg(test)]
    pub fn played_frames(&self) -> u64 {
        self.shared.played.load(Ordering::Relaxed)
    }

    /// The output's own delay, once measured.
    pub fn output_latency_ms(&self) -> Option<f64> {
        let us = self.shared.output_latency_us.load(Ordering::Relaxed);
        (us > 0).then(|| us as f64 / 1000.0)
    }

    /// A device went away or the OS stopped a stream.
    pub fn failed(&self) -> bool {
        self.shared.failed.load(Ordering::Relaxed)
    }

    /// What the OS said when it stopped a stream.
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().ok().and_then(|e| e.clone())
    }
}

fn stream_config(mut config: StreamConfig, range: &SupportedBufferSize, wanted: Option<u32>) -> StreamConfig {
    config.buffer_size = match (wanted, range) {
        (Some(frames), SupportedBufferSize::Range { min, max }) => BufferSize::Fixed(frames.clamp(*min, *max)),
        (Some(frames), SupportedBufferSize::Unknown) => BufferSize::Fixed(frames),
        (None, _) => BufferSize::Default,
    };
    config
}

fn writer_push(writer: &mut Writer, samples: &[f32]) {
    writer.push(samples);
}

/// The loudest sample on each side of interleaved stereo.
pub fn peaks(stereo: &[f32]) -> (f32, f32) {
    stereo.as_chunks::<2>().0.iter().fold((0.0f32, 0.0f32), |(l, r), f| (l.max(f[0].abs()), r.max(f[1].abs())))
}

/// Keeps the larger of what `slot` holds and `value` (both non-negative `f32`s, as bits).
fn raise(slot: &AtomicU32, value: f32) {
    let mut old = slot.load(Ordering::Relaxed);
    while value > f32::from_bits(old) {
        match slot.compare_exchange_weak(old, value.to_bits(), Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(now) => old = now,
        }
    }
}

/// Interleaved `channels`-channel sound as interleaved stereo (mono is doubled; extra channels
/// beyond the first two are left out).
pub fn to_stereo(data: &[f32], channels: usize, out: &mut Vec<f32>) {
    out.clear();
    match channels {
        0 => {}
        1 => out.extend(data.iter().flat_map(|&s| [s, s])),
        n => out.extend(data.chunks_exact(n).flat_map(|f| [f[0], f[1]])),
    }
}

/// The output callback's state: everything it needs, allocated up front.
struct Player {
    reader: Reader,
    pacer: Pacer,
    resampler: Async<f32>,
    input: Vec<f32>,
    chunk: Vec<f32>,
    /// Frames of `chunk` already played, and frames it holds.
    played: usize,
    held: usize,
    out_channels: usize,
    cushion_ms: f64,
    last: Option<Instant>,
    shared: Arc<Shared>,
    started: Instant,
}

impl Player {
    fn new(reader: Reader, in_rate: u32, out_rate: u32, out_channels: usize, cushion_ms: f64, shared: Arc<Shared>, started: Instant) -> Result<Self, String> {
        let resampler = Async::<f32>::new_poly(out_rate as f64 / in_rate as f64, 1.1, PolynomialDegree::Cubic, CHUNK, 2, FixedAsync::Output)
            .map_err(|e| format!("Sound resampler: {e}"))?;
        let input = vec![0.0; resampler.input_frames_max() * 2];
        Ok(Self {
            reader,
            pacer: Pacer::new(in_rate, cushion_ms),
            resampler,
            input,
            chunk: vec![0.0; CHUNK * 2],
            played: 0,
            held: 0,
            out_channels,
            cushion_ms,
            last: None,
            shared,
            started,
        })
    }

    fn fill(&mut self, data: &mut [f32], info: &cpal::OutputCallbackInfo) {
        let now = Instant::now();
        let dt = self.last.map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        self.last = Some(now);
        let stamp = info.timestamp();
        let latency = stamp.playback.saturating_duration_since(stamp.callback);
        self.shared.output_latency_us.store(latency.as_micros() as u64, Ordering::Relaxed);

        let offset = self.shared.offset_ms.load(Ordering::Relaxed).max(0) as f64;
        self.pacer.set_target_ms(self.cushion_ms + offset);
        data.fill(0.0);
        match self.pacer.step(self.reader.len() / 2, dt) {
            Step::Silence => return,
            Step::Skip(frames) => {
                self.reader.skip(frames * 2);
                self.held = 0;
            }
            Step::Play { relative_ratio } => {
                let _ = self.resampler.set_resample_ratio_relative(relative_ratio, true);
            }
        }
        let gain = if self.shared.muted.load(Ordering::Relaxed) {
            0.0
        } else {
            // Squared, so the slider feels even to the ear.
            let v = f32::from_bits(self.shared.volume_bits.load(Ordering::Relaxed)).clamp(0.0, 1.0);
            v * v
        };
        let channels = self.out_channels.max(1);
        let mut played = 0u64;
        for frame in data.chunks_exact_mut(channels) {
            if self.played == self.held && !self.refill() {
                break;
            }
            played += 1;
            let (l, r) = (self.chunk[self.played * 2] * gain, self.chunk[self.played * 2 + 1] * gain);
            self.played += 1;
            if channels == 1 {
                frame[0] = 0.5 * (l + r);
            } else {
                frame[0] = l;
                frame[1] = r;
            }
        }
        self.shared.played.fetch_add(played, Ordering::Relaxed);
    }

    /// Resamples the next chunk from the ring. `false` if the ring ran dry.
    fn refill(&mut self) -> bool {
        let need = self.resampler.input_frames_next();
        if !self.reader.pop_exact(&mut self.input[..need * 2]) {
            // Count it only while sound is still arriving: a gone or silent input isn't a dropout.
            let seen = std::time::Duration::from_micros(self.shared.input_seen_us.load(Ordering::Relaxed));
            if self.started.elapsed().saturating_sub(seen).as_secs_f64() < 0.1 {
                self.shared.underruns.fetch_add(1, Ordering::Relaxed);
            }
            self.pacer.underrun();
            self.held = 0;
            self.played = 0;
            return false;
        }
        let (Ok(input), Ok(mut output)) = (InterleavedSlice::new(&self.input[..need * 2], 2, need), InterleavedSlice::new_mut(&mut self.chunk, 2, CHUNK)) else {
            return false;
        };
        match self.resampler.process_into_buffer(&input, &mut output, None) {
            Ok((_, produced)) => {
                self.held = produced;
                self.played = 0;
                produced > 0
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioDevice, by_serial, peaks, same_container, to_stereo};

    #[test]
    fn peaks_per_side() {
        assert_eq!(peaks(&[0.1, -0.5, -0.8, 0.2, 0.3, 0.0]), (0.8, 0.5));
        assert_eq!(peaks(&[]), (0.0, 0.0));
    }

    /// Real devices: the default input played (muted) on the default output for 3 s. Run by hand
    /// with `cargo test -- --ignored live_sound`; needs a microphone and an output.
    #[test]
    #[ignore = "needs real audio devices"]
    fn live_sound_flows_without_dropouts() {
        use super::{AudioPipeline, Mix, inputs};
        let input = inputs().into_iter().next().expect("an input device");
        let pipeline = AudioPipeline::start(&input.id, None, None, Mix { volume: 1.0, muted: true, offset_ms: 0 }).expect("starts");
        std::thread::sleep(std::time::Duration::from_secs(3));
        assert!(!pipeline.failed(), "streams still running: {:?}", pipeline.error());
        assert!(pipeline.output_latency_ms().is_some(), "output delay measured");
        assert!(pipeline.played_frames() > 48_000, "sound played: {} frames", pipeline.played_frames());
        assert!(pipeline.underruns() <= 1, "dropouts: {}", pipeline.underruns());
        eprintln!(
            "input {} · output latency {:.1} ms · buffer {:?} · played {} frames · underruns {}",
            input.name,
            pipeline.output_latency_ms().unwrap_or(0.0),
            pipeline.buffer,
            pipeline.played_frames(),
            pipeline.underruns()
        );
    }

    #[test]
    fn mono_is_doubled_and_extra_channels_left_out() {
        let mut out = Vec::new();
        to_stereo(&[0.1, 0.2], 1, &mut out);
        assert_eq!(out, [0.1, 0.1, 0.2, 0.2]);
        to_stereo(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3, &mut out);
        assert_eq!(out, [1.0, 2.0, 4.0, 5.0]);
    }

    #[test]
    fn pairs_the_cards_own_audio_by_serial_not_by_name() {
        let inputs = [
            AudioDevice { id: "BuiltInMicrophoneDevice".into(), name: "MacBook Pro Microphone".into() },
            AudioDevice { id: "AppleUSBAudioEngine:MACROSILICON:USB2 Video:12345678:4,3".into(), name: "USB2 Digital Audio".into() },
            AudioDevice { id: "AppleUSBAudioEngine:MACROSILICON:USB2 Video:93869598:4,3".into(), name: "USB2 Digital Audio".into() },
        ];
        assert_eq!(by_serial("USB2 Video", Some("93869598"), &inputs).as_deref(), Some("AppleUSBAudioEngine:MACROSILICON:USB2 Video:93869598:4,3"));
        assert_eq!(by_serial("USB2 Video", None, &inputs), None, "no serial, no guess");
    }

    #[test]
    fn pairs_the_cards_own_audio_by_container_on_windows() {
        let inputs = [("mic".to_owned(), None), ("other-card".to_owned(), Some(7u32)), ("card-audio".to_owned(), Some(42u32))];
        assert_eq!(same_container(42, &inputs).as_deref(), Some("card-audio"));
        assert_eq!(same_container(9, &inputs), None, "no match, no guess");
    }
}
