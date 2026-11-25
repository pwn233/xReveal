//! Sources, their Video formats and monitors (docs/spec/v1.md §2.1, §2.4, §3.1): what can be
//! picked, what is picked by default, and how a remembered choice is recognised again. Pure: the
//! platforms list devices and formats, this decides.

/// How the Source sends a format's pixels. Never shown to the user; chosen automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PixelKind {
    /// Anything xReveal can't draw.
    Other,
    #[cfg_attr(all(target_os = "macos", not(test)), expect(dead_code, reason = "macOS never sees MJPEG: the OS decodes it"))]
    Mjpeg,
    Yuy2,
    Nv12,
}

/// One format the Source offers, exactly as the OS lists it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeFormat {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub pixel: PixelKind,
}

/// What the user picks: resolution + FPS (the **Video format**, minus the hidden pixel format).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

impl VideoFormat {
    fn same_rate(&self, fps: f64) -> bool {
        (self.fps - fps).abs() < 0.5
    }
}

impl std::fmt::Display for VideoFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{} @ {}", self.width, self.height, self.fps.round())
    }
}

/// The resolution + FPS choices a Source offers (formats xReveal can draw), largest first.
pub fn choices(formats: &[NativeFormat]) -> Vec<VideoFormat> {
    let mut out: Vec<VideoFormat> = Vec::new();
    for f in formats.iter().filter(|f| f.pixel != PixelKind::Other) {
        if !out.iter().any(|c| c.width == f.width && c.height == f.height && c.same_rate(f.fps)) {
            out.push(VideoFormat { width: f.width, height: f.height, fps: f.fps });
        }
    }
    out.sort_by(|a, b| (b.width * b.height).cmp(&(a.width * a.height)).then(b.fps.total_cmp(&a.fps)));
    out
}

/// The native format to open for `choice`: lowest latency first (NV12 > YUY2 > MJPEG).
pub fn best_native(formats: &[NativeFormat], choice: VideoFormat) -> Option<NativeFormat> {
    formats
        .iter()
        .filter(|f| f.width == choice.width && f.height == choice.height && choice.same_rate(f.fps) && f.pixel != PixelKind::Other)
        .max_by_key(|f| f.pixel)
        .copied()
}

/// The console's frame rate the default aims for.
const CONSOLE_FPS: f64 = 60.0;

/// The default Video format (docs/spec/v1.md §2.4): the highest frame rate that matches the
/// console (60), then the highest resolution at that rate. Among those, exact 16:9 modes come
/// first: consoles send 16:9 TV modes, and cards also list PC modes such as 1360×768.
pub fn default_choice(formats: &[NativeFormat]) -> Option<VideoFormat> {
    let all = choices(formats);
    let rate = all.iter().map(|c| c.fps).filter(|fps| *fps <= CONSOLE_FPS + 0.5).max_by(f64::total_cmp)?;
    let at_rate = all.iter().filter(|c| c.same_rate(rate));
    let widescreen = |c: &&VideoFormat| c.width * 9 == c.height * 16;
    at_rate.clone().filter(widescreen).chain(at_rate).next().copied()
}

/// How a Source is identified: the OS's own ID (depends on the USB port for cards without a
/// serial), the card's serial number if it has one, and its model (USB vendor + product ID).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct SourceIdentity {
    pub os_id: String,
    pub serial: Option<String>,
    pub model: Option<(u16, u16)>,
    pub name: String,
}

/// A capture device and the formats it offers right now.
#[derive(Clone, Debug)]
pub struct SourceInfo {
    pub identity: SourceIdentity,
    /// A separate capture device (a card) rather than the computer's own camera.
    pub external: bool,
    pub formats: Vec<NativeFormat>,
}

