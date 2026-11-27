//! What's new (#144, docs/spec/v1.md §2.9): once after an update, a dialog in the middle of the
//! window with this version's notes. The notes are this version's section of
//! `CHANGELOG.md`, built into the app, so showing them needs no network.

use gpui_kit::*;

use crate::theme::Palette;

/// This version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Where xReveal lives, and this version's release page.
pub const REPOSITORY: &str = "https://github.com/pwn233/xReveal";
pub const RELEASE_PAGE: &str = concat!("https://github.com/pwn233/xReveal/releases/tag/v", env!("CARGO_PKG_VERSION"));

const CHANGELOG: &str = include_str!("../CHANGELOG.md");

/// This version's notes: the `- ` lines of its `## x.y.z` section.
pub fn notes() -> Vec<&'static str> {
    section(CHANGELOG, VERSION)
}

fn section<'a>(changelog: &'a str, version: &str) -> Vec<&'a str> {
    let mut lines = changelog.lines().skip_while(|l| !is_heading_for(l, version));
    lines.next();
    lines.take_while(|l| !l.starts_with("## ")).filter_map(|l| l.strip_prefix("- ")).map(str::trim).collect()
}

/// `## 0.1.0`, `## 0.1.0 – 2026-11-28`: the version, then nothing or a space.
fn is_heading_for(line: &str, version: &str) -> bool {
    line.strip_prefix("## ").and_then(|rest| rest.strip_prefix(version)).is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
}

/// The notes as a dialog in the middle of the window, over a dimmed backdrop, solid so the notes
/// read over any game. The list scrolls past a few lines (`scroll`), with a fade at the top and
/// the bottom while there's more that way.
///
/// `actions` are Got it, Full notes on GitHub and "it scrolled" (to redraw the fades); without
/// them it only shows: Windows draws the visible dialog in the click-through overlay window and an
/// invisible one, with the actions, in the main window, both on the same `scroll`.
pub fn dialog(pal: Palette, notes: &[&'static str], scroll: &ScrollHandle, actions: Option<Actions>) -> AnyElement {
    let bullets = notes.iter().map(|n| {
        div()
            .flex()
            .gap_2()
            .child(div().mt(px(7.)).size(px(4.)).flex_none().rounded_full().bg(pal.accent))
            .child(div().flex_1().min_w_0().text_sm().line_height(px(20.)).text_color(pal.text).child(*n))
    });

    let mut list = div().id("whats-new-notes").max_h(px(LIST_HEIGHT)).overflow_y_scroll().track_scroll(scroll).flex().flex_col().gap_2().pr_1().children(bullets);
    let mut more = div().id("whats-new-github").text_xs().text_color(pal.muted).cursor_pointer().hover(|d| d.text_color(pal.text)).child("Full notes on GitHub");
    let mut ok = div().id("whats-new-ok").px_4().py_1().rounded(px(6.)).bg(pal.accent).text_color(pal.deep).text_sm().cursor_pointer().hover(|d| d.opacity(0.9)).child("Got it");
    if let Some(Actions { got_it, github, scrolled }) = actions {
        ok = ok.on_click(move |_, window, cx| got_it(window, cx));
        more = more.on_click(move |_, window, cx| github(window, cx));
        list = list.on_scroll(move |_, window, cx| scrolled(window, cx));
    }
    let panel = div()
        .id("whats-new")
        .w(px(420.))
        .max_w_full()
        .p_5()
        .rounded(px(14.))
        .bg(pal.raised)
        .border_1()
        .border_color(pal.line)
        .shadow_lg()
        .occlude()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .flex()
                .items_baseline()
                .justify_between()
                .child(div().text_lg().font_weight(FontWeight::MEDIUM).text_color(pal.text).child(format!("What's new in {VERSION}")))
                .child(div().text_xs().text_color(pal.muted).child("Just updated")),
        )
        .child(div().relative().child(list).children(scroll_fades(scroll, pal.raised)))
        .child(div().flex().justify_between().items_center().child(more).child(ok));
    // The backdrop dims the picture and takes the clicks: only Got it closes the dialog.
    div().absolute().inset_0().flex().items_center().justify_center().p_6().bg(hsla(0., 0., 0., 0.55)).occlude().child(panel).into_any_element()
}

