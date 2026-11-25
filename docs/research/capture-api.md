# Research: which capture API gives the lowest latency for capture cards?

Ticket: [#4](https://github.com/pwn233/xReveal/issues/4) (parent map [#2](https://github.com/pwn233/xReveal/issues/2)). Researched 2026-11-25.

## Answer

- **Windows: Media Foundation (MF), using the Source Reader in async mode, called directly through the `windows` crate.** Use DirectShow only to open the vendor settings dialog. MF is the API Microsoft supports for capture, and DirectShow is marked legacy. The one public A/B measurement found puts MF and DirectShow within 0.2 ms of each other on a capture card, so streaming through DirectShow buys no latency. The vendor dialog (`ISpecifyPropertyPages`) has a documented path only on a DirectShow filter.
- **macOS: AVFoundation, using `AVCaptureSession` with `AVCaptureVideoDataOutput`, called directly through `objc2-av-foundation`.** Keep `alwaysDiscardsLateVideoFrames = true`. Request a bi-planar 4:2:0 (NV12-layout) `CVPixelBuffer` and hand it to Metal with no copy through `CVMetalTextureCache`. macOS has no OS vendor settings dialog.
- **Don't use `nokhwa`.** Both of its backends copy every frame into a `Vec<u8>`. Its macOS backend queues frames on an unbounded channel and returns the oldest queued frame, not the newest. Nothing reaches the GPU without a copy.
- **Pixel format preference:** NV12 first, then YUY2, then MJPEG only as a last resort.
  - NV12 maps straight onto GPU texture formats on both OSes: `DXGI_FORMAT_NV12`, and `420v`/`420f` + `CVMetalTextureCache`.
  - YUY2 is uncompressed 4:2:2. It needs one shader unpack, or a conversion on macOS.
  - MJPEG adds a decode step. It is unavoidable on USB 2 or above about 1080p60 on cheap UVC chips.
- **Frames from a USB card arrive in system memory.** On Windows, expect one CPU-to-GPU upload per frame. On macOS, the CVPixelBuffer → Metal path can avoid the copy. GPUI already does this inside its `surface` element.

## Windows

### Media Foundation vs DirectShow

| | Media Foundation | DirectShow |
| --- | --- | --- |
| Status | The recommended capture API ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/directshow/directshow)) | "Legacy feature… superseded by … Audio/Video Capture in Media Foundation" ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/directshow/directshow)) |
| Path through the OS | Goes through the Windows Camera **Frame Server** service, which virtualises the camera so several apps can share it ([MS Learn, DShow Bridge](https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/dshow-bridge-implementation-guidance-for-usb-video-class-devices)) | Goes through Frame Server only when the device opts into "DShow Bridge". Otherwise it is the legacy pipeline "in maintenance mode" ([same page](https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/dshow-bridge-implementation-guidance-for-usb-video-class-devices)) |
| Measured latency (AVerMedia GC553, 720p60 NV12, barcode + vblank method) | p50 **18.1 ms**, p10–p90 spread 0.2 ms | p50 **17.9 ms**, spread 0.1 ms |
| Driver timestamps | Yes. On that card the driver stamped frames 6.2 ms after vblank, which lets you split latency into parts | No graph clock |
| Vendor property dialog | No documented `ISpecifyPropertyPages` on an MF source (see below) | `ISpecifyPropertyPages` + `OleCreatePropertyFrame` ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/directshow/displaying-a-filters-property-pages)) |

