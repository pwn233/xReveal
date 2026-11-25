# Research: how do we measure xReveal's added input-to-screen latency?

Ticket: [#20](https://github.com/pwn233/xReveal/issues/20) (parent map [#2](https://github.com/pwn233/xReveal/issues/2), feeds the performance budgets in [#10](https://github.com/pwn233/xReveal/issues/10)). Researched 2026-11-25.

Builds on [#4 capture API](https://github.com/pwn233/xReveal/blob/research/capture-api/docs/research/capture-api.md), [#5 GPUI video](https://github.com/pwn233/xReveal/blob/research/gpui-video/docs/research/gpui-video.md) and [#7 capture card families](https://github.com/pwn233/xReveal/blob/research/capture-card-families/docs/research/capture-card-families.md).

Labels: **[primary]** = tool/vendor/OS docs or source. **[measurement]** = a published third-party test. **[inference]** = my reasoning; no source says it outright.

## Answer

Use two methods together. They measure different spans, and each covers the other's blind spot.

1. **End-to-end (the number the user feels): a Leo Bodnar 1080p Video Signal Input Lag Tester, about £82.50.** It replaces the PS5 as the HDMI source and its photosensor reads the monitor.
   - Measure the monitor once with the tester plugged straight in. Then plug the tester into the Reference card and hold it over xReveal's fullscreen picture.
   - **Added latency = through-xReveal reading − direct reading.** That is exactly "what I lose by playing through the PC instead of plugging the PS5 into the monitor".
   - It needs no software and works the same on Windows and macOS. That makes it the fair referee for xReveal vs OBS.
   - Limits: 1080p60 or 720p60 only, one reading at a time, and ±1 ms claimed accuracy. For 1080p120 you need the £330 4K tester. Free fallback: a 240 fps phone camera (see options).
2. **In-app (the number we budget and regress against): per-frame timestamps inside xReveal, on one host clock, logged to a histogram.**
   - **Windows:** the capture timestamp is `MFSampleExtension_DeviceTimestamp` on the QPC epoch. The display time comes from `IDXGISwapChain::GetFrameStatistics` (`PresentCount` → `SyncQPCTime`). Cross-check with PresentMon's `MsUntilDisplayed`.
   - **macOS:** the capture timestamp is the sample buffer's PTS on `AVCaptureSession.synchronizationClock`, converted to host time with `CMSyncConvertTime`. The display time comes from `MTLDrawable.presentedTime`, read in `addPresentedHandler`.
   - This splits xReveal's own share (capture stamp → callback → shader → present → on glass) per frame, for free, on every run. It **can't** see the card's internal delay before the driver stamp, or the monitor's processing and pixel response. That is why the end-to-end method stays the referee.
3. **Card floor, Windows: run [caplat](https://github.com/virgil724/caplat) once on the Reference card.** It needs only an HDMI cable from a second PC output into the card. It gives source vblank → driver timestamp → app arrival with no xReveal involved, so we know which part of the total belongs to the card and which part xReveal could still win back.

**xReveal vs OBS:** same monitor, same mode, same Source settings, same tester position. Compare the two through-app readings directly. The monitor's lag cancels out, so you don't even need the direct reading for the A/B. On Windows, PresentMon can also compare the present→display span of both processes. Details below.

## What "added input-to-screen latency" covers

```
PS5 input ─► game ─► HDMI out ─► card (scaler, USB) ─► driver stamp ─► OS API ─► xReveal callback
          ─► upload/shader ─► present ─► compositor / flip ─► scanout ─► monitor processing ─► pixel response
```

- Everything left of "HDMI out" is the same whether the PS5 goes to the monitor or to the card. So "added" = HDMI out → light, through xReveal, minus HDMI out → light, direct.
- The direct path still contains the monitor's own processing and scanout. The through-app path contains them again, possibly at a different refresh rate (the PC may drive the monitor at 144 Hz while the PS5 sends 60 Hz). Subtracting the direct reading measures the user's real trade, not a pure software number [inference].
- A 60 Hz frame has to arrive whole over HDMI before a card can deliver it, so roughly 16.7 ms is spent before any software runs ([caplat README](https://github.com/virgil724/caplat)) [primary]. Published end-to-end figures for MS2130 cards are 40–66 ms at 1080p60 (see #7). Our budget is the part above the card's floor.

## Options compared

| Method | Cost | Precision | Span covered | Windows | macOS | Can measure OBS? |
| --- | --- | --- | --- | --- | --- | --- |
| **Leo Bodnar 1080p lag tester** | £82.50 | "within 1 millisecond accuracy" [primary] | HDMI source → light (full chain, minus input device and game) | Yes | Yes | Yes |
| Leo Bodnar 4K lag tester | £330 | 1 ms [primary] | Same, plus 1080p up to 240 Hz and 4K60 | Yes | Yes | Yes |
| 240 fps phone camera (+ HDMI splitter, or the PS5 itself) | Free–~£15 | ~4.2 ms per frame. 1–2 ms with averaging and partial-frame counting [primary blog] | Whatever you film, e.g. direct screen vs xReveal screen side by side | Yes | Yes | Yes |
| OSLTT (open-source photodiode tool) | £170 | Sensor 15 µs [primary] | Click/trigger → light. Logs raw data | Yes (app is Windows) | Monitor only; trigger is a mouse, key, mic or tap | Yes, with a suitable trigger |
| NVIDIA LDAT | Not sold | High | Mouse click → light | — | — | Not available to us |
| NVIDIA Reflex Latency Analyzer | Needs a G-SYNC Reflex monitor | High | Mouse click on the *same PC* → light | — | — | **Doesn't fit**: the input goes to the PS5, not the PC |
| **caplat (HDMI loopback)** | One HDMI cable + a spare output | p10–p90 spread 0.1–0.2 ms on one card [measurement] | Source vblank → driver stamp → app arrival. No display side | Yes | **No** (Windows-only) | Only via OBS Virtual Camera (53 ms in its README) |
| **In-app timestamps** | Free (code) | Sub-ms on the host clock | Driver stamp → scanout start for xReveal's own frames | Yes | Yes | No |
| PresentMon | Free | ETW, sub-ms | `Present()` → display, for any D3D/GL/Vulkan process | Yes | No | **Yes**, the present→display span |
| Instruments, Metal System Trace | Free | Visual | Frame on display, how long it stayed | — | Yes | No (needs Metal app trace) |

### Leo Bodnar lag testers (recommended end-to-end)

- The tester "produces an HDMI video signal" with three flashing bars, and the photosensor on it reads the screen. It "[m]easures combined input lag and pixel response time" ([1080p tester](https://www.leobodnar.com/shop/index.php?main_page=product_info&products_id=212)) [primary].
  - 1080p or 720p, 60 Hz only, two AA batteries, £82.50, "within 1 millisecond accuracy".
- The 4K version (£330) adds 1080p at 120/144/180/240 Hz, 720p up to 120 Hz and 4K at 24–60 Hz. It runs on USB power ([4K tester](https://www.leobodnar.com/shop/index.php?main_page=product_info&products_id=317)) [primary]. AVLatency found it matched an oscilloscope reading exactly (8.3 ms) on one monitor ([AVLatency review](https://avlatency.com/tools/leo-bodnar-4k-lag-tester/)) [measurement].
- Read the **middle bar**. The vendor says the bars differ with scanout position and that the average of the three "should be equivalent to using the centre bar" ([4K quick-start PDF](https://www.leobodnar.com/files/4KLagDetectorQuickStartV2.pdf)) [primary].
- The same method has been used for capture cards. EposVox full-screened each card's preview, plugged the tester into the card, and wrote "Subtract the monitor's measured input latency from that of the capture card" ([OBS forum, 2019](https://obsproject.com/forum/resources/capture-card-documentation-latency-decode-modes-formats-more.777/)) [measurement]. His caveat: readings "bounce back and forth between two ranges".
- **Expect the reading to wander.** The vendor lists frame-rate conversion as a cause of drifting readings (quick-start PDF). Through a capture app, the card's frame clock and the PC's display refresh are not locked, so the phase between them slides and the reading should cycle over roughly one display refresh [inference]. Record min / typical / max over ~30 s, or film the tester's screen with a phone and read many values. Don't take the vendor's "use the lowest value" advice for this case.
- Fit for the Reference card: MS2130 cards do 1080p60, which matches the £82.50 model. Testing 1080p120 (which #7 shows is faster on MS2130: ~49 vs ~66 ms) needs the 4K model.

### Phone slow-motion camera (free fallback)

- At 240 fps one frame ≈ 4.17 ms. Count from the first frame of the event to the first frame of the change ([Philip Rosedale](https://medium.com/@philiprosedale/quick-hack-for-measuring-latency-238f8b7f5189)).
- piLagTester refines this: count the first frame as half, subtract the part of the target already drawn in the last frame, and average 5–8 events. That "seems to give a resolution of 1-2ms" ([alantechreview](https://alantechreview.blogspot.com/2020/05/input-lag-measurement-using-slow-high.html)).
- **Setup for xReveal [inference]:** a cheap HDMI splitter feeds one copy of a source to the monitor directly and the other to the card. Use a flashing pattern from a laptop, or a PS5 menu cursor. Film both screens in one shot. The gap between them is the added latency, plus the difference between the two screens.
  - With one monitor, film the PS5 controller and the screen instead. Then game latency adds tens of ms of noise, so only an A/B with many samples means anything.
  - Many splitters strip or handle HDCP oddly. PS5 HDCP has to be off for capture anyway.
- Cheap, but slow to analyse by hand. Use it to sanity-check the tester, not as the main loop.

### NVIDIA LDAT / Reflex Latency Analyzer (not usable)

- LDAT is a luminance sensor that measures mouse-click-to-light. NVIDIA gives it to reviewers. It isn't sold ([NVIDIA LDAT](https://developer.nvidia.com/nvidia-latency-display-analysis-tool), [TFTCentral](https://tftcentral.co.uk/articles/nvidia_reflex)).
- The Reflex Latency Analyzer is built into G-SYNC Reflex monitors. It times from a click on a mouse plugged into the monitor's USB passthrough to a luminance change in a chosen rectangle ([NVIDIA](https://www.nvidia.com/en-us/geforce/news/reflex-low-latency-platform/), [Tom's Hardware](https://www.tomshardware.com/news/nvidia-reflex-latency-analyzer)).
- Our input goes into a PS5 controller, not a PC mouse, so neither fits. You could rig a mouse click to fire a flash on the HDMI source, but that's more work than the Leo Bodnar for no gain [inference].

### OSLTT

- OSLTT is an open-source (GPL-3.0) photodiode tool "similar to NVIDIA's LDAT". It measures click-to-photon, on-display, in-game and audio latency, with raw sensor data "accurate to 15us" ([docs](https://osrtt.github.io/OSLTTDocs/), [repo](https://github.com/OSRTT/OSLTT)). It costs £170 for the CS revision ([store](https://www.osrtt.com/osltt)).
- Release notes mention a "console mode" that uses the microphone as trigger and the light sensor as data ([releases](https://github.com/OSRTT/OSLTT/releases)). Whether that suits a capture-card chain is **unverified**.
- Its strength is many logged samples. Its trigger still needs an input event, so for a pure capture chain the Leo Bodnar is simpler and cheaper.

### caplat (HDMI loopback, card floor)

All of this is from the [caplat README](https://github.com/virgil724/caplat) (MIT) [primary for method, measurement for numbers]:

- The PC's second HDMI output goes into the capture card. caplat draws a barcode with a 24-bit frame number on that output, takes each frame's scanout time from DXGI `GetFrameStatistics` (QPC), decodes the barcode from the captured frame, and records `latency = arrival − vblank` in a 0.1 ms histogram.
- With MF it also splits the latency at the driver timestamp. On an AVerMedia GC553 at 720p60 NV12: p50 **18.1 ms** (MF), **17.9 ms** (DirectShow), driver stamp ≈ **6.2 ms after vblank**, stamp → app p50 **11.9 ms**. OBS via its Virtual Camera: **53 ms**.
- Windows only (DXGI, MF, DirectShow, WASAPI). It also measures audio latency and A/V skew, which helps #6.
- Known quirk: "one sample in thousands, 16.7 ms off" carries the previous frame's driver timestamp. Filter outliers in our own timestamps as well.
- For xReveal: run it once on the Reference card to learn the floor (vblank → driver stamp → MF arrival). Anything xReveal measures above "stamp → arrival" is ours.

### In-app timestamps (recommended software method)

Record per frame, all on one host clock:

| Point | Windows | macOS |
| --- | --- | --- |
| t0 capture stamp | `MFSampleExtension_DeviceTimestamp`: "set on media samples created by a media source for a capture device … sharing an epoch with query performance counter (QPC) time and always expressed in 100ns units" ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/medfound/mfsampleextension-devicetimestamp)) [primary]. A related `MFSampleExtension_DeviceReferenceSystemTime` "may or may not carry the actual value of the query performance counter" ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/medfound/mfsampleextension-devicereferencesystemtime)), so prefer `DeviceTimestamp` | Sample buffer PTS. "All capture output sample buffer timestamps are on the synchronization clock's timebase" (`AVCaptureSession.synchronizationClock`, macOS 12.3+) ([Apple](https://developer.apple.com/documentation/avfoundation/avcapturesession/synchronizationclock)). Convert with `CMSyncConvertTime(pts, sessionClock, CMClockGetHostTimeClock())`: "To convert to or from host time, pass `CMClockGetHostTimeClock()`" ([Apple](https://developer.apple.com/documentation/coremedia/cmsyncconverttime(_:from:to:))). The host clock "uses `mach_absolute_time`" ([Apple](https://developer.apple.com/documentation/coremedia/cmclockgethosttimeclock())) |
| t1 callback arrival | `QueryPerformanceCounter` in the Source Reader callback | `CACurrentMediaTime()`, derived from `mach_absolute_time()` ([Apple](https://developer.apple.com/documentation/quartzcore/cacurrentmediatime())) |
| t2 present call | QPC just before `Present` | Host time just before `present` / commit |
| t3 on display (scanout start) | `IDXGISwapChain::GetFrameStatistics` → `DXGI_FRAME_STATISTICS.PresentCount` + `SyncQPCTime` (same as QPC). Match it to our present via `GetLastPresentCount` ([struct](https://learn.microsoft.com/en-us/windows/win32/api/dxgi/ns-dxgi-dxgi_frame_statistics), [method](https://learn.microsoft.com/en-us/windows/win32/api/dxgi/nf-dxgi-idxgiswapchain-getframestatistics)) | `MTLDrawable.presentedTime`: "The host time, in seconds, when the drawable was displayed onscreen", `0.0` if dropped. Read it in `addPresentedHandler` (macOS 10.15.4+) ([presentedTime](https://developer.apple.com/documentation/metal/mtldrawable/presentedtime), [addPresentedHandler](https://developer.apple.com/documentation/metal/mtldrawable/addpresentedhandler(_:))) |

- Report `t1−t0` (OS/driver delivery), `t2−t1` (xReveal's work: upload, shader, queue wait), `t3−t2` (present → glass), and the sum `t3−t0`, as p50/p90/p99 plus a count of dropped frames. Show it in a debug overlay and dump it to CSV so #10's budgets can be checked on every build [inference].
- **Caveats [primary]:**
  - `GetFrameStatistics` works only for flip-model or fullscreen swap chains. "Statistics are not reliable in many multiple monitor scenarios", and on systems with Hardware Flip Queue it "may be necessary to call DwmFlush" first ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/api/dxgi/nf-dxgi-idxgiswapchain-getframestatistics)). xReveal will often run with two monitors, so cross-check with PresentMon.
  - `PresentCount` counts images shown, "not necessarily the same as the number of times that you called" `Present` ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/api/dxgi/ns-dxgi-dxgi_frame_statistics)).
- **Blind spots:**
  - t0 is the driver's stamp, not HDMI arrival. On the GC553 it fell 6.2 ms after vblank (caplat). **What MS2130 + UVC driver / AVFoundation stamp (start of frame, end of USB transfer?) is undocumented.**
  - t3 is scanout start, not light at the middle of the screen. Monitor processing, half a scanout and pixel response come on top.
  - So in-app numbers are a lower bound on our share. Use the end-to-end tester to keep them honest.
- **Which present path this times:** #5 recommends an owned surface (Windows child `HWND` + own flip-model swap chain; macOS `CAMetalLayer` in a `NativeView`). Both give direct access to these APIs. GPUI's own swap chain doesn't expose them [inference from #5].

### PresentMon (Windows cross-check, works on OBS too)

- PresentMon (MIT, Intel) captures "graphics applications on Windows" via ETW and works across "DirectX, OpenGL, and Vulkan". It needs admin or the "Performance Log Users" group ([repo](https://github.com/GameTechDev/PresentMon)).
- Useful columns ([console README](https://github.com/GameTechDev/PresentMon/blob/main/README-ConsoleApplication.md), [capture app README](https://github.com/GameTechDev/PresentMon/blob/main/README-CaptureApplication.md)):
  - `MsUntilDisplayed`: "The time between the Present() call and when the frame was displayed"
  - `MsBetweenDisplayChange`
  - `PresentMode`: "Hardware: Independent Flip" vs "Composed: Flip". This also checks #5's open question of whether xReveal's fullscreen video gets independent flip.
  - `--process_name obs64.exe` targets one process.
- It sees only present → display. It can't see when the capture frame arrived, so it can't give OBS's total. Use it to compare the render/present half of xReveal and OBS. The end-to-end tester gives the total.
- macOS has no PresentMon. The nearest is Instruments' Metal System Trace "display surfaces" track, which shows when a frame was swapped onto the display and how long it stayed ([WWDC15 610](https://developer.apple.com/videos/play/wwdc2015/610/)). It traces only our own process.

## How to run xReveal vs OBS

Same machine, same monitor and refresh rate, same Reference card port, same Source mode (1080p60, same pixel format where both allow it).

1. **Baseline:** Leo Bodnar → monitor directly. Middle bar. Record min/typ/max over ~30 s.
2. **OBS:** Leo Bodnar → Reference card.
   - Add a Video Capture Device source. Set **Buffering: Disable** (default is Auto-Detect; "Disable … can help if you are experiencing a delay" ([OBS KB](https://obsproject.com/kb/video-capture-sources))). Set the resolution/FPS explicitly, video format = YUY2 (or NV12 if offered), and the OBS canvas and FPS to 1920×1080 at 60.
   - Show it with **Fullscreen Projector (Source)** on the test monitor ([OBS KB](https://obsproject.com/kb/power-of-projectors)). Hold the tester over the middle bar and record. Repeat with Buffering at Auto-Detect, since that is what most users run.
3. **xReveal:** same input, Smart stretch off (Fit or Stretch at 1:1), borderless fullscreen on the same monitor. Record the tester values and dump the in-app CSV over the same window.
4. **Windows extra:** run PresentMon with `--process_name` for each app during steps 2–3. Compare `MsUntilDisplayed` and `PresentMode`.
5. **Report:** for each app, added latency = reading − baseline, as min/typ/max. The headline is the xReveal-minus-OBS delta. Repeat across 3 runs and app restarts (caplat saw ffmpeg pick one or two extra frames of latency at random on startup, so check for bimodal startup states).
6. **Optional, more A/B axes:** monitor at 60 vs 144 Hz, vsync vs tearing present (#5), 1080p120 Source (needs the 4K tester), and macOS vs Windows on the same card.

The xReveal-vs-OBS delta needs no baseline: the monitor term cancels. The baseline matters only for the absolute "added" number in #10 [inference].

## Open questions

1. **Where does the MS2130 / UVC path stamp a frame?** At start of frame, end of USB transfer, or arrival in the driver? This holds on both OSes, and it decides how much of the card is hidden from the in-app numbers. caplat on Windows answers it for MF (vblank → `DeviceTimestamp`). **macOS has no equivalent tool.**
2. **Port caplat's loopback to macOS?** Barcode on the Mac's HDMI output, vblank from `presentedTime` / `CAMetalDisplayLink`, capture via AVFoundation. That would give the card floor on macOS too. Is it worth building as an xReveal dev tool (`--latency-selftest`)?
3. **Is a 60 Hz-only tester enough?** The PS5 can send 1080p120, MS2130 is faster at 120 Hz (#7), and a 120 Hz Source may be the recommended setup. If so, buy the £330 4K tester instead of the £82.50 one.
4. **Monitor refresh for the A/B.** Should budgets be stated at the user's real setup (PC at 144 Hz, PS5 at 60) or at matched 60/60? The direct baseline differs between them.
5. **`presentedTime` in borderless fullscreen with GPUI's window on top.** Does it report true scanout when the layer is composited rather than sent direct to display? Undocumented; compare it against the tester.
6. **`GetFrameStatistics` on a multi-monitor desktop.** Microsoft calls it unreliable there. Does it hold for a child-`HWND` flip swap chain in independent flip? Cross-check with PresentMon on the dev Windows box (map: "Building and testing on Windows").
7. **OBS on macOS:** does its Video Capture Device have a buffering control like Windows? The KB doesn't describe the macOS options, so the A/B may not be like-for-like on macOS.
