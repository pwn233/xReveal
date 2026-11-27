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
use crate::update::{Check, UpdaterView};

const KEY: &str = "update-button";
/// The tip's widest: wide enough for a few lines of notes.
const TIP_WIDTH: f32 = 300.;
/// Past this the notes scroll.
const NOTES_HEIGHT: f32 = 180.;
/// How long the tip stays after the pointer leaves the button and the tip.
const CLOSE_AFTER: Duration = Duration::from_millis(300);
/// The ring's top speed, in turns per second: however fast the download, the ring takes at least
/// this long to fill (0.7 s), and a ring that's behind catches up smoothly (#184).
const RING_SPEED: f32 = 1. / 0.7;
/// How quickly the ring closes on the real progress (per second, exponential).
const RING_EASE: f32 = 6.;

/// The download ring as drawn: it eases after the real progress, never faster than
/// [`RING_SPEED`], and fills the whole way before the button turns into Restart.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ring {
    /// How full it's drawn, while it's drawn.
    shown: Option<f32>,
    last: Option<std::time::Instant>,
}

impl Ring {
    /// Moves on to `now`, after the real state: downloading (`Some(fraction)`, `None` while the
    /// size isn't known) or `ready`. Returns how full to draw the ring (`Some(None)` for just its
    /// track) and whether it's still moving; `None` when there's no ring.
    pub fn step(&mut self, downloading: Option<Option<f32>>, ready: bool, now: std::time::Instant) -> Option<(Option<f32>, bool)> {
        let dt = self.last.map_or(0., |t| now.duration_since(t).as_secs_f32().min(0.1));
        self.last = Some(now);
        let target = match (downloading, ready) {
            (Some(Some(fraction)), _) => fraction.clamp(0., 1.),
            // Its size isn't known yet: the track only, where it was.
            (Some(None), _) => return Some((self.shown.filter(|s| *s > 0.), false)),
            // Done: a ring still filling finishes first; none drawn yet means none to finish.
            (None, true) if self.shown.is_some_and(|s| s < 1.) => 1.,
            _ => {
                self.shown = None;
                return None;
            }
        };
        let shown = self.shown.unwrap_or(0.);
        let eased = (target - shown) * (1. - (-RING_EASE * dt).exp());
        let mut next = shown + eased.clamp(-RING_SPEED * dt, RING_SPEED * dt);
        if (target - next).abs() < 0.004 {
            next = target;
        }
        self.shown = Some(next);
        let moving = next != target;
        if ready && !moving {
            // Full: the button turns into Restart from the next frame.
            self.shown = None;
            return Some((Some(1.), true));
        }
        Some((Some(next), moving))
    }

    /// An empty ring, from the press of the download button.
    pub fn started() -> Self {
        Ring { shown: Some(0.), last: None }
    }

    /// The ring is still drawn: during a download, or finishing one that's already done.
    pub fn finishing(&self) -> bool {
        self.shown.is_some()
    }
}

