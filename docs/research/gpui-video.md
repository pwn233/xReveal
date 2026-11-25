# Can GPUI Kit show a 60–120 fps video stream through a custom shader with near-zero added latency?

Research for [#5](https://github.com/pwn233/xReveal/issues/5) (map: [#2](https://github.com/pwn233/xReveal/issues/2)).
Researched 2026-11-25 against the exact crates in `Cargo.lock`: `gpui-kit 0.11.0` → `gpui-pre 0.3.14`
("Zed's GPU-accelerated UI framework (gpui-pre snapshot of zed@f126950)", `gpui-pre-0.3.14/Cargo.toml:24`).

All file references below are relative to `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`.

## Short answer

**No, not through GPUI's own renderer.** GPUI 0.3.14 has no custom-shader or custom-render-pass hook on any
platform. Its only video-ish primitive, `surface()`, exists on macOS only, accepts only full-range NV12 with a
hard-coded BT.601 matrix, and is drawn on GPUI's main-thread frame tick. On Windows that primitive is a no-op, so
the only way in is a CPU-side BGRA image re-uploaded through the sprite atlas every frame.

**Recommendation: draw the video on a native GPU surface that xReveal owns, and keep GPUI for the UI.**

- **macOS:** an `NSView` backed by xReveal's own `CAMetalLayer`, embedded with GPUI's **`NativeView`**, which
  gpui-pre supports on macOS only. GPUI composites its UI *over* the view, so the settings overlay still works.
- **Windows:** GPUI has no native-view support. Use a **child `HWND` with xReveal's own flip-model swap chain**,
  parented to GPUI's window through `raw-window-handle`. A child `HWND` shows *above* GPUI's DirectComposition
  content, so GPUI cannot draw over the video. The settings overlay needs its own window, or the video hides
  while settings are open. A later ticket decides which (see Open questions).
- The shader (YUV→RGB with the right matrix and range, and the Fill modes including Smart stretch) and its
  presentation then belong entirely to xReveal. They can run on the capture thread when a frame arrives instead of
  waiting for GPUI's next frame tick. That is the main latency win.

## Facts that drive the decision

1. **Backends:** macOS uses GPUI's own **Metal** renderer and Windows uses its own **Direct3D 11 + DXGI flip-model
   swap chain over DirectComposition**. GPUI only uses `wgpu` on Linux, so `wgpu` is not compiled for macOS or
   Windows today.
2. **No custom shaders or passes.** The scene is a closed `enum Primitive` with nine kinds. Shaders are compiled
   into each backend (`shaders.metal`, `shaders.hlsl`). No public API registers a pipeline or touches the
   device or command encoder.
3. **`surface()` (macOS only):** zero-copy `CVPixelBuffer` → `CVMetalTextureCache`. But it accepts only
   `kCVPixelFormatType_420YpCbCr8BiPlanarFullRange` (any other format fails the **whole window's frame**). It
   converts with a fixed **BT.601 full-range** matrix, which gives wrong colours for typical BT.709 limited-range
   HDMI capture. On Windows `draw_surfaces` is an empty function.
4. **`NativeView` exists on macOS only** (`NativeView::new` fails on every other platform). It embeds an `NSView`
   beneath GPUI's layer, and GPUI elements painted later cover it.
5. **Frame pacing:** GPUI renders on the main thread when its display link fires (macOS `CAMetalDisplayLink`,
   Windows a `DwmFlush` thread). A capture frame therefore waits up to one refresh before GPUI draws it, then
   goes through GPUI's 3-buffer swap chain and the compositor. An owned swap chain can present when the frame
   arrives.
6. **Upstream precedent:** Zed's own screen-share viewer uses exactly these two paths: `surface()` on macOS,
   and `img()` with a per-frame `RenderImage` plus manual `drop_image` everywhere else. Neither Zed nor GPUI Kit
   has a custom-shader video example.

## Evidence

### 1. Which GPU backend per OS

| OS | Platform crate | Renderer | Evidence |
|---|---|---|---|
| macOS | `gpui-pre-macos` → `gpui-pre-apple` | Metal (`CAMetalLayer`, BGRA8, max 3 drawables) | `gpui-pre-platform-0.3.14/src/gpui_platform.rs` `current_platform` (`target_os = "macos" => MacPlatform`); `gpui-pre-apple-0.3.14/src/metal_renderer.rs:183-208` |
| Windows | `gpui-pre-windows` | Direct3D 11 (feature level 11.1/11.0/10.1), DXGI `FLIP_SEQUENTIAL`, 3 buffers, presented through DirectComposition | `gpui-pre-windows-0.3.14/src/directx_devices.rs:156-165`; `directx_renderer.rs:1344-1395`, `:1720` (`BUFFER_COUNT = 3`); `Cargo.toml:172-178` (D3D11, DirectComposition features) |
| Linux | `gpui-pre-linux` + `gpui-pre-wgpu` | wgpu 30 | `gpui-pre-platform-0.3.14/Cargo.toml:117-120` (`gpui_wgpu` is `cfg(target_os = "linux")` only) |

GPUI Kit picks `gpui-pre` by default. `gpui-fast` is an opt-in feature (`gpui-kit-0.11.0/src/lib.rs`,
`#[cfg(not(feature = "gpui-fast"))] pub use ::gpui::*`). `gpui-fast 0.1.19` has the same limits: its Windows
`draw_surfaces` is also empty (`gpui-fast-windows-0.1.19/src/directx_renderer.rs:877-882`) and only its macOS
crate implements `attach_native_view`.

### 2. How external frames can get in

**a) `surface()`: macOS/iOS only, zero-copy, but fixed format and matrix**

- The element and source type exist only on macOS and iOS: `SurfaceSource::Surface(CFRetained<CVPixelBuffer>)` and
  `pub fn surface(...)` are both `#[cfg(any(target_os = "macos", target_os = "ios"))]`
  (`gpui-pre-0.3.14/src/elements/surface.rs:11-41`). `Window::paint_surface` is the same
  (`gpui-pre-0.3.14/src/window.rs:5649-5666`).
- Metal draw: makes `R8Unorm` (Y) and `RG8Unorm` (CbCr) textures from the pixel buffer through
  `CVMetalTextureCache`, so there is no CPU copy (`gpui-pre-apple-0.3.14/src/metal_renderer.rs:1273-1305`).
- **Format gate:** `anyhow::ensure!(pixel_format == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange, ...)`
  (`metal_renderer.rs:1267-1271`). The error propagates out of `render_frame`, and `draw` then logs
  `failed to render` and **returns without presenting** (`metal_renderer.rs:554-560`). One video-range (`420v`)
  buffer therefore blanks the whole window's frame, UI included.
- **Colour:** the fragment shader hard-codes a BT.601 full-range YCbCr→RGB matrix
  (`gpui-pre-apple-0.3.14/src/shaders.metal:953-969`). It cannot be configured for BT.709 or limited range.
- **Scaling:** `Surface::object_fit` supports `Fill`/`Contain`/`Cover` (`surface.rs:44-48`,
  `gpui-pre-0.3.14/src/style.rs:29-40`). That covers Stretch, Fit and Crop. The vertex shader maps the *whole*
  texture to the unclipped bounds and then clips to the content mask (`shaders.metal:932-950`). A
  *piecewise-linear* Smart stretch could therefore be faked by painting the same buffer as three clipped strips.
  A smooth (non-linear) Smart stretch cannot be done this way. (Inference from the shader; not tried.)
- **Windows:** `draw_surfaces` returns `Ok(())` and draws nothing
  (`gpui-pre-windows-0.3.14/src/directx_renderer.rs:845-850`). `PaintSurface` has no non-Apple payload
  (`gpui-pre-0.3.14/src/scene.rs:1167-1175`).

**b) `img()` with a `RenderImage`: every OS, CPU upload every frame**

