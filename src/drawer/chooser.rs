//! The drawer's selectors (Input, Video format, the sound Input and Output): xReveal's own, so a
//! long name wraps onto more lines instead of being cut off with "…" (#127, docs/spec/v1.md §2.3).
//! GPUI Kit's Select gives every row of its list the height of one measured row, so its rows
//! can't wrap.
//!
//! Choosing with the mouse goes through the same `Nav::Choice` pick as ←/→ (`nav.rs`), so the two
//! never disagree.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::*;

use super::SettingsDrawer;
use super::nav::Nav;
use crate::theme;

pub const CARD: usize = 0;
pub const FORMAT: usize = 1;
pub const SOUND_INPUT: usize = 2;
pub const OUTPUT: usize = 3;
pub const WINDOW: usize = 4;
pub const DENSITY: usize = 5;
pub const COUNT: usize = 6;

/// What one selector lists, and which entry is chosen.
#[derive(Default)]
pub struct Chooser {
    labels: Vec<String>,
    chosen: Option<usize>,
    /// Where the closed selector is (window coordinates), so the open list sits right under it
    /// at its width.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The window's height, to open the list upwards when there's no room below.
    window_height: Rc<Cell<Pixels>>,
}

impl Chooser {
    pub fn set(&mut self, labels: Vec<String>, chosen: Option<usize>) {
        self.labels = labels;
        self.chosen = chosen;
    }
}

impl SettingsDrawer {
    /// Selector `which`, whose ←/→ row is titled `title`, showing `placeholder` when nothing is
    /// chosen. Its list floats under it, over the settings below.
    pub(super) fn chooser(&self, which: usize, title: &'static str, placeholder: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let chooser = &self.choosers[which];
        self.chooser_of(which, title, placeholder, &chooser.labels, chooser.chosen, cx)
    }

    /// A selector whose entries come from the caller (Window, Density) rather than the model's
    /// lists.
    pub(super) fn chooser_of(&self, which: usize, title: &'static str, placeholder: &'static str, labels: &[String], chosen: Option<usize>, cx: &mut Context<Self>) -> AnyElement {
        let pal = theme::palette();
        let chooser = &self.choosers[which];
        let open = self.open_chooser == Some(which);
        let shown = chosen.and_then(|i| labels.get(i));
        let bounds = chooser.bounds.clone();
        let window_height = chooser.window_height.clone();
        let trigger = div()
            .id(("chooser", which))
            .relative()
            .px_3()
            .py(px(6.))
            .rounded(px(8.))
            .bg(pal.raised)
            .border_1()
            .border_color(if open { pal.accent_line } else { pal.line })
            .flex()
            // The chevron sits in the middle, also when a long name wraps onto more lines.
            .items_center()
            .gap_2()
            .cursor_pointer()
            .hover(|d| d.bg(pal.raised_hover))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .line_height(px(19.))
                    .text_color(if shown.is_some() { pal.text } else { pal.muted })
                    .child(shown.cloned().unwrap_or_else(|| placeholder.to_owned())),
            )
            .child(Icon::new(IconName::ChevronDown).size_4().flex_none().text_color(pal.muted))
            // Remember where it is, for the list.
            .child(
                canvas(
                    move |b, window, _| {
                        bounds.set(b);
                        window_height.set(window.viewport_size().height);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_chooser = if this.open_chooser == Some(which) { None } else { Some(which) };
                cx.notify();
            }));
        if !open {
            return trigger.into_any_element();
        }

        let rows = labels.iter().cloned().enumerate().map(|(i, label)| {
            let chosen = chosen == Some(i);
            div()
                .id(("chooser-option", which * 1000 + i))
                .px_2()
                .py(px(5.))
                .rounded(px(6.))
                .flex()
                .items_start()
                .gap_2()
                .cursor_pointer()
                .hover(|d| d.bg(pal.wash))
                // The label lines up with the selector's own text, and the tick with its chevron:
                // both 13 px in from either edge (border, padding).
                .child(div().flex_1().min_w_0().text_sm().line_height(px(19.)).text_color(pal.text).child(label))
                .child(div().w(px(16.)).flex_none().mt(px(2.5)).flex().justify_center().children(chosen.then(|| Icon::new(IconName::Check).size_3p5().text_color(pal.accent))))
                .on_click(cx.listener(move |this, _, window, cx| this.choose(which, title, i, window, cx)))
        });
        let list = div()
            .id(("chooser-list", which))
            .occlude()
            .w(chooser.bounds.get().size.width)
            .max_h(rems(18.))
            .overflow_y_scroll()
            .p_1()
            .flex()
            .flex_col()
            .gap(px(2.))
            .rounded(px(8.))
            .bg(pal.raised)
            .border_1()
            .border_color(pal.line)
            .shadow_lg()
            .children(rows)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.open_chooser = None;
                cx.notify();
            }));
        div()
            .flex()
            .flex_col()
            .child(trigger)
            .child(deferred(place_list(chooser, labels.len(), list)).with_priority(1))
            .into_any_element()
    }

    /// Chooses entry `i` of selector `which`, the same way ←/→ on its row would.
    fn choose(&mut self, which: usize, title: &'static str, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.open_chooser = None;
        self.choosers[which].chosen = Some(i);
        if let Some(row) = self.nav_rows().into_iter().find(|r| r.title == title)
            && let Nav::Choice { pick, .. } = row.nav
        {
            pick(self, i, window, cx);
        }
        cx.notify();
    }
}

/// Where an open list goes: right under its selector, or right above it when the window has no
/// room below. Placed from the selector's measured bounds: a deferred element has no place in the
/// flow, so left to itself the list was drawn over the selector.
fn place_list(chooser: &Chooser, rows: usize, list: impl IntoElement) -> Anchored {
    let selector = chooser.bounds.get();
    // The list's height before it's laid out: rows of about 31 px, at most 18 rem.
    let height = px((rows as f32 * 31. + 12.).min(18. * 16.));
    let room_below = chooser.window_height.get() - selector.bottom() - px(GAP + 8.);
    let above = height > room_below && selector.top() - height - px(GAP + 8.) > px(0.);
    let at = if above { (Anchor::BottomLeft, point(selector.left(), selector.top() - px(GAP))) } else { (Anchor::TopLeft, point(selector.left(), selector.bottom() + px(GAP))) };
    anchored().anchor(at.0).position(at.1).snap_to_window_with_margin(px(8.)).child(list)
}

/// Between a selector and its open list.
const GAP: f32 = 4.;
