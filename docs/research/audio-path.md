# Which audio path plays capture audio with low latency and an A/V sync offset?

Research for [#6](https://github.com/pwn233/xReveal/issues/6) (map: [#2](https://github.com/pwn233/xReveal/issues/2)).
Researched 2026-11-25 against `cpal 0.18.2` (latest on crates.io, published 2026-08-16) and `rubato 5.1.0`
(latest, 2026-11-12). Both crate sources were downloaded from `static.crates.io` and read directly. File references
like `cpal/src/host/wasapi/device.rs:976` point into those crate sources.

## Short answer

**Use `cpal` for both the capture stream and the output stream on both OSes. Join them with a lock-free SPSC ring
buffer. Do drift and rate matching, volume, mute and the A/V sync offset ourselves in the output callback.**

- **The Source's audio is a normal OS audio input device** for generic UVC/UAC USB cards on both OSes: a WASAPI
  capture endpoint on Windows and a Core Audio input device on macOS. Some Windows cards (typically PCIe or
  vendor-driver cards) instead expose audio only as an **audio pin on the DirectShow video capture filter**. cpal
  cannot reach that pin. See [Source audio](#1-how-the-sources-audio-reaches-the-os).
- **Windows:** cpal opens WASAPI in **shared, event-driven mode with the plain `IAudioClient`**. The callback
  period is the device period, **10 ms by default**. cpal does not use `IAudioClient3`, so the sub-10 ms shared
  periods that Windows 10+ offers are out of reach without our own WASAPI code. Exclusive mode is not supported by
  cpal and is not wanted, because it locks other apps off the output device.
- **macOS:** cpal opens a HAL output AudioUnit (AUHAL) per device and lets us set
  `kAudioDevicePropertyBufferFrameSize`. Small buffers are allowed, for example **15–29 frames minimum** on the
  devices probed on the dev Mac.
- **Drift and rate mismatch:** the card and the output device run on separate clocks. Measure the ring buffer's
  fill level, feed it to a slow PI controller, and adjust a **`rubato::Async` (polynomial) resampler's ratio** by
  a few hundred ppm. The rubato README names "a capture card feeding a sound card" as the use case for this. The
  same resampler covers fixed rate differences, such as 48 kHz card → 96 kHz MacBook speakers.
- **A/V sync offset:** apply it as **extra audio delay**: a larger target fill level in the ring buffer. That costs
  only memory (48 kHz stereo f32 ≈ 384 KB per second of delay). Only a negative offset (audio arrives *after*
  video) needs the picture delayed. That costs input latency, so it is a product decision (see Open questions).
- **Size:** nothing in the current dependency tree plays audio. cpal + rubato (no default features) added
  **~182 KiB** to a stripped release binary on macOS arm64, and they reuse crates GPUI already pulls in:
  `windows 0.62`, `objc2 0.6` and `objc2-core-foundation 0.3`.

## 1. How the Source's audio reaches the OS

| Case | Windows | macOS |
| --- | --- | --- |
| Generic USB UVC+UAC card (most USB cards; likely the Onten Reference card, unverified) | A separate **audio capture endpoint** (usbaudio.sys for UAC1, usbaudio2.sys for UAC2), enumerated through WASAPI / MMDevice. Its parent device instance is the same USB device as the video function. | A separate **Core Audio input device** |
| Vendor-driver / PCIe card | Sometimes only an **audio output pin on the video capture filter** (DirectShow) | Vendor-dependent (not researched) |

Evidence:

- Media Foundation enumerates audio capture devices separately from video. Audio is "supported through Windows
  Audio Session API (WASAPI)", and an audio device's identity is its **audio endpoint ID**. Combining the two
  needs an explicit aggregate source (`MFCreateAggregateSource`). For "Audio preview" Microsoft says to use WASAPI.
  ([MS: Audio/Video Capture in Media Foundation](https://learn.microsoft.com/en-us/windows/win32/medfound/audio-video-capture-in-media-foundation))
- OBS's Windows capture library first looks for a `MEDIATYPE_Audio` capture pin **on the video filter itself**.
  If there is none, it falls back to finding a **separate audio filter whose parent device instance path matches
  the video device** ([libdshowcapture `source/dshowcapture.cpp:257-266`, `source/dshow-base.cpp:584-640`](https://github.com/obsproject/libdshowcapture/blob/c13d4b7b0c66979396ba0a9060c9aafc15bb7b22/source/dshowcapture.cpp#L257-L266),
  commit `c13d4b7`). Both shapes therefore exist in the wild. The second gives us a way to **auto-pick the Audio
  route input that belongs to the chosen Source** on Windows.
- Elgato: "All Elgato Capture Devices capture sound in the Stereo (2.0) format". On Mac, OBS does not take audio
  from the video device automatically, so the user adds an "Audio Input Capture" source, which is a normal Core
  Audio input. ([Elgato: No Game Sound From Elgato Capture Device](https://help.elgato.com/hc/en-us/articles/360041852811-Troubleshooting-No-Game-Sound-From-Elgato-Capture-Device))
- The Windows inbox USB Audio 2.0 driver supports asynchronous, synchronous and adaptive endpoints, and treats
  Adaptive IN the same as Asynchronous IN ([MS: USB Audio 2.0 Drivers](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/usb-2-0-audio-drivers)).
  Either way the card's audio clock is the card's own, not the output device's. That is why drift handling is
  required (section 4).

**Uncertain:** how many current Elgato or AVerMedia PCIe cards expose *only* the DirectShow pin, with no WASAPI
endpoint. Supporting those needs a DirectShow audio path. That is outside cpal and would be a separate,
Windows-only adapter.

## 2. Windows detail (WASAPI via cpal)

What cpal 0.18.2 actually does:

- **Shared mode only.** Both stream builders hard-code `AUDCLNT_SHAREMODE_SHARED` (`cpal/src/host/wasapi/device.rs:867,968`).
  The flags are `AUDCLNT_STREAMFLAGS_EVENTCALLBACK` (`device.rs:55`). The output stream also gets `AUTOCONVERTPCM |
  SRC_DEFAULT_QUALITY` (`device.rs:974-976`), so Windows' own resampler converts any PCM rate (8–384 kHz,
  `device.rs:1327-1329`) to the engine mix format. Capture streams get no auto-convert and accept only native
  formats (`device.rs:656-658`).
- **Plain `IAudioClient::Initialize`, not `IAudioClient3`.** The code says "the callback period is always
  `GetDevicePeriod()` regardless of what is requested here; the value only affects ring-buffer latency"
  (`device.rs:847-849`). `BufferSize::Fixed(n)` therefore sets buffer *depth*, not the callback period.
- Timestamps include the stream latency from `GetStreamLatency()` and the `IAudioClock` position
  (`device.rs:914-918`, `stream.rs:916-983`). That makes them usable for measuring the pipeline.
- The optional **`realtime` feature** raises the WASAPI callback thread to real-time or high priority
  (`cpal/README.md:71`, `stream.rs:715-724`). Enable it.

What Windows offers (Microsoft, [Low Latency Audio](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/low-latency-audio)):

- By default, every app uses **10 ms** buffers to render and capture. An app gets smaller ones only through
  `IAudioClient3::InitializeSharedAudioStream` (or AudioGraph). The legal periods are multiples of
  `fundamentalPeriodInFrames` between min and max from `GetSharedModeEnginePeriod`. Once one app requests small
  buffers, the engine switches **all** streams on that endpoint to that size.
- Since Windows 10 the audio engine adds about **1.3 ms on render and about 0 ms on capture**.
- Small buffers need **driver support**. The inbox HDAudio driver supports **128 samples (2.66 ms) to 480 samples
  (10 ms) at 48 kHz**. "Not all systems" support this. Whether usbaudio.sys or usbaudio2.sys (the capture card side)
  and USB headsets support it is **not documented**.
- Exclusive mode bypasses the engine, but "no other application can use that endpoint". That is wrong for a viewer
  people use alongside Discord and other apps.
- `AUDCLNT_STREAMFLAGS_RATEADJUST` + `IAudioClockAdjustment::SetSampleRate` lets a **shared-mode render** stream
  change its sample rate on the fly. This is Windows' built-in drift compensator
  ([flags](https://learn.microsoft.com/en-us/windows/win32/coreaudio/audclnt-streamflags-xxx-constants),
  [IAudioClockAdjustment](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nn-audioclient-iaudioclockadjustment)).
  cpal does not expose it.

**Native-API alternative:** write our own WASAPI on the `windows 0.62` crate, which is already in the tree via
GPUI. Use `IAudioClient3` for the smallest period, and use `RATEADJUST` so Windows does the drift correction
instead of rubato. This is the only way below a 10 ms period on Windows. Worth doing only if measurements show
audio is the *late* stream (see section 5). Otherwise the 10 ms period is hidden under the sync delay.

## 3. macOS detail (Core Audio via cpal)

- cpal uses a **HAL output AudioUnit (AUHAL)** bound to the chosen device through
  `kAudioOutputUnitProperty_CurrentDevice` (`cpal/src/host/coreaudio/macos/device.rs:242-248`). That is the
  lowest-level public per-device I/O path, so a native rewrite gains nothing on latency.
- `BufferSize::Fixed(n)` is checked against `kAudioDevicePropertyBufferFrameSizeRange` and then written to
  `kAudioDevicePropertyBufferFrameSize`. cpal notes that this "is a DEVICE-LEVEL property"
  (`device.rs:1012-1037`). Apple's header defines it as "the number of frames in the IO buffers"
  (`CoreAudio.framework/Headers/AudioHardware.h:945-949`, macOS SDK 27.2).
- Probe on the dev Mac (cpal `default_*_config().buffer_size()`): built-in speakers **96 kHz, 29–4096 frames**;
  built-in mic **96 kHz, 29–4096**; a Continuity iPhone mic **48 kHz, 15–4096**. A 64-frame buffer is 1.3 ms at
  48 kHz. **No capture card was attached**, so the card's own range is still unmeasured.
- cpal timestamps add `kAudioDevicePropertyLatency + kAudioDevicePropertySafetyOffset`
  (`device.rs:1042-1050`). Apple's header says stream latency must be added to device latency
  (`AudioHardwareBase.h:688-693`).
- **Side effect to avoid:** building any stream (input *or output*) makes cpal try to set the device's physical
  format or **nominal sample rate** to the requested rate (`device.rs:713-725`, `837-849`). Opening the output at
  48 kHz on 96 kHz MacBook speakers would switch the system's output rate. **Open the output at the device's
  current nominal rate and resample ourselves.**
- Apple's AUHAL guidance: two devices need **two AUHALs on separate I/O procs**, which "cannot make a direct
  connection", so data has to be passed between them. The AUHAL converter does *not* do sample-rate conversion
  ("the device's sample rate should match the desired sample rate"). Rate conversion belongs on a separate path
  with buffering. ([Apple TN2091](https://developer.apple.com/library/archive/technotes/tn2091/_index.html))
  This is exactly the cpal two-streams + ring buffer + resampler design.
- Core Audio says devices with clock domain 0 must be "assumed to be separate from every other device's clock
  domain" (`AudioHardwareBase.h:665-671`). `kAudioDevicePropertyActualSampleRate` gives the rate "as measured by
  its time stamps" (`AudioHardware.h:980-982`), a possible drift-estimate input.
- **Alternatives considered:**
  - `AVCaptureAudioPreviewOutput` (macOS 10.7+) plays captured audio to any Core Audio output by
    `outputDeviceUniqueID`, with `volume` 0.0–1.0
    ([Apple docs](https://developer.apple.com/documentation/avfoundation/avcaptureaudiopreviewoutput)). It would
    be zero code for route, volume and mute. But it exposes **no buffer-size or delay control**, so there is no A/V
    sync offset, and its latency is undocumented. Rejected for v1.
  - An aggregate device with `kAudioSubDeviceDriftCompensationKey` (`AudioHardware.h:1796-1801`) would let Core
    Audio do the drift correction. But it creates a system-visible device, and the resulting latency is
    unmeasured. Not recommended.

## 4. Drift and resampling

- **Why it is needed:** the card's audio clock (USB async/adaptive endpoint, section 1) and the output device's
  clock are independent. Even when both say 48 kHz they differ by some ppm. Without correction the ring buffer
  slowly fills (latency grows) or empties (clicks). Rates also differ for real: the probe found 96 kHz Mac
  speakers against a 48 kHz card.
- **rubato 5.1.0 fits:**
  - `Async` resamplers allow "changing the resampling ratio at any time to compensate for this drift". The README
    recommends `Async` for "a capture card feeding a sound card". `Async::new_poly` is the cheap variant: no
    anti-aliasing filter, "the quality loss is often subtle".
  - `Slip` inserts or drops single frames with a crossfade, for rates "a few ppm" apart. It has no delay and very
    little CPU cost.
  - `process_into_buffer` does no allocation and is real-time safe.
  - The `fft_resampler` default feature can be disabled "to reduce the resulting binary size".

  Sources: `rubato/README.md` (sections "Asynchronous resampling", "Choosing a resampler", "Real-time
  considerations", "Cargo features") and `rubato/src/slip.rs`.
- **Delay added by the resampler:** `output_delay() = nbr_points × ratio / 2` (`rubato/src/asynchro.rs:747-749`).
  Cubic poly uses 4 points (`asynchro_fast.rs:30-36`), so the delay is about 2 frames (0.04 ms). `Slip` adds 0.
- **Recommended loop:**
  1. The capture callback converts samples to f32 and pushes them into an SPSC ring buffer. No other work there.
  2. The output callback pulls `output_frames_next()` frames through `Async::new_poly(Cubic)` with
     `FixedAsync::Output`, then applies gain.
  3. Every callback, compare ring fill to the **target fill** (base safety margin + A/V sync offset). A slow PI
     controller sets `set_resample_ratio_relative(1 ± ε, ramp=true)` with ε clamped to about ±0.1–0.5 %, which is
     inaudible.
  4. If fill leaves a hard window (device hiccup, Source re-plug), drop or pad with a short crossfade and reset the
     controller.

  rubato's README advises against heavy work in the *capture* callback. Running a cubic poly resampler in the
  *output* callback is cheap, but whether that holds on a weak laptop is **unmeasured**. Fallback: a dedicated
  resampler thread, which adds one chunk of latency.
- On Windows, instead of rubato, Windows can do both jobs: `AUTOCONVERTPCM` handles fixed rate differences, and
  `RATEADJUST` handles drift. This needs our own WASAPI code (section 2).

## 5. Applying the A/V sync offset

- **Mechanism:** the offset in ms becomes `offset × rate / 1000` frames added to the ring buffer's target fill.
  The drift controller in section 4 already steers fill to the target, so a changed offset can be eased in by
  the same controller, or applied at once with a pad or drop plus crossfade. Memory cost: 48 kHz × 2 ch × 4 B ≈
  **384 KB per second** of delay.
- **Delaying audio versus delaying video.** This is reasoning from the latency figures above and is **not yet
  measured** on the Reference card:
  - Video path: at least one 60 Hz frame of capture (16.7 ms) plus at least one display refresh, plus card
    processing.
  - Audio path on Windows: about 10 ms capture period + about 10 ms render buffer + 1.3 ms engine + ring margin.
  - Audio path on macOS: a few ms.

  So audio will usually arrive *earlier* than video, and the offset will mostly be a **positive audio delay**.
  That means **audio latency below video latency is free**: the 10 ms cpal period on Windows ends up hidden inside
  the sync delay, and `IAudioClient3` buys nothing.
- **When audio is late** (Bluetooth headphones, which add their own latency; or a card that delays audio more than
  video), the only fix is holding video frames back. That adds input latency, which goes against priority #1.
- cpal's input and output timestamps (`InputCallbackInfo`/`OutputCallbackInfo`, which include device and stream
  latency, section 2 and 3) let xReveal **estimate the audio path latency itself**. That could give a sensible
  default offset instead of 0, if the video path's latency is known too.

## 6. Volume and mute

Apply gain in software in the output callback: multiply each sample, with a ~5–10 ms linear ramp on every change
to avoid zipper noise. Mute ramps to gain 0 and **keeps both streams running**, so the drift controller and buffer
stay locked and unmuting is instant. This is the same code on both OSes. It avoids changing the system or
endpoint volume, which would also affect other apps.

## 7. Size and dependencies

- **Current tree:** `Cargo.lock` has no audio I/O crate (no `cpal`, `rodio`, `coreaudio-*`, `objc2-core-audio`,
  `objc2-avf-audio` or WASAPI wrapper). The capture-like crates present (`zed-scap`, `screencapturekit 0.2.8`,
  `windows-capture 1.5.0`) are GPUI's screen-capture feature (`gpui-pre` feature `screen-capture = ["scap"]`) and
  carry no audio. `gpui-pre-windows` enables only `Win32_Media` of the `windows` crate, not `Win32_Media_Audio`.
- **cpal 0.18.2 deps:**
  - Windows: `windows 0.62` + `windows-core 0.62`, the same major as GPUI's `windows 0.62.2`, with extra features
    such as `Win32_Media_Audio`; plus `dasp_sample`.
  - macOS: `coreaudio-rs 0.14`, `objc2-core-audio`, `objc2-audio-toolbox`, `objc2-core-audio-types`,
    `objc2-avf-audio` (pulled in by coreaudio-rs) and `mach2`. `objc2 0.6`, `objc2-foundation 0.3`,
    `objc2-core-foundation 0.3` and `mach2 0.6` are already in the lockfile.

  Source: `cpal/Cargo.toml`, plus crates.io dependency data for `coreaudio-rs 0.14.2`.
- **Measured:** a probe binary opening a cpal input and output stream plus a `rubato::Async` poly resampler was
  **286,096 B → 472,400 B (+186 KB ≈ 182 KiB)**. Build settings: release, `lto`, `codegen-units=1`,
  `opt-level="s"`, stripped, macOS arm64, rustc 1.100.0. Inside xReveal the real delta should be smaller because
  objc2 and windows are shared. The Windows size is not measured.

## Open questions (for later tickets)

1. **Negative A/V sync offset:** may xReveal delay *video* (costing input latency) when audio is late, for example
   with Bluetooth headphones? Or does v1 only delay audio, clamping the offset at 0 and warning?
2. **DirectShow-pin-only cards on Windows:** does v1 support cards whose audio exists only as a pin on the video
   capture filter? That needs a Windows-only DirectShow audio adapter beside cpal. It hangs on the capture-backend
   choice (Media Foundation vs DirectShow), which is being researched separately.
3. **Auto-pairing the Audio route with the Source:** match the audio endpoint to the video device by
   parent device instance on Windows (as OBS does), and by USB location or model on macOS. This touches
   Remembered setup identity.
4. **Measure on the Reference card:** the card's audio rate and format, its buffer-size range on macOS, the real
   drift in ppm, and video-vs-audio arrival skew. The default sync offset should come from this.
5. **Own WASAPI later?** Only if measurement shows audio lagging video on Windows: `IAudioClient3` minimum period
   plus `RATEADJUST`, written on the `windows` crate already in the tree.
6. **Where the resampler runs:** inline in the output callback (lowest latency) or on its own thread (safer on weak
   CPUs). Decide once the CPU cost is measured.

## Sources

- cpal 0.18.2 source: <https://crates.io/crates/cpal/0.18.2>, <https://github.com/RustAudio/cpal> (files cited inline)
- rubato 5.1.0 source and README: <https://crates.io/crates/rubato/5.1.0>, <https://docs.rs/rubato/5.1.0>
- Microsoft, Low Latency Audio: <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/low-latency-audio>
- Microsoft, AUDCLNT_STREAMFLAGS_XXX: <https://learn.microsoft.com/en-us/windows/win32/coreaudio/audclnt-streamflags-xxx-constants>
- Microsoft, IAudioClockAdjustment: <https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nn-audioclient-iaudioclockadjustment>
- Microsoft, Audio/Video Capture in Media Foundation: <https://learn.microsoft.com/en-us/windows/win32/medfound/audio-video-capture-in-media-foundation>
- Microsoft, USB Audio 2.0 Drivers: <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/usb-2-0-audio-drivers>
- OBS libdshowcapture @ c13d4b7: <https://github.com/obsproject/libdshowcapture>
- Apple TN2091, Device input using the HAL Output Audio Unit: <https://developer.apple.com/library/archive/technotes/tn2091/_index.html>
- Apple, AVCaptureAudioPreviewOutput: <https://developer.apple.com/documentation/avfoundation/avcaptureaudiopreviewoutput>
- Apple CoreAudio headers `AudioHardware.h`, `AudioHardwareBase.h` (macOS SDK 27.2, Xcode)
- Elgato, No Game Sound From Elgato Capture Device: <https://help.elgato.com/hc/en-us/articles/360041852811-Troubleshooting-No-Game-Sound-From-Elgato-Capture-Device>
