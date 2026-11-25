# Capture card families: what they expose, and what xReveal must handle

Research for [#7](https://github.com/pwn233/xReveal/issues/7) (map: [#2](https://github.com/pwn233/xReveal/issues/2)). Researched 2026-11-25.

Sources are primary unless labelled. Primary means manufacturer spec or knowledge-base pages, the USB-IF UVC 1.5 spec, OS vendor docs, or Linux kernel source. **[measurement]** marks a published latency test by a third party. **[community]** marks forum or wiki reports and retailer listings, which carry less weight. **[inference]** marks my own reasoning that no source states outright.

## Short answer

- **v1 can support every UVC/UAC card through the standard OS capture APIs alone**: AVFoundation on macOS and Media Foundation on Windows, with no vendor software. That covers:
  - generic MacroSilicon cards (MS2109, MS2130 and newer);
  - Elgato's USB line (Cam Link 4K, HD60 S+, HD60 X, Neo, 4K S, 4K X);
  - AVerMedia's USB line (Live Gamer Ultra GC553, Ultra 2.1 GC553G2, Live Gamer Portable 2 Plus GC513).

  The Reference card is almost certainly in this group, since every Onten listing found says it is UVC/UAC and driverless.
- **Driver-based cards** are the PCIe cards plus the old proprietary-USB and Thunderbolt models:
  - Elgato 4K60 Pro MK.2, 4K Pro, HD60 Pro, HD60 S and 4K60 S+;
  - AVerMedia Live Gamer 4K GC573 and Live Gamer BOLT GC555.

  These do **not** need a vendor SDK to deliver video. They need the **vendor driver**, and on Windows they show up as **DirectShow** devices. Microsoft documents Media Foundation capture only for UVC devices, so supporting these cards means adding a DirectShow path. On macOS most of them have no driver at all. Elgato's PCIe cards and the 4K60 S+ are Windows-only, and the GC555 works only on Intel Macs up to macOS 13. The 4K60 S+ also sends compressed video with about 250 ms preview latency, so it is unusable for play in any case.
- **A vendor SDK or utility is needed only for extras**, never for the picture itself. Examples:
  - Elgato's HDR-tonemapping toggle, colour-range Shrink/Expand, EDID mode and HDMI/analog audio switch;
  - AVerMedia's 5.1 audio, which is Windows-only and goes through RECentral or an OBS plugin.

  The v1 scope does not need any of these.
- **Recommendation**: v1 supports UVC video plus UAC audio on both OSes. Driver-based cards are a separate decision (see [New questions](#new-questions-for-later-tickets)).

## Reference card (Onten): chipset not confirmed

- Onten sells several HDMI-to-USB capture cards, and its own pages give no chipset ([OTN-332](https://www.onten.com.cn/enn-18.html), [OTN-UC323](https://www.onten.com.cn/enp-395.html)). The OTN-332 captures at 4K30 with a 34 ms passthrough, which an MS2130 cannot do.
- The OTN-US331 retailer listing says "1080p@60Hz out, YUV422 / JPEG, UAC, driverless" [community] ([retailer](https://mustafacomputers.pk/products/onten-otn-us331-usb-3-0-video-capture-card-hdmi-input-4k-60hz-output-1080p-60hz-plug-play-for-dslr-ps5-xbox-nintendo-switch-set-top-boxes-laptops)). That is exactly the MS2130 profile. Models listed with 1080p30 output, such as the US323T and US302, match MS2109-class chips [community] ([US323T](https://www.imediastores.com/product/onten-us323t-usb-3-0-audio-video-capture-card/), [US302](https://www.imediastores.com/product/onten-us302-usb-3-0-audio-video-capture-card-0-2m/)). **Uncertain**: which model the user owns.
- **How to settle it**: plug the card in and read its USB IDs, for example with `ioreg -p IOUSB -l -w0 | grep -E '"idVendor"|"idProduct"|"USB Product Name"'`.
  - `0x345f:0x2130` ("USB3 Video" or "UltraSemi USB3 Video") means **MS2130**.
  - `0x534d:0x2109` means **MS2109**.
  - Sources for these IDs: [linux-hardware.org 345f:2130](https://linux-hardware.org/?id=usb:345f-2130), the [kernel log for both IDs](https://hackaday.io/project/196140-streamo-encodo-cheapo/log/229995-syslog-for-ms2130-dongle) [community], and the MS2109 quirk in the Linux kernel ([quirks-table.h](https://github.com/torvalds/linux/blob/master/sound/usb/quirks-table.h)).
  - The `bcdDevice` field shows the USB link speed. An MS2130 on a USB 2 port reports `21.00` and "USB2 Video" instead of `31.00` ([syslog](https://hackaday.io/project/196140-streamo-encodo-cheapo/log/229995-syslog-for-ms2130-dongle)).

## Family tables

### 1. Generic MacroSilicon UVC/UAC (MS2109, MS2130 and newer). Probably the Reference card.

| Aspect | Finding |
|---|---|
| Visible to the OS with no vendor software | Yes, on Windows and macOS. MS2130: "USB video conforms to UVC1.0 … USB audio conforms to UAC1.0", OS list includes Windows and Mac OS ([MacroSilicon via UltraSemi](https://www.ultrasemi.com/en/ic/macrosilicon/ms2130), [LCSC](https://www.lcsc.com/product-detail/C5905137.html)). The device also has a HID interface ([syslog](https://hackaday.io/project/196140-streamo-encodo-cheapo/log/229995-syslog-for-ms2130-dongle)) [community]. |
| Video formats at 1080p60+ | **MS2130 on USB 3**: YUV422 (YUY2) at 1920×1080@60 uncompressed, plus MJPEG. Its MJPEG tops out at 2560×1440@30 ([UltraSemi selection guide](https://www.ultrasemi.com/en/ic/macrosilicon/video-capture)). The datasheet table lists 1080p at 60/50/30/20/10 Hz in both YUV422 and MJPEG ([datasheet, Scribd mirror](https://www.scribd.com/document/896789866/MS2130-datasheet)) [community mirror]. HDMI input goes up to 4K30, which the card downscales. **MS2130 on USB 2**: YUV422 1080p only at 10/5 Hz, so 1080p60 is impossible and MJPEG reaches 1080p50 at most (same datasheet). **MS2109S (USB 2)**: YUV422 1080p@10, MJPEG 1080p@60 ([UltraSemi](https://www.ultrasemi.com/en/ic/macrosilicon/video-capture)); older MS2109 boards report MJPEG 1080p30 [community] ([VideoHelp](https://forum.videohelp.com/threads/407889-MS2130-chipset-the-new-low-budget-Hdmi-capture-card-killer)). **Newer chips (MS2130S, MS2131S, MS2132)**: only MJPEG reaches 1080p240 or 4K60. Uncompressed tops out at RGB24 1080p60 or YUV420 4K30 ([UltraSemi](https://www.ultrasemi.com/en/ic/macrosilicon/video-capture)). |
| Controls | Standard UVC Processing Unit controls, so there are no colour-range, colour-space or buffering controls (see [UVC limits](#what-the-uvc-spec-does-and-does-not-signal)). MS2130 enhancements (scaling, sharpening, luma/chroma processing) are vendor registers reached outside UVC ([osmocom hsdaoh slides](https://people.osmocom.org/steve-m/hsdaoh_slides/rendered/osmodevcon2024_hsdaoh.pdf)). Measured colour behaviour of MS2130 YUV: **full range, FCC-like coefficients** [measurement] ([HyperHDR #499](https://github.com/awawa-dev/HyperHDR/discussions/499)). Another viewer assumes YUYV is *limited* and MJPEG *full* [community] ([protoHDMIcapture](https://github.com/mattiasbrandt/protoHDMIcapture)). **The two contradict each other, so this is unresolved.** |
| Vendor settings dialog | No vendor app. On Windows only the generic UVC property page (see [Differences](#differences-xreveal-must-handle), item 4). On macOS none. |
| Audio | A separate UAC capture device. MS2130 defaults to 48 kHz stereo ([UltraSemi](https://www.ultrasemi.com/en/ic/macrosilicon/video-capture)). **MS2109 quirk**: "These claim 96kHz 1ch in the descriptors, but are actually 48kHz 2ch … channels to be swapped and out of phase." Linux patches this in the kernel ([quirks-table.h](https://github.com/torvalds/linux/blob/master/sound/usb/quirks-table.h), [quirks.c](https://github.com/torvalds/linux/blob/master/sound/usb/quirks.c)). The kernel has no such quirk for `345f:2130`, so MS2130 audio is presumably fine [inference]. |
| Known latency | MS2130 YUY2: about 66 ms at 1080p60 and about 49 ms at 1080p120 end to end, versus about 80–100 ms for the MS2109 (MJPEG) [measurement] ([HyperHDR #499](https://github.com/awawa-dev/HyperHDR/discussions/499)). A community wiki gives 40–50 ms for MS2130 cards at 1080p60 YUY2 [community] ([Veyra wiki](https://github.com/Likely7/Veyra-NRVideo/wiki/Guide-03-Capture-Card-Buying)). MJPEG CPU decode adds only about 2 ms at 720p and 5 ms at 1080p [community] ([protoHDMIcapture](https://github.com/mattiasbrandt/protoHDMIcapture)). So most of the MS2109→MS2130 gain comes from the USB 3 hardware path, not from skipping the decode. Also note that the MS2130 "output is a configured framerate, irregardless of the input framerate" ([osmocom slides](https://people.osmocom.org/steve-m/hsdaoh_slides/rendered/osmodevcon2024_hsdaoh.pdf)). |

### 2. Elgato UVC cards (HD60 S+, HD60 X, Neo, 4K S, 4K X, Cam Link 4K)

| Aspect | Finding |
|---|---|
| Visible to the OS with no vendor software | Yes. Elgato's driver table marks Cam Link 4K, HD60 S+, HD60 X, 4K X, Neo and 4K S as "UVC Device (no download)" ([Elgato drivers](https://help.elgato.com/hc/en-us/articles/360027961152-Elgato-Hardware-Drivers)). They work in macOS UVC software such as OBS, and the Mac needs Camera and Microphone permission ([HD60 S+ software](https://help.elgato.com/hc/en-us/articles/360034401051-Elgato-Game-Capture-HD60-S-What-Software-Can-It-Use)). |
| Video formats at 1080p60+ | **Neo**: YUY2 and NV12 only, 1080p60/30 and 720p60/30, with no MJPEG ([Neo resolutions](https://help.elgato.com/hc/en-us/articles/26438781217933-Elgato-Game-Capture-Neo-Supported-passthrough-and-capture-resolution)). **4K X**: MJPG, P010, RGB, YUY2, i420 and NV12. On a 5 Gbps port, 1080p60 is available uncompressed (NV12/i420/YUY2) and NV12 also reaches 1080p120. On 10 Gbps, NV12 reaches 1080p240 and 4K60. MJPG reaches 4K144 on either link ([4K X resolutions](https://help.elgato.com/hc/en-us/articles/23479175821069-Elgato-Game-Capture-4K-X-Supported-Resolutions-and-Frame-Rates)). **HD60 X**: YUYV up to 1080p60 and NV12 up to 1080p120 or 4K30, with no MJPEG [measurement] ([HyperHDR review](https://www.hyperhdr.eu/2022/07/elgato-game-capture-hd60x-review.html)). HDR capture is Windows-only; on macOS the 4K X converts HDR to SDR itself ([4K X resolutions](https://help.elgato.com/hc/en-us/articles/23479175821069-Elgato-Game-Capture-4K-X-Supported-Resolutions-and-Frame-Rates)). The Neo always tonemaps on the device ([Neo specs](https://help.elgato.com/hc/en-us/articles/26022609172749-Elgato-Game-Capture-Neo-Technical-Specifications)). |
| Controls | Not exposed through UVC as range or colour-space controls. Elgato's **Capture Device Utility** (macOS) adds: HDMI colour range (Bypass, Shrink to 16-235, Expand to 0-255), EDID mode, USB bulk/isochronous mode and HDR tonemapping on/off ([CDU 1.1 notes](https://help.elgato.com/hc/en-us/articles/17094488517645-Capture-Device-Utility-1-1-Release-Notes)). There is also a public HID API for HD60 S+/HD60 X tonemapping and HDR status on Windows and macOS ([elgatosf/capture-device-support](https://github.com/elgatosf/capture-device-support)). |
| Vendor settings dialog | No OS-level vendor dialog. Settings live in Elgato's own apps (Studio, 4K Capture Utility, Capture Device Utility). |
| Audio | A separate UAC input device with the card's name. On macOS OBS adds it as "Audio Input Capture → Elgato Game Capture HD60 X" ([HD60 X on macOS](https://help.elgato.com/hc/en-us/articles/5617210293645-Elgato-Game-Capture-HD60-X-Set-Up-in-OBS-Studio-on-macOS)). Switching HDMI and analog input needs Elgato software ([audio switch](https://help.elgato.com/hc/en-us/articles/17903430838925-Elgato-Capture-Device-Utility-Switching-Audio-Input-between-HDMI-and-Analog-on-macOS)). |
| Other | **No multi-app support**: HD60 S+, HD60 X, 4K X and Neo cannot feed two apps at once ([Elgato comparison](https://help.elgato.com/hc/en-us/articles/360041842611-Elgato-Capture-Devices-Device-comparison)). |
| Known latency | HD60 X NV12: about 60 ms at 720p60 and about 44 ms at 1080p120 [measurement] ([HyperHDR review](https://www.hyperhdr.eu/2022/07/elgato-game-capture-hd60x-review.html)). 4K X: about 35 ms [community] ([Veyra wiki](https://github.com/Likely7/Veyra-NRVideo/wiki/Guide-03-Capture-Card-Buying)). Elgato markets "Instant Gameview" low latency but publishes no figures ([comparison](https://help.elgato.com/hc/en-us/articles/360041842611-Elgato-Capture-Devices-Device-comparison)). |

### 3. Elgato driver-based cards (4K60 Pro MK.2, 4K Pro, HD60 Pro, HD60 S, 4K60 S+)

| Aspect | Finding |
|---|---|
| Visible to the OS with no vendor software | **No.** All of them are "Download" (need a driver) in Elgato's table ([drivers](https://help.elgato.com/hc/en-us/articles/360027961152-Elgato-Hardware-Drivers)). With the driver they are DirectShow devices: "4K60 Pro uses DirectShow and as such any DirectShow software should be compatible" ([3rd-party support](https://help.elgato.com/hc/en-us/articles/360028228471-Elgato-Gaming-hardware-and-3rd-Party-Software-support), [anti-cheat note](https://help.elgato.com/hc/en-us/articles/9482241809805-No-Signal-When-Launching-Certain-PC-Games)). **macOS: none.** The 4K60 Pro MK.2 "does not support macOS at all" ([MK.2 software](https://help.elgato.com/hc/en-us/articles/360034029932-Elgato-Game-Capture-4K60-Pro-MK-2-What-Elgato-Software-Can-It-Use)), and the 4K60 S+ "cannot be connected to a Mac" ([4K60 S+](https://help.elgato.com/hc/en-us/articles/360038512571-Elgato-Game-Capture-4K60-S-Windows-10-Is-Supported-macOS-is-not-Supported)). The older USB HD60 S on macOS only works in Elgato's legacy Game Capture HD app ([3rd-party support](https://help.elgato.com/hc/en-us/articles/360028228471-Elgato-Gaming-hardware-and-3rd-Party-Software-support)). |
| Video formats at 1080p60+ | The PCIe cards deliver uncompressed video, with P010 for HDR on the DirectShow pin ([capture-device-support](https://github.com/elgatosf/capture-device-support)). The **4K60 S+ uses an on-device HEVC/H.264 hardware encoder** ([comparison](https://help.elgato.com/hc/en-us/articles/360041842611-Elgato-Capture-Devices-Device-comparison), [capture-device-support](https://github.com/elgatosf/capture-device-support)). |
| Controls | In OBS's dshow source: "Configure Video" shows signal and driver info, and "Configure Crossbar" is unused. The Color Space, Color Range and Buffering options are **OBS's own** settings, not the card's ([PCIe OBS setup](https://help.elgato.com/hc/en-us/articles/360037206051-Elgato-internal-PCIe-Capture-Card-OBS-Studio-Setup)). Vendor features go through `IKsPropertySet` ([capture-device-support](https://github.com/elgatosf/capture-device-support)). |
| Vendor settings dialog | Yes, on Windows: the DirectShow filter's property page ("Configure Video") ([PCIe OBS setup](https://help.elgato.com/hc/en-us/articles/360037206051-Elgato-internal-PCIe-Capture-Card-OBS-Studio-Setup)). |
| Audio | Through the DirectShow device. OBS offers "Audio Output Mode" on the video source, or a custom audio device ([PCIe OBS setup](https://help.elgato.com/hc/en-us/articles/360037206051-Elgato-internal-PCIe-Capture-Card-OBS-Studio-Setup)). **Uncertain**: whether a separate WASAPI endpoint also appears. |
| Other | Anti-cheat in games such as Valorant and Fortnite can block DirectShow capture drivers. Elgato's UVC devices are not affected ([Elgato](https://help.elgato.com/hc/en-us/articles/9482241809805-No-Signal-When-Launching-Certain-PC-Games)). |
| Known latency | 4K60 S+: "~250ms Preview Latency" ([comparison](https://help.elgato.com/hc/en-us/articles/360041842611-Elgato-Capture-Devices-Device-comparison)). 4K60 Pro in Elgato's own 4K Capture Utility preview: about 200 ms. Across all cards tested in OBS 23, the gap between fastest and slowest was about 30–40 ms [measurement] ([EposVox, OBS forum, 2019](https://obsproject.com/forum/resources/capture-card-documentation-latency-decode-modes-formats-more.777/)). |

### 4. AVerMedia UVC cards (Live Gamer Ultra GC553, Ultra 2.1 GC553G2, Live Gamer Portable 2 Plus GC513)

| Aspect | Finding |
|---|---|
| Visible to the OS with no vendor software | Yes. GC553: "(Support UVC Protocol)", Windows 10 x64 and macOS 10.13+ ([GC553 page](https://www.avermedia.com/product-detail/GC553)). GC553G2: plug and play, macOS 13/14 or later ([datasheet](https://storage.avermedia.com/web_release_www/datasheet/GC553G2_Live_Gamer_Ultra_2.1_Datasheet_EN_2024.pdf)). GC513: "Interface: USB 2.0 (UVC)", with Mac OS X support marked "(beta)" ([GC513 datasheet](https://storage.avermedia.com/web_release_www/GC513/GC513_datasheet_20170831.pdf)). |
| Video formats at 1080p60+ | **GC553G2**: YUY2, NV12, RGB24, P010 (HDR) and MJPEG. At 1080p on 10 Gbps: YUY2 up to 120, NV12 up to 240, MJPEG up to 240. On 5 Gbps: YUY2 60 and NV12 120. At 4K, MJPEG reaches 144, while NV12 reaches 60 (10 Gbps) or 30 (5 Gbps) ([GC553G2 manual 2024, p.14](https://storage.avermedia.com/web_release_www/user_manual/GC553G2%20User%20Manual%202024.pdf)). On macOS: "For 4K60 MJPEG capture, requires M1 Ultra / M2 Ultra or above", and HDR capture is not supported (same manual, p.11). **GC553**: maximum capture 2160p30 / 1440p60 / 1080p120 / 1080p60 HDR on USB 3.1 Gen 1. Its pixel formats are not listed on the page ([GC553 page](https://www.avermedia.com/product-detail/GC553)). **Uncertain**. **GC513**: USB 2.0, so 1080p60 must be compressed (MJPEG or H.264). It records in "MPEG 4 (H.264+AAC) / MJPEG" [community] ([retailer listing](https://www.amazon.fr/AverMedia-Live-Gamer-Portable-Plus/dp/B075TY4933)). The exact UVC formats are **uncertain**. |
| Controls | No range or colour-space controls documented. AVerMedia's Gaming Utility handles extras such as duplicate-audio removal and the HDMI 2.0 mode for 4K30 RGB24 ([GC553G2 manual](https://storage.avermedia.com/web_release_www/user_manual/GC553G2%20User%20Manual%202024.pdf)). One kernel quirk: an AVerMedia GC515 advertises the UVC 1.5 protocol but behaves as UVC 1.1 ([uvc_driver.c](https://github.com/torvalds/linux/blob/master/drivers/media/usb/uvc/uvc_driver.c)). |
| Vendor settings dialog | No OS-level dialog documented. Settings are in AVerMedia apps (RECentral, Gaming Utility). |
| Audio | A separate UAC device whose **name differs from the video device**: "Digital Audio Interface(Live Gamer Portable 2 Plus)". AVerMedia suggests an OBS sync offset of −50 to −200 ms ([AVerMedia FAQ](https://www.avermedia.com/support/faq/how-do-i-set-up-the-audio-in-obs-and-streamlabs-obs)). On the GC553G2, 5.1 capture is Windows-only and needs RECentral or an OBS plugin ([manual](https://storage.avermedia.com/web_release_www/user_manual/GC553G2%20User%20Manual%202024.pdf)). |
| Known latency | No AVerMedia figures. RECentral preview beat OBS on most cards [measurement] ([EposVox](https://obsproject.com/forum/resources/capture-card-documentation-latency-decode-modes-formats-more.777/)). Reviewers say the GC513's preview over USB 2 is "far from a latency free experience" [community] ([Hardcore Gamer](https://hardcoregamer.com/reviews/review-avermedia-live-gamer-portable-2-plus/274361/)). |

### 5. AVerMedia driver-based cards (Live Gamer 4K GC573, Live Gamer BOLT GC555)

| Aspect | Finding |
|---|---|
| Visible to the OS with no vendor software | **No.** GC573: PCIe Gen 2 x4, Windows 10 x64 or later, with a vendor driver ([GC573 page](https://www.avermedia.com/product-detail/GC573), [downloads](https://www.avermedia.com/US/support/download?model_number=GC573)). GC555 (Thunderbolt 3): macOS driver up to macOS 13. "macOS 14 (inclusive) or later are not supported, and M series versions are not supported either" ([GC555 page](https://www.avermedia.com/product-detail/GC555)). |
| Video formats at 1080p60+ | GC573: YUY2, NV12, RGB24 and P010 (HDR), all uncompressed, up to 2160p60 HDR, 1440p144 or 1080p240 ([GC573 page](https://www.avermedia.com/product-detail/GC573)). |
| Controls, dialog, audio | Not documented in primary sources; **uncertain**. The GC573 is presumably a DirectShow-style driver like Elgato's PCIe cards [inference]. |
| Known latency | No primary figure. |

## What the UVC spec does and does not signal

These limits apply to every family above, because the UVC cards all go through the same class driver.

- **Pixel formats**: the uncompressed payload defines YUY2, NV12, M420 and I420 by GUID ([UVC 1.5 Uncompressed Payload, §2.2](https://www.usb.org/document-library/video-class-v15-document-set)). MJPEG, H.264 and frame-based payloads are separate specs in the same set.
- **Colour**: the optional Color Matching descriptor carries `bColorPrimaries`, `bTransferCharacteristics` and `bMatrixCoefficients` (UVC 1.5 §3.9.2.6). **It has no field for quantisation range (full vs limited).** When the descriptor is absent, the matrix defaults to **SMPTE 170M (BT.601)**. HD HDMI sources are normally BT.709, so a card that omits the descriptor is, by the spec, saying "601". Linux maps the same three fields and has no range field either ([uvc_driver.c](https://github.com/torvalds/linux/blob/master/drivers/media/usb/uvc/uvc_driver.c)).
- **Controls**: the Processing Unit offers backlight compensation, brightness, contrast, gain, power-line frequency, hue, saturation, sharpness, gamma, white balance, digital multiplier and analog video standard (UVC 1.5 §4.2.2.3). There is **no colour-range, colour-space or buffering control**. Vendors add those through Extension Units or HID, as Elgato does.
- **Windows**: the inbox `usbvideo.sys` handles UVC 1.0/1.1/1.5, MJPEG, DV, YUY2, NV12 and frame-based formats, and H.264 from Windows 8 ([MS: UVC driver overview](https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/usb-video-class-driver-overview)). Media Foundation: "Video capture devices are supported through the UVC class driver and must be compatible with UVC 1.1. Audio capture devices are supported through … WASAPI" ([MS: A/V capture in MF](https://learn.microsoft.com/en-us/windows/win32/medfound/audio-video-capture-in-media-foundation)). UAC 2.0 has an inbox driver (`usbaudio2.sys`) since Windows 10 1703 ([MS: USB Audio 2.0 drivers](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/usb-2-0-audio-drivers)).
- **macOS**: Apple ships a class-compliant UVC camera extension, and CoreMediaIO camera extensions replace DAL plug-ins, which were deprecated in 12.3 and later disabled ([WWDC22 10022](https://developer.apple.com/videos/play/wwdc2022/10022/)). `AVCaptureDevice.DeviceType.external` exists from macOS 14 ([Apple docs](https://developer.apple.com/documentation/avfoundation/avcapturedevice/devicetype-swift.struct/external)). Class-compliant UAC needs no driver, according to an Apple engineer ([Apple Developer Forums](https://developer.apple.com/forums/thread/133318)). AVFoundation does **not** expose UVC Processing Unit controls. Apps reach them by sending UVC control requests over IOKit [community] ([uvc-util](https://github.com/jtfrey/uvc-util), [webcam-control](https://github.com/romeo6770/webcam-control)).

## Differences xReveal must handle

1. **The set of Video formats differs per card *and* per USB link speed.**
   - On USB 2 the MS2130 loses uncompressed 1080p entirely.
   - The 4K X and GC553G2 have different format and FPS tables for 5 Gbps and 10 Gbps links.

   xReveal must enumerate formats live every time. It must not assume a format from the card model. It should warn when a USB 3 card is on a USB 2 link, which `bcdDevice` or a missing 1080p60 YUY2 mode reveals.
2. **Pixel format zoo.** Seen at 1080p60: YUY2 (packed 4:2:2), NV12/I420 (planar 4:2:0), RGB24, P010 (10-bit HDR) and MJPEG.
   - The low-latency auto-pick order is NV12 ≈ YUY2 < RGB24 < MJPEG [community] ([Veyra wiki](https://github.com/Likely7/Veyra-NRVideo/wiki/Guide-03-Capture-Card-Buying)). Uncompressed costs no decode [measurement] ([EposVox](https://obsproject.com/forum/resources/capture-card-documentation-latency-decode-modes-formats-more.777/)).
   - The renderer needs GPU conversion for at least NV12 and YUY2.
   - An MJPEG decode path is still needed for MS2109-class cards, for USB 2 links, and for high modes such as 4K60 and 1080p240 on the GC553G2 or MS2130S-class chips.
   - P010 must never be picked, because v1 is SDR only.
3. **Colour range and matrix are not reliably signalled** (see [UVC limits](#what-the-uvc-spec-does-and-does-not-signal)).
   - Measured MS2130 output is full-range with an odd matrix, and other sources assume limited.
   - Some cards can rescale range themselves (Elgato's Shrink/Expand/Bypass).
   - The PS5 has its own RGB range setting.

   So "color space + range" must be a real per-Source setting with a remembered value. "Auto" can only be a best guess: the descriptor if present, else a per-chip default.
4. **The "vendor settings dialog" exists only on Windows, and only through DirectShow.**
   - A DirectShow filter exposes `ISpecifyPropertyPages` and the app shows it with `OleCreatePropertyFrame` ([MS docs](https://learn.microsoft.com/en-us/windows/win32/directshow/displaying-a-filters-property-pages)). KsProxy-based capture filters, which includes UVC devices, provide property pages plus `IAMVideoProcAmp`, `IAMCameraControl` and `IAMStreamConfig` ([WDM Video Capture filter](https://learn.microsoft.com/en-us/windows/win32/directshow/wdm-video-capture-filter)).
   - Microsoft warns these pages "are meant for debugging" (same page) and calls DirectShow legacy.
   - Media Foundation has no OS dialog.
   - macOS has no dialog at all. UVC cards' real settings live in vendor apps (Elgato Studio/CDU, AVerMedia Gaming Utility) that xReveal cannot open.
5. **"Buffering" is an app-side setting, not a card control.** OBS's Buffering option belongs to OBS, and Elgato recommends turning it off ([PCIe OBS setup](https://help.elgato.com/hc/en-us/articles/360037206051-Elgato-internal-PCIe-Capture-Card-OBS-Studio-Setup)). In xReveal it means its own frame-queue depth.
6. **Audio is a separate device that must be paired with the video device.**
   - Its name can differ from the video device's name (AVerMedia's "Digital Audio Interface(…)").
   - On macOS it is a separate Audio Input device even for Elgato cards.
   - The MS2109 lies in its descriptors (96 kHz mono, really 48 kHz stereo with swapped L/R). Windows and macOS have no kernel quirk for this, so xReveal would have to reinterpret the samples itself [inference from the Linux quirk].
   - 5.1 audio exists on some cards but only through vendor software on Windows.
7. **Exclusive access.** Elgato's UVC cards lack multi-app support, so if OBS or Elgato Studio has the card open, xReveal cannot open it. xReveal needs a "Source busy" state.
8. **HDCP gives a black picture.** The PS5 enables HDCP by default, and capture cards then show black or no signal until it is turned off in Settings → System → HDMI ([Elgato HDCP](https://help.elgato.com/hc/en-us/articles/360040482032-HDCP-and-Elgato-Capture-devices)). xReveal should detect a sustained black or missing signal and point the user to that setting.
9. **HDR input.**
   - Some cards tonemap on the device: the Neo always, the HD60 X/S+ when the toggle is on, and the 4K X on macOS.
   - If a user turned tonemapping off in vendor software, the SDR picture looks washed out.
   - xReveal can only detect and explain this. It cannot fix it without the vendor HID API.
10. **Capture FPS is decoupled from the input FPS.** The MS2130 emits its configured rate regardless of the input rate. A PS5 at 120 Hz into a 60 fps capture mode gives dropped or duplicated frames. Frame pacing must follow the capture timestamps, not the input.
11. **OS permissions.** macOS needs Camera and Microphone permission for UVC/UAC capture ([Elgato](https://help.elgato.com/hc/en-us/articles/360034401051-Elgato-Game-Capture-HD60-S-What-Software-Can-It-Use)).

## Latency summary

Glass-to-glass or capture-path figures as published. No source tested on macOS.

| Card / mode | Latency | Source type |
|---|---|---|
| MS2130, YUY2 1080p60 | ~66 ms (1080p120: ~49 ms) | [measurement] [HyperHDR #499](https://github.com/awawa-dev/HyperHDR/discussions/499) |
| MS2130 cards, YUY2 1080p60 | 40–50 ms | [community] [Veyra wiki](https://github.com/Likely7/Veyra-NRVideo/wiki/Guide-03-Capture-Card-Buying) |
| MS2109, MJPEG | ~80–100 ms | [measurement] [HyperHDR #499](https://github.com/awawa-dev/HyperHDR/discussions/499) |
| Elgato HD60 X, NV12 720p60 / 1080p120 | ~60 ms / ~44 ms | [measurement] [HyperHDR review](https://www.hyperhdr.eu/2022/07/elgato-game-capture-hd60x-review.html) |
| Elgato 4K X | ~35 ms | [community] [Veyra wiki](https://github.com/Likely7/Veyra-NRVideo/wiki/Guide-03-Capture-Card-Buying) |
| Elgato 4K60 S+ (hardware-encoded path) | ~250 ms preview | primary [Elgato comparison](https://help.elgato.com/hc/en-us/articles/360041842611-Elgato-Capture-Devices-Device-comparison) |
| Cards in OBS (Windows, 2019) | spread of ~30–40 ms between fastest and slowest | [measurement] [EposVox](https://obsproject.com/forum/resources/capture-card-documentation-latency-decode-modes-formats-more.777/) |

Takeaway: the card's own pipeline dominates. An uncompressed USB 3 path is about 35–66 ms, and MJPEG decode adds only a few ms. Hardware-encoded paths are an order of magnitude worse. xReveal's own budget should be judged against these numbers.

## Open questions (facts still to verify)

- **Reference card chipset and model**: read the USB IDs (see above).
- **macOS native formats for the MS2130**: which `AVCaptureDevice.Format` media subtypes AVFoundation reports (`yuvs`/`2vuy` for YUY2? `dmb1` for MJPEG?), and whether it tags them video-range or full-range. Delivering a non-native pixel format costs a conversion. Verify on the Reference card.
- **Do Elgato and AVerMedia PCIe cards enumerate in Media Foundation at all**, or only in DirectShow? Microsoft documents MF only for UVC, and no source tested this. This decides whether Windows support for these cards needs a DirectShow backend.
- **MS2130 colour range**: measure on the Reference card with the PS5 set to Full and then Limited RGB range, because the community sources contradict each other.
- **No-signal and mode-change behaviour per family**: does the card keep sending frames (a black or "no signal" pattern) or stop, and does a PS5 resolution switch change the UVC format or get scaled inside the card? No primary source covers this, and it feeds the map's "Live signal changes" item.
- **Video format of the GC513 and other USB 2 cards over UVC** (MJPEG or H.264), and their real latency.

## New questions for later tickets

1. **Driver-based cards in v1: yes or no?** Supporting them means a DirectShow backend on Windows, and they cannot work on macOS. Recommendation: no for v1. Say "UVC/UAC capture cards" in the spec.
2. **Keep the "vendor settings dialog" in v1?** It only exists via DirectShow on Windows, Microsoft labels it for debugging, and it never exists on macOS. Alternative: xReveal exposes the standard UVC ProcAmp controls (brightness, contrast, saturation) itself on both OSes, over IOKit on macOS.
3. **Colour range and matrix policy**: what "Auto" means (descriptor, then per-chip table, then BT.709 limited?) and whether the Remembered setup stores an explicit override per Source.
4. **MJPEG in v1**: required for MS2109-class cards and USB 2 links. Which decoder: software (libjpeg-turbo or zune-jpeg) or hardware (VideoToolbox, MF MJPEG decoder)?
5. **Pairing a Source's audio with its video**: by USB container or location ID, not by name. This also feeds the "Remembered setup identity" item on the map.
6. **"Source busy" and "HDCP / no signal" user experience.**
