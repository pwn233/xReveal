//! Arrow keys in the drawer (docs/spec/v1.md §2.2), like a TV's on-screen menu: ↑/↓ move between
//! settings, ←/→ change the one highlighted, Enter presses. On the tab's title, ←/→ switch tabs.

use std::cell::Cell;

use gpui_kit::component::slider::SliderState;
use gpui_kit::*;

use super::{DrawerEvent, SettingsDrawer, Tab, TargetRatio, preset_label};
use crate::setup::{Accent, Corner, Density, DrawerSide, Look, StatsBox, ThemeChoice, WindowMode};
use crate::source;
use crate::video::picture::{Choice, ColourRange, ColourSpace, FillMode, PictureSettings};
use crate::video::timing::DelaySettings;

actions!(drawer_nav, [NavUp, NavDown, NavLeft, NavRight, NavLeftFast, NavRightFast, NavPress]);

/// The key context the drawer's own keys live in.
pub const KEY_CONTEXT: &str = "Drawer";

/// Binds the arrow keys. Call once at startup.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", NavUp, Some(KEY_CONTEXT)),
        KeyBinding::new("down", NavDown, Some(KEY_CONTEXT)),
        KeyBinding::new("left", NavLeft, Some(KEY_CONTEXT)),
        KeyBinding::new("right", NavRight, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-left", NavLeftFast, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-right", NavRightFast, Some(KEY_CONTEXT)),
        KeyBinding::new("enter", NavPress, Some(KEY_CONTEXT)),
        KeyBinding::new("space", NavPress, Some(KEY_CONTEXT)),
    ]);
}

type Drawer = SettingsDrawer;
type Cx<'a> = Context<'a, Drawer>;

/// What ←/→ and Enter do on a row.
pub enum Nav {
    /// The tab's title: ←/→ switch tabs.
    Tabs,
    /// One of `len` choices, `at` chosen.
    Choice { len: usize, at: usize, pick: fn(&mut Drawer, usize, &mut Window, &mut Cx) },
    /// A slider; Enter does `press` where there is one (Volume: mute).
    Slider { value: f32, min: f32, max: f32, step: f32, set: fn(&mut Drawer, f32, &mut Window, &mut Cx), press: Option<fn(&mut Drawer, &mut Cx)> },
    /// `len` switches: ←/→ move between them, Enter flips the one under the cursor.
    Switches { len: usize, flip: fn(&mut Drawer, usize, &mut Cx) },
    /// A button.
    Press(fn(&mut Drawer, &mut Window, &mut Cx)),
}

pub struct Row {
    /// The setting's title, which is how the highlight finds it.
    pub title: &'static str,
    /// Which section of the pane holds it (to scroll it into view); `None` outside the pane.
    pub section: Option<usize>,
    pub nav: Nav,
}

