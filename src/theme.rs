//! xReveal's look: a quiet on-screen menu over a running game, with one accent for the selection.
//! The live picture is the bright thing; everything xReveal draws stays out of its way. Applied to
//! GPUI Kit's component theme too, so its controls match the panels.
//!
//! The look follows the Interface tab ([`Look`]): Dark (graphite, the default), Light or the
//! OS's appearance; one of a few accents; how solid the panels are; and how dense they are.
//! [`palette`] and [`spacing`] hand out the current values to any render code.

use std::cell::Cell;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::setup::{Accent, Density, Look, ThemeChoice};

/// The colours everything xReveal draws uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// The deepest surface: the drawer's tab rail.
    pub deep: Hsla,
    /// Panels over the picture, at the chosen opacity (the video layer can't be blurred).
    pub panel: Hsla,
    /// Raised: controls, the selected tab.
    pub raised: Hsla,
    pub raised_hover: Hsla,
    /// Hairlines: primary text at a low opacity.
    pub line: Hsla,
    pub text: Hsla,
    pub secondary: Hsla,
    pub muted: Hsla,
    /// Muted at half strength: a control with nothing to do.
    pub faint: Hsla,
    /// A barely-there wash for hovered quiet things.
    pub wash: Hsla,
    /// The one accent: selection, the live dot, slider fill.
    pub accent: Hsla,
    /// The accent as a quiet fill behind selected chips, and its border.
    pub accent_fill: Hsla,
    pub accent_line: Hsla,
    /// Warnings such as "+17 ms input lag".
    pub warn: Hsla,
    pub danger: Hsla,
    pub on_danger: Hsla,
}

/// Spacing that follows the Density choice, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    /// Above and below each section of the drawer.
    pub section: f32,
    /// Between the settings inside one section.
    pub gap: f32,
    /// The height of chips and small buttons.
    pub chip: f32,
    /// The window's base text size (1 rem): text, icons and rem-based gaps scale with it.
    pub rem: f32,
    /// The drawer's width and its tab rail's.
    pub drawer: f32,
    pub rail: f32,
}

fn hex(rgb_hex: u32) -> Hsla {
    rgb(rgb_hex).into()
}

fn with_alpha(colour: Hsla, a: f32) -> Hsla {
    Hsla { a, ..colour }
}

/// An accent's colour: lighter on dark panels, deeper on light ones, so text in it stays readable.
pub fn accent_colour(accent: Accent, dark: bool) -> Hsla {
    hex(match (accent, dark) {
        (Accent::Blue, true) => 0x6aa9ff,
        (Accent::Teal, true) => 0x4fd1c5,
        (Accent::Green, true) => 0x7bd88f,
        (Accent::Amber, true) => 0xf0b35a,
        (Accent::Pink, true) => 0xf28cb1,
        (Accent::Blue, false) => 0x1f63c4,
        (Accent::Teal, false) => 0x0b736b,
        (Accent::Green, false) => 0x23773b,
        (Accent::Amber, false) => 0x8f5600,
        (Accent::Pink, false) => 0xb13a6e,
    })
}

/// Whether `look` comes out dark, given whether the OS appearance is dark.
pub fn is_dark(look: &Look, system_dark: bool) -> bool {
    match look.theme {
        ThemeChoice::System => system_dark,
        ThemeChoice::Dark => true,
        ThemeChoice::Light => false,
    }
}

pub fn palette_for(look: &Look, system_dark: bool) -> Palette {
    let dark = is_dark(look, system_dark);
    let accent = accent_colour(look.accent, dark);
    let opacity = look.opacity();
    if dark {
        let text = hex(0xededed);
        let muted = hex(0x85858b);
        Palette {
            dark,
            deep: hex(0x0f0f10),
            panel: with_alpha(hex(0x141415), opacity),
            raised: hex(0x232325),
            raised_hover: hex(0x2b2b2e),
            line: with_alpha(text, 0.08),
            text,
            secondary: hex(0xa1a1a6),
            muted,
            faint: with_alpha(muted, 0.5),
            wash: with_alpha(text, 0.04),
            accent,
            accent_fill: with_alpha(accent, 0.14),
            accent_line: with_alpha(accent, 0.4),
            warn: hex(0xe2b65a),
            danger: hex(0xe5484d),
            on_danger: hex(0xffffff),
        }
    } else {
        let text = hex(0x1c1c1e);
        let muted = hex(0x6c6c73);
        Palette {
            dark,
            deep: hex(0xe6e6e9),
            panel: with_alpha(hex(0xf5f5f7), opacity),
            raised: hex(0xffffff),
            raised_hover: hex(0xebebee),
            line: with_alpha(text, 0.1),
            text,
            secondary: hex(0x4f4f56),
            muted,
            faint: with_alpha(muted, 0.5),
            wash: with_alpha(text, 0.05),
            accent,
            accent_fill: with_alpha(accent, 0.12),
            accent_line: with_alpha(accent, 0.45),
            warn: hex(0x8a5a00),
            danger: hex(0xc9302c),
            on_danger: hex(0xffffff),
        }
    }
}

pub fn spacing_for(density: Density) -> Spacing {
    match density {
        Density::Compact => Spacing { section: 14.0, gap: 16.0, chip: 26.0, rem: 14.0, drawer: 340.0, rail: 64.0 },
        Density::Default => Spacing { section: 20.0, gap: 24.0, chip: 30.0, rem: 16.0, drawer: 380.0, rail: 72.0 },
        Density::Spacious => Spacing { section: 28.0, gap: 32.0, chip: 34.0, rem: 17.5, drawer: 430.0, rail: 80.0 },
    }
}