/// What to show when there is no Remembered setup yet: the first card (or any device), at its
/// default Video format. Returns the Source's index, the choice and the native format to open.
/// Replaced by the Welcome screen and the Remembered setup in their own slices.
pub fn initial_pick(sources: &[SourceInfo]) -> Option<(usize, VideoFormat, NativeFormat)> {
    let index = sources.iter().position(|s| s.external).or((!sources.is_empty()).then_some(0))?;
    let choice = default_choice(&sources[index].formats)?;
    let native = best_native(&sources[index].formats, choice)?;
    Some((index, choice, native))
}

/// What to show at launch.
#[derive(Debug, PartialEq)]
pub enum LaunchPick {
    /// Show this Source (index) at this Video format.
    Show(usize, VideoFormat),
    /// The remembered Source isn't plugged in: the Card-not-found picker waits for a click
    /// (docs/spec/v1.md §2.1). Holds its name.
    NotConnected(String),
    /// Nothing to show.
    Nothing,
}

/// Decides what to show at launch from the Remembered setup's Source and Video format: that
/// Source if it's recognised (at the remembered format, or its default if that format is gone);
/// nothing if it isn't plugged in; with no Remembered setup, [`initial_pick`].
pub fn launch_pick(sources: &[SourceInfo], remembered: Option<(&SourceIdentity, Option<VideoFormat>)>) -> LaunchPick {
    let Some((identity, format)) = remembered else {
        return initial_pick(sources).map_or(LaunchPick::Nothing, |(i, choice, _)| LaunchPick::Show(i, choice));
    };
    let identities: Vec<SourceIdentity> = sources.iter().map(|s| s.identity.clone()).collect();
    match match_source(identity, &identities) {
        SourceMatch::Found(i) => {
            let formats = &sources[i].formats;
            let choice = format.filter(|f| best_native(formats, *f).is_some()).or_else(|| default_choice(formats));
            choice.map_or(LaunchPick::Nothing, |c| LaunchPick::Show(i, c))
        }
        SourceMatch::Ask => LaunchPick::NotConnected(identity.name.clone()),
    }
}

/// Result of recognising the remembered Source among those plugged in.
#[derive(Debug, PartialEq, Eq)]
pub enum SourceMatch {
    /// Index into the available Sources.
    Found(usize),
    /// Show the Card-not-found picker.
    Ask,
}

/// Recognises the remembered Source (docs/spec/v1.md §2.1): the same OS ID, then the same serial
/// on any port, then the same model if exactly one such card is plugged in; otherwise ask.
/// Two identical cards without serials: the one on the remembered port (the OS ID) or ask.
pub fn match_source(remembered: &SourceIdentity, available: &[SourceIdentity]) -> SourceMatch {
    if let Some(i) = available.iter().position(|s| s.os_id == remembered.os_id) {
        return SourceMatch::Found(i);
    }
    if let Some(serial) = &remembered.serial
        && let Some(i) = available.iter().position(|s| s.serial.as_ref() == Some(serial))
    {
        return SourceMatch::Found(i);
    }
    if remembered.model.is_some() {
        let same_model: Vec<usize> = (0..available.len()).filter(|&i| available[i].model == remembered.model).collect();
        if let [only] = same_model[..] {
            return SourceMatch::Found(only);
        }
    }
    SourceMatch::Ask
}

/// Picks the monitor to go fullscreen on: the remembered one (recognised by the display's own
/// identity, not its port or position), else the primary one (index 0 in `available`).
pub fn choose_monitor(remembered: Option<&str>, available: &[String]) -> usize {
    remembered.and_then(|id| available.iter().position(|m| m == id)).unwrap_or(0)
}

/// USB vendor + product ID from a macOS `AVCaptureDevice.modelID` such as
/// `"UVC Camera VendorID_13407 ProductID_8497"` (decimal).
#[cfg_attr(all(target_os = "windows", not(test)), expect(dead_code, reason = "macOS device strings only"))]
pub fn model_from_mac_model_id(model_id: &str) -> Option<(u16, u16)> {
    let number = |key: &str| -> Option<u16> {
        let rest = &model_id[model_id.find(key)? + key.len()..];
        rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
    };
    Some((number("VendorID_")?, number("ProductID_")?))
}

