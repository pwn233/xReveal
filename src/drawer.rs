//! The Settings drawer (docs/spec/v1.md §2.3): a see-through panel over the live picture with a
//! side rail of four tabs. Every setting has an always-visible hint line and applies live.
//!
//! The drawer only shows and edits a snapshot ([`DrawerModel`]); every change goes out as a
//! [`DrawerEvent`] and the app applies it to the video pipeline and the Remembered setup. That
//! keeps it hostable in its own window too (Windows draws it in a separate transparent window).

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::base::Selectable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::setup::DrawerSide;
use crate::source::{self, SourceIdentity, SourceInfo, VideoFormat};
use crate::video::picture::{Adjustments, Choice, ColourRange, ColourSpace, FillMode, PictureSettings, TargetRatio};
use crate::video::timing::DelaySettings;

pub const WIDTH: f32 = 380.0;

/// What the drawer shows: a snapshot the app keeps up to date.
#[derive(Clone, Debug, Default)]
pub struct DrawerModel {
    pub sources: Vec<SourceInfo>,
    pub current: Option<(SourceIdentity, VideoFormat)>,
    /// The picture settings in effect, including the current Source's colour choices.
    pub picture: PictureSettings,
    pub delay: DelaySettings,
    pub side: DrawerSide,
    /// The Source's frame interval in seconds, to say how much Buffering costs.
    pub frame_interval: f64,
}

/// A change made in the drawer, for the app to apply.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawerEvent {
    /// Show this Source (OS ID) at this Video format. The picture restarts briefly.
    Source(String, VideoFormat),
    Picture(PictureSettings),
    Delay(DelaySettings),
    Side(DrawerSide),
    ResetAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Source,
    Display,
    Colour,
    Audio,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Source, Tab::Display, Tab::Colour, Tab::Audio];

    fn label(self) -> &'static str {
        match self {
            Tab::Source => "Source",
            Tab::Display => "Display",
            Tab::Colour => "Colour",
            Tab::Audio => "Audio",
        }
    }
}

/// Plain-language hint lines, one per setting (docs/spec/v1.md §2.3).
pub mod hints {
    pub const CARD: &str = "The capture card your console is plugged into.";
    pub const VIDEO_FORMAT: &str = "What the card sends. Changing it restarts the picture for a moment.";
    pub const BUFFERING: &str = "Holds frames back to smooth uneven delivery. Off is fastest.";
    pub const HDCP: &str = "Black picture on a PS5? Turn off HDCP in its HDMI settings.";
    pub const TARGET_RATIO: &str = "The shape of the picture on your screen. Pick your monitor's shape (21:9 for ultrawide) to fill it.";
    pub const FILL_MODE: &str = "How the console's 16:9 picture fits that shape: Fit adds black bars, Stretch widens everything, Crop zooms in, Smart stretch keeps the middle natural.";
    pub const STRENGTH: &str = "0% looks just like Stretch. Higher keeps the centre more natural and stretches the edges more.";
    pub const CUSTOM_RATIO: &str = "Width ÷ height of your own shape, e.g. 2.39 for a cinema ratio.";
    pub const DRAWER_SIDE: &str = "Which side this panel opens from.";
    pub const RANGE: &str = "Match your console's RGB range. Blacks look grey? Choose Limited, or set your console to Full.";
    pub const SPACE: &str = "The picture's colour formula. Auto picks BT.709 for HD pictures.";
    pub const BRIGHTNESS: &str = "Lifts or lowers the whole picture.";
    pub const CONTRAST: &str = "Spreads darks and lights further apart.";
    pub const SATURATION: &str = "How strong the colours are. 0% is black and white.";
    pub const GAMMA: &str = "Brightens or darkens the mid-tones without touching black and white.";
    pub const HUE: &str = "Turns every colour around the colour wheel.";
    pub const AV_SYNC: &str = "Lines sound up with the picture. Positive delays the sound; negative delays the picture.";
    pub const AUDIO_LATER: &str = "Sound playback, input and output choice, volume and mute arrive with the audio pipeline.";
    pub const RESET_ALL: &str = "Forgets every setting and every card, as if xReveal had never run.";
}

