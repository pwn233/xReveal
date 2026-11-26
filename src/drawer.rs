//! The Settings drawer (docs/spec/v1.md §2.3): a panel over the live picture with a side rail of
//! four tabs. Every setting has an always-visible hint line and applies live.
//!
//! The drawer only shows and edits a snapshot ([`DrawerModel`]); every change goes out as a
//! [`DrawerEvent`] and the app applies it to the video pipeline and the Remembered setup. That
//! keeps it hostable in its own window too (Windows draws it in a separate transparent window).

use gpui_kit::base::IndexPath;
use gpui_kit::assets::IconName;
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::{Duration, Instant};

use crate::setup::{Corner, DrawerSide, StatsBox, WindowMode};
use crate::source::{self, SourceIdentity, SourceInfo, VideoFormat};
use crate::theme::{mono_family, palette::*};
use crate::video::picture::{Adjustments, Choice, ColourRange, ColourSpace, FillMode, PictureSettings, TargetRatio};
use crate::video::timing::DelaySettings;

pub const WIDTH: f32 = 380.0;
const RAIL: f32 = 72.0;
/// How long the drawer takes to slide in (decelerating) and out (accelerating away).
const SLIDE_IN: Duration = Duration::from_millis(320);
pub const SLIDE_OUT: Duration = Duration::from_millis(200);

/// The drawer sliding in from its edge or back out: how much of it shows, from 0 to 1. A slide
/// reversed half way carries on from where it is.
#[derive(Clone, Copy, Debug)]
pub struct Slide {
    started: Instant,
    from: f32,
    to: f32,
}

impl Slide {
    pub fn closed() -> Self {
        Self { started: Instant::now() - SLIDE_IN, from: 0.0, to: 0.0 }
    }

    /// Starts sliding open or closed from wherever it is now.
    pub fn toward(self, open: bool) -> Self {
        let now = Instant::now();
        Self { started: now, from: self.shown_at(now), to: if open { 1.0 } else { 0.0 } }
    }

    fn duration(&self) -> Duration {
        if self.to >= self.from { SLIDE_IN } else { SLIDE_OUT }
    }

    fn shown_at(&self, now: Instant) -> f32 {
        let t = (now.saturating_duration_since(self.started).as_secs_f32() / self.duration().as_secs_f32()).min(1.0);
        let eased = if self.to >= self.from { ease_out_quint()(t) } else { t * t * t };
        self.from + (self.to - self.from) * eased
    }

    /// Whether it is still moving (keep asking for frames).
    pub fn moving(&self) -> bool {
        self.started.elapsed() < self.duration()
    }

    /// Whether any of the drawer is on screen.
    pub fn visible(&self) -> bool {
        self.to > 0.0 || self.moving()
    }

    /// How far the drawer sits past its edge right now: 0 when fully open, -WIDTH when closed.
    pub fn offset(&self) -> Pixels {
        px(-WIDTH * (1.0 - self.shown_at(Instant::now())))
    }
}

/// What the drawer shows: a snapshot the app keeps up to date.
#[derive(Clone, Debug, Default)]
pub struct DrawerModel {
    pub sources: Vec<SourceInfo>,
    pub current: Option<(SourceIdentity, VideoFormat)>,
    /// The picture settings in effect, including the current Source's colour choices.
    pub picture: PictureSettings,
    pub delay: DelaySettings,
    pub side: DrawerSide,
    pub window_mode: WindowMode,
    pub stats: StatsBox,
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
    WindowMode(WindowMode),
    Stats(StatsBox),
    ResetAll,
    Quit,
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

    fn icon(self) -> IconName {
        match self {
            Tab::Source => IconName::Cable,
            Tab::Display => IconName::Monitor,
            Tab::Colour => IconName::Palette,
            Tab::Audio => IconName::Volume2,
        }
    }
}