- `RenderImage` is "a cached and processed image, in BGRA format" (`gpui-pre-0.3.14/src/assets.rs:42-49`). The
  YUV→BGRA conversion would have to happen on the CPU first.
- `Window::paint_image` inserts it into the **sprite atlas**, keyed by image id, and draws a `PolychromeSprite`
  (`window.rs:5547-5645`). A new frame has a new id, so it means a new atlas allocation and upload. The upload is
  `replaceRegion` on Metal (`gpui-pre-apple-0.3.14/src/metal_atlas.rs:272`) and `UpdateSubresource` on D3D11
  (`gpui-pre-windows-0.3.14/src/directx_atlas.rs:304`). An image larger than 1024² gets its own atlas texture
  (`metal_atlas.rs:139-150`). The caller must `drop_image` old frames by hand (`window.rs:5669-5681`).
- Cost estimate (arithmetic, not measured): 1920×1080×4 B ≈ 8.3 MB per frame, which is ≈ 1 GB/s at 120 fps,
  plus a CPU colour conversion and a texture allocation every frame. That conflicts with the "~10% of OBS" goal.
- There is no shader hook here either. Only the built-in sprite shader applies (opacity, grayscale, corner radii).

**c) `NativeView`: macOS only, an owned GPU surface under GPUI's UI**