/// "+N ms input lag" for a picture delay, or `None` when there is none (docs/spec/v1.md §2.3).
pub fn input_lag_warning(delay: DelaySettings, frame_interval: f64) -> Option<String> {
    let ms = (delay.seconds(frame_interval) * 1000.0).round() as i64;
    (ms > 0).then(|| format!("+{ms} ms input lag"))
}

/// The preset's label for `ratio`, or `None` for a Custom ratio.
pub fn preset_label(ratio: TargetRatio) -> Option<&'static str> {
    TargetRatio::PRESETS.iter().find(|(_, r)| (r.0 - ratio.0).abs() < 0.005).map(|(label, _)| *label)
}

pub struct SettingsDrawer {
    model: DrawerModel,
    tab: Tab,
    more_open: bool,
    confirm_reset: bool,
    custom_ratio: bool,
    sliders: Sliders,
    _subscriptions: Vec<Subscription>,
}

struct Sliders {
    strength: Entity<SliderState>,
    custom_ratio: Entity<SliderState>,
    brightness: Entity<SliderState>,
    contrast: Entity<SliderState>,
    saturation: Entity<SliderState>,
    gamma: Entity<SliderState>,
    hue: Entity<SliderState>,
    av_sync: Entity<SliderState>,
}

impl EventEmitter<DrawerEvent> for SettingsDrawer {}

