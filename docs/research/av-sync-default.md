# How is a card's sound-vs-picture skew measured, and what is the generic default A/V sync offset?

Research for [#27](https://github.com/pwn233/xReveal/issues/27) (map: [#2](https://github.com/pwn233/xReveal/issues/2)).
Researched 2026-11-25. Builds on [#6 audio path](https://github.com/pwn233/xReveal/blob/research/audio-path/docs/research/audio-path.md),
[#7 capture card families](https://github.com/pwn233/xReveal/blob/research/capture-card-families/docs/research/capture-card-families.md),
[#10 performance budgets](https://github.com/pwn233/xReveal/issues/10),
[#14 negative offsets](https://github.com/pwn233/xReveal/issues/14) and
[#20 latency measurement](https://github.com/pwn233/xReveal/blob/research/latency-measurement/docs/research/latency-measurement.md).

Labels: **[primary]** standard, vendor doc or source code. **[measurement]** a published instrumented test by a
third party. **[community]** forum posts, gists, wikis, which carry less weight. **[measured here]** run on the dev Mac
(MacBook Pro Mac15,3, macOS 26.6.2) with the Reference card plugged in. **[inference]** my own reasoning that no
source states.

**Sign used throughout.** *Sound lead* = how many ms the sound reaches the user before the picture it belongs to.
The A/V sync offset that cancels a skew is equal to the sound lead: sound 20 ms early needs +20 ms (delay the sound,
free); sound 20 ms late needs −20 ms (delay the picture, costs 20 ms of input lag, per #14).

## Short answer

- **Generic default: 0 ms.** Confidence: **medium** that 0 is unnoticeable for unknown UVC/UAC cards played to a
  wired output, **low** on any card's exact skew. The best estimate for a generic USB card through xReveal is a
  sound lead of roughly **−40 to +10 ms**: usually the sound is slightly *late*. That sits inside the ITU-R BT.1359
  detectability window (+45 ms early to −125 ms late) and almost entirely inside EBU R37 (+40/−60 ms).
  - A positive default would make the likely sound-late case worse.
  - A negative default would add input lag on every unknown card, against #10 and #14.
  - 0 is also what OBS, VLC and mpv default to.
- **Published skews are almost nonexistent.** The only instrumented capture-card A/V measurement found is caplat's
  AVerMedia GC553: **sound arrives 1–6 ms after the picture** at the app [measurement]. No credible tester has
  published a sound-vs-picture skew for MacroSilicon MS21xx or Elgato cards. The numbers that circulate are OBS
  Sync Offset settings: +400 ms (MS2109 gist), −50 to −200 ms (AVerMedia FAQ for the GC513) and 650 ms (Elgato's
  old HD60). They compensate OBS's own buffering and pipeline, disagree in sign, and **don't transfer to xReveal**.
- **xReveal's own output path is probably the bigger part, and it makes the sound late.**
  - The audio path through cpal, the ring buffer and rubato costs about **25–42 ms** on the Mac's built-in
    speakers. This is device-reported Core Audio latency measured here plus buffer sizes. On Windows it is about
    25–35 ms (10 ms shared periods each way).
  - xReveal's video path is budgeted at **≤ 8.3 ms** (#10).
  - So xReveal's own share is a sound lead of about **−10 to −35 ms**.
  - The card side is roughly −6 to +20 ms. Video waits for a whole frame and the MJPEG encode, while audio waits in
    the card's buffer.
  - This **reverses #6's working assumption** that the offset would "mostly be a positive audio delay". For wired
    outputs it will be near 0 or slightly negative. Bluetooth headphones push it to **−40 to −160 ms**.
- **Method:** three layers, from cheapest to most precise:
  - **(A) End to end, both OSes:** the PS5 plays a flash + beep MP4 from USB through Media Gallery. A 240 fps phone
    films the screen with sound. A script reads the recording's own timestamps. The clip generator and analysis
    script are in [`tools/av-sync/`](../../tools/av-sync).
  - **(B) In-app arrival skew, both OSes, no camera:** xReveal detects the flash in captured frames and the beep in
    captured samples on the host clock. It adds its own output path from cpal timestamps and present times.
  - **(C) Card-only loopback:** `caplat sync` on Windows today. A macOS port is an open question from #20.

## 1. How much skew people notice

| Standard | Sound early (lead) | Sound late (lag) | Notes |
|---|---|---|---|
| ITU-R BT.1359-1, detectability | **+45 ms** | **−125 ms** | "subjective evaluations show that detectability thresholds are about +45 ms to −125 ms and acceptability thresholds are about +90 ms to −185 ms on the average, a positive value indicates that sound is advanced with respect to vision" [primary] ([ITU PDF](https://www.itu.int/dms_pubrec/itu-r/rec/bt/R-REC-BT.1359-1-199811-I!!PDF-E.pdf)) |
| ITU-R BT.1359-1, acceptability | +90 ms | −185 ms | Same source. The broadcast chain budget is tighter: +22.5 / −30 ms from final source selection to transmitter |
| EBU R37 | +40 ms | −60 ms | End to end. Each stage gets +5 / −15 ms ([Wikipedia summary](https://en.wikipedia.org/wiki/Audio-to-video_synchronization), [EBU R37](https://tech.ebu.ch/publications/r037)) [secondary: not read in the original] |
| ATSC IS-191 | +15 ms | −45 ms | Same Wikipedia summary [secondary] |

- **Takeaway [inference]:** people tolerate late sound about 3× better than early sound. Late sound happens in
  nature, because sound travels about 2.9 ms per metre.
  - So when unsure, err towards slightly late sound.
  - Correcting a lag under about 45 ms is not worth input lag. A negative offset is only worth its cost for big lags
    such as Bluetooth.
- caplat's README says BT.1359's threshold is "45 ms late". The standard says **45 ms early** (sound advanced). Late
  sound is detectable only at 125 ms.

## 2. Published sound-vs-picture skews

| Card | Skew | How it was obtained | Weight |
|---|---|---|---|
| AVerMedia GC553 (USB 3, NV12 720p60, Windows) | Sound **1–6 ms after** the picture, measured at app arrival (sound lead −1 to −6 ms) | `caplat sync`: the PC's own HDMI output feeds the card. Barcode frames and chirps are timed from emission (DXGI vblank, `IAudioClock`) to arrival, on QPC. p50 audio minus p50 video | **[measurement]** ([caplat README](https://github.com/virgil724/caplat)). One card, one OS, arrival only (no display or speaker) |
| GC553 audio alone | 7–20 ms from emission to driver stamp, as a **sawtooth**: it creeps up 0.26 ms/s, then drops about 20 ms | Same tool. The card's audio clock and the PC's HDMI clock disagree, and the card slips its buffer | [measurement] (same) |
| MS2109 (USB 2, MJPEG 1080p30), OBS | OBS Sync Offset **+400 ms** ("Different resolutions will need different sync offsets") | Gist settings table. The value isn't explained, and the method isn't stated | [community] ([Iksas gist](https://gist.github.com/Iksas/a5db5a6551956854f89cf1dac1a01950)) |
| AVerMedia Live Gamer Portable 2 Plus (GC513), OBS | OBS Sync Offset **−50 to −200 ms** for recordings and streams | Vendor FAQ. No method given | [primary for OBS advice] ([AVerMedia FAQ](https://www.avermedia.com/support/faq/how-do-i-set-up-the-audio-in-obs-and-streamlabs-obs)) |
| Elgato Game Capture HD60 (original, H.264 over USB 2), OBS Classic / XSplit | **650 ms**, "the average delay of the Elgato Game Capture HD60" | Vendor KB, for the old Game Capture HD software path (the page now returns 403; quoted via search) | [primary, obsolete] ([Elgato KB](https://help.elgato.com/hc/en-us/articles/360027953872-Elgato-Game-Capture-HD60-Adjusting-audio-delay-for-XSplit-and-OBS)) |
| Elgato HD60 X | Marketing says "Sub 100ms for seamless audio/video syncing". No skew is published. The only reviews with figures measure video only (about 60 ms at 720p60 capture) | — | [primary marketing] + [measurement, video only] ([HyperHDR review](https://www.hyperhdr.eu/2022/07/elgato-game-capture-hd60x-review.html)) |
| "Typical" offsets on SEO guides (50–300 ms, ±) | Contradictory, with no method | — | Not used |

**Why the OBS numbers don't transfer [primary + inference]:**
- OBS defaults every source's sync offset to **0** (`source->sync_offset = 0`, `libobs/obs-source.c:221`). It lines
  up audio and video by **timestamps**.
  - The audio timestamp gets `in.timestamp += sync_offset` (`obs-source.c:1687-1688`).
  - The Windows monitoring path delays audio towards the last video frame's timestamp, applies `sync_offset` too
    (`libobs/audio-monitoring/win32/wasapi-output.c:52-110`, line 68), and holds audio that is "rushing" more than
    75 ms ahead.
  - So in OBS the Sync Offset only corrects what the device timestamps get wrong, plus OBS's own buffering.
- OBS's DirectShow source turns on video **buffering** in Auto mode for encoded formats and named Elgato devices
  (`IsDelayedDevice`, `plugins/win-dshow/win-dshow.cpp:857-883`). That changes the skew the user sees.
  - caplat showed DirectShow audio through ffmpeg defaults at **210 ms** and video through default ffmpeg at
    **60–76 ms** on the same card that is 18 ms through MF.
- Source: OBS master, fetched 2026-11-25 (`raw.githubusercontent.com/obsproject/obs-studio/master/...`).
- Older OBS forum threads claim monitoring ignores the Sync Offset. Current source applies it on Windows
  ([forum, old](https://obsproject.com/forum/threads/sync-offset-problem.64365/)) [community, outdated].
- Other players: VLC's audio desync and mpv's `--audio-delay` also default to 0. They are file players that trust
  container timestamps [inference: well-known defaults, not re-checked in source].

## 3. Where the skew comes from: card versus xReveal

Stages for the Reference card (MS2131, USB 2, 720p60 MJPEG decoded by macOS, separate UAC "USB2 Digital Audio"
48 kHz). Sound lead contribution = (audio stage time) subtracted from (video stage time).

| Stage | Video | Audio | Source |
|---|---|---|---|
| **Card, before the host clock** | ≥ 1 whole frame before it can be sent (16.7 ms at 60 Hz, 20 ms at 50 Hz), plus MJPEG encode and USB 2 transfer (unknown, a few ms) | Card's audio buffer: **unknown for MS21xx**. GC553: 7–20 ms, sawtooth | caplat README [measurement]; #7 |
| macOS MJPEG decode (AVFoundation) | about 2 ms at 720p, 5 ms at 1080p | — | [community] via #7 |
| Capture side on the host | — | Card input fixed latency **74 + 74 = 148 frames = 3.1 ms** (device latency + safety offset), plus the IO buffer: **15–4096 frames**, current 512 = 10.7 ms | **[measured here]** with [`tools/av-sync/coreaudio-latency.swift`](../../tools/av-sync/coreaudio-latency.swift) |
| xReveal processing | Owned surface, present on arrival. Budget **≤ 8.3 ms** from arrival to on screen | Ring safety margin (a design choice, about 1 output buffer) + rubato cubic poly **≈ 0.04 ms** | #10; #6 (`rubato/src/asynchro.rs:747-749`) |
| Output side on the host | Scanout to mid-screen ≈ ½ refresh (2.8 ms at 180 Hz) + monitor processing (a few ms on a gaming monitor) | MacBook Pro speakers fixed **140 + 148 + 1390 = 1678 frames at 96 kHz = 17.5 ms** + IO buffer (29–4096, current 512 = 5.3 ms) | [measured here]; monitor: [inference] |
| Windows audio instead | — | Shared WASAPI: driver stamp to program p50 9.4 ms (10 ms periods) on capture; 10 ms render buffer + 1.3 ms engine on render | caplat [measurement]; [MS Low Latency Audio](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/low-latency-audio) [primary] |
| Bluetooth output instead | — | AirPods Pro 2 **126 ms** vs built-in speakers 83 ms on an iPad, so about +43 ms (AirPods Pro 1: 167 ms). RTINGS reports about 160 ms for SBC on Windows 11 | [measurement] ([Coyle](https://stephencoyle.net/airpods-pro-2)); RTINGS figure via search summary [secondary] |

**Adding it up [inference]:**
- **xReveal's own share** (arrival on the host clock to light and sound):
  - Audio: about 3.1 + buffers + margin + 17.5 ms. That is **≈ 26 ms with 128-frame buffers** and **≈ 42 ms with
    the current 512-frame defaults** on the Mac speakers. Windows is about 25–35 ms.
  - Video: about 5–15 ms (≤ 8.3 ms budget plus scanout and monitor).
  - Net sound lead from xReveal itself: **about −10 to −35 ms** (sound late). Only the IO buffer sizes and the ring
    margin are xReveal's choice: about 5–20 ms on macOS, a fixed 10 ms period each way on Windows. The Core Audio
    fixed latencies and Windows' 10 ms periods are not.
- **The card's share:** video waits at least a frame plus the encode, and audio waits in the card's buffer.
  - GC553 (uncompressed): these cancel to −1 to −6 ms.
  - MJPEG USB 2 cards add encode time to the video only, so expect roughly 0 to +20 ms. Unmeasured.
- **Total for a generic card on a wired output: about −40 to +10 ms.** Add −40 to −160 ms for Bluetooth.
- Measured device latency for the dev Mac's built-in mic was 36 ms. It is irrelevant here but shows that "a few ms
  on macOS" (#6 section 5) understates real Core Audio devices.
- **Observation:** the Reference card also exposes a 2-channel **output** endpoint ("USB2 Digital Audio" out,
  50 + 50 frames = 2.1 ms fixed). It may be the card's headphone jack. Unverified [measured here + inference].

## 4. Measurement method

### A. End to end with the PS5 and a phone (macOS and Windows; the referee)

1. **Make the clip:** `tools/av-sync/make-clip.sh avsync-1080p60.mp4` (needs ffmpeg).
   - It makes 120 s at 1080p60. Each second, one full-white frame and a 1 kHz tone burst of the same length (1/60 s)
     start together at t = k + 0.5 s. H.264 High L4.2 + AAC-LC in MP4, which the PS5 plays: "MP4 Video: H.264/MPEG-4
     AVC High Profile Level 5.2 Audio: AAC LC", from an exFAT/FAT32 USB drive, inside a folder, via **Media
     Gallery → USB** [primary] ([PlayStation support](https://www.playstation.com/en-us/support/hardware/play-video-music-discs-usb-drives/)).
   - **AAC priming trap [primary]:** AAC encoders put silent priming frames first. Apple: "the audio will be 2112
     samples behind the video" if a player doesn't trim them, and players should assume 2112
     ([Apple TN2258](https://developer.apple.com/library/archive/technotes/tn2258/_index.html)). 2112 frames is
     44 ms at 48 kHz. ffmpeg's own `aac` encoder primes 1024 frames and signals that only in the MP4 edit list.
   - So the script uses Apple's encoder (`aac_at`, on macOS ffmpeg). It primes 2112 frames **and** writes a
     2112-frame edit list (checked with `ffprobe -v trace`: `media time: 2112`). A player that follows either
     convention plays it in sync. It also uses `-bf 0`, so the video track needs no edit list.
   - Self-check [measured here]: `av_skew.py` on the clip gives **0.0 ms on all 120 pairs**. On copies with the
     audio shifted +40 ms and −25 ms (the second re-encoded at 240 fps), it reads +39.7 and −25.0 ms. Whether the
     PS5 player honours priming is unverified.
2. **Set up:**
   - PS5 → Reference card → xReveal fullscreen, with the Audio route you want to calibrate (the monitor's speakers,
     headphones from the Mac, and so on).
   - The A/V sync offset is **0**.
   - The PS5's video output range and resolution are as played.
   - Put the phone's microphone **within 0.5 m of the speaker or headphone driver**. Each metre adds 2.9 ms of
     sound delay. Headphones: hold one earcup against the phone mic.
3. **Calibrate the phone once:** film a clap or clapperboard at 0.3 m in the same mode.
   - Phones have their own camera-vs-mic offset. Catchin' Sync, an iPhone A/V-sync app, says "Each device and IOS
     combo will have a slight variation" and has users enter a per-model offset [primary vendor]
     ([Catchin' Sync](https://www.quietart.co.nz/catchinsync/)).
   - Subtract this from every later reading.
4. **Record** at least 30 flashes in slow motion (240 fps ≈ 4.2 ms per frame; 120 fps is acceptable). Do it 3 times
   with a restart of xReveal in between, and once more after about 10 minutes of play. caplat saw a bimodal start-up
   state in ffmpeg and a card audio sawtooth that only long runs show.
5. **Analyse the original file**, not the slowed-down playback: `python3 -I tools/av-sync/av_skew.py rec.mov --crop W:H:X:Y`.
   - It finds flashes by the YAVG jump (ffmpeg `signalstats`) and beep onsets by a 1 ms envelope. It pairs them and
     prints the median skew and the offset that cancels it.
   - iPhone originals carry real-time timestamps for the slow-motion video [inference: check that `ffprobe` shows
     about 240 fps and a wall-clock duration]. Some Android slow-motion modes drop audio. Use a normal 60 fps
     recording there and average more events (bias ≈ +½ camera frame).
   - Manual alternative: count frames, as in the frame-counting method of
     [tembrica's A/V sync test](https://tembrica.com/en/av-sync-test): "Frames between flash visible and audio peak × (1000/240) = ms delay".
6. **Result:**
   - Required offset = −(median skew) − phone calibration.
   - Expected resolution is about ±5 ms: one 240 fps frame plus phone jitter. This is limited by the camera, so for
     sub-frame accuracy use B or C.
   - This number includes the Source and its player, the card, xReveal, the monitor and the speakers. That is exactly
     what a user hears and sees.

### B. In-app arrival skew (both OSes, no camera; a dev mode inside xReveal)

- Play the same clip. xReveal detects the flash (mean-luma jump in a captured frame) and the beep onset (envelope
  in captured samples), each on the **host clock**:
  - Video: Media Foundation `MFSampleExtension_DeviceTimestamp` (QPC) on Windows, or the AVFoundation PTS converted
    with `CMSyncConvertTime` to host time on macOS [primary, from #20].
  - Audio: cpal's `InputCallbackInfo` capture timestamp. On macOS this already includes device latency + safety
    offset (`cpal/src/host/coreaudio/macos/device.rs:1042-1050`). On Windows it includes `GetStreamLatency` and the
    `IAudioClock` position [primary, from #6].
- `audio arrival − video arrival` is the **card + Source** skew.
- Add **xReveal's own share**, read live:
  - Audio: cpal output `playback` instant minus `callback` instant, plus the ring fill in ms.
  - Video: capture stamp to `MTLDrawable.presentedTime` (macOS) or DXGI `SyncQPCTime` (Windows) [primary, from #20].
- **A − (B arrival + own share)** = the last metre (monitor processing, speakers beyond what the OS reports) plus
  phone bias.
- This splits the ticket's question, "how much is xReveal vs the card", per run and without guessing.

### C. Card-only loopback (precise; for known-card defaults)

- **Windows today:** `caplat sync` with a PC's second HDMI output feeding the card. Emission is timed by vblank and
  `IAudioClock`, arrival by MF/WASAPI, on one clock. It is sample-exact on audio and gives a p50 histogram. It needs
  neither a PS5, a phone nor a clip player [measurement tool] ([caplat](https://github.com/virgil724/caplat)).
- **macOS:** the same idea is #20's open question 2 (port the loopback: HDMI out from the Mac, `presentedTime` for
  emission). The audio half would use the Core Audio output timestamp.
- Use C (or B with A as a cross-check) to set the built-in default for known cards. That measures the card
  without any one user's monitor or speakers.

## 5. Recommended generic default

**0 ms.**

- **Confidence: medium** that 0 is unnoticeable on unknown UVC/UAC cards with wired output. The estimated residual
  of −40 to +10 ms is inside BT.1359 detectability and mostly inside EBU R37.
  - Weakest link 1: the card-side audio buffering of MS21xx and Elgato cards, which is unpublished.
  - Weakest link 2: xReveal's final buffer sizes, which are not yet chosen.
- **Confidence: low** on any card's exact number. Every figure except caplat's GC553 and the Core Audio values
  measured here is inference.
- **Why not positive:** the likely total is sound-late. A positive default would add lag on top, and OBS-style
  "+100 to +400 ms" advice comes from a different pipeline.
- **Why not negative:** it would add input lag on every unknown card, and #10 budgets apply at offset ≥ 0.
- **Revisit when** the Reference card is measured, a build-time step with method A + B or C. If it lands outside
  ±20 ms, consider using it as a **family default** for MacroSilicon vendor IDs (`345f`, `534d`) rather than for
  that one product ID.

## Open questions (for later tickets)

1. **Known-card defaults that come out negative.** #14 assumed a known card's default "delays the sound to match the
   picture". Section 3 suggests the total may be *sound-late* on wired outputs, which needs a negative offset (input
   lag). Should built-in defaults be clamped to ≥ 0, so they only ever delay sound, with a negative value offered
   but not applied?
2. **Route-aware compensation.** Should xReveal subtract the Audio route's *reported* output latency (Core Audio
   device + stream latency, WASAPI stream latency) automatically, so the user's A/V sync offset only covers the
   card and the last metre?
   - Bluetooth would then show up without user action.
   - Unverified: whether macOS reports Bluetooth latency truthfully through `kAudioDevicePropertyLatency`. The probe
     here saw no Bluetooth device.
3. **Audio buffer sizes are now a sync lever, not only a latency nicety.**
   - macOS: the card allows 15-frame IO buffers and the speakers 29. The 512-frame defaults cost about 16 ms of
     sound lateness.
   - Windows: #6 open question 5 (own WASAPI / `IAudioClient3`) gains weight if measurement confirms sound is the
     late stream. caplat found every endpoint stuck at 10 ms shared periods even with `IAudioClient3`.
4. **Does the PS5 Media Gallery player honour AAC priming?** Check it once: the clip read via method B arrival
   should match the same clip played from a Mac's HDMI output.
5. **MS21xx card-side audio buffering and drift sawtooth:** unmeasured. It needs B or C on the Reference card,
   with long runs.
6. **What is the Reference card's output endpoint** ("USB2 Digital Audio" out)? If it is a headphone jack on the
   card, is it a sensible Audio route?

## Sources

- ITU-R BT.1359-1 (1998), *Relative timing of sound and vision for broadcasting*: <https://www.itu.int/dms_pubrec/itu-r/rec/bt/R-REC-BT.1359-1-199811-I!!PDF-E.pdf>
- EBU R37 page: <https://tech.ebu.ch/publications/r037>; summary of R37 and ATSC IS-191: <https://en.wikipedia.org/wiki/Audio-to-video_synchronization>
- caplat (capture latency and A/V skew tool, GC553 measurements): <https://github.com/virgil724/caplat>
- OBS Studio source (master, 2026-11-25): `libobs/obs-source.c`, `libobs/audio-monitoring/win32/wasapi-output.c`, `plugins/win-dshow/win-dshow.cpp` at <https://github.com/obsproject/obs-studio>
- AVerMedia FAQ, GC513 OBS audio: <https://www.avermedia.com/support/faq/how-do-i-set-up-the-audio-in-obs-and-streamlabs-obs>
- Elgato KB, HD60 audio delay (403 to direct fetch; quoted via search): <https://help.elgato.com/hc/en-us/articles/360027953872-Elgato-Game-Capture-HD60-Adjusting-audio-delay-for-XSplit-and-OBS>
- MS2109 notes gist [community]: <https://gist.github.com/Iksas/a5db5a6551956854f89cf1dac1a01950>
- HyperHDR HD60 X review [measurement, video only]: <https://www.hyperhdr.eu/2022/07/elgato-game-capture-hd60x-review.html>
- PlayStation support, USB video playback: <https://www.playstation.com/en-us/support/hardware/play-video-music-discs-usb-drives/>
- Apple TN2258, AAC encoder delay: <https://developer.apple.com/library/archive/technotes/tn2258/_index.html>
- Catchin' Sync (phone A/V-sync app, per-device offsets): <https://www.quietart.co.nz/catchinsync/>
- tembrica A/V sync test (flash + click, frame counting): <https://tembrica.com/en/av-sync-test>
- Harkwood Sync-One2 (dedicated hardware meter, ±750 ms range) as a paid alternative: <https://sync-one2.harkwood.co.uk/>
- Stephen Coyle, AirPods Pro 2 latency: <https://stephencoyle.net/airpods-pro-2>
- Microsoft, Low Latency Audio: <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/low-latency-audio>
- Measured here: `tools/av-sync/coreaudio-latency.swift` output on the dev Mac with the Reference card attached:

  ```
  USB2 Digital Audio [usb ] in  48000 Hz devLat=74  safety=74  streamLat=0    buffer=512 (range 15-4096)
  USB2 Digital Audio [usb ] out 48000 Hz devLat=50  safety=50  streamLat=0    buffer=512 (range 15-4096)
  MacBook Pro Microphone    in  96000 Hz devLat=0   safety=100 streamLat=2888 buffer=512 (range 29-4096)
  MacBook Pro Speakers      out 96000 Hz devLat=140 safety=148 streamLat=1390 buffer=512 (range 29-4096)
  ```