/// The USB serial from a macOS card's *audio* device ID, such as
/// `"AppleUSBAudioEngine:MACROSILICON:USB2 Video:93869598:4,3"` (macOS doesn't expose the serial
/// on the video device; the card's audio carries it, with the video product's name — #25).
#[cfg_attr(all(target_os = "windows", not(test)), expect(dead_code, reason = "macOS device strings only"))]
pub fn serial_from_mac_audio_id(audio_id: &str, video_name: &str) -> Option<String> {
    let parts: Vec<&str> = audio_id.split(':').collect();
    let at = parts.iter().position(|p| *p == video_name)?;
    let serial = parts.get(at + 1)?;
    (!serial.is_empty() && serial.chars().all(|c| c.is_ascii_alphanumeric())).then(|| serial.to_string())
}

/// USB vendor + product ID and serial from a Windows device symbolic link such as
/// `\\?\usb#vid_345f&pid_2131&mi_00#6&2b2c1a4&0&0000#{…}`. The instance part is a serial only when
/// the device reports one; Windows otherwise makes one up from the port (it contains `&`).
#[cfg_attr(all(target_os = "macos", not(test)), expect(dead_code, reason = "Windows device links only"))]
pub fn identity_from_windows_link(link: &str) -> (Option<(u16, u16)>, Option<String>) {
    let lower = link.to_ascii_lowercase();
    let hex = |key: &str| -> Option<u16> {
        let rest = &lower[lower.find(key)? + key.len()..];
        u16::from_str_radix(rest.get(..4)?, 16).ok()
    };
    let model = hex("vid_").zip(hex("pid_"));
    let parts: Vec<&str> = link.split('#').collect();
    let serial = parts
        .get(2)
        .filter(|p| !p.contains('&') && !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|p| p.to_string());
    (model, serial)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Reference card's formats on macOS (#3): NV12 (`420v`) and YUY2 (`yuvs`).
    fn reference_card() -> Vec<NativeFormat> {
        let mut f = Vec::new();
        let mut add = |w, h, rates: &[f64], pixel| {
            for &fps in rates {
                f.push(NativeFormat { width: w, height: h, fps, pixel });
            }
        };
        for (w, h) in [(640, 480), (720, 480)] {
            add(w, h, &[60.0, 50.0, 30.0, 20.0, 10.0], PixelKind::Nv12);
            add(w, h, &[60.0, 30.0, 20.0, 10.0], PixelKind::Yuy2);
        }
        for (w, h) in [(1024, 768), (1280, 720), (1280, 1024), (1360, 768)] {
            add(w, h, &[60.0, 50.0, 30.0, 20.0, 10.0], PixelKind::Nv12);
            add(w, h, &[20.0, 10.0], PixelKind::Yuy2);
        }
        for (w, h) in [(1600, 1200), (1920, 1080), (1920, 1200)] {
            add(w, h, &[50.0, 30.0, 25.0, 20.0, 10.0], PixelKind::Nv12);
            add(w, h, &[10.0, 5.0], PixelKind::Yuy2);
        }
        for (w, h) in [(2560, 1440), (2560, 1600)] {
            add(w, h, &[30.0, 25.0, 20.0, 10.0], PixelKind::Nv12);
        }
        f
    }

    #[test]
    fn reference_card_defaults_to_720p60() {
        let choice = default_choice(&reference_card()).unwrap();
        assert_eq!((choice.width, choice.height), (1280, 720), "not the 1360x768 PC mode");
        assert_eq!(choice.fps, 60.0);
    }

    #[test]
    fn default_falls_back_to_the_best_rate_below_60() {
        let formats = [
            NativeFormat { width: 1920, height: 1080, fps: 50.0, pixel: PixelKind::Nv12 },
            NativeFormat { width: 1280, height: 720, fps: 30.0, pixel: PixelKind::Nv12 },
            NativeFormat { width: 1920, height: 1080, fps: 120.0, pixel: PixelKind::Mjpeg },
        ];
        let choice = default_choice(&formats).unwrap();
        assert_eq!((choice.width, choice.height, choice.fps), (1920, 1080, 50.0), "120 fps is above the console's 60");
    }

    #[test]
    fn non_widescreen_used_when_nothing_else_at_that_rate() {
        let formats = [NativeFormat { width: 1360, height: 768, fps: 60.0, pixel: PixelKind::Nv12 }];
        assert_eq!(default_choice(&formats).unwrap().width, 1360);
        assert_eq!(default_choice(&[]), None);
    }

    #[test]
    fn choices_hide_pixel_formats_and_undrawable_ones() {
        let formats = [
            NativeFormat { width: 1280, height: 720, fps: 60.0, pixel: PixelKind::Mjpeg },
            NativeFormat { width: 1280, height: 720, fps: 59.94, pixel: PixelKind::Nv12 },
            NativeFormat { width: 1920, height: 1080, fps: 30.0, pixel: PixelKind::Other },
        ];
        let all = choices(&formats);
        assert_eq!(all.len(), 1, "one 720p60 entry, no undrawable 1080p30: {all:?}");
        assert_eq!(all[0].to_string(), "1280x720 @ 60");
    }

    #[test]
    fn lowest_latency_pixel_format_wins() {
        let formats = [
            NativeFormat { width: 1920, height: 1080, fps: 60.0, pixel: PixelKind::Mjpeg },
            NativeFormat { width: 1920, height: 1080, fps: 60.0, pixel: PixelKind::Yuy2 },
            NativeFormat { width: 1920, height: 1080, fps: 60.0, pixel: PixelKind::Nv12 },
        ];
        let choice = VideoFormat { width: 1920, height: 1080, fps: 60.0 };
        assert_eq!(best_native(&formats, choice).unwrap().pixel, PixelKind::Nv12);
        assert_eq!(best_native(&formats[..2], choice).unwrap().pixel, PixelKind::Yuy2);
        assert_eq!(best_native(&formats[..1], choice).unwrap().pixel, PixelKind::Mjpeg);
        assert_eq!(best_native(&formats, VideoFormat { fps: 30.0, ..choice }), None);
    }

    fn card(os_id: &str, serial: Option<&str>, model: Option<(u16, u16)>) -> SourceIdentity {
        SourceIdentity { os_id: os_id.into(), serial: serial.map(Into::into), model, name: "USB2 Video".into() }
    }

    #[test]
    fn source_matching_order() {
        let onten = (0x345f, 0x2131);
        let remembered = card("port-A", Some("93869598"), Some(onten));
        // Same port.
        assert_eq!(match_source(&remembered, &[card("x", None, None), card("port-A", None, None)]), SourceMatch::Found(1));
        // Moved to another port: found by serial.
        assert_eq!(match_source(&remembered, &[card("port-B", Some("93869598"), Some(onten))]), SourceMatch::Found(0));
        // No serial match, but exactly one card of that model.
        let no_serial = card("port-A", None, Some(onten));
        assert_eq!(match_source(&no_serial, &[card("port-C", None, Some(onten))]), SourceMatch::Found(0));
        // Two identical cards without serials, neither on the remembered port: ask, never guess.
        assert_eq!(match_source(&no_serial, &[card("port-C", None, Some(onten)), card("port-D", None, Some(onten))]), SourceMatch::Ask);
        // Twins, one on the remembered port: that one.
        assert_eq!(match_source(&no_serial, &[card("port-C", None, Some(onten)), card("port-A", None, Some(onten))]), SourceMatch::Found(1));
        // Nothing like it.
        assert_eq!(match_source(&remembered, &[card("other", Some("1"), Some((1, 2)))]), SourceMatch::Ask);
    }

    #[test]
    fn initial_pick_prefers_a_card_at_its_default() {
        let camera = SourceInfo {
            identity: card("facetime", None, None),
            external: false,
            formats: vec![NativeFormat { width: 1920, height: 1080, fps: 30.0, pixel: PixelKind::Nv12 }],
        };
        let onten = SourceInfo { identity: card("onten", None, Some((0x345f, 0x2131))), external: true, formats: reference_card() };
        let (index, choice, native) = initial_pick(&[camera.clone(), onten]).unwrap();
        assert_eq!(index, 1);
        assert_eq!(choice.to_string(), "1280x720 @ 60");
        assert_eq!(native.pixel, PixelKind::Nv12);
        assert_eq!(initial_pick(&[camera]).unwrap().0, 0, "no card: the camera is still offered");
        assert_eq!(initial_pick(&[]), None);
    }

    #[test]
    fn launch_pick_uses_the_remembered_setup() {
        let onten = SourceInfo { identity: card("port-A", Some("93869598"), Some((0x345f, 0x2131))), external: true, formats: reference_card() };
        let sources = [onten.clone()];
        let p1080 = VideoFormat { width: 1920, height: 1080, fps: 50.0 };
        // Remembered card and format.
        assert_eq!(launch_pick(&sources, Some((&onten.identity, Some(p1080)))), LaunchPick::Show(0, p1080));
        // Remembered format no longer offered: the card's default.
        let gone = VideoFormat { width: 3840, height: 2160, fps: 60.0 };
        let LaunchPick::Show(0, choice) = launch_pick(&sources, Some((&onten.identity, Some(gone)))) else { panic!() };
        assert_eq!(choice.to_string(), "1280x720 @ 60");
        // Remembered card unplugged, another device present: don't start it, wait for a click.
        let camera = SourceInfo { identity: card("facetime", None, None), external: false, formats: vec![] };
        assert_eq!(launch_pick(&[camera], Some((&onten.identity, Some(p1080)))), LaunchPick::NotConnected("USB2 Video".into()));
        // No Remembered setup: the initial pick.
        assert_eq!(launch_pick(&sources, None), LaunchPick::Show(0, choice));
        assert_eq!(launch_pick(&[], None), LaunchPick::Nothing);
    }

    #[test]
    fn monitor_choice_falls_back_to_primary() {
        let monitors = ["primary-uuid".to_string(), "ultrawide-uuid".to_string()];
        assert_eq!(choose_monitor(Some("ultrawide-uuid"), &monitors), 1);
        assert_eq!(choose_monitor(Some("unplugged-uuid"), &monitors), 0);
        assert_eq!(choose_monitor(None, &monitors), 0);
    }

    #[test]
    fn parses_os_device_strings() {
        assert_eq!(model_from_mac_model_id("UVC Camera VendorID_13407 ProductID_8497"), Some((0x345f, 0x2131)));
        assert_eq!(model_from_mac_model_id("FaceTime HD Camera"), None);
        assert_eq!(
            serial_from_mac_audio_id("AppleUSBAudioEngine:MACROSILICON:USB2 Video:93869598:4,3", "USB2 Video"),
            Some("93869598".into())
        );
        assert_eq!(serial_from_mac_audio_id("BuiltInMicrophoneDevice", "USB2 Video"), None);
        let (model, serial) = identity_from_windows_link(r"\\?\usb#vid_345f&pid_2131&mi_00#6&2b2c1a4&0&0000#{e5323777-f976-4f5b-9b55-b94699c46e44}\global");
        assert_eq!(model, Some((0x345f, 0x2131)));
        assert_eq!(serial, None, "a port-derived instance ID isn't a serial");
        let (_, serial) = identity_from_windows_link(r"\\?\usb#vid_0fd9&pid_0066#A1B2C3D4E5#{e5323777-f976-4f5b-9b55-b94699c46e44}");
        assert_eq!(serial, Some("A1B2C3D4E5".into()));
    }
}