thread_local! {
    // UI code runs on the main thread only; this is what it draws with right now.
    static CURRENT: Cell<(Palette, Spacing)> = Cell::new((palette_for(&Look::default(), true), spacing_for(Density::Default)));
}

/// The colours in effect.
pub fn palette() -> Palette {
    CURRENT.with(|c| c.get().0)
}

/// The spacing in effect.
pub fn spacing() -> Spacing {
    CURRENT.with(|c| c.get().1)
}

/// xReveal's type: IBM Plex Sans for the interface, Plex Mono for figures and readouts. Bundled
/// (assets/fonts, SIL Open Font License), so nothing is fetched.
pub const SANS: &str = "IBM Plex Sans";
pub const MONO: &str = "IBM Plex Mono";

/// The font for figures and readouts (formats, values).
pub fn mono_family() -> &'static str {
    MONO
}

/// Registers the bundled Plex fonts. Call once at startup, before [`apply`].
pub fn load_fonts(cx: &App) {
    let fonts: Vec<std::borrow::Cow<'static, [u8]>> = vec![
        std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf")),
        std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf")),
        std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf")),
        std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf")),
    ];
    let _ = cx.text_system().add_fonts(fonts);
}

/// Makes `look` the current look, points GPUI Kit's component theme at it and redraws every window.
pub fn apply(look: &Look, system_dark: bool, cx: &mut App) {
    let p = palette_for(look, system_dark);
    CURRENT.with(|c| c.set((p, spacing_for(look.density))));

    Theme::change(if p.dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
    let solid = |c: Hsla| with_alpha(c, 1.0);
    // Through `update`, which also refreshes the component tokens (the slider reads those) and
    // redraws every window.
    Theme::update(cx, |theme| {
        theme.font_family = SANS.into();
        theme.mono_font_family = MONO.into();
        theme.background = solid(p.panel);
        theme.foreground = p.text;
        theme.border = p.line;
        theme.input = p.line;
        theme.ring = p.accent;
        theme.muted = p.raised;
        theme.muted_foreground = p.secondary;
        theme.popover = if p.dark { hex(0x19191b) } else { hex(0xffffff) };
        theme.popover_foreground = p.text;
        theme.accent = p.raised_hover;
        theme.accent_foreground = p.text;
        theme.list_active = p.accent_fill;
        theme.list_active_border = p.accent_line;
        theme.list_hover = p.raised_hover;
        theme.selection = with_alpha(p.accent, 0.25);
        theme.primary = p.accent;
        theme.primary_hover = p.accent.blend(with_alpha(p.text, 0.1));
        theme.primary_active = p.accent;
        theme.primary_foreground = if p.dark { p.deep } else { hex(0xffffff) };
        theme.secondary = p.raised;
        theme.secondary_hover = p.raised_hover;
        theme.secondary_active = p.raised_hover;
        theme.secondary_foreground = p.text;
        theme.slider_bar = p.accent;
        theme.slider_thumb = if p.dark { p.text } else { hex(0xffffff) };
        theme.scrollbar_thumb = with_alpha(p.text, 0.2);
        theme.scrollbar_thumb_hover = with_alpha(p.text, 0.33);
        theme.danger = p.danger;
        theme.warning = p.warn;
    });
}

/// Whether the OS shows windows in its dark appearance.
pub fn appearance_is_dark(appearance: WindowAppearance) -> bool {
    matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

gpui_kit::assets::icon_assets!(pub ExtraIcons, [Cable, Monitor, Palette, Volume2, SunMoon, MonitorOff, Unplug, Lock, ChevronDown, ChevronRight, ChevronLeft, RotateCcw, X, Check, Settings, Ban]);

/// GPUI Kit's component icons plus the few extra lucide icons xReveal uses.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        // The drawer's notch: its fill and its outline, for either side (assets/notch).
        let notch: Option<&'static [u8]> = match path {
            "xreveal/notch/fill-right.svg" => Some(include_bytes!("../assets/notch/fill-right.svg")),
            "xreveal/notch/line-right.svg" => Some(include_bytes!("../assets/notch/line-right.svg")),
            "xreveal/notch/fill-left.svg" => Some(include_bytes!("../assets/notch/fill-left.svg")),
            "xreveal/notch/line-left.svg" => Some(include_bytes!("../assets/notch/line-left.svg")),
            _ => None,
        };
        if let Some(bytes) = notch {
            return Ok(Some(std::borrow::Cow::Borrowed(bytes)));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude has its own `test` attribute that would replace std's.
    use super::{Accent, Hsla, Look, ThemeChoice, palette_for};

    /// WCAG contrast ratio between two opaque colours.
    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let luminance = |c: Hsla| {
            let c = c.to_rgb();
            let lin = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
        };
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn every_accent_reads_as_text_on_its_panels_in_both_themes() {
        for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
            for accent in Accent::ALL {
                let p = palette_for(&Look { theme, accent, ..Look::default() }, true);
                let panel = Hsla { a: 1.0, ..p.panel };
                for (name, surface) in [("panel", panel), ("raised", p.raised)] {
                    let ratio = contrast(p.accent, surface);
                    assert!(ratio >= 4.5, "{accent:?} on {name} in {theme:?}: {ratio:.2}");
                }
                assert!(contrast(p.text, panel) >= 7.0, "{theme:?} text");
                assert!(contrast(p.muted, panel) >= 4.5, "{theme:?} muted");
            }
        }
    }

    #[test]
    fn system_follows_the_os_and_opacity_reaches_the_panels() {
        let system = Look { theme: ThemeChoice::System, ..Look::default() };
        assert!(palette_for(&system, true).dark);
        assert!(!palette_for(&system, false).dark);
        assert_eq!(palette_for(&Look { opacity: 0.7, ..Look::default() }, true).panel.a, 0.7);
    }
}
