//! The screens shown instead of (or over) the live picture (docs/spec/v1.md §2.1, §2.8): an icon
//! and a short message in the centre of the window, with actions below where there are any.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::permission::{Access, Privacy};
use crate::theme::Palette;
use crate::video::signal::Signal;

/// What the window shows where the picture goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Screen {
    /// The live picture, nothing over it.
    Picture,
    /// A capture just opened; black until its first frame.
    Starting,
    /// macOS: the first screen's Camera and Microphone cards (#143), until Camera is allowed and
    /// Microphone is allowed or skipped.
    Permission { camera: Access, microphone: Access },
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
    /// A card's button: ask for that access, or open its settings once turned off.
    pub allow: SettingsHandler,
    pub open_settings: SettingsHandler,
    /// **Continue without sound** (#143).
    pub skip_sound: Handler,
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
            // The last frame stays on the video surface; dim it (docs/spec/v1.md §2.8). On Windows
            // the surface hides while a screen shows, and the window is see-through under it, so
            // dimming would show the desktop behind xReveal: plain black there.
            Screen::Unplugged { .. } if !cfg!(target_os = "windows") => Some(Hsla::black().opacity(0.6)),
            _ => Some(Hsla::black()),
        }
    }

    /// Icon, title and hint.
    fn words(&self) -> Option<(IconName, String, String)> {
        Some(match self {
            Screen::Picture | Screen::Starting => return None,
            // Its own layout (see `access_cards`).
            Screen::Permission { .. } => return None,
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
        if let Screen::Permission { camera, microphone } = *self {
            return Some(access_cards(camera, microphone, pal, actions));
        }
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
            Screen::Blocked => {
                let open = actions.open_settings.clone();
                Some(button("open-settings", "Open Windows settings", false).on_click(move |_, w, cx| open(Privacy::Camera, w, cx)).into_any_element())
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

/// The first screen on macOS (#143, docs/spec/v1.md §2.1): a card each for Camera (needed for the
/// picture) and Microphone (optional), side by side, each with its status and its own button, and
/// **Continue without sound** once Camera is allowed.
fn access_cards(camera: Access, microphone: Access, pal: Palette, actions: &Actions) -> AnyElement {
    let card = |privacy: Privacy, access: Access| {
        let (icon, name, tag, why) = match privacy {
            Privacy::Camera => (IconName::Camera, "Camera", "Needed", "For the picture from your capture card."),
            Privacy::Microphone => (IconName::Mic, "Microphone", "Optional", "For the card's sound. You can play without it."),
        };
        let allow = actions.allow.clone();
        let primary = privacy == Privacy::Camera && access == Access::NotAsked;
        let granted = access == Access::Granted;
        // The button says where it stands: Allowed once given (and does nothing then).
        let label = match access {
            Access::Granted => "Allowed",
            Access::Denied => "Open System Settings",
            Access::NotAsked => "Allow access",
        };
        div()
            .w(px(220.))
            .p_4()
            .rounded(px(12.))
            .bg(pal.raised)
            .border_1()
            .border_color(if access == Access::Granted { pal.accent_line } else { pal.line })
            .flex()
            .flex_col()
            .gap_2()
            .child(div().flex().items_center().justify_between().child(Icon::new(icon).size_6().text_color(pal.text)).child(div().text_xs().text_color(pal.muted).child(tag)))
            .child(div().text_base().font_weight(FontWeight::MEDIUM).text_color(pal.text).child(name))
            .child(div().text_sm().line_height(px(19.)).text_color(pal.secondary).child(why))
            .child(
                div()
                    .id(match privacy {
                        Privacy::Camera => "allow-camera",
                        Privacy::Microphone => "allow-microphone",
                    })
                    .mt_1()
                    .h(px(30.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .text_sm()
                    .when(granted, |d| d.bg(pal.accent_fill).border_1().border_color(pal.accent_line).text_color(pal.accent).child(Icon::new(IconName::Check).size_4()))
                    .when(primary, |d| d.cursor_pointer().bg(pal.accent).text_color(pal.deep).hover(|d| d.opacity(0.9)))
                    .when(!primary && !granted, |d| d.cursor_pointer().bg(pal.raised).border_1().border_color(pal.line).text_color(pal.text).hover(|d| d.bg(pal.raised_hover)))
                    .child(label)
                    .when(!granted, |d| d.on_click(move |_, w, cx| allow(privacy, w, cx))),
            )
    };
    let skip = actions.skip_sound.clone();
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p_8()
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_4()
                .text_center()
                .child(div().text_lg().font_weight(FontWeight::MEDIUM).text_color(pal.text).child("Let xReveal use your capture card"))
                .child(div().text_sm().text_color(pal.secondary).child("macOS asks once for each. You can change them later in System Settings."))
                .child(div().flex().flex_wrap().justify_center().gap_3().child(card(Privacy::Camera, camera)).child(card(Privacy::Microphone, microphone)))
                .when(camera == Access::Granted && microphone != Access::Granted, |d| {
                    d.child(
                        div()
                            .id("skip-sound")
                            .text_sm()
                            .text_color(pal.secondary)
                            .cursor_pointer()
                            .hover(|d| d.text_color(pal.text))
                            .child("Continue without sound")
                            .on_click(move |_, w, cx| skip(w, cx)),
                    )
                }),
        )
        .into_any_element()
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
        // Windows hides the video surface under a screen, so there's no last frame to dim.
        let unplugged = if cfg!(target_os = "windows") { 1.0 } else { 0.6 };
        assert_eq!(Screen::Unplugged { card: "x".into() }.backdrop().map(|c| c.a), Some(unplugged));
        assert_eq!(Screen::NoSignal { card: "x".into() }.backdrop().map(|c| c.a), Some(1.0));
        assert_eq!(Screen::no_input(None).backdrop().map(|c| c.a), Some(1.0));
        assert_eq!(Screen::no_card("Couldn't open USB2 Video."), Screen::NoCard { title: "Couldn't open USB2 Video".into() });
    }
}
