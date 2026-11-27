//! The update button in the drawer's footer (#145, docs/spec/v1.md §2.10), next to the GitHub
//! mark. It's a **refresh** button while this version is the latest: hovering it says which
//! version this is and how the last check went, and pressing it asks GitHub again. When a newer
//! version is out it becomes a **download** button: hovering it shows that version's notes (they
//! scroll), and pressing it opens its download page.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{DrawerEvent, SettingsDrawer, lock};
use crate::theme;
use crate::update::Check;

const KEY: &str = "update-button";
/// The tip's widest: wide enough for a few lines of notes.
const TIP_WIDTH: f32 = 300.;
/// Past this the notes scroll.
const NOTES_HEIGHT: f32 = 180.;
/// How long the tip stays after the pointer leaves the button and the tip.
const CLOSE_AFTER: Duration = Duration::from_millis(300);

impl SettingsDrawer {
    pub(super) fn update_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = theme::palette();
        let offer = self.model.update.clone();
        let checking = self.model.update_check == Check::Checking;
        let (icon, colour) = match (&offer, checking) {
            (Some(_), _) => (IconName::Download, pal.accent),
            (None, true) => (IconName::LoaderCircle, pal.muted),
            (None, false) => (IconName::RefreshCw, pal.muted),
        };
        let bounds = self.lock_bounds.clone();
        let tip = self.update_tip_open.then(|| self.lock_bounds.borrow().get(KEY).copied()).flatten().map(|(anchor, window)| {
            lock::bubble(KEY, self.update_tip(cx), TIP_WIDTH, anchor, window, self.update_tip_closing)
        });
        let icon = Icon::new(icon).size_3p5().text_color(colour);
        // While checking, the icon breathes.
        let icon: AnyElement = if checking {
            icon.with_animation("update-checking", Animation::new(Duration::from_millis(900)).repeat().with_easing(pulsating_between(0.35, 1.)), |icon, t| icon.opacity(t)).into_any_element()
        } else {
            icon.into_any_element()
        };
        div()
            .id(KEY)
            .relative()
            .size(px(22.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|d| d.bg(pal.wash))
            .child(icon)
            .child(canvas(move |b, window, _| bounds.borrow_mut().insert(KEY, (b, window.viewport_size().width)), |_, _, _, _| {}).absolute().inset_0())
            .on_hover(cx.listener(|this, hovering: &bool, _, cx| {
                this.update_hover.0 = *hovering;
                this.update_hover_changed(cx);
            }))
            .on_click(cx.listener(move |this, _, _, cx| match &offer {
                Some((version, _)) => cx.open_url(&crate::update::release_page(version)),
                None if this.model.update_check != Check::Checking => cx.emit(DrawerEvent::CheckNow),
                None => {}
            }))
            .children(tip)
            .into_any_element()
    }

    /// Opens the tip while the pointer is on the button or the tip. Leaving both closes it a
    /// moment later, so the pointer can cross the gap from the button to the tip (and its link).
    fn update_hover_changed(&mut self, cx: &mut Context<Self>) {
        if self.update_hover.0 || self.update_hover.1 {
            self.update_tip_close = None;
            if !self.update_tip_open || self.update_tip_closing {
                self.update_tip_open = true;
                self.update_tip_closing = false;
                cx.notify();
            }
            return;
        }
        // A moment to cross to the tip, then a fade out like the other tips.
        self.update_tip_close = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CLOSE_AFTER).await;
            let still_away = this
                .update(cx, |this, cx| {
                    let away = !(this.update_hover.0 || this.update_hover.1);
                    if away {
                        this.update_tip_closing = true;
                        cx.notify();
                    }
                    away
                })
                .unwrap_or(false);
            if still_away {
                cx.background_executor().timer(lock::FADE_OUT).await;
                let _ = this.update(cx, |this, cx| {
                    if this.update_tip_closing {
                        this.update_tip_open = false;
                        this.update_tip_closing = false;
                        cx.notify();
                    }
                });
            }
        }));
    }

    /// What hovering the button shows: the newer version and its notes, or this version and how
    /// the last check went.
    fn update_tip(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = theme::palette();
        let current = env!("CARGO_PKG_VERSION");
        let panel = div()
            .id("update-tip")
            .w_full()
            .px_3()
            .py_2()
            .rounded(px(10.))
            .bg(pal.raised)
            .border_1()
            .border_color(pal.line)
            .shadow_lg()
            .occlude()
            .flex()
            .flex_col()
            .gap_2()
            .text_xs()
            .line_height(px(17.))
            .text_color(pal.text)
            // Kept open while the pointer is on it, so the notes can be scrolled.
            .on_hover(cx.listener(|this, hovering: &bool, _, cx| {
                this.update_hover.1 = *hovering;
                this.update_hover_changed(cx);
            }));
        let panel = match &self.model.update {
            Some((version, notes)) => {
                let bullets = notes.iter().map(|n| {
                    div()
                        .flex()
                        .gap_2()
                        .child(div().mt(px(6.)).size(px(4.)).flex_none().rounded_full().bg(pal.accent))
                        .child(div().flex_1().min_w_0().child(n.clone()))
                });
                panel
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(format!("xReveal {version} is out")))
                    .child(div().text_color(pal.secondary).child("Click to open its download page."))
                    .when(!notes.is_empty(), |d| {
                        // Long notes scroll, with a fade wherever there's more.
                        let list = div()
                            .id("update-notes")
                            .max_h(px(NOTES_HEIGHT))
                            .overflow_y_scroll()
                            .track_scroll(&self.update_notes_scroll)
                            .on_scroll(cx.listener(|_, _, _, cx| cx.notify()))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(bullets);
                        d.child(div().relative().child(list).children(crate::whats_new::scroll_fades(&self.update_notes_scroll, pal.raised)))
                    })
                    .child({
                        let page = crate::update::release_page(version);
                        div()
                            .id("update-release-notes")
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_color(pal.muted)
                            .cursor_pointer()
                            .hover(|d| d.text_color(pal.text))
                            .child("Release notes on GitHub")
                            .child(Icon::new(IconName::ExternalLink).size_3())
                            .on_click(move |_, _, cx| cx.open_url(&page))
                    })
            }
            None => {
                let status = match self.model.update_check {
                    Check::Checking => "Checking for a newer version…",
                    Check::Answered => "You have the latest version. Click to check again.",
                    Check::Failed => "Couldn't reach GitHub. Click to try again.",
                    Check::NotYet => "Click to check for a newer version.",
                };
                panel.child(div().text_sm().font_weight(FontWeight::MEDIUM).child(format!("xReveal {current}"))).child(div().text_color(pal.secondary).child(status))
            }
        };
        panel.into_any_element()
    }
}