- "A view owned by the platform's UI toolkit, shown inside a GPUI window. Only macOS shows native views so far,
  where the view is an `NSView`. `NativeView::new` fails on every other platform"
  (`gpui-pre-0.3.14/src/window/native_view.rs:30-68`). `unsafe fn new(view: NonNull<c_void>, ...)` takes the raw
  `NSView` (`native_view.rs:98-117`).
- Layering: "composited beneath the window's GPUI content … elements painted after it cover it"
  (`native_view.rs:35-38`). `PaintNativeView` makes the renderer erase what was painted earlier inside its bounds
  (`scene.rs:1183-1203`).
- Platform hook: `PlatformWindow::attach_native_view` returns `None` by default (`gpui-pre-0.3.14/src/platform.rs:1370-1385`).
  Only `gpui-pre-macos-0.3.14/src/window.rs:2197` implements it (hosting code in `window/native_views.rs`).
  The Windows renderer ignores the `NativeViews` batch (`directx_renderer.rs:442`).
- Cost: while any native view shows, GPUI's `CAMetalLayer` becomes non-opaque ("Native views sit beneath the
  layer, so it can only be opaque, and eligible for direct-to-display presentation, while none are showing",
  `metal_renderer.rs:521-527`). That rules out direct-to-display for GPUI's layer. Whether the *video* layer can
  still go direct-to-display when GPUI draws nothing over it is unverified (see Open questions).
- This is a gpui-pre feature. It is not in a GPUI Kit or Zed example that I found.

**d) Raw window handle: every OS**

- `Window` implements `HasWindowHandle` and `HasDisplayHandle` (`gpui-pre-0.3.14/src/window.rs:8112-8124`). On
  Windows that is the top-level `HWND` (`gpui-pre-windows-0.3.14/src/window.rs:598-604`). This is how a Windows
  child `HWND` (or a `wgpu` surface) gets parented to GPUI's window.

### 3. Custom shaders or custom render passes

**Not possible without forking GPUI.**

- The scene is a closed set: `enum Primitive { Shadow, Quad, Path, Underline, MonochromeSprite, SubpixelSprite,
  PolychromeSprite, Surface, NativeView }` (`gpui-pre-0.3.14/src/scene.rs:493-503`). `PrimitiveBatch` is matched
  exhaustively by each renderer (`scene.rs:806-826`; `directx_renderer.rs:440-442`).
- Shaders are compiled into the backend crates (`gpui-pre-apple-0.3.14/src/shaders.metal`,
  `gpui-pre-windows-0.3.14/src/shaders.hlsl`, `gpui-pre-wgpu-0.3.14/src/shaders.wgsl`). A search of `gpui-pre`'s
  source found no public API for custom shaders, render passes or device access.
- Upstream search: no gpui custom-shader issue or PR turned up in `zed-industries/zed` (GitHub issue search for
  "shader" and "gpui video", 2026-11-25). Absence of evidence only.

### 4. Present mode, vsync and frame pacing

**macOS**

- macOS 14+: each window is paced by a `CAMetalDisplayLink` on its layer, which hands out the drawable for each
  frame (`gpui-pre-macos-0.3.14/src/display_link.rs:1-5`; `window.rs:648-655`). Older systems use a shared
  `CVDisplayLink`.
- The link runs on the **main run loop** because "GPUI renders on the foreground thread"
  (`gpui-pre-apple-0.3.14/src/display_link.rs:47-83`). The default frame-rate range follows the display (so
  120 Hz ProMotion is allowed). `preferredFrameLatency` is never set, so Apple's default applies.
- Presentation: `presentDrawable` and then commit. During live resize it switches to `presentsWithTransaction` +
  `waitUntilScheduled` (`metal_renderer.rs:563-570`; `gpui-pre-macos-0.3.14/src/window.rs:3454-3470`).
  `maximumDrawableCount = 3` (`metal_renderer.rs:203`).