/// The fades over a scrolling list on a `background`: at the top once it's scrolled down, at the
/// bottom while there's more below. Its parent must be `relative`. Shared with the update
/// button's tip (#145).
pub fn scroll_fades(scroll: &ScrollHandle, background: Hsla) -> Vec<AnyElement> {
    let (top, bottom) = fades(scroll.offset().y.as_f32(), scroll.max_offset().y.as_f32());
    let fade = |top: bool| {
        let gradient = linear_gradient(if top { 180. } else { 0. }, linear_color_stop(background, 0.), linear_color_stop(background.opacity(0.), 1.));
        let d = div().absolute().left_0().right_0().h(px(28.)).bg(gradient);
        if top { d.top_0() } else { d.bottom_0() }.into_any_element()
    };
    let mut out = Vec::new();
    if top {
        out.push(fade(true));
    }
    if bottom {
        out.push(fade(false));
    }
    out
}

/// Whether a list scrolled to `offset` (0 at the top, negative further down) of at most `max`
/// shows its top and bottom fades. GPUI's wheel handler adds the delta without a limit and the next
/// layout clamps it, so a render in between can see an offset past either end: clamped here first,
/// or a wheel turn on a list with nothing more to show would flash a fade for a frame.
fn fades(offset: f32, max: f32) -> (bool, bool) {
    let max = max.max(0.);
    let offset = offset.clamp(-max, 0.);
    (offset < -1., -offset < max - 1.)
}

/// Past this the list scrolls.
const LIST_HEIGHT: f32 = 260.;

/// One of the dialog's actions.
pub type Action = Box<dyn Fn(&mut Window, &mut App)>;

/// What the dialog's buttons and list do.
pub struct Actions {
    pub got_it: Action,
    pub github: Action,
    pub scrolled: Action,
}

#[cfg(test)]
mod tests {
    use super::{fades, is_heading_for, section};

    const SAMPLE: &str = "# Changelog\n\nIntro.\n\n## 0.2.0 – 2026-12-20\n\n- New thing.\n- Fixed thing.\n\n## 0.1.0 – 2026-11-28\n\n- First.\n";

    #[test]
    fn a_version_gets_its_own_section_only() {
        assert_eq!(section(SAMPLE, "0.2.0"), ["New thing.", "Fixed thing."]);
        assert_eq!(section(SAMPLE, "0.1.0"), ["First."]);
        assert!(section(SAMPLE, "0.3.0").is_empty(), "no section, no card");
    }

    #[test]
    fn a_heading_names_exactly_its_version() {
        assert!(is_heading_for("## 0.1.0", "0.1.0"));
        assert!(is_heading_for("## 0.1.0 – unreleased", "0.1.0"));
        assert!(!is_heading_for("## 0.1.01", "0.1.0"));
        assert!(!is_heading_for("### 0.1.0", "0.1.0"));
    }

    #[test]
    fn this_version_has_notes() {
        assert!(!super::notes().is_empty(), "CHANGELOG.md needs a section for this version");
    }

    #[test]
    fn fades_show_only_where_there_is_more() {
        assert_eq!(fades(0., 100.), (false, true), "at the top: more below");
        assert_eq!(fades(-50., 100.), (true, true), "in the middle: more both ways");
        assert_eq!(fades(-100., 100.), (true, false), "at the bottom: more above");
    }

    #[test]
    fn an_offset_past_either_end_counts_as_that_end() {
        // A wheel turn before the next layout clamps the offset (#168).
        assert_eq!(fades(-60., 0.), (false, false), "nothing to scroll: no fade, however far the wheel turned");
        assert_eq!(fades(-160., 100.), (true, false), "past the bottom");
        assert_eq!(fades(60., 100.), (false, true), "past the top");
    }
}
