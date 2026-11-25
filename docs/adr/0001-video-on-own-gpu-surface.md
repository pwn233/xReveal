# Video is drawn on xReveal's own GPU surface, not by GPUI

xReveal shows the live Source picture on a native GPU surface it owns (on macOS a Metal layer in a native view under GPUI's content; on Windows a child window with its own swap chain), and uses GPUI only for the controls and settings. Each video frame is presented the moment it arrives, independent of GPUI's redraw loop. We chose this because GPUI (as shipped in GPUI Kit 0.11) cannot run custom shaders, and xReveal needs one for Smart stretch and for correcting the colour range and matrix that capture cards report wrongly. GPUI's built-in video element is also macOS-only, accepts only full-range NV12 and hard-codes BT.601 colour, and on Windows the only route through GPUI is converting every frame on the CPU. Presenting through GPUI's frame loop would also add up to one extra screen refresh of delay, and latency is xReveal's top priority.

Inside that surface, xReveal draws with the **native APIs: Metal on macOS and Direct3D 11 on Windows**, the same ones GPUI itself uses. On Windows it uses its own Direct3D 11 device rather than reaching into GPUI's. Native APIs give zero-copy capture frames on macOS (`CVMetalTextureCache`) and full control of presentation timing (Metal display timing, Windows' low-latency flip-model swap chain), which the half-frame latency budget needs. They also add no new graphics dependency. The cost is writing the small video shader twice (MSL and HLSL). wgpu was rejected: it isn't compiled for macOS or Windows today, it would add size and memory, importing capture frames needs `wgpu-hal` interop, and it gives less control over present timing. Decided in [What draws the picture inside xReveal's video surface: wgpu or native Metal + D3D11?](https://github.com/pwn233/xReveal/issues/18).

## Considered Options

- **Inside GPUI via per-frame image upload**: rejected. It needs CPU YUV→RGB conversion (about 1 GB/s at 1080p120), adds latency, and has no shader for Smart stretch.
- **GPUI's `surface()` element**: rejected. macOS only, wrong colour for HDMI sources, no Windows equivalent.
- **Forking or patching GPUI** to add custom shaders and Windows video: rejected for now, because of the ongoing maintenance against upstream. Revisit if contributing native-view support upstream becomes the answer for the Windows settings overlay.

## Consequences

- Two platform-specific presentation paths to maintain (macOS and Windows).
- On Windows, a child video window sits on top of GPUI's UI in the same window, so the settings overlay needs its own answer (ticket "How does the settings UI appear over the picture on Windows?").
- The video shader (colour correction, Picture adjustments, Smart stretch) exists in two languages, MSL and HLSL, which must stay in step.

Research: [GPUI video rendering](https://github.com/pwn233/xReveal/blob/research/gpui-video/docs/research/gpui-video.md). Decided in [Does the picture render inside GPUI or in a native GPU surface?](https://github.com/pwn233/xReveal/issues/9).