/// Plain-language hint lines, one per setting (docs/spec/v1.md §2.3).
pub mod hints {
    pub const CARD: &str = "The capture card your console is plugged into. Black picture on a PS5? Turn off HDCP in its HDMI settings.";
    pub const VIDEO_FORMAT: &str = "What the card sends. Changing it restarts the picture for a moment.";
    pub const BUFFERING: &str = "Holds frames back to smooth uneven delivery. Off is fastest.";
    pub const WINDOW_MODE: &str = "Fullscreen covers the whole screen with no border. F11 switches between them.";
    pub const TARGET_RATIO: &str = "The shape of the picture on your screen. Pick your monitor's shape, like 21:9 for ultrawide, to fill it.";
    pub const FILL_MODE: &str = "How the console's 16:9 picture fits that shape: Fit adds black bars, Stretch widens everything, Crop zooms in, Smart stretch keeps the middle natural.";
    pub const STRENGTH: &str = "0% looks just like Stretch. Higher keeps the centre more natural and stretches the edges more.";
    pub const CUSTOM_RATIO: &str = "Width ÷ height of your own shape, like 2.39 for a cinema ratio";
    pub const DRAWER_SIDE: &str = "Which side this panel opens from";
    pub const STATS: &str = "Live timing over the picture. F3 shows or hides it too.";
    pub const STATS_CORNER: &str = "It moves aside when this panel opens on the same side";
    pub const STATS_SHOWS: &str = "What the box lists";
    pub const RANGE: &str = "Match your console's RGB range. Blacks look grey? Choose Limited, or set your console to Full.";
    pub const SPACE: &str = "The picture's colour formula. Auto picks BT.709 for HD pictures.";
    pub const BRIGHTNESS: &str = "Lifts or lowers the whole picture";
    pub const CONTRAST: &str = "Spreads darks and lights further apart";
    pub const SATURATION: &str = "How strong the colours are. 0% is black and white.";
    pub const GAMMA: &str = "Brightens or darkens the mid-tones without touching black and white";
    pub const HUE: &str = "Turns every colour around the colour wheel";
    pub const AV_SYNC: &str = "Lines sound up with the picture. Positive delays the sound, negative delays the picture.";
    pub const AUDIO_LATER: &str = "Sound playback, input and output choice, volume and mute arrive with the audio pipeline";
    pub const RESET_ALL: &str = "Puts every setting back to its default";
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

/// A Video format as an on-screen-menu readout: `1280 × 720 · 60`.
pub fn readout(format: VideoFormat) -> String {
    format!("{} × {} · {}", format.width, format.height, format.fps.round())
}

pub struct SettingsDrawer {
    model: DrawerModel,
    tab: Tab,
    more_open: bool,
    confirm_reset: bool,
    custom_ratio: bool,
    sliders: Sliders,
    card_select: Entity<SelectState<SearchableVec<String>>>,
    format_select: Entity<SelectState<SearchableVec<String>>>,
    /// What the Selects list, in order: OS IDs of the cards, and the Video formats of the current one.
    card_ids: Vec<String>,
    formats: Vec<VideoFormat>,
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
    pub fn new(model: DrawerModel, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let card_select = cx.new(|cx| SelectState::new(SearchableVec::new(Vec::<String>::new()), None, window, cx));
        let format_select = cx.new(|cx| SelectState::new(SearchableVec::new(Vec::<String>::new()), None, window, cx));

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
        subscriptions.push(cx.subscribe(&card_select, |this: &mut Self, select, event: &SelectEvent<SearchableVec<String>>, cx| {
            if let SelectEvent::Confirm(Some(_)) = event
                && let Some(index) = select.read(cx).selected_index(cx)
                && let Some(os_id) = this.card_ids.get(index.row).cloned()
                && this.model.current.as_ref().map(|(i, _)| &i.os_id) != Some(&os_id)
                && let Some(info) = this.model.sources.iter().find(|s| s.identity.os_id == os_id)
                && let Some(format) = source::default_choice(&info.formats)
            {
                cx.emit(DrawerEvent::Source(os_id, format));
            }
        }));
        subscriptions.push(cx.subscribe(&format_select, |this: &mut Self, select, event: &SelectEvent<SearchableVec<String>>, cx| {
            if let SelectEvent::Confirm(Some(_)) = event
                && let Some(index) = select.read(cx).selected_index(cx)
                && let Some(format) = this.formats.get(index.row).copied()
                && let Some((identity, current)) = this.model.current.clone()
                && current != format
            {
                cx.emit(DrawerEvent::Source(identity.os_id, format));
            }
        }));
        let custom_ratio = preset_label(p.target_ratio).is_none();
        let mut this = Self {
            model: DrawerModel::default(),
            tab: Tab::Source,
            more_open: false,
            confirm_reset: false,
            custom_ratio,
            sliders,
            card_select,
            format_select,
            card_ids: Vec::new(),
            formats: Vec::new(),
            _subscriptions: subscriptions,
        };
        this.set_model(model, window, cx);
        this
    }