The latency rows come from the third-party [caplat](https://github.com/virgil724/caplat) README. It is one card, one run length (~30 s) and one author, so treat it as a strong hint, not proof. The same source puts ffmpeg+DirectShow at 35–60 ms, because ffmpeg adds queued frames. That shows most capture latency comes from app-side buffering, not from the OS API.

**Verdict:** MF for streaming. The latency is the same, MF is the supported API, and it gives driver timestamps, which the latency-measurement ticket can use. DirectShow appears only for the vendor dialog.

### Device enumeration and coverage

- Enumerate with `MFEnumDeviceSources` and `MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID`. Open a device again with `MFCreateDeviceSource` plus its symbolic link ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/medfound/audio-video-capture-in-media-foundation)).
- MF docs say video capture is "supported through the UVC class driver and must be compatible with UVC 1.1" ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/medfound/audio-video-capture-in-media-foundation)). Driver-based (AVStream) cards appear in MF only if they register under `KSCATEGORY_VIDEO_CAMERA`. Microsoft's camera wiki says legacy devices "that only register under the KSCATEGORY_VIDEO|KSCATEGORY_CAPTURE (which includes all the old tuner/capture cards …)" lack sensor groups ([microsoft/Windows-Camera wiki](https://github-wiki-see.page/m/microsoft/Windows-Camera/wiki)). The modern registration is described in [Frame Server Custom Media Source](https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/frame-server-custom-media-source).
- **Uncertain:** whether every driver-based card that matters (Elgato, AVerMedia PCIe, and so on) shows up in `MFEnumDeviceSources`. A card visible only to DirectShow would need a DirectShow streaming fallback. See open questions.

### Settings v1 needs, on Windows (MF)

| Setting | How |
| --- | --- |
| Resolution + FPS | Enumerate native media types (`IMFMediaTypeHandler::GetMediaTypeByIndex`), pick one, set `MF_MT_FRAME_RATE` within `MF_MT_FRAME_RATE_RANGE_MIN/MAX`, then call `SetCurrentMediaType` ([format](https://learn.microsoft.com/en-us/windows/win32/medfound/how-to-set-the-video-capture-format), [frame rate](https://learn.microsoft.com/en-us/windows/win32/medfound/how-to-set-the-video-capture-frame-rate)). Ignore `FORMAT_VideoInfo` duplicates when a `FORMAT_VideoInfo2` twin exists (same page). |
| Color space + range | The media type may carry `MF_MT_YUV_MATRIX` and `MF_MT_VIDEO_NOMINAL_RANGE` (`MFNominalRange_0_255` / `_16_235`) ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-mt-video-nominal-range-attribute)). These describe what the driver *reports*. In xReveal the setting is effectively our YUV→RGB shader's interpretation (Auto = trust the reported value, else override). Microsoft's guidance is BT.601 for SD and BT.709 for HD ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/medfound/recommended-8-bit-yuv-formats-for-video-rendering)). |
| Buffering | No "N frames" knob on the Source Reader. Set `MF_LOW_LATENCY` on the reader's attributes ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-low-latency), [reader attributes](https://learn.microsoft.com/en-us/windows/win32/medfound/source-reader-attributes)). Use async mode (`MF_SOURCE_READER_ASYNC_CALLBACK`) with one `ReadSample` always outstanding. xReveal's "buffering" setting is then the depth of *our own* frame queue (0 = latest-frame slot). Whether `MF_LOW_LATENCY` changes anything for a capture source, rather than codecs, is **not documented**; measure it. |
| Vendor settings dialog | See next section. |
| Pixel format | Prefer the native NV12, then YUY2, then MJPEG. Keep `MF_READWRITE_DISABLE_CONVERTERS` on, or at least don't enable `MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING`, which inserts a video processor MFT for YUV→RGB and other conversions ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-source-reader-enable-advanced-video-processing)). We do YUV→RGB in our own shader. |

### Vendor property dialog (`ISpecifyPropertyPages`)