- GPUI draws only when something calls `cx.notify()` / `request_animation_frame` ("useful for … a video
  player", `gpui-pre-0.3.14/src/window.rs:3085-3099`), and the link pauses when idle
  (`gpui-pre-macos-0.3.14/src/display_link.rs:297-305`).

**Windows**

- A `VSyncProvider` thread blocks on `DwmFlush()` and then `RedrawWindow(RDW_INVALIDATE)`s every GPUI window
  (`gpui-pre-windows-0.3.14/src/vsync.rs:35-63`; `platform.rs:429-480`). The draw then happens in `WM_PAINT` on
  the UI thread (`events.rs:1345-1400`).
- Present is `Present(0, DXGI_PRESENT(0))`, sync interval 0 (`directx_renderer.rs:262-272`), on a
  `FLIP_SEQUENTIAL` swap chain with 3 buffers. The swap chain is for composition (premultiplied alpha) by default,
  or for the `HWND` when `GPUI_DISABLE_DIRECT_COMPOSITION` is set (`directx_renderer.rs:27`, `:1344-1395`). There
  is no `SetMaximumFrameLatency` or waitable-object use, and no `ALLOW_TEARING` flag (a grep for those found
  nothing).
- The DirectComposition target is created with `topmost = false`
  (`directx_renderer.rs:1033-1036`: `CreateTargetForHwnd(hwnd, false)`). Microsoft documents that with
  `topmost = FALSE` the visual tree shows **behind the window's child windows**
  ([IDCompositionDevice::CreateTargetForHwnd](https://learn.microsoft.com/en-us/windows/win32/api/dcomp/nf-dcomp-idcompositiondevice-createtargetforhwnd)).
  So a video child `HWND` would cover GPUI's UI.

**What this means for latency (reasoning, not measured)**

When GPUI draws the frames, a capture frame that arrives just after a display-link or `DwmFlush` tick waits up to
one refresh for GPUI's next frame. It then renders on the main/UI thread, together with layout and any UI work,
and goes through the compositor at the next vsync. That is roughly 1–2 refresh intervals on top of capture and
display. An owned swap chain can render and present from the capture thread when the frame arrives. On Windows
that also allows `DXGI_PRESENT_ALLOW_TEARING` or a waitable swap chain with frame latency 1. On macOS it allows
`CAMetalLayer.displaySyncEnabled = false` or presenting at once. That removes the wait for GPUI's tick and the
main-thread coupling. The real numbers belong to the latency-measurement ticket.

### 5. Upstream examples

- Zed's `crates/livekit_client/src/remote_video_track_view.rs` (zed@`14628482a3007b014043b73507a5a1437e13c2b2`):
  on macOS it renders `gpui::surface(latest_frame)`. On other platforms it renders `gpui::img(latest_frame)` and
  calls `window.drop_image` on the previous frame each render. It schedules frames with `cx.notify()` from an async
  task. This is the reference "video in GPUI" pattern, and it has no shader control.
- No GPUI Kit (`longbridge/gpui-kit`) example uses `surface()` or `NativeView` for video (code search 2026-11-25).

## Options compared

| Option | Custom shader (YUV matrix/range, Smart stretch) | macOS | Windows | Added latency | UI over video | Cost / risk |
|---|---|---|---|---|---|---|
| **A. GPUI `surface()`** | No. Fixed BT.601 full-range, NV12 full-range only. Piecewise Smart stretch through clipped strips at best | Yes | **No** (no-op) | GPUI tick + main thread | Yes | Wrong colours for 709/limited. A bad format blanks the frame |
| **B. GPUI `img()` + `RenderImage`** | No. CPU conversion only | Yes | Yes | GPUI tick + CPU convert + upload | Yes | ~1 GB/s upload at 1080p120, atlas churn, high CPU |
| **C. Fork or patch GPUI** to add a custom primitive or external-texture path in Metal and D3D11 | Yes | Yes | Yes | GPUI tick + main thread | Yes | Maintaining a fork of a moving snapshot (gpui-pre tracks zed HEAD). Two renderers to patch |
| **D. Owned native surface (recommended)**: macOS `NativeView`(`NSView`+`CAMetalLayer`); Windows child `HWND` + own swap chain | Yes, xReveal's own shader | Yes (`NativeView`) | Yes (child `HWND` via `raw-window-handle`) | Present on frame arrival. Swap-chain settings are xReveal's choice | macOS yes; **Windows no** (child `HWND` is above GPUI's DirectComposition content) | Two platform integrations. Windows overlay needs a separate window or hiding the video |
| E. Separate top-level video window, with GPUI in an owned overlay window | Yes | Yes | Yes | Same as D | Via a transparent owned window | Window management, focus and fullscreen edge cases |

Rendering API inside option D (decide in a later ticket):

- **`wgpu` 30** (Metal on macOS, DX12 on Windows; already in `Cargo.lock` but only built for Linux today). One
  WGSL shader works on both. Zero-copy import of `CVPixelBuffer`/`IOSurface` or a Media Foundation D3D11 texture
  needs `wgpu-hal` interop (`create_texture_from_hal`). It adds binary size on macOS and Windows.
- **Native APIs** (`objc2-metal` and the `windows` crate are already compiled for GPUI). That means two shaders
  (MSL and HLSL), but direct zero-copy (`CVMetalTextureCache`; a D3D11 texture straight from Media Foundation on
  the same device), full control of present and tearing, and no new heavy dependency.

## Per-OS detail of the recommendation

**macOS**

1. Create an `NSView` with `wantsLayer = YES` and a `CAMetalLayer` (opaque, BGRA8 or RGB10A2), and wrap it with
   `unsafe { NativeView::new(ptr, window, cx) }`. Render it as an element that fills the window.
2. On the capture callback thread: `CVPixelBuffer` → `CVMetalTextureCache` (Y and CbCr planes) → xReveal's
   fragment shader (matrix and range from the Video format, plus Fill mode and Smart stretch) → `nextDrawable` →
   present. Nothing goes through GPUI's main thread.
3. GPUI UI (settings, toasts) paints over the view whenever it is shown, and is idle the rest of the time. With
   the video playing and no UI, GPUI draws nothing, because the link pauses when idle.

**Windows**

1. Get GPUI's `HWND` from `window.window_handle()` and create a child `HWND` (`WS_CHILD | WS_VISIBLE`) the size of
   the client area.
2. Create a D3D11 (or DX12 through wgpu) device and a flip-model swap chain for that child `HWND`
   (`FLIP_DISCARD`, 2 buffers, `ALLOW_TEARING` where supported, `SetMaximumFrameLatency(1)` / waitable object).
   Render and present on the capture thread.
3. Because GPUI's composition target is not topmost, the child covers GPUI. Open settings in a separate GPUI
   window, or hide the child while settings are open. Alternatively, contribute `attach_native_view` for Windows
   to gpui-pre: the macOS design is the template, but it would need GPUI's DComp tree to host a child visual.

## Open questions (for later tickets)

1. **Windows settings overlay:** separate owned GPUI window, hide the video while settings are open, or
   contribute Windows `NativeView` support upstream? This is a product/UX decision with a latency side: is the
   video allowed to stop updating while settings show?
2. **wgpu or native Metal + D3D11** inside the owned surface. It is a trade between one shader and a larger
   binary, versus two shaders and simpler zero-copy interop with AVFoundation and Media Foundation. It depends on
   the capture-backend research (what texture type each OS's capture API hands over).
3. **Direct-to-display / independent flip:** in borderless fullscreen, does the video layer or swap chain get
   direct scan-out (macOS) or independent flip / MPO (Windows) while GPUI's window is non-opaque (macOS) or has a
   DirectComposition visual (Windows)? This needs a measurement on the Reference card setup.
4. **Present policy:** tearing or immediate present (lowest latency) versus vsync with frame latency 1 (no
   tearing). Should this be a user setting? It interacts with 60 fps sources on 120/144 Hz monitors.
5. **Smart stretch definition:** piecewise-linear (three strips: the centre undistorted, the edges stretched
   linearly) or a smooth curve? Only the smooth version strictly needs a custom shader, and the Glossary term
   should say which.
6. **Main-thread independence on macOS:** confirm that rendering into the `NativeView`'s `CAMetalLayer` from the
   capture thread is safe while AppKit and GPUI resize the view on the main thread (resize races, drawable size
   updates). A small prototype could settle this.

## Uncertainty

- The latency figures are reasoning from the code paths. Nothing was measured.
- "No custom shader API" is from reading `gpui-pre 0.3.14`. A later gpui-pre or Zed release could add one.
- Apple's default `CAMetalDisplayLink.preferredFrameLatency` value was not checked against the docs.
- The Windows child-`HWND`-over-DirectComposition layering follows Microsoft's documentation for
  `topmost = FALSE`. It was not run.
