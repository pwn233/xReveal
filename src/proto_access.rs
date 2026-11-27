//! PROTOTYPE, throwaway (#134, "What do the separate Camera and Microphone access boxes look
//! like?"). Three layouts for the base page's Camera and Microphone boxes, and matching locked
//! banners in the drawer, switchable from a floating bar at the bottom of the window.
//!
//! Run: `XREVEAL_PROTO_ACCESS=1` (e.g. `open -n --env XREVEAL_PROTO_ACCESS=1 target/dev-app/xReveal.app`).
//! Camera and Microphone access are faked in memory (the real permissions are never touched):
//! **Allow** grants, and the bar's Camera / Microphone pills cycle not asked → allowed → turned off.
//! This file and everything gated on `active()` lives only on the `prototype/access-boxes` branch.

use std::sync::Mutex;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::permission::{Access, Privacy};
use crate::theme::Palette;

pub const VARIANTS: [&str; 3] = ["A · Two cards", "B · Checklist", "C · One at a time"];

#[derive(Clone, Copy, Debug)]
pub struct State {
    pub variant: usize,
    pub camera: Access,
    pub microphone: Access,
    /// **Continue without sound** was pressed.
    pub skipped_sound: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Whether the prototype runs (`XREVEAL_PROTO_ACCESS` set).
pub fn active() -> bool {
    let mut state = STATE.lock().unwrap();
    if state.is_none() && std::env::var_os("XREVEAL_PROTO_ACCESS").is_some() {
        *state = Some(State { variant: 0, camera: Access::NotAsked, microphone: Access::NotAsked, skipped_sound: false });
    }
    state.is_some()
}

pub fn state() -> Option<State> {
    active().then(|| STATE.lock().unwrap().unwrap())
}

fn update(f: impl FnOnce(&mut State)) {
    if let Some(state) = STATE.lock().unwrap().as_mut() {
        f(state);
    }
}

pub fn access(privacy: Privacy) -> Option<Access> {
    state().map(|s| match privacy {
        Privacy::Camera => s.camera,
        Privacy::Microphone => s.microphone,
    })
}

/// A stub for macOS's prompt: the person says yes. The real prompt still comes once, so the
/// prototype's picture can actually start after it.
pub fn grant(privacy: Privacy) {
    #[cfg(target_os = "macos")]
    if crate::platform::macos::access::access(privacy) == Access::NotAsked {
        crate::platform::macos::access::request();
    }
    update(|s| match privacy {
        Privacy::Camera => s.camera = Access::Granted,
        Privacy::Microphone => s.microphone = Access::Granted,
    });
}

fn cycle(access: Access) -> Access {
    match access {
        Access::NotAsked => Access::Granted,
        Access::Granted => Access::Denied,
        Access::Denied => Access::NotAsked,
    }
}

/// The base page stays while the camera isn't allowed, or the microphone hasn't been decided.
pub fn base_page_needed() -> bool {
    state().is_some_and(|s| s.camera != Access::Granted || (s.microphone == Access::NotAsked && !s.skipped_sound))
}

fn status(access: Access) -> &'static str {
    match access {
        Access::NotAsked => "Not allowed yet",
        Access::Granted => "Allowed",
        Access::Denied => "Turned off",
    }
}

fn button_label(access: Access) -> &'static str {
    if access == Access::Denied { "Open System Settings" } else { "Allow access" }
}

/// What a box's button does: grant (the prompt's stub) or, once turned off, the settings page.
fn press(privacy: Privacy, access: Access) {
    if access == Access::Denied {
        crate::permission::open_settings(privacy);
    } else {
        grant(privacy);
    }
}

fn button(id: impl Into<ElementId>, label: &'static str, primary: bool, pal: Palette) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(30.))
        .px_3()
        .rounded(px(8.))
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .cursor_pointer()
        .when(primary, |d| d.bg(pal.accent).text_color(pal.deep).hover(|d| d.opacity(0.9)))
        .when(!primary, |d| d.bg(pal.raised).border_1().border_color(pal.line).text_color(pal.text).hover(|d| d.bg(pal.raised_hover)))
        .child(label)
}