    /// Replaces the snapshot (e.g. after a Source switch or a reset) and moves the controls to match.
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

        // Card list, with the one on screen selected.
        let current_id = model.current.as_ref().map(|(i, _)| i.os_id.clone());
        self.card_ids = model.sources.iter().map(|s| s.identity.os_id.clone()).collect();
        let names: Vec<String> = model.sources.iter().map(|s| s.identity.name.clone()).collect();
        let card_index = current_id.as_ref().and_then(|id| self.card_ids.iter().position(|c| c == id));
        self.card_select.update(cx, |s, cx| {
            s.set_items(SearchableVec::new(names), window, cx);
            s.set_selected_index(card_index.map(IndexPath::new), window, cx);
        });

        // That card's Video formats, with the one on screen selected.
        let info = current_id.as_ref().and_then(|id| model.sources.iter().find(|s| &s.identity.os_id == id));
        self.formats = info.map(|i| source::choices(&i.formats)).unwrap_or_default();
        let labels: Vec<String> = self.formats.iter().copied().map(readout).collect();
        let format_index = model.current.as_ref().and_then(|(_, f)| self.formats.iter().position(|c| c == f));
        self.format_select.update(cx, |s, cx| {
            s.set_items(SearchableVec::new(labels), window, cx);
            s.set_selected_index(format_index.map(IndexPath::new), window, cx);
        });

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
                .h(px(64.))
                .mx_2()
                .rounded(px(10.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .cursor_pointer()
                .text_xs()
                .when(on, |d| d.bg(rgb(RAISED)).text_color(rgb(TEXT)))
                .when(!on, |d| d.text_color(rgb(MUTED)).hover(|d| d.text_color(rgb(SECONDARY)).bg(rgba(0xededed0a))))
                .child(Icon::new(tab.icon()).size_5().text_color(if on { rgb(ACCENT) } else { rgb(MUTED) }))
                .child(tab.label())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = tab;
                    this.confirm_reset = false;
                    cx.notify();
                }))
        });
        div()
            .w(px(RAIL))
            .flex_none()
            .pt_3()
            .bg(rgb(DEEP))
            .border_r_1()
            .border_color(rgba(LINE))
            .flex()
            .flex_col()
            .gap_1()
            .children(tabs)
            .into_any_element()
    }

    /// The live readout at the top of the Source tab: what is on screen right now.
    fn live(&self) -> AnyElement {
        let Some((identity, format)) = &self.model.current else {
            return div().text_sm().text_color(rgb(SECONDARY)).child("No capture card on screen").into_any_element();
        };
        div()
            .p_3()
            .rounded(px(10.))
            .bg(rgb(RAISED))
            .border_1()
            .border_color(rgba(LINE))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .child(div().flex_none().size(px(8.)).rounded_full().bg(rgb(ACCENT)))
                    .child(div().min_w_0().truncate().child(identity.name.clone())),
            )
            .child(div().pl(px(16.)).text_xs().font_family(mono_family()).text_color(rgb(SECONDARY)).child(readout(*format)))
            .into_any_element()
    }

    fn source_tab(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let delay = self.model.delay;
        let buffering = chips(
            "buffering",
            &["Off", "1 frame", "2 frames"],
            delay.buffering_frames as usize,
            cx.listener(|this, i: &usize, _, cx| this.set_delay(DelaySettings { buffering_frames: *i as u8, ..this.model.delay }, cx)),
        );
        let warning = input_lag_warning(DelaySettings { av_sync_offset_ms: 0, ..delay }, self.model.frame_interval);
        vec![
            self.live(),
            setting("Capture card", hints::CARD, Select::new(&self.card_select).placeholder("No capture card").menu_max_h(rems(18.))),
            setting("Video format", hints::VIDEO_FORMAT, Select::new(&self.format_select).placeholder("No formats").menu_max_h(rems(18.))),
            setting("Buffering", hints::BUFFERING, div().flex().flex_col().gap_2().child(buffering).children(warning.map(warn))),
        ]
    }

    fn display_tab(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.model.picture;
        let mut labels: Vec<&str> = TargetRatio::PRESETS.iter().map(|(label, _)| *label).collect();
        labels.push("Custom");
        let selected = if self.custom_ratio {
            labels.len() - 1
        } else {
            preset_label(p.target_ratio).and_then(|l| labels.iter().position(|x| *x == l)).unwrap_or(1)
        };
        let ratios = chips(
            "ratio",
            &labels,
            selected,
            cx.listener(|this, i: &usize, _, cx| match TargetRatio::PRESETS.get(*i) {
                Some((_, ratio)) => {
                    this.custom_ratio = false;
                    this.set_picture(PictureSettings { target_ratio: *ratio, ..this.model.picture }, cx);
                }
                None => {
                    this.custom_ratio = true;
                    cx.notify();
                }
            }),
        );
        let modes = [WindowMode::Windowed, WindowMode::Fullscreen];
        let window_mode = chips(
            "window-mode",
            &["Windowed", "Fullscreen"],
            modes.iter().position(|m| *m == self.model.window_mode).unwrap_or(0),
            cx.listener(move |this, i: &usize, _, cx| {
                if modes[*i] != this.model.window_mode {
                    this.model.window_mode = modes[*i];
                    cx.emit(DrawerEvent::WindowMode(modes[*i]));
                    cx.notify();
                }
            }),
        );
        let mut out = vec![setting("Window", hints::WINDOW_MODE, window_mode), setting("Target ratio", hints::TARGET_RATIO, ratios)];
        if self.custom_ratio {
            out.push(slider_setting("Custom ratio", hints::CUSTOM_RATIO, &self.sliders.custom_ratio, format!("{:.2}", p.target_ratio.0)));
        }
        let modes = [FillMode::Fit, FillMode::Stretch, FillMode::Crop, FillMode::SmartStretch];
        let fill = chips(
            "fill",
            &["Fit", "Stretch", "Crop", "Smart stretch"],
            modes.iter().position(|m| *m == p.fill_mode).unwrap_or(0),
            cx.listener(move |this, i: &usize, _, cx| this.set_picture(PictureSettings { fill_mode: modes[*i], ..this.model.picture }, cx)),
        );
        out.push(setting("Fill mode", hints::FILL_MODE, fill));
        if p.fill_mode == FillMode::SmartStretch {
            let label = format!("{:.0}%", p.smart_stretch_strength * 100.0);
            out.push(slider_setting("Smart stretch strength", hints::STRENGTH, &self.sliders.strength, label));
        }
        let sides = [DrawerSide::Left, DrawerSide::Right];
        let side = chips(
            "side",
            &["Left", "Right"],
            sides.iter().position(|s| *s == self.model.side).unwrap_or(1),
            cx.listener(move |this, i: &usize, _, cx| {
                if sides[*i] != this.model.side {
                    this.model.side = sides[*i];
                    cx.emit(DrawerEvent::Side(sides[*i]));
                    cx.notify();
                }
            }),
        );
        out.push(setting("Drawer side", hints::DRAWER_SIDE, side));
        out.push(self.stats_settings(cx));
        out
    }

    /// The performance box: on or off, which corner, and what it lists (one section).
    fn stats_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let stats = self.model.stats;
        let shown = chips(
            "stats-shown",
            &["Off", "On"],
            stats.shown as usize,
            cx.listener(|this, i: &usize, _, cx| this.set_stats(StatsBox { shown: *i == 1, ..this.model.stats }, cx)),
        );
        let mut section = div().flex().flex_col().gap_6().child(setting("Performance box", hints::STATS, shown));
        if stats.shown {
            let corner = chips(
                "stats-corner",
                &["Top left", "Top right", "Bottom left", "Bottom right"],
                Corner::ALL.iter().position(|c| *c == stats.corner).unwrap_or(0),
                cx.listener(|this, i: &usize, _, cx| this.set_stats(StatsBox { corner: Corner::ALL[*i], ..this.model.stats }, cx)),
            );
            let shows = toggles(
                "stats-shows",
                &["Latency", "Frame rate", "Dropped frames", "Video format"],
                &[stats.latency, stats.fps, stats.frames, stats.format],
                cx.listener(|this, i: &usize, _, cx| {
                    let mut stats = this.model.stats;
                    let flag = match i {
                        0 => &mut stats.latency,
                        1 => &mut stats.fps,
                        2 => &mut stats.frames,
                        _ => &mut stats.format,
                    };
                    *flag = !*flag;
                    this.set_stats(stats, cx);
                }),
            );
            section = section.child(setting("Corner", hints::STATS_CORNER, corner)).child(setting("Shows", hints::STATS_SHOWS, shows));
        }
        section.into_any_element()
    }

    fn set_stats(&mut self, stats: StatsBox, cx: &mut Context<Self>) {
        if stats != self.model.stats {
            self.model.stats = stats;
            cx.emit(DrawerEvent::Stats(stats));
            cx.notify();
        }
    }

    fn colour_tab(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.model.picture;
        let ranges = [Choice::Auto, Choice::Manual(ColourRange::Full), Choice::Manual(ColourRange::Limited)];
        let range = chips(
            "range",
            &["Auto", "Full", "Limited"],
            ranges.iter().position(|r| *r == p.colour_range).unwrap_or(0),
            cx.listener(move |this, i: &usize, _, cx| this.set_picture(PictureSettings { colour_range: ranges[*i], ..this.model.picture }, cx)),
        );
        let spaces = [Choice::Auto, Choice::Manual(ColourSpace::Bt709), Choice::Manual(ColourSpace::Bt601)];
        let space = chips(
            "space",
            &["Auto", "BT.709", "BT.601"],
            spaces.iter().position(|s| *s == p.colour_space).unwrap_or(0),
            cx.listener(move |this, i: &usize, _, cx| this.set_picture(PictureSettings { colour_space: spaces[*i], ..this.model.picture }, cx)),
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
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(rgb(SECONDARY))
            .cursor_pointer()
            .hover(|d| d.text_color(rgb(TEXT)))
            .child(Icon::new(if self.more_open { IconName::ChevronDown } else { IconName::ChevronRight }).size_4())
            .child("More")
            .on_click(cx.listener(|this, _, _, cx| {
                this.more_open = !this.more_open;
                cx.notify();
            }));
        let extra = self.more_open.then(|| {
            div()
                .flex()
                .flex_col()
                .gap_6()
                .child(slider_setting("Gamma", hints::GAMMA, &self.sliders.gamma, format!("{:.2}", a.gamma)))
                .child(slider_setting("Hue", hints::HUE, &self.sliders.hue, format!("{:+.0}°", a.hue)))
        });
        out.push(div().flex().flex_col().gap_6().child(more).children(extra).into_any_element());
        out
    }

    fn audio_tab(&self) -> Vec<AnyElement> {
        let delay = self.model.delay;
        let label = format!("{:+} ms", delay.av_sync_offset_ms);
        let warning = input_lag_warning(DelaySettings { buffering_frames: 0, ..delay }, self.model.frame_interval);
        vec![
            div()
                .flex()
                .flex_col()
                .gap_2()
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
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(div().text_sm().mr_1().child("Reset every setting?"))
                .child(
                    div()
                        .id("reset-yes")
                        .h(px(30.))
                        .px_3()
                        .rounded(px(8.))
                        .flex()
                        .items_center()
                        .text_sm()
                        .cursor_pointer()
                        .bg(rgb(DANGER))
                        .text_color(rgb(0xffffff))
                        .child("Reset")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_reset = false;
                            cx.emit(DrawerEvent::ResetAll);
                        })),
                )
                .child(quiet_button("reset-no", IconName::X, "Cancel").on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_reset = false;
                    cx.notify();
                })))
                .into_any_element()
        } else {
            let link = |id: &'static str, label: &'static str| {
                div().id(id).text_sm().text_color(rgb(SECONDARY)).cursor_pointer().hover(|d| d.text_color(rgb(TEXT))).child(label)
            };
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(link("reset-all", "Reset all settings").on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_reset = true;
                    cx.notify();
                })))
                .child(link("quit", "Quit xReveal").on_click(cx.listener(|_, _, _, cx| cx.emit(DrawerEvent::Quit))))
                .into_any_element()
        };
        div()
            .flex_none()
            .px_5()
            .py_4()
            .border_t_1()
            .border_color(rgba(LINE))
            .flex()
            .flex_col()
            .gap_1()
            .child(body)
            .when(!self.confirm_reset, |d| d.child(hint(hints::RESET_ALL)))
            .into_any_element()
    }

    /// Whether this tab's settings are all at their defaults (nothing for its Reset to do).
    fn tab_is_default(&self, tab: Tab) -> bool {
        let (p, d) = (self.model.picture, PictureSettings::default());
        match tab {
            Tab::Source => self.model.delay.buffering_frames == 0 && self.default_format().is_none(),
            Tab::Display => {
                p.target_ratio == d.target_ratio
                    && p.fill_mode == d.fill_mode
                    && p.smart_stretch_strength == d.smart_stretch_strength
                    && self.model.side == DrawerSide::default()
                    && self.model.window_mode == WindowMode::default()
                    && self.model.stats == StatsBox::default()
                    && !self.custom_ratio
            }
            Tab::Colour => p.colour_range == d.colour_range && p.colour_space == d.colour_space && p.adjustments == d.adjustments,
            Tab::Audio => self.model.delay.av_sync_offset_ms == 0,
        }
    }

    /// The current card's default Video format, when something else is on screen.
    fn default_format(&self) -> Option<(String, VideoFormat)> {
        let (identity, current) = self.model.current.as_ref()?;
        let info = self.model.sources.iter().find(|s| s.identity.os_id == identity.os_id)?;
        let default = source::default_choice(&info.formats)?;
        (default != *current).then(|| (identity.os_id.clone(), default))
    }

    /// Puts this tab's settings back to their defaults.
    fn reset_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let defaults = PictureSettings::default();
        let mut model = self.model.clone();
        let mut events = Vec::new();
        match self.tab {
            Tab::Source => {
                model.delay.buffering_frames = 0;
                events.push(DrawerEvent::Delay(model.delay));
                if let Some((os_id, format)) = self.default_format() {
                    events.push(DrawerEvent::Source(os_id, format));
                }
            }
            Tab::Display => {
                self.custom_ratio = false;
                model.picture.target_ratio = defaults.target_ratio;
                model.picture.fill_mode = defaults.fill_mode;
                model.picture.smart_stretch_strength = defaults.smart_stretch_strength;
                events.push(DrawerEvent::Picture(model.picture));
                if model.side != DrawerSide::default() {
                    model.side = DrawerSide::default();
                    events.push(DrawerEvent::Side(model.side));
                }
                if model.stats != StatsBox::default() {
                    model.stats = StatsBox::default();
                    events.push(DrawerEvent::Stats(model.stats));
                }
                if model.window_mode != WindowMode::default() {
                    model.window_mode = WindowMode::default();
                    events.push(DrawerEvent::WindowMode(model.window_mode));
                }
            }
            Tab::Colour => {
                model.picture.colour_range = defaults.colour_range;
                model.picture.colour_space = defaults.colour_space;
                model.picture.adjustments = Adjustments::default();
                events.push(DrawerEvent::Picture(model.picture));
            }
            Tab::Audio => {
                model.delay.av_sync_offset_ms = 0;
                events.push(DrawerEvent::Delay(model.delay));
            }
        }
        self.set_model(model, window, cx);
        for event in events {
            cx.emit(event);
        }
    }

    /// The tab's title, with a Reset for just this tab.
    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let idle = self.tab_is_default(self.tab);
        let reset = div()
            .id("reset-tab")
            .h(px(28.))
            .px_2()
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_xs()
            .when(idle, |d| d.text_color(rgba(0x85858b80)))
            .when(!idle, |d| {
                d.text_color(rgb(SECONDARY))
                    .cursor_pointer()
                    .hover(|d| d.bg(rgb(RAISED)).text_color(rgb(TEXT)))
                    .on_click(cx.listener(|this, _, window, cx| this.reset_tab(window, cx)))
            })
            .child(Icon::new(IconName::RotateCcw).size_3p5())
            .child("Reset");
        div()
            .flex_none()
            .px_5()
            .pt_5()
            .pb_1()
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_base().font_weight(FontWeight::MEDIUM).child(self.tab.label()))
            .child(reset)
            .into_any_element()
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
        let header = self.header(cx);
        let reset_all = self.reset_all(cx);
        let rail = self.rail(cx);
        // Each setting is a section, divided from the next by a hairline.
        let sections = rows.into_iter().enumerate().map(|(i, row)| {
            div().w_full().py_5().when(i > 0, |d| d.border_t_1().border_color(rgba(LINE))).child(row)
        });
        div()
            .size_full()
            .flex()
            .text_color(rgb(TEXT))
            .bg(rgba(PANEL))
            .child(rail)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(
                        div()
                            .id("drawer-pane")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(div().w_full().px_5().flex().flex_col().children(sections)),
                    )
                    .child(reset_all),
            )
    }
}