impl SettingsDrawer {
    pub fn new(model: DrawerModel, cx: &mut Context<Self>) -> Self {
        let p = model.picture;
        let slider = |cx: &mut Context<Self>, min: f32, max: f32, step: f32, value: f32| {
            cx.new(|_| SliderState::new().min(min).max(max).step(step).default_value(value))
        };
        let sliders = Sliders {
            strength: slider(cx, 0.0, 100.0, 1.0, p.smart_stretch_strength * 100.0),
            custom_ratio: slider(cx, 1.0, 4.0, 0.01, p.target_ratio.0 as f32),
            brightness: slider(cx, -50.0, 50.0, 1.0, p.adjustments.brightness * 100.0),
            contrast: slider(cx, 0.0, 200.0, 1.0, p.adjustments.contrast * 100.0),
            saturation: slider(cx, 0.0, 200.0, 1.0, p.adjustments.saturation * 100.0),
            gamma: slider(cx, 50.0, 200.0, 1.0, p.adjustments.gamma * 100.0),
            hue: slider(cx, -180.0, 180.0, 1.0, p.adjustments.hue),
            av_sync: slider(cx, -500.0, 500.0, 1.0, model.delay.av_sync_offset_ms as f32),
        };
        let mut subscriptions = Vec::new();
        let mut on_picture = |state: &Entity<SliderState>, apply: fn(&mut PictureSettings, f32)| {
            subscriptions.push(cx.subscribe(state, move |this: &mut Self, _, event: &SliderEvent, cx| {
                if let SliderEvent::Change(SliderValue::Single(value)) = event {
                    let mut picture = this.model.picture;
                    apply(&mut picture, *value);
                    this.set_picture(picture, cx);
                }
            }));
        };
        on_picture(&sliders.strength, |p, v| p.smart_stretch_strength = v / 100.0);
        on_picture(&sliders.custom_ratio, |p, v| p.target_ratio = TargetRatio(v as f64));
        on_picture(&sliders.brightness, |p, v| p.adjustments.brightness = v / 100.0);
        on_picture(&sliders.contrast, |p, v| p.adjustments.contrast = v / 100.0);
        on_picture(&sliders.saturation, |p, v| p.adjustments.saturation = v / 100.0);
        on_picture(&sliders.gamma, |p, v| p.adjustments.gamma = v / 100.0);
        on_picture(&sliders.hue, |p, v| p.adjustments.hue = v);
        subscriptions.push(cx.subscribe(&sliders.av_sync, |this: &mut Self, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(SliderValue::Single(value)) = event {
                let delay = DelaySettings { av_sync_offset_ms: value.round() as i32, ..this.model.delay };
                this.set_delay(delay, cx);
            }
        }));
        let custom_ratio = preset_label(p.target_ratio).is_none();
        Self { model, tab: Tab::Source, more_open: false, confirm_reset: false, custom_ratio, sliders, _subscriptions: subscriptions }
    }

    /// Replaces the snapshot (e.g. after a Source switch or a reset) and moves the sliders to match.
    pub fn set_model(&mut self, model: DrawerModel, window: &mut Window, cx: &mut Context<Self>) {
        let p = model.picture;
        let values = [
            (&self.sliders.strength, p.smart_stretch_strength * 100.0),
            (&self.sliders.custom_ratio, p.target_ratio.0 as f32),
            (&self.sliders.brightness, p.adjustments.brightness * 100.0),
            (&self.sliders.contrast, p.adjustments.contrast * 100.0),
            (&self.sliders.saturation, p.adjustments.saturation * 100.0),
            (&self.sliders.gamma, p.adjustments.gamma * 100.0),
            (&self.sliders.hue, p.adjustments.hue),
            (&self.sliders.av_sync, model.delay.av_sync_offset_ms as f32),
        ];
        for (state, value) in values {
            state.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        self.custom_ratio = self.custom_ratio || preset_label(p.target_ratio).is_none();
        self.model = model;
        cx.notify();
    }

    fn set_picture(&mut self, picture: PictureSettings, cx: &mut Context<Self>) {
        if picture != self.model.picture {
            self.model.picture = picture;
            cx.emit(DrawerEvent::Picture(picture));
            cx.notify();
        }
    }

    fn set_delay(&mut self, delay: DelaySettings, cx: &mut Context<Self>) {
        if delay != self.model.delay {
            self.model.delay = delay;
            cx.emit(DrawerEvent::Delay(delay));
            cx.notify();
        }
    }

    fn rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let tabs = Tab::ALL.into_iter().map(|tab| {
            let on = tab == self.tab;
            div()
                .id(SharedString::from(tab.label()))
                .px_2()
                .py_3()
                .text_xs()
                .text_center()
                .cursor_pointer()
                .text_color(if on { rgb(ACCENT) } else { rgb(MUTED) })
                .when(on, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(tab.label())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = tab;
                    this.confirm_reset = false;
                    cx.notify();
                }))
        });
        div().w(px(64.)).flex_none().pt_2().border_r_1().border_color(rgba(LINE)).flex().flex_col().children(tabs).into_any_element()
    }

    fn source_tab(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let current = self.model.current.clone();
        let current_id = current.as_ref().map(|(i, _)| i.os_id.clone());
        let cards = self.model.sources.iter().enumerate().map(|(i, s)| {
            let os_id = s.identity.os_id.clone();
            let chosen = Some(&os_id) == current_id.as_ref();
            let default = source::default_choice(&s.formats);
            choice_row(("card", i), &s.identity.name, chosen).on_click(cx.listener(move |_, _, _, cx| {
                if !chosen && let Some(format) = default {
                    cx.emit(DrawerEvent::Source(os_id.clone(), format));
                }
            }))
        });
        let mut out = vec![setting("Capture card", hints::CARD, div().flex().flex_col().children(cards))];
        if let Some((identity, format)) = current
            && let Some(info) = self.model.sources.iter().find(|s| s.identity.os_id == identity.os_id)
        {
            let rows = source::choices(&info.formats).into_iter().enumerate().map(|(i, choice)| {
                let chosen = choice == format;
                let os_id = identity.os_id.clone();
                choice_row(("format", i), &choice.to_string(), chosen).on_click(cx.listener(move |_, _, _, cx| {
                    if !chosen {
                        cx.emit(DrawerEvent::Source(os_id.clone(), choice));
                    }
                }))
            });
            out.push(setting("Video format", hints::VIDEO_FORMAT, div().flex().flex_col().max_h(px(240.)).overflow_hidden().children(rows)));
        }
        let delay = self.model.delay;
        let buffering = segmented(
            "buffering",
            &["Off", "1 frame", "2 frames"],
            delay.buffering_frames as usize,
            cx.listener(move |this, picked: &Vec<usize>, _, cx| {
                if let Some(&i) = picked.first() {
                    this.set_delay(DelaySettings { buffering_frames: i as u8, ..this.model.delay }, cx);
                }
            }),
        );
        let warning = input_lag_warning(DelaySettings { av_sync_offset_ms: 0, ..delay }, self.model.frame_interval);
        out.push(setting("Buffering", hints::BUFFERING, div().child(buffering).children(warning.map(warn))));
        out.push(note(hints::HDCP));
        out
    }

    fn display_tab(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.model.picture;
        let mut labels: Vec<&str> = TargetRatio::PRESETS.iter().map(|(label, _)| *label).collect();
        labels.push("Custom");
        let selected = if self.custom_ratio { labels.len() - 1 } else { preset_label(p.target_ratio).and_then(|l| labels.iter().position(|x| *x == l)).unwrap_or(1) };
        let ratios = segmented(
            "ratio",
            &labels,
            selected,
            cx.listener(|this, picked: &Vec<usize>, _, cx| {
                let Some(&i) = picked.first() else { return };
                match TargetRatio::PRESETS.get(i) {
                    Some((_, ratio)) => {
                        this.custom_ratio = false;
                        this.set_picture(PictureSettings { target_ratio: *ratio, ..this.model.picture }, cx);
                    }
                    None => {
                        this.custom_ratio = true;
                        cx.notify();
                    }
                }
            }),
        );
        let mut out = vec![setting("Target ratio", hints::TARGET_RATIO, ratios)];
        if self.custom_ratio {
            out.push(slider_setting("Custom ratio", hints::CUSTOM_RATIO, &self.sliders.custom_ratio, format!("{:.2} : 1", p.target_ratio.0)));
        }
        let modes = [FillMode::Fit, FillMode::Stretch, FillMode::Crop, FillMode::SmartStretch];
        let fill = segmented(
            "fill",
            &["Fit", "Stretch", "Crop", "Smart stretch"],
            modes.iter().position(|m| *m == p.fill_mode).unwrap_or(0),
            cx.listener(move |this, picked: &Vec<usize>, _, cx| {
                if let Some(&i) = picked.first() {
                    this.set_picture(PictureSettings { fill_mode: modes[i], ..this.model.picture }, cx);
                }
            }),
        );
        out.push(setting("Fill mode", hints::FILL_MODE, fill));
        if p.fill_mode == FillMode::SmartStretch {
            let label = format!("{:.0}%", p.smart_stretch_strength * 100.0);
            out.push(slider_setting("Smart stretch strength", hints::STRENGTH, &self.sliders.strength, label));
        }
        let sides = [DrawerSide::Left, DrawerSide::Right];
        let side = segmented(
            "side",
            &["Left", "Right"],
            sides.iter().position(|s| *s == self.model.side).unwrap_or(1),
            cx.listener(move |this, picked: &Vec<usize>, _, cx| {
                if let Some(&i) = picked.first()
                    && sides[i] != this.model.side
                {
                    this.model.side = sides[i];
                    cx.emit(DrawerEvent::Side(sides[i]));
                    cx.notify();
                }
            }),
        );
        out.push(setting("Drawer side", hints::DRAWER_SIDE, side));
        out
    }

    fn colour_tab(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.model.picture;
        let ranges = [Choice::Auto, Choice::Manual(ColourRange::Full), Choice::Manual(ColourRange::Limited)];
        let range = segmented(
            "range",
            &["Auto", "Full", "Limited"],
            ranges.iter().position(|r| *r == p.colour_range).unwrap_or(0),
            cx.listener(move |this, picked: &Vec<usize>, _, cx| {
                if let Some(&i) = picked.first() {
                    this.set_picture(PictureSettings { colour_range: ranges[i], ..this.model.picture }, cx);
                }
            }),
        );
        let spaces = [Choice::Auto, Choice::Manual(ColourSpace::Bt709), Choice::Manual(ColourSpace::Bt601)];
        let space = segmented(
            "space",
            &["Auto", "BT.709", "BT.601"],
            spaces.iter().position(|s| *s == p.colour_space).unwrap_or(0),
            cx.listener(move |this, picked: &Vec<usize>, _, cx| {
                if let Some(&i) = picked.first() {
                    this.set_picture(PictureSettings { colour_space: spaces[i], ..this.model.picture }, cx);
                }
            }),
        );
        let a = p.adjustments;
        let mut out = vec![
            setting("Range", hints::RANGE, range),
            setting("Colour space", hints::SPACE, space),
            slider_setting("Brightness", hints::BRIGHTNESS, &self.sliders.brightness, format!("{:+.0}", a.brightness * 100.0)),
            slider_setting("Contrast", hints::CONTRAST, &self.sliders.contrast, format!("{:.0}%", a.contrast * 100.0)),
            slider_setting("Saturation", hints::SATURATION, &self.sliders.saturation, format!("{:.0}%", a.saturation * 100.0)),
        ];
        let more = div()
            .id("more")
            .mt_2()
            .text_sm()
            .cursor_pointer()
            .text_color(rgb(ACCENT))
            .child(if self.more_open { "▾ More" } else { "▸ More" })
            .on_click(cx.listener(|this, _, _, cx| {
                this.more_open = !this.more_open;
                cx.notify();
            }));
        out.push(more.into_any_element());
        if self.more_open {
            out.push(slider_setting("Gamma", hints::GAMMA, &self.sliders.gamma, format!("{:.2}", a.gamma)));
            out.push(slider_setting("Hue", hints::HUE, &self.sliders.hue, format!("{:+.0}°", a.hue)));
        }
        let reset = Button::new("reset-picture").label("Reset picture").compact().on_click(cx.listener(|this, _, window, cx| {
            let picture = PictureSettings { adjustments: Adjustments::default(), ..this.model.picture };
            let mut model = this.model.clone();
            model.picture = picture;
            this.set_model(model, window, cx);
            cx.emit(DrawerEvent::Picture(picture));
        }));
        out.push(div().mt_3().child(reset).into_any_element());
        out
    }

    fn audio_tab(&self) -> Vec<AnyElement> {
        let delay = self.model.delay;
        let label = format!("{:+} ms", delay.av_sync_offset_ms);
        let warning = input_lag_warning(DelaySettings { buffering_frames: 0, ..delay }, self.model.frame_interval);
        vec![
            div()
                .child(slider_setting("A/V sync offset", hints::AV_SYNC, &self.sliders.av_sync, label))
                .children(warning.map(warn))
                .into_any_element(),
            note(hints::AUDIO_LATER),
        ]
    }

    fn reset_all(&self, cx: &mut Context<Self>) -> AnyElement {
        let body: AnyElement = if self.confirm_reset {
            div()
                .flex()
                .gap_2()
                .items_center()
                .child(div().text_sm().child("Forget all settings?"))
                .child(Button::new("reset-yes").label("Reset").compact().danger().on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_reset = false;
                    cx.emit(DrawerEvent::ResetAll);
                })))
                .child(Button::new("reset-no").label("Cancel").compact().ghost().on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_reset = false;
                    cx.notify();
                })))
                .into_any_element()
        } else {
            Button::new("reset-all")
                .label("Reset all settings")
                .compact()
                .ghost()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_reset = true;
                    cx.notify();
                }))
                .into_any_element()
        };
        div().pt_3().mt_3().border_t_1().border_color(rgba(LINE)).child(body).child(hint(hints::RESET_ALL)).into_any_element()
    }
}

