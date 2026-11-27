//! PROTOTYPE, throwaway (#135, "How does What's new appear on the first launch after an update?").
//! Three ways to show What's new, switchable from a floating bar at the bottom of the window:
//! A a top bar that opens the notes in the drawer, B a card in a corner of the picture with the
//! notes in it, C the drawer opening by itself on a notes view.
//!
//! Run: `XREVEAL_PROTO_WHATSNEW=1` (e.g. `open -n --env XREVEAL_PROTO_WHATSNEW=1 target/dev-app/xReveal.app`).
//! "Just updated" is faked in memory; **Replay** shows it again. Lives only on the
//! `prototype/whats-new` branch.

use std::sync::Mutex;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::Palette;

pub const VARIANTS: [&str; 3] = ["A · Top bar → drawer", "B · Corner card", "C · Drawer opens itself"];

pub const VERSION: &str = "1.0.0";
pub const NOTES: [&str; 5] = [
    "Camera and Microphone are asked for separately, and you can play without sound.",
    "Swipe sideways with two fingers on the trackpad to open and close Settings.",
    "Uses about half the memory on Mac.",
    "Fullscreen on Mac goes straight to the display: the picture shows sooner.",
    "Your version, and any update, now show at the bottom of Settings.",
];

#[derive(Clone, Copy, Debug)]
pub struct State {
    pub variant: usize,
    /// Just updated, and What's new hasn't been seen yet.
    pub pending: bool,
    /// The drawer shows the notes instead of the settings.
    pub notes_open: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

pub fn active() -> bool {
    let mut state = STATE.lock().unwrap();
    if state.is_none() && std::env::var_os("XREVEAL_PROTO_WHATSNEW").is_some() {
        *state = Some(State { variant: 0, pending: true, notes_open: false });
    }
    state.is_some()
}

pub fn state() -> Option<State> {
    active().then(|| STATE.lock().unwrap().unwrap())
}

pub fn update(f: impl FnOnce(&mut State)) {
    if let Some(state) = STATE.lock().unwrap().as_mut() {
        f(state);
    }
}

pub fn notes_open() -> bool {
    state().is_some_and(|s| s.notes_open)
}

/// The notes as a list, in `pal`.
pub fn notes_list(pal: Palette) -> Div {
    div().flex().flex_col().gap_2().children(NOTES.iter().map(|n| {
        div()
            .flex()
            .gap_2()
            .child(div().mt(px(7.)).size(px(4.)).flex_none().rounded_full().bg(pal.accent))
            .child(div().flex_1().text_sm().line_height(px(20.)).text_color(pal.text).child(*n))
    }))
}

/// B: the card in the bottom-left corner of the picture, notes inside.
pub fn corner_card(pal: Palette) -> AnyElement {
    div()
        .absolute()
        .left(px(16.))
        .bottom(px(64.))
        .w(px(340.))
        .p_4()
        .rounded(px(12.))
        .bg(pal.panel)
        .border_1()
        .border_color(pal.line)
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_baseline()
                .justify_between()
                .child(div().text_base().font_weight(FontWeight::MEDIUM).text_color(pal.text).child(format!("What's new in {VERSION}")))
                .child(div().text_xs().text_color(pal.muted).child("Just updated")),
        )
        .child(notes_list(pal))
        .child(
            div().flex().justify_between().items_center().child(div().text_xs().text_color(pal.muted).child("Full notes on GitHub")).child(
                div()
                    .id("proto-card-ok")
                    .px_3()
                    .py_1()
                    .rounded(px(6.))
                    .bg(pal.accent)
                    .text_color(pal.deep)
                    .text_sm()
                    .cursor_pointer()
                    .child("Got it")
                    .on_click(|_, w, _| {
                        update(|s| s.pending = false);
                        w.refresh();
                    }),
            ),
        )
        .into_any_element()
}

/// The drawer's notes view (A after "See what's new", and C), in place of the settings.
pub fn drawer_notes(pal: Palette, on_back: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(
            div()
                .px_5()
                .pt_5()
                .pb_3()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_lg().font_weight(FontWeight::MEDIUM).text_color(pal.text).child(format!("What's new in {VERSION}")))
                .child(div().text_xs().text_color(pal.muted).child("xReveal was just updated · 28 Nov 2026")),
        )
        .child(div().flex_1().min_h_0().px_5().child(notes_list(pal)))
        .child(
            div()
                .p_4()
                .border_t_1()
                .border_color(pal.line)
                .flex()
                .justify_between()
                .items_center()
                .child(div().text_xs().text_color(pal.muted).child("Full notes on GitHub"))
                .child(
                    div()
                        .id("proto-notes-back")
                        .px_3()
                        .py_1()
                        .rounded(px(6.))
                        .bg(pal.raised)
                        .border_1()
                        .border_color(pal.line)
                        .text_sm()
                        .cursor_pointer()
                        .child("Back to settings")
                        .on_click(on_back),
                ),
        )
        .into_any_element()
}

/// The floating prototype bar.
pub fn bar() -> AnyElement {
    let s = state().unwrap();
    let pill = |id: &'static str, label: String| {
        div().id(id).px_2().py_1().rounded(px(6.)).bg(hsla(0., 0., 0., 0.1)).cursor_pointer().hover(|d| d.bg(hsla(0., 0., 0., 0.2))).child(label)
    };
    let n = VARIANTS.len();
    div()
        .absolute()
        .bottom(px(14.))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .rounded_full()
                .bg(hsla(50. / 360., 1., 0.5, 1.))
                .text_color(hsla(0., 0., 0., 1.))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .child("PROTOTYPE")
                .child(pill("proto-prev", "◀".into()).on_click(move |_, w, _| {
                    update(|s| s.variant = (s.variant + n - 1) % n);
                    w.refresh();
                }))
                .child(VARIANTS[s.variant])
                .child(pill("proto-next", "▶".into()).on_click(move |_, w, _| {
                    update(|s| s.variant = (s.variant + 1) % n);
                    w.refresh();
                }))
                .child(format!("What's new: {}", if s.pending { "not seen yet" } else { "seen" }))
                .child(pill("proto-replay", "Replay".into()).on_click(|_, w, _| {
                    update(|s| {
                        s.pending = true;
                        s.notes_open = false;
                    });
                    w.refresh();
                })),
        )
        .into_any_element()
}
