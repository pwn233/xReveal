//! The screens shown instead of (or over) the live picture (docs/spec/v1.md §2.8): an icon and a
//! short message in the centre of the window, with an action below where there is one.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::Palette;
use crate::video::signal::Signal;

/// What the window shows where the picture goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Screen {
    /// The live picture, nothing over it.
    Picture,
    /// A capture just opened; black until its first frame.
    Starting,
    /// Nothing to show: no card, the remembered card isn't plugged in, or it failed to open.
    NoCard { title: String, hint: &'static str },
    NoSignal { card: String },
    Unplugged { card: String },
    Busy { card: String },
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

    /// The screen when no Source is wanted, from the reason the pipeline gave.
    pub fn no_card(reason: &str) -> Screen {
        Screen::NoCard {
            title: reason.trim_end_matches('.').to_owned(),
            hint: "Plug in your capture card, then choose it in Settings",
        }
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

    /// Icon, title, hint, and whether it offers "Pick another card".
    fn content(&self) -> Option<(IconName, String, String, bool)> {
        Some(match self {
            Screen::Picture | Screen::Starting => return None,
            Screen::NoCard { title, hint } => (IconName::Cable, title.clone(), (*hint).to_owned(), true),
            Screen::NoSignal { card } => (
                IconName::MonitorOff,
                format!("No signal from {card}"),
                "Check the console is on and the HDMI cable. Black picture on a PS5? Turn off HDCP in its HDMI settings.".into(),
                false,
            ),
            Screen::Unplugged { card } => (IconName::Unplug, format!("{card} was unplugged"), "Waiting for it…".into(), true),
            Screen::Busy { card } => (
                IconName::Lock,
                format!("{card} is being used by another app"),
                "Close it (OBS, Discord, a browser…) to continue. xReveal tries again every 2 seconds.".into(),
                true,
            ),
        })
    }

    /// The centred message, or `None` for the picture. `pal` is a dark palette: these sit on black.
    pub fn render(&self, pal: Palette, on_pick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Option<AnyElement> {
        let (icon, title, hint, pick) = self.content()?;
        let action = pick.then(|| {
            div()
                .id("pick-card")
                .mt_2()
                .h(px(32.))
                .px_4()
                .rounded(px(8.))
                .flex()
                .items_center()
                .text_sm()
                .cursor_pointer()
                .bg(pal.raised)
                .border_1()
                .border_color(pal.line)
                .text_color(pal.text)
                .hover(|d| d.bg(pal.raised_hover))
                .child("Pick another card")
                .on_click(on_pick)
        });
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
                        .max_w(px(440.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_3()
                        .text_center()
                        .child(Icon::new(icon).size_8().text_color(pal.muted))
                        .child(div().text_lg().font_weight(FontWeight::MEDIUM).text_color(pal.text).child(title))
                        .child(div().text_sm().line_height(px(20.)).text_color(pal.secondary).child(hint))
                        .when_some(action, |d, a| d.child(a)),
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
        assert_eq!(Screen::no_card("USB2 Video isn't connected."), Screen::NoCard {
            title: "USB2 Video isn't connected".into(),
            hint: "Plug in your capture card, then choose it in Settings"
        });
    }
}