impl Render for SettingsDrawer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = match self.tab {
            Tab::Source => self.source_tab(cx),
            Tab::Display => self.display_tab(cx),
            Tab::Colour => self.colour_tab(cx),
            Tab::Audio => self.audio_tab(),
        };
        let reset_all = self.reset_all(cx);
        let rail = self.rail(cx);
        div()
            .size_full()
            .flex()
            .text_color(rgb(TEXT))
            .bg(rgba(GLASS))
            .child(rail)
            .child(
                div()
                    .id("drawer-pane")
                    .flex_1()
                    .p_4()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .children(rows)
                    .child(reset_all),
            )
    }
}

// Colours from the chosen prototype: dark glass over the picture.
pub const GLASS: u32 = 0x14161bb8;
pub const LINE: u32 = 0xffffff1f;
pub const TEXT: u32 = 0xeef0f4;
pub const MUTED: u32 = 0x9aa1ad;
pub const ACCENT: u32 = 0x6aa9ff;
pub const WARN: u32 = 0xffcc66;

fn hint(text: &'static str) -> Div {
    div().mt_1().text_xs().text_color(rgb(MUTED)).child(text)
}

fn note(text: &'static str) -> AnyElement {
    div().text_xs().text_color(rgb(MUTED)).child(text).into_any_element()
}

