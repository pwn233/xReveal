# MJPEG decoding for xReveal (facts for "Is MJPEG decoding in v1, and with which decoder?")

Gathered 2026-11-25 for [issue #17](https://github.com/pwn233/xReveal/issues/17). Benchmarks were run on an M3 Mac with the Reference card (MS2131) plugged in. The decoder benchmark is in `tools/mjpeg-bench/`, and the macOS native-format probe is `tools/capture-probe/mjpeg-native-format.swift`.

## macOS
- AVFoundation never exposes the card's MJPEG. The MS2131 lists only `420v` and `yuvs`.
- `videoSettings = [:]` (device-native) delivers `420v` image buffers with no compressed data. `videoSettings = nil` delivers `2vuy`, which costs an extra conversion.
- `availableVideoCodecTypes` (`avc1`, `jpeg`) are output *compression* settings, not a way to receive camera MJPEG. Elgato 4K X on macOS reportedly shows NV12 for every mode ([StreamGuides](https://streamguides.gg/2024/02/elgato-4k-x-4k-pro-review/)).

## Windows: Media Foundation
- Setting an uncompressed output type on the Source Reader makes it load a decoder automatically ([Processing media data with the Source Reader](https://learn.microsoft.com/en-us/windows/win32/medfound/processing-media-data-with-the-source-reader)).
- The built-in decoder is the "MJPEG Decoder MFT" (`mfmjpegdec.dll`). It is synchronous software and registered for **YUY2 output only** ([alax.info](https://alax.info/blog/1375)). Getting NV12 would need `MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING`, an extra pass. Its runtime output types and its speed are **unconfirmed**.
- Vendor hardware decoders exist (Intel Hardware M-JPEG Decoder MFT, AMD MFT MJPEG Decoder, NVIDIA `nvDecMFTMjpeg.dll`). They need `MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS` plus a D3D manager, and are reported buggy: Intel's fails `SetInputType` ([Intel forum](https://community.intel.com/t5/Media-Intel-oneAPI-Video/Intel-Hardware-M-JPEG-Decoder-MFT/td-p/1004407)); AMD's spawns a thread per frame and drops frames at high rates ([alax.info](https://alax.info/blog/1996)).
- Direct3D MJPEG decode profiles (`MJPEG_VLD_420/422/444/4444`) are defined in [DirectX-Headers](https://github.com/microsoft/DirectX-Headers). Which drivers expose them is unknown.
- Windows "MJPEG at source autodecode" only applies when the device opts in via INF/firmware. Otherwise MJPEG stays visible to apps ([Learn](https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/mjpeg-at-source-autodecode-for-uvc)).

## Windows: DirectShow and OBS
- The MJPEG Decompressor filter (quartz.dll) is software and outputs RGB ([Learn](https://learn.microsoft.com/en-us/windows/win32/directshow/mjpeg-decompressor-filter)), so it means CPU colour conversion.
- OBS `win-dshow` takes MJPEG compressed and decodes it with libavcodec's `mjpeg` decoder. libavcodec's MJPEG hardware paths are NVDEC/VAAPI only, so OBS on Windows decodes MJPEG in software.

## Rust decoders (M3, one thread, 1080p 4:2:2)

| Avg frame | zune-jpeg 0.5.15 RGB | zune-jpeg YCbCr | jpeg-decoder 0.3.2 | libavcodec |
|---|---|---|---|---|
| 113 KB | 2.85 ms | 2.48 ms | 9.3 ms | 2.2 ms |
| 402 KB | 6.35 ms | 6.02 ms | 12.2 ms | 5.8 ms |
| 735 KB | 9.1 ms | 8.8 ms | 14.6 ms | 8.8 ms |

- Added size (stripped, LTO, arm64): zune-jpeg about 146 KB, jpeg-decoder about 147 KB.
- Planar YUV output: turbojpeg (`decompress_to_yuv`, needs CMake and NASM); zune-jpeg only in 0.5.16-rc.
- USB 2 caps MJPEG frames at roughly 400 KB at 60 fps. About 6 ms × 60 fps ≈ 36% of one core ≈ 2.3% of a 16-thread machine, 4.5% of an 8-thread one.
- x86 timings are unmeasured. zune-jpeg has no NEON code, so x86 AVX2 may be a little faster.

## Cards that need MJPEG for their headline modes
| Card | MJPEG for | Uncompressed limit |
|---|---|---|
| MS2109 | 1080p (its "1080p60" is really 30) | YUY2 1080p @ 5 fps |
| MS2130/2131 on USB 2 | 1080p | YUY2 1080p60 needs USB 3 |
| Elgato 4K S | 4K60, 1080p144/240 | NV12 1080p120 |
| Elgato 4K X | 4K120/144 | NV12 4K60, 1080p240 (10 Gbps) |
| AVerMedia GC553G2 | 4K120/144 | NV12 1080p240 |
