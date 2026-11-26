//! The screens shown instead of (or over) the live picture (docs/spec/v1.md §2.1, §2.8): an icon
//! and a short message in the centre of the window, with actions below where there are any.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::permission::Privacy;
use crate::theme::Palette;
use crate::video::signal::Signal;

/// What the window shows where the picture goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Screen {
    /// The live picture, nothing over it.
    Picture,
    /// A capture just opened; black until its first frame.
    Starting,
    /// macOS: camera access not given yet (`denied` once the person said no).
    Permission { denied: bool },
    /// Windows: the privacy switch blocks camera access for desktop apps.
    Blocked,
    /// No input on screen: none chosen yet, or the remembered one isn't plugged in. Settings
    /// (Tab) is where one is chosen; nothing starts by itself (docs/spec/v1.md §2.1).
    NoInput { title: String, hint: &'static str },
    /// The video pipeline couldn't start, or the input failed to open for another reason.
    NoCard { title: String },
    NoSignal { card: String },
    Unplugged { card: String },
    Busy { card: String },
}

pub type Handler = Rc<dyn Fn(&mut Window, &mut App)>;
pub type SettingsHandler = Rc<dyn Fn(Privacy, &mut Window, &mut App)>;

/// What the screens' buttons do.
#[derive(Clone)]
pub struct Actions {
    pub pick_another: Handler,
    pub allow: Handler,
    pub open_settings: SettingsHandler,
}

impl Screen {
    /// The screen for `signal` while showing (or trying to show) `card`.
    pub fn for_signal(signal: Signal, card: &str, has_frame: bool) -> Screen {
        let card = card.to_owned();
        match signal {
            Signal::Live => Screen::Picture,
            Signal::Starting if has_frame => Screen::Picture,
            Signal::Starting => Screen::Starting,
            Signal::NoSignal => Screen::NoSignal { card },
            Signal::Unplugged => Screen::Unplugged { card },
            Signal::Busy => Screen::Busy { card },
        }
    }

    /// The screen with nothing on it: no input chosen, or `missing` (the remembered one) unplugged.
    pub fn no_input(missing: Option<&str>) -> Screen {
        match missing {
            Some(name) => Screen::NoInput { title: format!("{name} isn't connected"), hint: "Plug it back in, or press Tab to choose another input" },
            None => Screen::NoInput { title: "No input selected".into(), hint: "Press Tab to open settings and choose your input" },
        }
    }

    /// The screen when the input couldn't open, from the reason given.
    pub fn no_card(reason: &str) -> Screen {
        Screen::NoCard { title: reason.trim_end_matches('.').to_owned() }
    }

    /// Whether the live picture shows (so the video surface stays visible).
    pub fn shows_picture(&self) -> bool {
        matches!(self, Screen::Picture)
    }

    /// What covers the picture area: black, the last frame dimmed, or nothing.
    pub fn backdrop(&self) -> Option<Hsla> {
        match self {
            Screen::Picture => None,
            // The last frame stays on the video surface; dim it (docs/spec/v1.md §2.8).
            Screen::Unplugged { .. } => Some(Hsla::black().opacity(0.6)),
            _ => Some(Hsla::black()),
        }
    }

    /// Icon, title and hint.
    fn words(&self) -> Option<(IconName, String, String)> {
        Some(match self {
            Screen::Picture | Screen::Starting => return None,
            Screen::Permission { denied: false } => (
                IconName::CameraOff,
                "Allow camera and microphone".into(),
                "xReveal needs Camera access for the picture and Microphone access for the sound. macOS will ask twice.".into(),
            ),
            Screen::Permission { denied: true } => (
                IconName::CameraOff,
                "Camera access is off".into(),
                "xReveal can't show the picture without it. Turn on xReveal under Privacy & Security › Camera, then come back here.".into(),
            ),
            Screen::Blocked => (
                IconName::CameraOff,
                "Windows is blocking camera access for desktop apps".into(),
                "Turn on \"Let desktop apps access your camera\" in Windows' privacy settings, then come back here.".into(),
            ),
            Screen::NoInput { title, hint } => (IconName::Cable, title.clone(), (*hint).to_owned()),
            Screen::NoCard { title } => (IconName::Cable, title.clone(), "Plug in your input device, then choose it in Settings".into()),
            Screen::NoSignal { card } => (
                IconName::MonitorOff,
                format!("No signal from {card}"),
                "Check the console is on and the HDMI cable. Black picture on a PS5? Turn off HDCP in its HDMI settings.".into(),
            ),
            Screen::Unplugged { card } => (IconName::Unplug, format!("{card} was unplugged"), "Waiting for it…".into()),
            Screen::Busy { card } => (
                IconName::Lock,
                format!("{card} is being used by another app"),
                "Close it (OBS, Discord, a browser…) to continue. xReveal tries again every 2 seconds.".into(),
            ),
        })
    }