thread_local! {
    // The row to highlight while the drawer renders, and the switch under the cursor.
    static FOCUS: Cell<Option<(&'static str, usize)>> = const { Cell::new(None) };
}

/// Sets (or clears) the highlight for this render.
pub fn set_focus(focus: Option<(&'static str, usize)>) {
    FOCUS.with(|f| f.set(focus));
}

/// Whether the setting titled `title` is highlighted.
pub fn focused(title: &str) -> bool {
    FOCUS.with(|f| f.get().is_some_and(|(t, _)| t == title))
}

/// The switch under the cursor, if `title` is highlighted.
pub fn switch_cursor(title: &str) -> Option<usize> {
    FOCUS.with(|f| f.get().filter(|(t, _)| *t == title).map(|(_, i)| i))
}

/// The choice after `at` going `by` (only its sign counts), kept within `len`.
pub fn step_choice(at: usize, len: usize, by: isize) -> usize {
    (at as isize + by.signum()).clamp(0, len as isize - 1).max(0) as usize
}

/// A slider's value after `by` steps, on the step grid and within its range.
pub fn step_slider(value: f32, min: f32, max: f32, step: f32, by: isize) -> f32 {
    let next = (value + step * by as f32).clamp(min, max);
    ((next / step).round() * step).clamp(min, max)
}

/// The tab after `tab` going `by`, wrapping around.
pub fn step_tab(tab: Tab, by: isize) -> Tab {
    let at = Tab::ALL.iter().position(|t| *t == tab).unwrap_or(0) as isize;
    Tab::ALL[(at + by.signum()).rem_euclid(Tab::ALL.len() as isize) as usize]
}

fn row(title: &'static str, section: usize, nav: Nav) -> Row {
    Row { title, section: Some(section), nav }
}

fn slide(state: &Entity<SliderState>, value: f32, window: &mut Window, cx: &mut Cx) {
    state.update(cx, |s, cx| s.set_value(value, window, cx));
}

fn picture(this: &mut Drawer, cx: &mut Cx, change: impl FnOnce(&mut PictureSettings)) {
    let mut p = this.model.picture;
    change(&mut p);
    this.set_picture(p, cx);
}

impl SettingsDrawer {
    /// The rows of the tab on show, top to bottom, ending with Reset all.
    pub(super) fn nav_rows(&self) -> Vec<Row> {
        let mut rows = vec![Row { title: self.tab.label(), section: None, nav: Nav::Tabs }];
        let p = self.model.picture;
        match self.tab {
            Tab::Source => {
                let current = self.model.current.as_ref();
                let card = current.and_then(|(i, _)| self.card_ids.iter().position(|c| c == &i.os_id)).unwrap_or(0);
                let format = current.and_then(|(_, f)| self.formats.iter().position(|c| c == f)).unwrap_or(0);
                rows.push(row("Input", 1, Nav::Choice { len: self.card_ids.len(), at: card, pick: |this, i, _, cx| {
                    if let Some(os_id) = this.card_ids.get(i).cloned()
                        && this.model.current.as_ref().map(|(c, _)| &c.os_id) != Some(&os_id)
                        && let Some(info) = this.model.sources.iter().find(|s| s.identity.os_id == os_id)
                        && let Some(format) = source::default_choice(&info.formats)
                    {
                        cx.emit(DrawerEvent::Source(os_id, format));
                    }
                } }));
                rows.push(row("Video format", 2, Nav::Choice { len: self.formats.len(), at: format, pick: |this, i, _, cx| {
                    if let Some(format) = this.formats.get(i).copied()
                        && let Some((identity, current)) = this.model.current.clone()
                        && current != format
                    {
                        cx.emit(DrawerEvent::Source(identity.os_id, format));
                    }
                } }));
                rows.push(row("Buffering", 3, Nav::Choice { len: 3, at: self.model.delay.buffering_frames as usize, pick: |this, i, _, cx| {
                    this.set_delay(DelaySettings { buffering_frames: i as u8, ..this.model.delay }, cx);
                } }));
            }
            Tab::Display => {
                let mut s = 0;
                rows.push(row("Window", s, Nav::Choice { len: 2, at: (self.model.window_mode == WindowMode::Fullscreen) as usize, pick: |this, i, _, cx| {
                    let mode = [WindowMode::Windowed, WindowMode::Fullscreen][i];
                    if mode != this.model.window_mode {
                        this.model.window_mode = mode;
                        cx.emit(DrawerEvent::WindowMode(mode));
                        cx.notify();
                    }
                } }));
                s += 1;
                let presets = TargetRatio::PRESETS.len();
                let at = if self.custom_ratio { presets } else { preset_label(p.target_ratio).and_then(|l| TargetRatio::PRESETS.iter().position(|(x, _)| *x == l)).unwrap_or(1) };
                rows.push(row("Target ratio", s, Nav::Choice { len: presets + 1, at, pick: |this, i, _, cx| match TargetRatio::PRESETS.get(i) {
                    Some((_, ratio)) => {
                        this.custom_ratio = false;
                        picture(this, cx, |p| p.target_ratio = *ratio);
                    }
                    None => {
                        this.custom_ratio = true;
                        cx.notify();
                    }
                } }));
                s += 1;
                if self.custom_ratio {
                    rows.push(row("Custom ratio", s, Nav::Slider { value: p.target_ratio.0 as f32, min: 1.0, max: 4.0, step: 0.01, press: None, set: |this, v, w, cx| {
                        slide(&this.sliders.custom_ratio.clone(), v, w, cx);
                        picture(this, cx, |p| p.target_ratio = TargetRatio(v as f64));
                    } }));
                    s += 1;
                }
                let modes = [FillMode::Fit, FillMode::Stretch, FillMode::Crop, FillMode::SmartStretch];
                rows.push(row("Fill mode", s, Nav::Choice { len: 4, at: modes.iter().position(|m| *m == p.fill_mode).unwrap_or(0), pick: |this, i, _, cx| {
                    let modes = [FillMode::Fit, FillMode::Stretch, FillMode::Crop, FillMode::SmartStretch];
                    picture(this, cx, |p| p.fill_mode = modes[i]);
                } }));
                s += 1;
                if p.fill_mode == FillMode::SmartStretch {
                    rows.push(row("Smart stretch strength", s, Nav::Slider { value: p.smart_stretch_strength * 100.0, min: 0.0, max: 100.0, step: 1.0, press: None, set: |this, v, w, cx| {
                        slide(&this.sliders.strength.clone(), v, w, cx);
                        picture(this, cx, |p| p.smart_stretch_strength = v / 100.0);
                    } }));
                    s += 1;
                }
                let stats = self.model.stats;
                rows.push(row("Performance box", s, Nav::Choice { len: 2, at: stats.shown as usize, pick: |this, i, _, cx| {
                    this.set_stats(StatsBox { shown: i == 1, ..this.model.stats }, cx);
                } }));
                if stats.shown {
                    rows.push(row("Corner", s, Nav::Choice { len: 4, at: Corner::ALL.iter().position(|c| *c == stats.corner).unwrap_or(0), pick: |this, i, _, cx| {
                        this.set_stats(StatsBox { corner: Corner::ALL[i], ..this.model.stats }, cx);
                    } }));
                    rows.push(row("Shows", s, Nav::Switches { len: 4, flip: |this, i, cx| {
                        let mut stats = this.model.stats;
                        let flag = match i {
                            0 => &mut stats.latency,
                            1 => &mut stats.fps,
                            2 => &mut stats.frames,
                            _ => &mut stats.format,
                        };
                        *flag = !*flag;
                        this.set_stats(stats, cx);
                    } }));
                }
            }
            Tab::Colour => {
                let ranges = [Choice::Auto, Choice::Manual(ColourRange::Full), Choice::Manual(ColourRange::Limited)];
                rows.push(row("Range", 0, Nav::Choice { len: 3, at: ranges.iter().position(|r| *r == p.colour_range).unwrap_or(0), pick: |this, i, _, cx| {
                    let ranges = [Choice::Auto, Choice::Manual(ColourRange::Full), Choice::Manual(ColourRange::Limited)];
                    picture(this, cx, |p| p.colour_range = ranges[i]);
                } }));
                let spaces = [Choice::Auto, Choice::Manual(ColourSpace::Bt709), Choice::Manual(ColourSpace::Bt601)];
                rows.push(row("Colour space", 1, Nav::Choice { len: 3, at: spaces.iter().position(|s| *s == p.colour_space).unwrap_or(0), pick: |this, i, _, cx| {
                    let spaces = [Choice::Auto, Choice::Manual(ColourSpace::Bt709), Choice::Manual(ColourSpace::Bt601)];
                    picture(this, cx, |p| p.colour_space = spaces[i]);
                } }));
                let a = p.adjustments;
                rows.push(row("Brightness", 2, Nav::Slider { value: a.brightness * 100.0, min: -50.0, max: 50.0, step: 1.0, press: None, set: |this, v, w, cx| {
                    slide(&this.sliders.brightness.clone(), v, w, cx);
                    picture(this, cx, |p| p.adjustments.brightness = v / 100.0);
                } }));
                rows.push(row("Contrast", 3, Nav::Slider { value: a.contrast * 100.0, min: 0.0, max: 200.0, step: 1.0, press: None, set: |this, v, w, cx| {
                    slide(&this.sliders.contrast.clone(), v, w, cx);
                    picture(this, cx, |p| p.adjustments.contrast = v / 100.0);
                } }));
                rows.push(row("Saturation", 4, Nav::Slider { value: a.saturation * 100.0, min: 0.0, max: 200.0, step: 1.0, press: None, set: |this, v, w, cx| {
                    slide(&this.sliders.saturation.clone(), v, w, cx);
                    picture(this, cx, |p| p.adjustments.saturation = v / 100.0);
                } }));
                rows.push(row("More", 5, Nav::Press(|this, _, cx| {
                    this.more_open = !this.more_open;
                    cx.notify();
                })));
                if self.more_open {
                    rows.push(row("Gamma", 5, Nav::Slider { value: a.gamma * 100.0, min: 50.0, max: 200.0, step: 1.0, press: None, set: |this, v, w, cx| {
                        slide(&this.sliders.gamma.clone(), v, w, cx);
                        picture(this, cx, |p| p.adjustments.gamma = v / 100.0);
                    } }));
                    rows.push(row("Hue", 5, Nav::Slider { value: a.hue, min: -180.0, max: 180.0, step: 1.0, press: None, set: |this, v, w, cx| {
                        slide(&this.sliders.hue.clone(), v, w, cx);
                        picture(this, cx, |p| p.adjustments.hue = v);
                    } }));
                }
            }
            Tab::Audio => {
                let audio = &self.model.audio;
                let input = audio.input.as_ref().and_then(|id| audio.inputs.iter().position(|d| &d.id == id)).unwrap_or(0);
                rows.push(row("Input", 0, Nav::Choice { len: audio.inputs.len(), at: input, pick: |this, i, _, cx| {
                    let Some(device) = this.model.audio.inputs.get(i) else { return };
                    let chosen = (this.model.audio.paired.as_ref() != Some(&device.id)).then(|| device.id.clone());
                    this.model.audio.input = Some(device.id.clone());
                    this.model.audio.chosen_input = chosen.clone();
                    cx.emit(DrawerEvent::AudioInput(chosen));
                    cx.notify();
                } }));
                let output = match &audio.output {
                    None => 0,
                    Some(id) => audio.outputs.iter().position(|d| &d.id == id).map_or(0, |i| i + 1),
                };
                rows.push(row("Output", 1, Nav::Choice { len: audio.outputs.len() + 1, at: output, pick: |this, i, _, cx| {
                    let chosen = i.checked_sub(1).and_then(|i| this.model.audio.outputs.get(i)).map(|d| d.id.clone());
                    if chosen != this.model.audio.output {
                        this.model.audio.output = chosen.clone();
                        cx.emit(DrawerEvent::AudioOutput(chosen));
                        cx.notify();
                    }
                } }));
                rows.push(row("Volume", 2, Nav::Slider {
                    value: audio.volume * 100.0,
                    min: 0.0,
                    max: 100.0,
                    step: 1.0,
                    set: |this, v, w, cx| {
                        slide(&this.sliders.volume.clone(), v, w, cx);
                        this.model.audio.volume = v / 100.0;
                        cx.emit(DrawerEvent::Volume(v / 100.0));
                        cx.notify();
                    },
                    press: Some(|this, cx| {
                        this.model.audio.muted = !this.model.audio.muted;
                        cx.emit(DrawerEvent::Mute(this.model.audio.muted));
                        cx.notify();
                    }),
                }));
                rows.push(row("A/V sync offset", 3, Nav::Slider { value: self.model.delay.av_sync_offset_ms as f32, min: -500.0, max: 500.0, step: 1.0, press: None, set: |this, v, w, cx| {
                    slide(&this.sliders.av_sync.clone(), v, w, cx);
                    this.set_delay(DelaySettings { av_sync_offset_ms: v.round() as i32, ..this.model.delay }, cx);
                } }));
            }
            Tab::Interface => {
                let look = self.model.look;
                let themes = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];
                rows.push(row("Theme", 0, Nav::Choice { len: 3, at: themes.iter().position(|t| *t == look.theme).unwrap_or(1), pick: |this, i, _, cx| {
                    let themes = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];
                    this.set_look(Look { theme: themes[i], ..this.model.look }, cx);
                } }));
                rows.push(row("Accent", 1, Nav::Choice { len: Accent::ALL.len(), at: Accent::ALL.iter().position(|a| *a == look.accent).unwrap_or(0), pick: |this, i, _, cx| {
                    this.set_look(Look { accent: Accent::ALL[i], ..this.model.look }, cx);
                } }));
                rows.push(row("Panel opacity", 2, Nav::Slider { value: look.opacity() * 100.0, min: Look::OPACITY.0 * 100.0, max: Look::OPACITY.1 * 100.0, step: 1.0, press: None, set: |this, v, w, cx| {
                    slide(&this.sliders.opacity.clone(), v, w, cx);
                    this.set_look(Look { opacity: v / 100.0, ..this.model.look }, cx);
                } }));
                let densities = [Density::Compact, Density::Default, Density::Spacious];
                rows.push(row("Density", 3, Nav::Choice { len: 3, at: densities.iter().position(|d| *d == look.density).unwrap_or(1), pick: |this, i, _, cx| {
                    let densities = [Density::Compact, Density::Default, Density::Spacious];
                    this.set_look(Look { density: densities[i], ..this.model.look }, cx);
                } }));
                rows.push(row("Drawer side", 4, Nav::Choice { len: 2, at: (self.model.side == DrawerSide::Right) as usize, pick: |this, i, _, cx| {
                    let side = [DrawerSide::Left, DrawerSide::Right][i];
                    if side != this.model.side {
                        this.model.side = side;
                        cx.emit(DrawerEvent::Side(side));
                        cx.notify();
                    }
                } }));
            }
        }
        rows.push(Row { title: "Reset all settings", section: None, nav: Nav::Press(|this, _, cx| {
            if this.confirm_reset {
                this.confirm_reset = false;
                cx.emit(DrawerEvent::ResetAll);
            } else {
                this.confirm_reset = true;
            }
            cx.notify();
        }) });
        rows
    }

    fn move_cursor(&mut self, by: isize, cx: &mut Cx) {
        let rows = self.nav_rows();
        let last = rows.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + by).clamp(0, last) as usize;
        self.switch = 0;
        self.keyboard = true;
        if let Some(section) = rows.get(self.cursor).and_then(|r| r.section) {
            self.scroll.scroll_to_item(section);
        }
        cx.notify();
    }

    fn adjust(&mut self, by: isize, window: &mut Window, cx: &mut Cx) {
        self.keyboard = true;
        let rows = self.nav_rows();
        let Some(row) = rows.get(self.cursor.min(rows.len().saturating_sub(1))) else { return };
        match row.nav {
            Nav::Tabs => self.show_tab(step_tab(self.tab, by), cx),
            Nav::Choice { len, at, pick } if len > 0 => {
                let next = step_choice(at, len, by);
                if next != at {
                    pick(self, next, window, cx);
                }
            }
            Nav::Choice { .. } => {}
            Nav::Slider { value, min, max, step, set, .. } => {
                let next = step_slider(value, min, max, step, by);
                if next != value {
                    set(self, next, window, cx);
                }
            }
            Nav::Switches { len, .. } => self.switch = step_choice(self.switch, len, by),
            Nav::Press(_) => {}
        }
        cx.notify();
    }

    fn press(&mut self, window: &mut Window, cx: &mut Cx) {
        self.keyboard = true;
        let rows = self.nav_rows();
        let Some(row) = rows.get(self.cursor.min(rows.len().saturating_sub(1))) else { return };
        match row.nav {
            Nav::Press(f) => f(self, window, cx),
            Nav::Switches { flip, .. } => flip(self, self.switch, cx),
            Nav::Slider { press: Some(f), .. } => f(self, cx),
            _ => {}
        }
        cx.notify();
    }

    /// The highlight for this render, only once the keyboard is in use.
    pub(super) fn focus_for_render(&self) -> Option<(&'static str, usize)> {
        if !self.keyboard {
            return None;
        }
        let rows = self.nav_rows();
        rows.get(self.cursor.min(rows.len().saturating_sub(1))).map(|r| (r.title, self.switch))
    }

    /// The drawer's key handlers, for its root element.
    pub(super) fn nav_handlers(&self, el: Div, cx: &mut Cx) -> Div {
        el.on_action(cx.listener(|this, _: &NavUp, _, cx| this.move_cursor(-1, cx)))
            .on_action(cx.listener(|this, _: &NavDown, _, cx| this.move_cursor(1, cx)))
            .on_action(cx.listener(|this, _: &NavLeft, w, cx| this.adjust(-1, w, cx)))
            .on_action(cx.listener(|this, _: &NavRight, w, cx| this.adjust(1, w, cx)))
            .on_action(cx.listener(|this, _: &NavLeftFast, w, cx| this.adjust(-10, w, cx)))
            .on_action(cx.listener(|this, _: &NavRightFast, w, cx| this.adjust(10, w, cx)))
            .on_action(cx.listener(|this, _: &NavPress, w, cx| this.press(w, cx)))
            // A click means the mouse is in use: hide the highlight until the next key.
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, _| this.keyboard = false))
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude has its own `test` attribute that would replace std's.
    use super::{Tab, step_choice, step_slider, step_tab};

    #[test]
    fn choices_stop_at_the_ends() {
        assert_eq!(step_choice(0, 3, -1), 0);
        assert_eq!(step_choice(0, 3, 1), 1);
        assert_eq!(step_choice(2, 3, 10), 2, "shift moves sliders faster, choices one at a time");
    }

    #[test]
    fn sliders_step_on_their_grid_within_range() {
        assert_eq!(step_slider(50.0, 0.0, 100.0, 1.0, 1), 51.0);
        assert_eq!(step_slider(50.0, 0.0, 100.0, 1.0, -10), 40.0);
        assert_eq!(step_slider(98.0, 0.0, 100.0, 1.0, 10), 100.0);
        assert!((step_slider(1.777, 1.0, 4.0, 0.01, 1) - 1.79).abs() < 1e-4, "lands back on the grid");
    }

    #[test]
    fn tabs_wrap_around() {
        assert_eq!(step_tab(Tab::Source, 1), Tab::Display);
        assert_eq!(step_tab(Tab::Source, -1), Tab::Interface);
        assert_eq!(step_tab(Tab::Interface, 1), Tab::Source);
    }
}
