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

/// What one selector lists, and which entry is chosen.
#[derive(Default)]
pub struct Chooser {
    labels: Vec<String>,
    chosen: Option<usize>,
    /// The closed selector's width, so the open list matches it.
    width: Rc<Cell<Pixels>>,
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
        let pal = theme::palette();
        let chooser = &self.choosers[which];
        let open = self.open_chooser == Some(which);
        let shown = chooser.chosen.and_then(|i| chooser.labels.get(i));
        let width = chooser.width.clone();
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
            .items_start()
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
            .child(Icon::new(IconName::ChevronDown).size_4().mt(px(1.5)).text_color(pal.muted))
            // Remember the width the list should take.
            .child(canvas(move |bounds, _, _| width.set(bounds.size.width), |_, _, _, _| {}).absolute().inset_0())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_chooser = if this.open_chooser == Some(which) { None } else { Some(which) };
                cx.notify();
            }));
        if !open {
            return trigger.into_any_element();
        }

        let rows = chooser.labels.iter().cloned().enumerate().map(|(i, label)| {
            let chosen = chooser.chosen == Some(i);
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
                .child(div().w(px(14.)).flex_none().mt(px(2.5)).children(chosen.then(|| Icon::new(IconName::Check).size_3p5().text_color(pal.accent))))
                .child(div().flex_1().min_w_0().text_sm().line_height(px(19.)).text_color(pal.text).child(label))
                .on_click(cx.listener(move |this, _, window, cx| this.choose(which, title, i, window, cx)))
        });
        let list = div()
            .id(("chooser-list", which))
            .occlude()
            .mt_1()
            .w(chooser.width.get())
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
            .child(deferred(anchored().snap_to_window_with_margin(px(8.)).child(list)).with_priority(1))
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
