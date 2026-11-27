//! Settings that need Camera or Microphone access stay locked until it's given (#146,
//! docs/spec/v1.md §2.3). The control is greyed out and takes no clicks; its title and hint stay
//! as they are, with a warning icon after the title. Hovering the icon says why, in a small tip
//! centred above it with an arrow pointing at it, that fades in after a short pause.
//!
//! The tip is xReveal's own: GPUI Kit's animated tooltip lives in its `Root` view, which xReveal
//! doesn't use, and GPUI's plain one appears at once at the pointer.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::*;

use super::SettingsDrawer;
use crate::theme;

/// Where each lock warning is, and the window's width, as last drawn.
pub(super) type Spots = Rc<RefCell<HashMap<&'static str, (Bounds<Pixels>, Pixels)>>>;

/// How long the pointer rests before the tip starts to show, and how long it takes to fade in.
const PAUSE: f32 = 0.3;
const FADE: f32 = 0.15;
/// The tip's widest; it's centred on the icon within this.
const TIP_WIDTH: f32 = 240.;
/// How close the tip may come to the window's edge.
const MARGIN: f32 = 8.;

/// `control`, greyed out and taking no clicks when there's a reason it's locked (else unchanged).
pub(super) fn locked(key: &'static str, why: Option<&'static str>, control: AnyElement) -> AnyElement {
    if why.is_none() {
        return control;
    }
    div()
        .relative()
        .child(div().opacity(0.4).child(control))
        // Takes the clicks, so nothing under it reacts.
        .child(div().id(key).absolute().inset_0().occlude().cursor(CursorStyle::OperationNotAllowed))
        .into_any_element()
}

impl SettingsDrawer {
    /// The warning icon after a locked setting's title, whose tip says `why`; nothing when it
    /// isn't locked. `key` tells the icons apart.
    pub(super) fn lock_badge(&self, key: &'static str, why: Option<&'static str>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let why = why?;
        let pal = theme::palette();
        let bounds = self.lock_bounds.clone();
        let hovered = self.hovered_lock == Some(key);
        let tip = hovered.then(|| self.lock_bounds.borrow().get(key).copied()).flatten().map(|(b, window)| tip(key, why, b, window));
        Some(
            div()
                .id(key)
                .relative()
                .flex()
                .cursor_pointer()
                .child(Icon::new(IconName::TriangleAlert).size_3p5().text_color(pal.warn))
                .child(canvas(move |b, window, _| bounds.borrow_mut().insert(key, (b, window.viewport_size().width)), |_, _, _, _| {}).absolute().inset_0())
                .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                    let now = if *hovering { Some(key) } else { this.hovered_lock.filter(|k| *k != key) };
                    if now != this.hovered_lock {
                        this.hovered_lock = now;
                        cx.notify();
                    }
                }))
                .children(tip)
                .into_any_element(),
        )
    }
}

/// The tip above the icon at `icon` (window coordinates): a small panel centred on it where the
/// window (`window` wide) leaves room, with its arrow always pointing at the icon.
fn tip(key: &'static str, why: &'static str, icon: Bounds<Pixels>, window: Pixels) -> AnyElement {
    let pal = theme::palette();
    let panel = div()
        .max_w(px(TIP_WIDTH))
        .px_3()
        .py(px(6.))
        .rounded(px(8.))
        .bg(pal.raised)
        .border_1()
        .border_color(pal.line)
        .shadow_md()
        .text_xs()
        .line_height(px(17.))
        .text_color(pal.text)
        .text_center()
        .child(why);
    // Centred on the icon, then kept inside the window; the arrow follows the icon.
    let centre = icon.origin.x + icon.size.width / 2.;
    let left = (centre - px(TIP_WIDTH / 2.)).clamp(px(MARGIN), (window - px(TIP_WIDTH + MARGIN)).max(px(MARGIN)));
    let arrow = svg().path("xreveal/tooltip/arrow-down.svg").w(px(16.)).h(px(8.)).mt(px(-1.)).ml(centre - left - px(8.)).text_color(pal.raised);
    let total = PAUSE + FADE;
    let bubble = div()
        .w(px(TIP_WIDTH))
        .flex()
        .flex_col()
        .child(div().flex().justify_center().child(panel))
        .child(arrow)
        .with_animation(
            ElementId::Name(format!("lock-tip-{key}").into()),
            Animation::new(Duration::from_secs_f32(total)).with_easing(move |t| {
                // Nothing during the pause, then ease out over the fade.
                let x = ((t * total - PAUSE) / FADE).clamp(0.0, 1.0);
                1.0 - (1.0 - x).powi(3)
            }),
            |el, shown| el.opacity(shown).mt(px((1.0 - shown) * 4.0)),
        );
    deferred(anchored().anchor(Anchor::BottomLeft).position(point(left, icon.origin.y - px(2.))).child(bubble))
        .with_priority(2)
    .into_any_element()
}