- The documented path is DirectShow: get the capture filter (`IBaseFilter`) from the device moniker, `QueryInterface` for `ISpecifyPropertyPages`, call `GetPages`, then `OleCreatePropertyFrame` ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/directshow/displaying-a-filters-property-pages)).
- OBS does exactly this in `Device::OpenDialog` / `OpenPropertyPages` ([libdshowcapture `dshowcapture.cpp` @ c13d4b7](https://github.com/obsproject/libdshowcapture/blob/c13d4b7b0c66979396ba0a9060c9aafc15bb7b22/source/dshowcapture.cpp#L123-L183)). It also opens crossbar dialogs, which some PCIe cards use for input selection.
- No Microsoft doc says an MF `IMFMediaSource` exposes `ISpecifyPropertyPages`. **Unverified:** that xReveal can open the DirectShow filter for the dialog *while MF is streaming the same device*. Frame Server sharing suggests yes, but test it on the Reference card and on one driver-based card.
- Rust: `windows` 0.62 (the version GPUI already uses) exposes `ISpecifyPropertyPages` and `OleCreatePropertyFrame` (`Win32_System_Ole`), plus the `Win32_Media_MediaFoundation` and `Win32_Media_DirectShow` modules. I checked the crate source locally.

### GPU path on Windows

- `DXGI_FORMAT_NV12` is a shader-readable texture format: an R8 view for Y and an R8G8 view for UV. `DXGI_FORMAT_YUY2` is read through an R8G8B8A8 view (Y0 U Y1 V) ([MS Learn, DXGI_FORMAT](https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format)). Either format can be uploaded as-is and converted in the pixel shader. NV12 is the smallest at 12 bpp vs 16 bpp.
- `MF_SOURCE_READER_D3D_MANAGER` gives a D3D device to *decoders* (DXVA). Microsoft says not to set it when you are not decoding ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-source-reader-d3d-manager)). For uncompressed UVC frames, the sample is a system-memory buffer. Expect one `UpdateSubresource`/map-copy into a D3D11 texture per frame (~3 MB for 1080p NV12).
- GPUI's Windows renderer is D3D11 (`directx_renderer.rs` holds an `ID3D11Device`). Its `draw_surfaces` is an **empty stub** in `gpui-pre-windows` 0.3.14, so GPUI has no video-surface element on Windows. This belongs to [#5](https://github.com/pwn233/xReveal/issues/5) but limits how capture hands frames over.

## macOS

### API: AVFoundation