fn hint(text: &'static str) -> Div {
    div().w_full().text_xs().line_height(px(16.)).text_color(rgb(MUTED)).child(text)
}

fn note(text: &'static str) -> AnyElement {
    hint(text).into_any_element()
}

fn warn(text: String) -> Div {
    div().text_xs().font_family(mono_family()).text_color(rgb(WARN)).child(text)
}

/// Title, control, hint: one setting.
fn setting(title: &'static str, hint_text: &'static str, control: impl IntoElement) -> AnyElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
        .child(control)
        .child(hint(hint_text))
        .into_any_element()
}

fn slider_setting(title: &'static str, hint_text: &'static str, state: &Entity<SliderState>, value: String) -> AnyElement {
    let control = div()
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .child(div().flex_1().min_w_0().child(Slider::new(state)))
        .child(div().w(px(56.)).flex_none().text_xs().font_family(mono_family()).text_right().text_color(rgb(SECONDARY)).child(value));
    setting(title, hint_text, control)
}

/// A group of choices that wraps onto a second row instead of overflowing.
fn chips(id: &'static str, labels: &[&str], selected: usize, on_pick: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Div {
    let on_pick = std::rc::Rc::new(on_pick);
    div().w_full().flex().flex_wrap().gap(px(6.)).children(labels.iter().enumerate().map(|(i, label)| {
        let chosen = i == selected;
        let on_pick = on_pick.clone();
        div()
            .id((id, i))
            .h(px(30.))
            .px_3()
            .rounded(px(8.))
            .flex()
            .items_center()
            .text_sm()
            .cursor_pointer()
            .border_1()
            .when(chosen, |d| d.bg(rgba(ACCENT_FILL)).border_color(rgba(0x6aa9ff66)).text_color(rgb(ACCENT)))
            .when(!chosen, |d| d.bg(rgb(RAISED)).border_color(rgba(LINE)).text_color(rgb(SECONDARY)).hover(|d| d.bg(rgb(RAISED_HOVER)).text_color(rgb(TEXT))))
            .child(label.to_string())
            .on_click(move |_, window, cx| on_pick(&i, window, cx))
    }))
}