impl SettingsDrawer {
    pub(super) fn update_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = theme::palette();
        let offer = self.model.update.clone();
        let updater = self.model.updater.clone();
        let checking = self.model.update_check == Check::Checking;
        let done = offer.is_some() && updater == UpdaterView::Ready;
        let real = match (&offer, &updater) {
            (Some(_), UpdaterView::Downloading(fraction)) => Some(*fraction),
            _ => None,
        };
        // The ring as drawn eases after the real progress (#184), and finishes before Restart shows.
        let mut ring = self.update_ring.get();
        let downloading = ring.step(real, done, std::time::Instant::now());
        let ready = done && !ring.finishing() && downloading.is_none();
        self.update_ring.set(ring);
        let (icon, colour) = match (&offer, checking) {
            (Some(_), _) if ready => (IconName::RotateCw, pal.accent),
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
            // Ready to restart: filled, so it reads as the thing to press.
            .when(ready, |d| d.bg(pal.accent_fill).border_1().border_color(pal.accent_line))
            .child(icon)
            .children(downloading.map(|(fraction, moving)| progress_ring(fraction, moving, pal.accent, pal.line)))
            .child(canvas(move |b, window, _| bounds.borrow_mut().insert(KEY, (b, window.viewport_size().width)), |_, _, _, _| {}).absolute().inset_0())
            .on_hover(cx.listener(|this, hovering: &bool, _, cx| {
                this.update_hover.0 = *hovering;
                this.update_hover_changed(cx);
            }))
            .on_click(cx.listener(move |this, _, _, cx| match (&offer, &this.model.updater) {
                // Still finishing the ring: Restart shows once it's full.
                (Some(_), UpdaterView::Ready) if this.update_ring.get().finishing() => {}
                (Some(_), UpdaterView::Ready) => cx.emit(DrawerEvent::RestartToUpdate),
                (Some(_), UpdaterView::Downloading(_)) => {}
                (Some(_), _) => {
                    // The ring shows from the press, even when the download is done before the next frame.
                    this.update_ring.set(Ring::started());
                    cx.emit(DrawerEvent::DownloadUpdate);
                }
                (None, _) if this.model.update_check != Check::Checking => cx.emit(DrawerEvent::CheckNow),
                (None, _) => {}
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
                let (title, line) = match &self.model.updater {
                    UpdaterView::Idle => (format!("xReveal {version} is out"), "Click to download it.".to_owned()),
                    UpdaterView::Downloading(Some(f)) => (format!("Downloading xReveal {version}… {:.0}%", f * 100.), "It installs when you restart.".to_owned()),
                    UpdaterView::Downloading(None) => (format!("Downloading xReveal {version}…"), "It installs when you restart.".to_owned()),
                    UpdaterView::Ready => (format!("xReveal {version} is ready"), "Click to restart and update.".to_owned()),
                    UpdaterView::Failed(why) => (format!("Couldn't get xReveal {version}"), format!("{why}. Click to try again.")),
                };
                panel
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                    .child(div().text_color(pal.secondary).child(line))
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

/// A thin ring around the button filling clockwise from the top as the download arrives; just the
/// faint track while its size isn't known yet. `moving`: it hasn't caught up yet (see [`Ring`]).
fn progress_ring(fraction: Option<f32>, moving: bool, fill: Hsla, track: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            // Easing after the real progress: draw again next frame until it has caught up.
            if moving {
                window.request_animation_frame();
            }
            let centre = bounds.center();
            let r = bounds.size.width.min(bounds.size.height) / 2. - px(1.5);
            let at = |turn: f32| {
                let a = turn * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                point(centre.x + r * a.cos(), centre.y + r * a.sin())
            };
            let arc = |from: f32, to: f32| {
                let mut path = PathBuilder::stroke(px(2.));
                path.move_to(at(from));
                // Two halves at most per arc, so a large arc never flips.
                let mid = (from + to) / 2.;
                path.arc_to(point(r, r), px(0.), false, true, at(mid));
                path.arc_to(point(r, r), px(0.), false, true, at(to));
                path.build().ok()
            };
            if let Some(track) = arc(0., 0.999).map(|p| (p, track)) {
                window.paint_path(track.0, track.1);
            }
            if let Some(f) = fraction.filter(|f| *f > 0.01)
                && let Some(path) = arc(0., f.min(0.999))
            {
                window.paint_path(path, fill);
            }
        },
    )
    .absolute()
    .inset_0()
}

#[cfg(test)]
mod tests {
    use super::{RING_SPEED, Ring};
    use std::time::{Duration, Instant};

    /// Steps `ring` every 1/60 s until it stops moving, at most `limit` steps; returns how many.
    fn run(ring: &mut Ring, downloading: Option<Option<f32>>, ready: bool, start: Instant, limit: usize) -> usize {
        for i in 1..=limit {
            match ring.step(downloading, ready, start + Duration::from_secs_f32(i as f32 / 60.)) {
                Some((_, true)) => {}
                _ => return i,
            }
        }
        limit
    }

    #[test]
    fn a_download_done_at_once_still_fills_the_ring_smoothly() {
        let start = Instant::now();
        let mut ring = Ring::started();
        ring.step(None, true, start);
        let mut last = 0.;
        let mut frames = 0;
        for i in 1..200 {
            match ring.step(None, true, start + Duration::from_secs_f32(i as f32 / 60.)) {
                Some((Some(f), true)) => {
                    assert!(f >= last, "never goes back");
                    assert!(f - last <= RING_SPEED / 60. + 1e-4, "never faster than its top speed");
                    last = f;
                    frames += 1;
                }
                Some((Some(f), false)) => panic!("done but still drawn at {f}"),
                _ => break,
            }
        }
        assert!(last == 1., "fills all the way before Restart");
        assert!(frames as f32 / 60. >= 0.6, "takes a moment, not one frame: {frames} frames");
        assert_eq!(ring.step(None, true, start + Duration::from_secs(5)), None, "then Restart shows");
    }

    #[test]
    fn follows_the_real_progress_and_settles_on_it() {
        let start = Instant::now();
        let mut ring = Ring::default();
        let frames = run(&mut ring, Some(Some(0.5)), false, start, 600);
        assert!(frames < 600, "settles");
        assert_eq!(ring.step(Some(Some(0.5)), false, start + Duration::from_secs(20)), Some((Some(0.5), false)));
    }

    #[test]
    fn already_done_when_the_drawer_opens_shows_restart_at_once() {
        let mut ring = Ring::default();
        assert_eq!(ring.step(None, true, Instant::now()), None);
        assert!(!ring.finishing());
    }

    #[test]
    fn a_failed_download_drops_the_ring() {
        let mut ring = Ring::started();
        assert_eq!(ring.step(None, false, Instant::now()), None);
    }
}