- `AVCaptureDevice` → `AVCaptureDeviceInput` → `AVCaptureSession` → `AVCaptureVideoDataOutput` with a sample-buffer delegate on a serial dispatch queue.
- **Late frames:** `alwaysDiscardsLateVideoFrames` defaults to `true`. With it on, the output "immediately discard[s] frames that captured while the dispatch queue handling existing frames blocks". With it off, memory "may increase significantly" ([Apple](https://developer.apple.com/documentation/avfoundation/avcapturevideodataoutput/alwaysdiscardslatevideoframes)). Keep it `true`.
- **Buffer pool:** sample buffers "directly reference pools of memory"; "if multiple sample buffers reference such pools of memory for too long … those samples will be dropped" ([Apple, captureOutput(_:didOutput:from:)](https://developer.apple.com/documentation/avfoundation/avcapturevideodataoutputsamplebufferdelegate/captureoutput(_:didoutput:from:))). Hold at most one or two `CVPixelBuffer`s at a time: the one on screen and the newest one.
- **Driver-based cards:** CoreMediaIO DAL plug-ins were deprecated in macOS 12.3, and Apple planned to disable them after Ventura. Camera Extensions replace them ([WWDC22 10022](https://developer.apple.com/videos/play/wwdc2022/10022/)). Cards that shipped DAL drivers only work in AVFoundation if the vendor ships a Camera Extension. UVC cards need no driver. `AVCaptureDevice.DeviceType.external` exists from macOS 14 and replaces the deprecated `externalUnknown` ([Apple](https://developer.apple.com/documentation/avfoundation/avcapturedevice/devicetype-swift.struct/external)).

### Settings v1 needs, on macOS

| Setting | How |
| --- | --- |
| Resolution + FPS | Inside `beginConfiguration`/`commitConfiguration` + `lockForConfiguration`, set `activeFormat` (one of `device.formats`), then `activeVideoMinFrameDuration`/`MaxFrameDuration`. Setting a value outside `activeFormat.videoSupportedFrameRateRanges` raises an exception ([activeFormat](https://developer.apple.com/documentation/avfoundation/avcapturedevice/activeformat), [activeVideoMinFrameDuration](https://developer.apple.com/documentation/avfoundation/avcapturedevice/activevideominframeduration)). |
| Color space + range | Range is chosen by the requested pixel format: `420v` = video range, luma 16–235 ([Apple](https://developer.apple.com/documentation/corevideo/kcvpixelformattype_420ypcbcr8biplanarvideorange)), vs `420f` = full range. The matrix comes as the `kCVImageBufferYCbCrMatrixKey` attachment on each buffer ([Apple](https://developer.apple.com/documentation/corevideo/kcvimagebufferycbcrmatrixkey)). `AVCaptureDevice.activeColorSpace` only picks sRGB/P3/HLG for camera pipelines ([Apple](https://developer.apple.com/documentation/avfoundation/avcapturedevice/activecolorspace)), so it doesn't help with HDMI YCbCr. As on Windows, the user-facing setting is our shader's interpretation. |
| Buffering | No OS knob beyond `alwaysDiscardsLateVideoFrames`. Our own queue depth is the setting. |
| Vendor settings dialog | **None.** AVFoundation/CoreMediaIO has no `ISpecifyPropertyPages` equivalent. Hide the option on macOS. (I found no Apple API for it; absence is hard to cite, so this is a negative finding.) |
| Pixel format | `videoSettings = [:]` delivers the device-native format. Otherwise set `kCVPixelBufferPixelFormatTypeKey` to one of `availableVideoPixelFormatTypes` ([Apple](https://developer.apple.com/documentation/avfoundation/avcapturevideodataoutput/videosettings)). Prefer a native bi-planar 4:2:0 format. If the card is YUY2-only (`yuvs`/`2vuy`), either ask for `420v` and let AVFoundation convert (**cost and latency undocumented**; measure), or take native and unpack in a shader. |

### GPU path on macOS

- `CVMetalTextureCacheCreateTextureFromImage` creates "a cached Core Video Metal texture object mapped to an image buffer, and a live binding to the underlying MTLTexture" ([Apple](https://developer.apple.com/documentation/corevideo/cvmetaltexturecachecreatetexturefromimage(_:_:_:_:_:_:_:_:_:))). Map plane 0 as `R8Unorm` and plane 1 as `RG8Unorm`, then convert in a shader. Apple doesn't literally say "zero-copy", but "mapped"/"live binding" means no copy.
- GPUI already does this. `gpui-pre`'s `surface()` element takes a `CVPixelBuffer` and maps both planes through `CVMetalTextureCache` (`gpui-pre-apple` 0.3.14 `metal_renderer.rs`). It has two limits:
  - it **rejects every pixel format except `kCVPixelFormatType_420YpCbCr8BiPlanarFullRange` (`420f`)**
  - its fragment shader hard-codes **BT.601 full-range** coefficients (`shaders.metal`, `surface_fragment`)

  HDMI sources at HD are normally BT.709, usually limited range. So the built-in surface would show wrong colours for a PS5 unless capture requests `420f` *and* the shader is replaced. For [#5](https://github.com/pwn233/xReveal/issues/5) to settle.

## Same Source after a reboot

| OS | Stable ID | Caveat |
| --- | --- | --- |
| Windows | MF symbolic link (`MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK`), "an opaque string" that "uniquely identifies the device on the system" ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-devsource-attribute-source-type-vidcap-symbolic-link)). It embeds the device instance ID. | The instance ID "contains serial number information, if supported … or some kind of location information" and "is persistent across system restarts" ([MS Learn](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/instance-ids)). For a USB device **without a serial number, the instance is tied to the port** ([MS Learn, IgnoreHWSerNum](https://learn.microsoft.com/en-us/windows-hardware/drivers/usbcon/usb-device-specific-registry-settings)). |
| macOS | `AVCaptureDevice.uniqueID` "persists on one system across device connections and disconnections, application restarts, and reboots" ([Apple](https://developer.apple.com/documentation/avfoundation/avcapturedevice/uniqueid)). `modelID` is the same for every unit of a model ([Apple](https://developer.apple.com/documentation/avfoundation/avcapturedevice/modelid)). | For UVC devices the uniqueID is, in practice, USB locationID + VID + PID, so it **changes when the card moves to another port**. Apple doesn't document this; it comes from developer reports ([Apple Developer Forums 803759](https://developer.apple.com/forums/thread/803759)). |

**Implication for the Remembered setup:** store the OS ID *and* a fallback key (friendly name + VID/PID, or `modelID` on macOS). Match exactly first, then fall back to a single model match. That keeps the setup working after a port change.

## Crates: `nokhwa` vs direct bindings

| | `nokhwa` 0.10.11 (latest on crates.io; [docs.rs](https://docs.rs/crate/nokhwa/latest)) | Direct `windows` + `objc2-*` |
| --- | --- | --- |
| Windows backend | MF Source Reader, **synchronous** `ReadSample`, `MF_READWRITE_DISABLE_CONVERTERS`, no `MF_LOW_LATENCY`. **Copies every frame into a new `Vec`** ([`nokhwa-bindings-windows/src/lib.rs` @ fa5a192 L1140-1215](https://github.com/l1npengtul/nokhwa/blob/fa5a19208ade2b22113bfaa48f20dfdd3758b751/nokhwa-bindings-windows/src/lib.rs#L1140-L1215)) | Async reader, `MF_LOW_LATENCY`, upload straight from the locked `IMFMediaBuffer` into a D3D11 texture |
| macOS backend | Old `objc` 0.2 + `core-video-sys`. **Copies every frame** (`to_vec`) in the delegate. **Unbounded** `flume` channel. `frame()` takes the **oldest** queued frame, then drains the rest. Maps `NV12` to `kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange` (a 10-bit format). Tags every frame `GRAY` ([`nokhwa-bindings-macos/src/lib.rs` @ fa5a192 L437-500, L2337-2357](https://github.com/l1npengtul/nokhwa/blob/fa5a19208ade2b22113bfaa48f20dfdd3758b751/nokhwa-bindings-macos/src/lib.rs#L437-L500); [`src/backends/capture/avfoundation.rs` L88, L282-292](https://github.com/l1npengtul/nokhwa/blob/fa5a19208ade2b22113bfaa48f20dfdd3758b751/src/backends/capture/avfoundation.rs#L282-L292)) | Keep the `CVPixelBuffer` and hand it to Metal / GPUI `surface()` with no copy |
| Vendor dialog | No | Yes (DirectShow, Windows) |
| GPU output | `output-wgpu` *copies* into a wgpu texture ([docs.rs](https://docs.rs/crate/nokhwa/latest)) | Native D3D11 / Metal, in GPUI's own device |
| Maintenance | `senpai` (0.11) branch unreleased; 57 open issues ([GitHub](https://github.com/l1npengtul/nokhwa)) | `windows` 0.62 is already in the tree via GPUI. [`objc2-av-foundation` 0.3.2](https://docs.rs/crate/objc2-av-foundation/latest) includes `AVCaptureVideoDataOutput` (features `AVCaptureOutputBase`, `AVCaptureVideoDataOutput`, `dispatch2`) ([docs.rs](https://docs.rs/objc2-av-foundation/latest/objc2_av_foundation/struct.AVCaptureVideoDataOutput.html)), and GPUI already depends on `objc2-core-video` |

**Verdict:** write a thin `Source` backend per OS on `windows` and `objc2-av-foundation`. It is roughly the size of nokhwa's bindings, but without the copies and the stale-frame queue.

## Pixel format ranking (auto-chosen, never shown)

| Rank | Format | Why |
| --- | --- | --- |
| 1 | NV12 (`420v`/`420f` on macOS) | 12 bpp. Native GPU texture format on D3D11 ([DXGI_FORMAT_NV12](https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format)) and Metal (two planes via `CVMetalTextureCache`). GPUI's macOS surface takes only this layout. |
| 2 | YUY2 / `yuvs`/`2vuy` | Uncompressed 4:2:2, 16 bpp. One shader unpack on D3D11 (`DXGI_FORMAT_YUY2`). On macOS, either convert or unpack. Often the only uncompressed format on cheap UVC cards. MS2130 lists "YUV422, MJPEG" USB output up to 1080p60 ([vendor IC listing](https://www.ultrasemi.com/en/ic/macrosilicon/ms2130); [CNX Software](https://www.cnx-software.com/2022/11/07/ms2130-based-4k-hdmi-to-usb-3-0-video-capture-dongle-sells-for-19/)), and the newer MS2130S adds NV12 ([vendor](https://www.ultrasemi.com/en/ic/macrosilicon/ms2130s)). |
| 3 | MJPEG | Needs a JPEG decode (CPU or HW MFT / VideoToolbox) before upload: more latency, CPU and jitter. Use only when the requested mode has no uncompressed format, e.g. USB 2 links or 4K on MS2130S-class chips. Frame Server can decode MJPEG once to YUY2 when the device opts in ([MS Learn](https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/dshow-bridge-implementation-guidance-for-usb-video-class-devices)). |
| — | RGB24/RGB32 | Avoid. Usually a driver-side conversion, with bigger frames. |

## Comparison summary

| | Windows: MF Source Reader | Windows: DirectShow | macOS: AVFoundation | `nokhwa` |
| --- | --- | --- | --- | --- |
| Latency | ~18 ms p50 glass-to-app on one card (caplat) | ≈ MF (17.9 ms) | No public measurement found | Extra copy per frame; macOS returns stale frames |
| Supported by OS vendor | Yes (recommended) | Legacy | Yes | n/a |
| Resolution + FPS | Media types | [`IAMStreamConfig`](https://learn.microsoft.com/en-us/windows/win32/api/strmif/nn-strmif-iamstreamconfig) | `activeFormat` + frame durations | Yes |
| Color space + range | Reported in media type; we interpret | Reported in [`VIDEOINFOHEADER2`](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/api/dvdmedia/ns-dvdmedia-videoinfoheader2) | Pixel format range + buffer attachments | Not exposed |
| Buffering control | Our queue; `MF_LOW_LATENCY` | [`IAMBufferNegotiation`](https://learn.microsoft.com/en-us/windows/win32/api/strmif/nn-strmif-iambuffernegotiation) | Our queue; `alwaysDiscardsLateVideoFrames` | Unbounded (macOS) |
| Vendor dialog | No (borrow DirectShow) | **Yes** | None exists | No |
| Frame on GPU without copy | No (USB frames are system memory; one upload) | No | **Yes** (`CVMetalTextureCache`) | No |
| Re-identify Source | Symbolic link (port-bound without serial) | Moniker path (same) | `uniqueID` (port-bound for UVC) | Index / misc string |

## Open questions surfaced

1. **Reference card formats.** What exact native formats and modes does the Onten card expose in MF and in AVFoundation? Which chip is it (MS2130 / MS2130S / other)? Is NV12 offered at 1080p60? This decides whether rank 1 or rank 2 is the real-world path.
2. **Driver-based card coverage on Windows.** Do Elgato and AVerMedia PCIe/USB cards appear in `MFEnumDeviceSources`, or do some appear only in DirectShow? If some are DirectShow-only, v1 needs a DirectShow streaming fallback or an explicit "unsupported".
3. **Vendor dialog while MF streams.** Can the DirectShow filter's property pages be opened while the same device streams through MF and Frame Server? Do changes made there apply live? Should crossbar dialogs (input selection on PCIe cards) count as part of "vendor settings dialog"?
4. **`MF_LOW_LATENCY` on a capture source.** Does it change anything? Is sync vs async `ReadSample` measurable? Needs the latency-measurement method (map "Not yet specified": Latency measurement).
5. **macOS YUY2 → `420f` conversion cost.** When a card is YUY2-only, does AVFoundation's conversion add a frame or run on the CPU? Or should xReveal take native `yuvs` and unpack in its own shader? This ties into [#5](https://github.com/pwn233/xReveal/issues/5).
6. **GPUI surface fit (for [#5](https://github.com/pwn233/xReveal/issues/5)).** GPUI's macOS `surface()` accepts only `420f` and uses a BT.601 full-range matrix. Its Windows `draw_surfaces` is empty. Either xReveal patches or forks this, or it draws video through its own GPU surface.
7. **Remembered setup matching rule.** Is "exact OS ID, else unique model match (name + VID/PID / `modelID`)" acceptable, and what happens with two identical cards? (Map "Not yet specified": Remembered setup identity.)
8. **macOS driver-based cards.** Which current Elgato/AVerMedia cards ship Camera Extensions (and so work in AVFoundation on macOS 14+), and which need the vendor app?