/// Like [`chips`], but each one switches on and off by itself.
fn toggles(id: &'static str, labels: &[&str], on: &[bool], on_toggle: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Div {
    let on_toggle = std::rc::Rc::new(on_toggle);
    div().w_full().flex().flex_wrap().gap(px(6.)).children(labels.iter().enumerate().map(|(i, label)| {
        let chosen = on.get(i).copied().unwrap_or(false);
        let on_toggle = on_toggle.clone();
        div()
            .id((id, i))
            .h(px(30.))
            .px_3()
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_sm()
            .cursor_pointer()
            .border_1()
            .when(chosen, |d| d.bg(rgba(ACCENT_FILL)).border_color(rgba(0x6aa9ff66)).text_color(rgb(ACCENT)).child(Icon::new(IconName::Check).size_3p5()))
            .when(!chosen, |d| d.bg(rgb(RAISED)).border_color(rgba(LINE)).text_color(rgb(SECONDARY)).hover(|d| d.bg(rgb(RAISED_HOVER)).text_color(rgb(TEXT))))
            .child(label.to_string())
            .on_click(move |_, window, cx| on_toggle(&i, window, cx))
    }))
}

fn quiet_button(id: &'static str, icon: IconName, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(30.))
        .px_3()
        .rounded(px(8.))
        .flex()
        .items_center()
        .gap_2()
        .text_sm()
        .cursor_pointer()
        .bg(rgb(RAISED))
        .border_1()
        .border_color(rgba(LINE))
        .text_color(rgb(SECONDARY))
        .hover(|d| d.bg(rgb(RAISED_HOVER)).text_color(rgb(TEXT)))
        .child(Icon::new(icon).size_4())
        .child(label)
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude has its own `test` attribute that would replace std's.
    use super::{DelaySettings, SLIDE_IN, SLIDE_OUT, Slide, TargetRatio, VideoFormat, input_lag_warning, preset_label, readout};
    use std::time::Duration;

    #[test]
    fn the_drawer_slides_in_and_back_out_from_where_it_is() {
        let closed = Slide::closed();
        assert!(!closed.visible());
        let opening = closed.toward(true);
        let start = opening.started;
        assert_eq!(opening.shown_at(start), 0.0);
        assert!(opening.shown_at(start + SLIDE_IN / 4) > 0.5, "decelerates: most of the way early");
        assert_eq!(opening.shown_at(start + SLIDE_IN), 1.0);

        // Closed half way through opening: it slides back from where it was, not from fully open.
        let mut closing = opening;
        let midway = start + SLIDE_IN / 4;
        closing = Slide { started: midway, from: closing.shown_at(midway), to: 0.0 };
        assert!(closing.shown_at(midway) < 1.0);
        assert_eq!(closing.shown_at(midway + SLIDE_OUT + Duration::from_millis(1)), 0.0);
    }

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

    #[test]
    fn formats_read_like_an_on_screen_menu() {
        assert_eq!(readout(VideoFormat { width: 1280, height: 720, fps: 60.0 }), "1280 × 720 · 60");
        assert_eq!(readout(VideoFormat { width: 1920, height: 1080, fps: 59.94 }), "1920 × 1080 · 60");
    }
}
