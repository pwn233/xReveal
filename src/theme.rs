//! xReveal's look: a quiet graphite on-screen menu over a running game, with one blue accent for
//! the selection. The live picture is the bright thing; everything xReveal draws stays out of its
//! way. Applied to GPUI Kit's component theme too, so its controls match the panels.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

/// Colours as `0xRRGGBB` (opaque) or `0xRRGGBBAA`.
pub mod palette {
    /// The deepest surface: the drawer's tab rail.
    pub const DEEP: u32 = 0x0f0f10;
    /// Panels over the picture: nearly opaque, because the video layer underneath can't be blurred.
    pub const PANEL: u32 = 0x141415fa;
    /// Raised: controls, the selected tab, hovered rows.
    pub const RAISED: u32 = 0x232325;
    pub const RAISED_HOVER: u32 = 0x2b2b2e;
    /// Hairlines: primary text at 8%.
    pub const LINE: u32 = 0xededed14;
    pub const TEXT: u32 = 0xededed;
    pub const SECONDARY: u32 = 0xa1a1a6;
    pub const MUTED: u32 = 0x85858b;
    /// The one accent: selection, the live dot, slider fill.
    pub const ACCENT: u32 = 0x6aa9ff;
    /// The accent as a quiet fill behind selected chips.
    pub const ACCENT_FILL: u32 = 0x6aa9ff24;
    /// Warnings such as "+17 ms input lag": amber, toned to sit in the palette.
    pub const WARN: u32 = 0xe2b65a;
    pub const DANGER: u32 = 0xe5484d;
}

/// The font for figures and readouts (formats, values): the OS's monospace.
pub fn mono_family() -> &'static str {
    if cfg!(target_os = "windows") { "Consolas" } else { "Menlo" }
}

/// Switches GPUI Kit to its dark theme and points its colours at the palette.
pub fn apply(cx: &mut App) {
    use palette::*;
    Theme::change(ThemeMode::Dark, None, cx);
    let c = |hex: u32| -> Hsla { rgb(hex).into() };
    let ca = |hex: u32| -> Hsla { rgba(hex).into() };
    let theme = Theme::global_mut(cx);
    theme.background = c(0x141415);
    theme.foreground = c(TEXT);
    theme.border = ca(LINE);
    theme.input = ca(LINE);
    theme.ring = c(ACCENT);
    theme.muted = c(RAISED);
    theme.muted_foreground = c(SECONDARY);
    theme.popover = c(0x19191b);
    theme.popover_foreground = c(TEXT);
    theme.accent = c(RAISED_HOVER);
    theme.accent_foreground = c(TEXT);
    theme.list_active = ca(ACCENT_FILL);
    theme.list_active_border = ca(0x6aa9ff66);
    theme.list_hover = c(RAISED_HOVER);
    theme.selection = ca(0x6aa9ff40);
    theme.primary = c(ACCENT);
    theme.primary_hover = c(0x7db5ff);
    theme.primary_active = c(0x5a9bf0);
    theme.primary_foreground = c(DEEP);
    theme.secondary = c(RAISED);
    theme.secondary_hover = c(RAISED_HOVER);
    theme.secondary_active = c(0x303034);
    theme.secondary_foreground = c(TEXT);
    theme.slider_bar = c(ACCENT);
    theme.slider_thumb = c(TEXT);
    theme.scrollbar_thumb = ca(0xededed33);
    theme.scrollbar_thumb_hover = ca(0xededed55);
    theme.danger = c(DANGER);
    theme.warning = c(WARN);
}

gpui_kit::assets::icon_assets!(pub ExtraIcons, [Cable, Monitor, Palette, Volume2, ChevronDown, ChevronRight, ChevronLeft, RotateCcw, X]);

/// GPUI Kit's component icons plus the few extra lucide icons xReveal uses.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
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