fn link(id: &'static str, label: &'static str, pal: Palette) -> Stateful<Div> {
    div().id(id).text_sm().text_color(pal.secondary).cursor_pointer().hover(|d| d.text_color(pal.text)).child(label)
}

fn status_icon(access: Access, pal: Palette) -> Icon {
    match access {
        Access::Granted => Icon::new(IconName::CircleCheck).text_color(pal.accent),
        Access::Denied => Icon::new(IconName::CircleX).text_color(pal.warn),
        Access::NotAsked => Icon::new(IconName::Circle).text_color(pal.muted),
    }
}

struct Topic {
    privacy: Privacy,
    icon: IconName,
    name: &'static str,
    why: &'static str,
    required: bool,
}

const CAMERA: Topic = Topic { privacy: Privacy::Camera, icon: IconName::Camera, name: "Camera", why: "For the picture from your capture card.", required: true };
const MICROPHONE: Topic = Topic { privacy: Privacy::Microphone, icon: IconName::Mic, name: "Microphone", why: "For the card's sound. You can play without it.", required: false };

fn access_of(s: State, privacy: Privacy) -> Access {
    match privacy {
        Privacy::Camera => s.camera,
        Privacy::Microphone => s.microphone,
    }
}

/// The base page in the current variant (dark palette, on black).
pub fn base_page(pal: Palette) -> AnyElement {
    let s = state().unwrap();
    let page = match s.variant {
        0 => two_cards(s, pal),
        1 => checklist(s, pal),
        _ => one_at_a_time(s, pal),
    };
    div().absolute().inset_0().flex().items_center().justify_center().p_8().child(page).into_any_element()
}

fn skip_sound() -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    |_, window, _| {
        update(|s| s.skipped_sound = true);
        window.refresh();
    }
}

fn on_press(privacy: Privacy, access: Access) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, window, _| {
        press(privacy, access);
        window.refresh();
    }
}

// A: two cards side by side, each with its own status and button.
fn two_cards(s: State, pal: Palette) -> Div {
    let card = |t: Topic| {
        let access = access_of(s, t.privacy);
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
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(Icon::new(t.icon).size_6().text_color(pal.text))
                    .child(div().text_xs().text_color(pal.muted).child(if t.required { "Needed" } else { "Optional" })),
            )
            .child(div().text_base().font_weight(FontWeight::MEDIUM).text_color(pal.text).child(t.name))
            .child(div().text_sm().line_height(px(19.)).text_color(pal.secondary).child(t.why))
            .child(div().flex().items_center().gap_1().text_xs().text_color(pal.secondary).child(status_icon(access, pal).size_3()).child(status(access)))
            .when(access != Access::Granted, |d| {
                d.child(button(("a-allow", t.required as usize), button_label(access), t.required && access == Access::NotAsked, pal).on_click(on_press(t.privacy, access)))
            })
    };
    let sound_pending = s.camera == Access::Granted && s.microphone != Access::Granted;
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_4()
        .child(div().text_lg().font_weight(FontWeight::MEDIUM).text_color(pal.text).child("Let xReveal use your capture card"))
        .child(div().text_sm().text_color(pal.secondary).child("macOS asks once for each. You can change them later in System Settings."))
        .child(div().flex().gap_3().child(card(CAMERA)).child(card(MICROPHONE)))
        .when(sound_pending, |d| d.child(link("a-skip", "Continue without sound", pal).on_click(skip_sound())))
}