fn warn(text: String) -> Div {
    div().mt_1().text_xs().text_color(rgb(WARN)).child(text)
}

fn setting(title: &'static str, hint_text: &'static str, control: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .child(div().mb_1().text_sm().font_weight(FontWeight::SEMIBOLD).child(title))
        .child(control)
        .child(hint(hint_text))
        .into_any_element()
}

fn slider_setting(title: &'static str, hint_text: &'static str, state: &Entity<SliderState>, value: String) -> AnyElement {
    let control = div().flex().items_center().gap_3().child(div().flex_1().child(Slider::new(state))).child(div().w(px(64.)).text_sm().text_right().child(value));
    setting(title, hint_text, control)
}

fn segmented(id: &'static str, labels: &[&str], selected: usize, on_click: impl Fn(&Vec<usize>, &mut Window, &mut App) + 'static) -> ButtonGroup {
    ButtonGroup::new(id)
        .compact()
        .children(labels.iter().enumerate().map(|(i, label)| Button::new((id, i)).label(label.to_string()).selected(i == selected)))
        .on_click(on_click)
}

fn choice_row(id: (&'static str, usize), label: &str, chosen: bool) -> Stateful<Div> {
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .when(chosen, |d| d.bg(rgba(0x6aa9ff33)).text_color(rgb(ACCENT)))
        .when(!chosen, |d| d.hover(|d| d.bg(rgba(0xffffff14))))
        .child(label.to_string())
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude has its own `test` attribute that would replace std's.
    use super::{DelaySettings, TargetRatio, input_lag_warning, preset_label};

    #[test]
    fn input_lag_warning_only_when_the_picture_waits() {
        let interval = 1.0 / 60.0;
        assert_eq!(input_lag_warning(DelaySettings::default(), interval), None);
        assert_eq!(input_lag_warning(DelaySettings { buffering_frames: 0, av_sync_offset_ms: 120 }, interval), None, "delays sound, not picture");
        assert_eq!(input_lag_warning(DelaySettings { buffering_frames: 1, av_sync_offset_ms: 0 }, interval).as_deref(), Some("+17 ms input lag"));
        assert_eq!(input_lag_warning(DelaySettings { buffering_frames: 0, av_sync_offset_ms: -40 }, interval).as_deref(), Some("+40 ms input lag"));
    }

    #[test]
    fn preset_labels() {
        assert_eq!(preset_label(TargetRatio(16.0 / 9.0)), Some("16:9"));
        assert_eq!(preset_label(TargetRatio(64.0 / 27.0)), Some("21:9"));
        assert_eq!(preset_label(TargetRatio(2.39)), None);
    }
}