    /// The centred message, or `None` for the picture. `pal` is a dark palette: these sit on black.
    pub fn render(&self, pal: Palette, actions: &Actions) -> Option<AnyElement> {
        let (icon, title, hint) = self.words()?;
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .mt_2()
                .h(px(32.))
                .px_4()
                .rounded(px(8.))
                .flex()
                .items_center()
                .text_sm()
                .cursor_pointer()
                .when(primary, |d| d.bg(pal.accent).text_color(pal.deep).hover(|d| d.opacity(0.9)))
                .when(!primary, |d| d.bg(pal.raised).border_1().border_color(pal.line).text_color(pal.text).hover(|d| d.bg(pal.raised_hover)))
                .child(label)
        };
        let below: Option<AnyElement> = match self {
            Screen::Permission { denied: false } => {
                let allow = actions.allow.clone();
                Some(button("allow-access", "Allow access", true).on_click(move |_, w, cx| allow(w, cx)).into_any_element())
            }
            Screen::Permission { denied: true } | Screen::Blocked => {
                let open = actions.open_settings.clone();
                let label = if matches!(self, Screen::Blocked) { "Open Windows settings" } else { "Open System Settings" };
                Some(button("open-settings", label, false).on_click(move |_, w, cx| open(Privacy::Camera, w, cx)).into_any_element())
            }
            Screen::NoCard { .. } | Screen::Unplugged { .. } | Screen::Busy { .. } => {
                let pick = actions.pick_another.clone();
                Some(button("pick-input", "Pick another input", false).on_click(move |_, w, cx| pick(w, cx)).into_any_element())
            }
            _ => None,
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .p_8()
                .child(
                    div()
                        .w_full()
                        .max_w(px(440.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_3()
                        .text_center()
                        .child(Icon::new(icon).size_8().text_color(pal.muted))
                        .child(div().text_lg().font_weight(FontWeight::MEDIUM).text_color(pal.text).child(title))
                        .child(div().text_sm().line_height(px(20.)).text_color(pal.secondary).child(hint))
                        .children(below),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude has its own `test` attribute that would replace std's.
    use super::{Screen, Signal};

    #[test]
    fn each_signal_has_its_screen() {
        assert_eq!(Screen::for_signal(Signal::Live, "USB2 Video", true), Screen::Picture);
        assert_eq!(Screen::for_signal(Signal::Starting, "USB2 Video", false), Screen::Starting, "black until the first frame");
        assert_eq!(Screen::for_signal(Signal::Starting, "USB2 Video", true), Screen::Picture);
        assert_eq!(Screen::for_signal(Signal::NoSignal, "USB2 Video", true), Screen::NoSignal { card: "USB2 Video".into() });
        assert_eq!(Screen::for_signal(Signal::Unplugged, "USB2 Video", true), Screen::Unplugged { card: "USB2 Video".into() });
        assert_eq!(Screen::for_signal(Signal::Busy, "USB2 Video", false), Screen::Busy { card: "USB2 Video".into() });
    }

    #[test]
    fn unplugged_dims_the_last_frame_and_the_rest_are_black() {
        assert!(Screen::Picture.backdrop().is_none());
        assert_eq!(Screen::Unplugged { card: "x".into() }.backdrop().map(|c| c.a), Some(0.6));
        assert_eq!(Screen::NoSignal { card: "x".into() }.backdrop().map(|c| c.a), Some(1.0));
        assert_eq!(Screen::no_input(None).backdrop().map(|c| c.a), Some(1.0));
        assert_eq!(Screen::no_card("Couldn't open USB2 Video."), Screen::NoCard { title: "Couldn't open USB2 Video".into() });
    }
}