// B: one checklist, a row per kind of access, and one Continue at the end.
fn checklist(s: State, pal: Palette) -> Div {
    let row = |t: Topic| {
        let access = access_of(s, t.privacy);
        div()
            .flex()
            .items_center()
            .gap_3()
            .py_3()
            .border_b_1()
            .border_color(pal.line)
            .child(status_icon(access, pal).size_5())
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(div().flex().items_center().gap_2().text_sm().text_color(pal.text).child(t.name).child(div().text_xs().text_color(pal.muted).child(if t.required { "needed" } else { "optional" })))
                    .child(div().text_xs().text_color(pal.secondary).child(format!("{} · {}", t.why, status(access)))),
            )
            .when(access != Access::Granted, |d| d.child(button(("b-allow", t.required as usize), button_label(access), false, pal).on_click(on_press(t.privacy, access))))
    };
    let can_continue = s.camera == Access::Granted;
    let continue_label = if s.microphone == Access::Granted { "Continue" } else { "Continue without sound" };
    div()
        .w(px(440.))
        .flex()
        .flex_col()
        .child(div().flex().items_center().gap_2().mb_2().child(Icon::new(IconName::CameraOff).size_5().text_color(pal.muted)).child(div().text_lg().font_weight(FontWeight::MEDIUM).text_color(pal.text).child("Before the picture")))
        .child(div().border_t_1().border_color(pal.line))
        .child(row(CAMERA))
        .child(row(MICROPHONE))
        .child(
            div().mt_4().flex().justify_end().child(if can_continue {
                button("b-continue", continue_label, true, pal).on_click(skip_sound()).into_any_element()
            } else {
                div().text_xs().text_color(pal.muted).child("Allow the camera to continue").into_any_element()
            }),
        )
}

// C: one step at a time, Camera first, then Microphone with a skip.
fn one_at_a_time(s: State, pal: Palette) -> Div {
    let (t, step) = if s.camera != Access::Granted { (CAMERA, 1usize) } else { (MICROPHONE, 2usize) };
    let access = access_of(s, t.privacy);
    let dots = div().flex().gap_1().children((1..=2).map(|i| div().size(px(6.)).rounded_full().bg(if i == step { pal.accent } else { pal.faint })));
    let title = match (t.privacy, access) {
        (Privacy::Camera, Access::Denied) => "Camera access is off",
        (Privacy::Camera, _) => "Allow the camera",
        (Privacy::Microphone, Access::Denied) => "Microphone access is off",
        (Privacy::Microphone, _) => "Allow the microphone too?",
    };
    let hint = match (t.privacy, access) {
        (Privacy::Camera, Access::Denied) => "xReveal can't show the picture without it. Turn on xReveal under Privacy & Security › Camera.",
        (Privacy::Camera, _) => "xReveal needs it for the picture from your capture card. macOS will ask.",
        (Privacy::Microphone, Access::Denied) => "Without it there's no sound. Turn it on under Privacy & Security › Microphone, or play without sound.",
        (Privacy::Microphone, _) => "For the card's sound. You can play without it and allow it later in Settings.",
    };
    div()
        .w(px(400.))
        .flex()
        .flex_col()
        .items_center()
        .gap_3()
        .text_center()
        .child(dots)
        .child(Icon::new(t.icon).size_8().text_color(pal.muted))
        .child(div().text_lg().font_weight(FontWeight::MEDIUM).text_color(pal.text).child(title))
        .child(div().text_sm().line_height(px(20.)).text_color(pal.secondary).child(hint))
        .child(button(("c-allow", step), button_label(access), access == Access::NotAsked, pal).on_click(on_press(t.privacy, access)))
        .when(step == 2, |d| d.child(link("c-skip", "Continue without sound", pal).on_click(skip_sound())))
}

/// The floating prototype bar: variant arrows, and the faked access states (click to cycle).
pub fn bar() -> AnyElement {
    let s = state().unwrap();
    let pill = |id: &'static str, label: String| {
        div().id(id).px_2().py_1().rounded(px(6.)).bg(hsla(0., 0., 1., 0.12)).cursor_pointer().hover(|d| d.bg(hsla(0., 0., 1., 0.22))).child(label)
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
                .id("proto-bar")
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
                .child(pill("proto-camera", format!("Camera: {}", status(s.camera))).on_click(|_, w, _| {
                    update(|s| s.camera = cycle(s.camera));
                    w.refresh();
                }))
                .child(pill("proto-mic", format!("Microphone: {}", status(s.microphone))).on_click(|_, w, _| {
                    update(|s| {
                        s.microphone = cycle(s.microphone);
                        s.skipped_sound = false;
                    });
                    w.refresh();
                }))
                .child(pill("proto-reset", "Reset".into()).on_click(|_, w, _| {
                    update(|s| {
                        s.camera = Access::NotAsked;
                        s.microphone = Access::NotAsked;
                        s.skipped_sound = false;
                    });
                    w.refresh();
                })),
        )
        .into_any_element()
}
