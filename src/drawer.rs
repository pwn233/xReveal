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

mod meter;
mod nav;

pub use nav::bind_keys;

use crate::audio::AudioModel;
use crate::permission::{Access, Privacy};
use crate::setup::{Accent, Corner, Density, DrawerSide, Look, StatsBox, ThemeChoice, WindowMode};
use crate::source::{self, SourceIdentity, SourceInfo, VideoFormat};
use crate::theme::{self, mono_family};
use crate::video::picture::{Adjustments, Choice, ColourRange, ColourSpace, FillMode, PictureSettings, TargetRatio};
use crate::video::timing::DelaySettings;

/// The drawer's width, which follows Density.
pub fn width() -> f32 {
    theme::spacing().drawer
}
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

    /// Fully shown, not moving.
    pub fn opened() -> Self {
        Self { started: Instant::now() - SLIDE_IN, from: 1.0, to: 1.0 }
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

    /// How much shows right now, 0 to 1.
    pub fn shown(&self) -> f32 {
        self.shown_at(Instant::now())
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
        px(-width() * (1.0 - self.shown_at(Instant::now())))
    }
}

/// What the drawer shows: a snapshot the app keeps up to date.
#[derive(Clone, Debug, Default)]
pub struct DrawerModel {
    pub sources: Vec<SourceInfo>,
    pub current: Option<(SourceIdentity, VideoFormat)>,
    /// The device on screen offers its own settings window (Windows drivers).
    pub device_settings: bool,
    /// The picture settings in effect, including the current Source's colour choices.
    pub picture: PictureSettings,
    pub delay: DelaySettings,
    pub side: DrawerSide,
    pub window_mode: WindowMode,
    /// Every screen (its identity and the name to show), and the one xReveal is on.
    pub monitors: Vec<(String, String)>,
    pub monitor: Option<String>,
    pub stats: StatsBox,
    pub look: Look,
    pub audio: AudioModel,
    /// The Source's frame interval in seconds, to say how much Buffering costs.
    pub frame_interval: f64,
    /// Camera and Microphone access: without it the inputs are listed locked (#113).
    pub camera: Access,
    pub microphone: Access,
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
    /// Show xReveal on this screen (by its identity).
    Monitor(String),
    /// Open the capture device's own settings window.
    DeviceSettings,
    /// "None" in the Input list: show no input.
    NoInput,
    Stats(StatsBox),
    Look(Look),
    /// The sound input: `None` follows the card (its own audio).
    AudioInput(Option<String>),
    /// The sound output: `None` is the system output.
    AudioOutput(Option<String>),
    Volume(f32),
    Mute(bool),
    /// An **Allow access** button on a locked input (#113).
    Allow(Privacy),
    ResetAll,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Source,
    Display,
    Colour,
    Audio,
    Interface,
}

impl Tab {
    const ALL: [Tab; 5] = [Tab::Source, Tab::Display, Tab::Colour, Tab::Audio, Tab::Interface];

    fn label(self) -> &'static str {
        match self {
            Tab::Source => "Source",
            Tab::Display => "Display",
            Tab::Colour => "Colour",
            Tab::Audio => "Audio",
            Tab::Interface => "Interface",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Tab::Source => IconName::Cable,
            Tab::Display => IconName::Monitor,
            Tab::Colour => IconName::Palette,
            Tab::Audio => IconName::Volume2,
            Tab::Interface => IconName::SunMoon,
        }
    }
}

/// Plain-language hint lines, one per setting (docs/spec/v1.md §2.3).
pub mod hints {
    pub const CARD: &str = "The device your console is plugged into. Black picture on a PS5? Turn off HDCP in its HDMI settings.";
    pub const VIDEO_FORMAT: &str = "What the input sends. Changing it restarts the picture for a moment.";
    pub const DEVICE_SETTINGS: &str = "The capture device's own settings window, from its driver";
    pub const BUFFERING: &str = "Holds frames back to smooth uneven delivery. Off is fastest.";
    pub const MONITOR: &str = "Which screen xReveal shows on. A window moves there; Fullscreen covers it.";
    pub const WINDOW_MODE: &str =
        "Borderless keeps the window's size with no border, centred on the screen. Fullscreen covers the whole screen. F11 switches to Fullscreen and back.";
    pub const TARGET_RATIO: &str = "The shape of the picture on your screen. Pick your monitor's shape, like 21:9 for ultrawide, to fill it.";
    pub const FILL_MODE: &str = "How the console's 16:9 picture fits that shape";
    pub const STRENGTH: &str = "0% looks just like Stretch. Higher keeps the centre more natural and stretches the edges more.";
    pub const CUSTOM_RATIO: &str = "Width ÷ height of your own shape, like 2.39 for a cinema ratio";
    pub const DRAWER_SIDE: &str = "Which side this panel opens from";
    pub const THEME: &str = "How xReveal's panels look. System follows your computer's light or dark mode.";
    pub const ACCENT: &str = "The colour that marks what's selected";
    pub const OPACITY: &str = "How much of the game shows through this panel";
    pub const STATS_OPACITY: &str = "How much of the game shows through the box";
    pub const DENSITY: &str = "How tightly the settings are packed";
    pub const STATS: &str = "Live timing over the picture. Click a corner to show it there, or the middle to hide it. F3 shows or hides it too.";
    pub const STATS_SHOWS: &str = "What the box lists";
    pub const RANGE: &str = "Match your console's RGB range. Blacks look grey? Choose Limited, or set your console to Full.";
    pub const SPACE: &str = "The picture's colour formula. Auto picks BT.709 for HD pictures.";
    pub const BRIGHTNESS: &str = "Lifts or lowers the whole picture";
    pub const CONTRAST: &str = "Spreads darks and lights further apart";
    pub const SATURATION: &str = "How strong the colours are. 0% is black and white.";
    pub const GAMMA: &str = "Brightens or darkens the mid-tones without touching black and white";
    pub const HUE: &str = "Turns every colour around the colour wheel";
    pub const AV_SYNC: &str = "Lines sound up with the picture. Positive delays the sound, negative delays the picture.";
    pub const AUDIO_INPUT: &str = "Where the sound comes from. Normally the input device's own audio.";
    pub const CARD_LOCKED: &str = "xReveal needs Camera access to show any input. macOS asks once; after that it's in System Settings › Privacy & Security › Camera.";
    pub const AUDIO_INPUT_LOCKED: &str =
        "xReveal needs Microphone access to play the input's sound. macOS asks once; after that it's in System Settings › Privacy & Security › Microphone.";
    pub const AUDIO_OUTPUT: &str = "Where you hear it. System output follows your computer's sound setting.";
    pub const VOLUME: &str = "xReveal's own volume, on top of your computer's";
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

/// A sound input's name in the Input list. The card's own audio is marked first, so the mark
/// still shows when a long name is cut off: `Input's own audio · Digital Audio Interface (…`.
pub fn input_label(name: &str, own: bool) -> String {
    if own { format!("Input's own audio · {name}") } else { name.to_owned() }
}

pub struct SettingsDrawer {
    model: DrawerModel,
    tab: Tab,
    more_open: bool,
    /// Arrow-key navigation: the row under the cursor, the switch under it in a row of switches,
    /// and whether the keyboard is in use (the highlight shows only then).
    cursor: usize,
    switch: usize,
    keyboard: bool,
    scroll: ScrollHandle,
    focus: FocusHandle,
    /// The Input level meter (Audio tab), fed while the drawer is open.
    meter: meter::Meter,
    confirm_reset: bool,
    custom_ratio: bool,
    sliders: Sliders,
    card_select: Entity<SelectState<SearchableVec<String>>>,
    format_select: Entity<SelectState<SearchableVec<String>>>,
    input_select: Entity<SelectState<SearchableVec<String>>>,
    output_select: Entity<SelectState<SearchableVec<String>>>,
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
    opacity: Entity<SliderState>,
    volume: Entity<SliderState>,
    stats_opacity: Entity<SliderState>,
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
            volume: slider(cx, 0.0, 100.0, 1.0, model.audio.volume * 100.0),
            stats_opacity: slider(cx, StatsBox::OPACITY.0 * 100.0, StatsBox::OPACITY.1 * 100.0, 1.0, model.stats.opacity() * 100.0),
            opacity: slider(cx, Look::OPACITY.0 * 100.0, Look::OPACITY.1 * 100.0, 1.0, model.look.opacity() * 100.0),
        };
        let card_select = cx.new(|cx| SelectState::new(SearchableVec::new(Vec::<String>::new()), None, window, cx));
        let format_select = cx.new(|cx| SelectState::new(SearchableVec::new(Vec::<String>::new()), None, window, cx));
        let input_select = cx.new(|cx| SelectState::new(SearchableVec::new(Vec::<String>::new()), None, window, cx));
        let output_select = cx.new(|cx| SelectState::new(SearchableVec::new(Vec::<String>::new()), None, window, cx));

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
        subscriptions.push(cx.subscribe(&sliders.stats_opacity, |this: &mut Self, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(SliderValue::Single(value)) = event {
                this.set_stats(StatsBox { opacity: value.round() / 100.0, ..this.model.stats }, cx);
            }
        }));
        subscriptions.push(cx.subscribe(&sliders.volume, |this: &mut Self, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(SliderValue::Single(value)) = event {
                let volume = value.round() / 100.0;
                if volume != this.model.audio.volume {
                    this.model.audio.volume = volume;
                    cx.emit(DrawerEvent::Volume(volume));
                    cx.notify();
                }
            }
        }));
        subscriptions.push(cx.subscribe(&input_select, |this: &mut Self, select, event: &SelectEvent<SearchableVec<String>>, cx| {
            if let SelectEvent::Confirm(Some(_)) = event
                && let Some(index) = select.read(cx).selected_index(cx)
                && let Some(device) = this.model.audio.inputs.get(index.row)
            {
                // Choosing the card's own audio goes back to following the card.
                let chosen = (this.model.audio.paired.as_ref() != Some(&device.id)).then(|| device.id.clone());
                if chosen != this.model.audio.chosen_input {
                    this.model.audio.chosen_input = chosen.clone();
                    cx.emit(DrawerEvent::AudioInput(chosen));
                }
            }
        }));
        subscriptions.push(cx.subscribe(&output_select, |this: &mut Self, select, event: &SelectEvent<SearchableVec<String>>, cx| {
            if let SelectEvent::Confirm(Some(_)) = event
                && let Some(index) = select.read(cx).selected_index(cx)
            {
                let chosen = index.row.checked_sub(1).and_then(|i| this.model.audio.outputs.get(i)).map(|d| d.id.clone());
                if chosen != this.model.audio.output {
                    this.model.audio.output = chosen.clone();
                    cx.emit(DrawerEvent::AudioOutput(chosen));
                }
            }
        }));
        subscriptions.push(cx.subscribe(&sliders.opacity, |this: &mut Self, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(SliderValue::Single(value)) = event {
                this.set_look(Look { opacity: value.round() / 100.0, ..this.model.look }, cx);
            }
        }));
        subscriptions.push(cx.subscribe(&card_select, |this: &mut Self, select, event: &SelectEvent<SearchableVec<String>>, cx| {
            let Some(row) = matches!(event, SelectEvent::Confirm(Some(_))).then(|| select.read(cx).selected_index(cx)).flatten().map(|i| i.row) else { return };
            // Row 0 is "None"; the inputs follow.
            if row == 0 {
                if this.model.current.is_some() {
                    cx.emit(DrawerEvent::NoInput);
                }
            } else if let Some(os_id) = this.card_ids.get(row - 1).cloned()
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
            cursor: 0,
            switch: 0,
            keyboard: false,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            meter: meter::Meter::default(),
            confirm_reset: false,
            custom_ratio,
            sliders,
            card_select,
            format_select,
            input_select,
            output_select,
            card_ids: Vec::new(),
            formats: Vec::new(),
            _subscriptions: subscriptions,
        };
        this.set_model(model, window, cx);
        this
    }

    /// The tab showing.
    pub fn tab(&self) -> Tab {
        self.tab
    }

    /// Switches to `tab` (e.g. Source, for "Pick another input").
    pub fn show_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.confirm_reset = false;
        self.cursor = 0;
        self.switch = 0;
        cx.notify();
    }

    /// A new reading for the Input level meter: the loudest samples since the last, or `None`
    /// when nothing plays. Redraws only while the Audio tab shows.
    pub fn set_peaks(&mut self, peaks: Option<(f32, f32)>, cx: &mut Context<Self>) {
        self.meter.update(peaks);
        if self.tab == Tab::Audio {
            cx.notify();
        }
    }

    /// Focus this to give the drawer the arrow keys.
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
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
            (&self.sliders.opacity, model.look.opacity() * 100.0),
            (&self.sliders.volume, model.audio.volume * 100.0),
            (&self.sliders.stats_opacity, model.stats.opacity() * 100.0),
        ];
        for (state, value) in values {
            state.update(cx, |s, cx| s.set_value(value, window, cx));
        }

        // Card list, with the one on screen selected.
        let current_id = model.current.as_ref().map(|(i, _)| i.os_id.clone());
        self.card_ids = model.sources.iter().map(|s| s.identity.os_id.clone()).collect();
        let names: Vec<String> = std::iter::once("None".to_owned()).chain(model.sources.iter().map(|s| s.identity.name.clone())).collect();
        let card_index = Some(current_id.as_ref().and_then(|id| self.card_ids.iter().position(|c| c == id)).map_or(0, |i| i + 1));
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

        // Sound inputs (the card's own audio marked) and outputs (the system output first).
        let audio = &model.audio;
        let input_names: Vec<String> = audio
            .inputs
            .iter()
            .map(|d| input_label(&d.name, audio.paired.as_ref() == Some(&d.id)))
            .collect();
        // What you chose, else the input device's own audio, else whatever plays: a choice made
        // while nothing plays (no input on screen) still shows as chosen.
        let shown_input = audio.chosen_input.as_ref().or(audio.paired.as_ref()).or(audio.input.as_ref());
        let input_index = shown_input.and_then(|id| audio.inputs.iter().position(|d| &d.id == id));
        self.input_select.update(cx, |s, cx| {
            s.set_items(SearchableVec::new(input_names), window, cx);
            s.set_selected_index(input_index.map(IndexPath::new), window, cx);
        });
        let output_names: Vec<String> = std::iter::once("System output".to_owned()).chain(audio.outputs.iter().map(|d| d.name.clone())).collect();
        let output_index = match &audio.output {
            None => Some(0),
            Some(id) => audio.outputs.iter().position(|d| &d.id == id).map(|i| i + 1),
        };
        self.output_select.update(cx, |s, cx| {
            s.set_items(SearchableVec::new(output_names), window, cx);
            s.set_selected_index(output_index.map(IndexPath::new), window, cx);
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
        let pal = theme::palette();
        let tabs = Tab::ALL.into_iter().map(|tab| {
            let on = tab == self.tab;
            div()
                .id(SharedString::from(tab.label()))
                // Square, centred in the rail; labels stay on one line.
                .size(px(theme::spacing().rail - 8.))
                .whitespace_nowrap()
                .mx_auto()
                .rounded(px(10.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .cursor_pointer()
                .text_xs()
                .when(on, |d| d.bg(pal.raised).text_color(pal.text))
                .when(!on, |d| d.text_color(pal.muted).hover(|d| d.text_color(pal.secondary).bg(pal.wash)))
                .child(Icon::new(tab.icon()).size_5().text_color(if on { pal.accent } else { pal.muted }))
                .child(tab.label())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = tab;
                    this.confirm_reset = false;
                    cx.notify();
                }))
        });
        let left = self.model.side == DrawerSide::Left;
        div()
            .w(px(theme::spacing().rail))
            .flex_none()
            .pt_3()
            .bg(pal.deep)
            // The rail sits on the drawer's outer edge, so its inner border faces the content.
            .when(left, |d| d.border_r_1())
            .when(!left, |d| d.border_l_1())
            .border_color(pal.line)
            .flex()
            .flex_col()
            .gap_1()
            .children(tabs)
            .into_any_element()
    }

    /// The live readout at the top of the Source tab: what is on screen right now.
    fn live(&self) -> AnyElement {
        let pal = theme::palette();
        let Some((identity, format)) = &self.model.current else {
            return div().text_sm().text_color(pal.secondary).child("No input on screen").into_any_element();
        };
        div()
            .p_3()
            .rounded(px(10.))
            .bg(pal.raised)
            .border_1()
            .border_color(pal.line)
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .child(div().flex_none().size(px(8.)).rounded_full().bg(pal.accent))
                    .child(div().min_w_0().truncate().child(identity.name.clone())),
            )
            .child(div().pl(px(16.)).text_xs().font_family(mono_family()).text_color(pal.secondary).child(readout(*format)))
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
        let input = if self.model.camera == Access::Granted {
            setting("Input", hints::CARD, Select::new(&self.card_select).placeholder("No input").menu_max_h(rems(18.)))
        } else {
            let names = self.model.sources.iter().map(|s| s.identity.name.clone()).collect();
            setting("Input", hints::CARD_LOCKED, locked_inputs(Privacy::Camera, self.model.camera, names, cx))
        };
        let mut rows = vec![
            self.live(),
            input,
            setting("Video format", hints::VIDEO_FORMAT, Select::new(&self.format_select).placeholder("No formats").menu_max_h(rems(18.))),
            setting("Buffering", hints::BUFFERING, div().flex().flex_col().gap_2().child(buffering).children(warning.map(warn))),
        ];
        if self.model.device_settings {
            let open = quiet_button("device-settings", IconName::Settings, "Device settings…").on_click(cx.listener(|_, _, _, cx| cx.emit(DrawerEvent::DeviceSettings)));
            rows.push(setting("Device settings…", hints::DEVICE_SETTINGS, div().flex().child(open)));
        }
        rows
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
        let modes = WindowMode::OFFERED;
        let mode_labels: Vec<&str> = modes.iter().map(|m| m.label()).collect();
        let window_mode = chips(
            "window-mode",
            &mode_labels,
            modes.iter().position(|m| *m == self.model.window_mode).unwrap_or(0),
            cx.listener(move |this, i: &usize, _, cx| {
                if modes[*i] != this.model.window_mode {
                    this.model.window_mode = modes[*i];
                    cx.emit(DrawerEvent::WindowMode(modes[*i]));
                    cx.notify();
                }
            }),
        );
        let labels: Vec<&str> = self.model.monitors.iter().map(|(_, label)| label.as_str()).collect();
        let at = self.model.monitor.as_ref().and_then(|m| self.model.monitors.iter().position(|(id, _)| id == m)).unwrap_or(usize::MAX);
        let monitor = chips(
            "monitor",
            &labels,
            at,
            cx.listener(|this, i: &usize, _, cx| {
                if let Some((id, _)) = this.model.monitors.get(*i).cloned()
                    && this.model.monitor.as_ref() != Some(&id)
                {
                    this.model.monitor = Some(id.clone());
                    cx.emit(DrawerEvent::Monitor(id));
                    cx.notify();
                }
            }),
        );
        let mut out = vec![
            setting("Window", hints::WINDOW_MODE, window_mode),
            setting("Monitor", hints::MONITOR, monitor),
            setting("Target ratio", hints::TARGET_RATIO, ratios),
        ];
        if self.custom_ratio {
            out.push(slider_setting("Custom ratio", hints::CUSTOM_RATIO, &self.sliders.custom_ratio, format!("{:.2}", p.target_ratio.0)));
        }
        let modes = [FillMode::Fit, FillMode::Stretch, FillMode::Crop, FillMode::SmartStretch];
        let fill = radios(
            "fill",
            &[
                ("Fit", "Black bars, nothing stretched"),
                ("Stretch", "Fills the shape, everything widened"),
                ("Crop", "Fills the shape, the edges cut off"),
                ("Smart stretch", "Fills the shape, the middle stays natural"),
            ],
            modes.iter().position(|m| *m == p.fill_mode).unwrap_or(0),
            cx.listener(move |this, i: &usize, _, cx| this.set_picture(PictureSettings { fill_mode: modes[*i], ..this.model.picture }, cx)),
        );
        out.push(setting("Fill mode", hints::FILL_MODE, fill));
        if p.fill_mode == FillMode::SmartStretch {
            let label = format!("{:.0}%", p.smart_stretch_strength * 100.0);
            out.push(slider_setting("Smart stretch strength", hints::STRENGTH, &self.sliders.strength, label));
        }
        out.push(self.stats_settings(cx));
        out
    }

    /// How xReveal itself looks: theme, accent, panel opacity, density, and the drawer's side.
    fn interface_tab(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let pal = theme::palette();
        let look = self.model.look;
        let themes = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];
        let theme_chips = chips(
            "theme",
            &["System", "Dark", "Light"],
            themes.iter().position(|t| *t == look.theme).unwrap_or(1),
            cx.listener(move |this, i: &usize, _, cx| this.set_look(Look { theme: themes[*i], ..this.model.look }, cx)),
        );
        let swatches = div().flex().flex_wrap().gap_3().children(Accent::ALL.into_iter().map(|accent| {
            let chosen = accent == look.accent;
            div()
                .id(("accent", accent as usize))
                .size(px(30.))
                .rounded_full()
                .p(px(3.))
                .border_2()
                .border_color(if chosen { theme::accent_colour(accent, pal.dark) } else { gpui_kit::transparent_black() })
                .cursor_pointer()
                .child(div().size_full().rounded_full().bg(theme::accent_colour(accent, pal.dark)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_look(Look { accent, ..this.model.look }, cx)))
        }));
        let densities = [Density::Compact, Density::Default, Density::Spacious];
        let density = chips(
            "density",
            &["Compact", "Default", "Spacious"],
            densities.iter().position(|d| *d == look.density).unwrap_or(1),
            cx.listener(move |this, i: &usize, _, cx| this.set_look(Look { density: densities[*i], ..this.model.look }, cx)),
        );
        let sides = [DrawerSide::Left, DrawerSide::Right];
        let side = chips(
            "side",
            &["Left", "Right"],
            sides.iter().position(|s| *s == self.model.side).unwrap_or(1),
            // The app moves the drawer (sliding out, then in on the new side) and then updates the
            // model, so the rail doesn't jump across while the drawer is still on screen.
            cx.listener(move |this, i: &usize, _, cx| {
                if sides[*i] != this.model.side {
                    cx.emit(DrawerEvent::Side(sides[*i]));
                }
            }),
        );
        vec![
            setting("Theme", hints::THEME, theme_chips),
            setting("Accent", hints::ACCENT, swatches),
            slider_setting("Panel opacity", hints::OPACITY, &self.sliders.opacity, format!("{:.0}%", look.opacity() * 100.0)),
            setting("Density", hints::DENSITY, density),
            setting("Drawer side", hints::DRAWER_SIDE, side),
        ]
    }

    fn set_look(&mut self, look: Look, cx: &mut Context<Self>) {
        if look != self.model.look {
            self.model.look = look;
            cx.emit(DrawerEvent::Look(look));
            cx.notify();
        }
    }

    /// The performance box: on or off, which corner, and what it lists (one section).
    fn stats_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let stats = self.model.stats;
        let placement = self.stats_placement(cx);
        let mut section = div().flex().flex_col().gap(px(theme::spacing().gap)).child(setting("Performance box", hints::STATS, placement));
        if stats.shown {
            let shows = checkboxes(
                "stats-shows",
                &[
                    ("Latency", "Capture to screen, and xReveal's share of it"),
                    ("Frame rate", "Frames per second actually arriving"),
                    ("Dropped frames", "Frames that came late or never arrived"),
                    ("Video format", "Resolution, frame rate and pixel format on screen"),
                ],
                &[stats.latency, stats.fps, stats.frames, stats.format],
                nav::switch_cursor("Shows"),
                cx.listener(|this, i: &usize, _, cx| this.set_stats(this.model.stats.flip(*i), cx)),
            );
            let opacity = slider_setting("Box opacity", hints::STATS_OPACITY, &self.sliders.stats_opacity, format!("{:.0}%", stats.opacity() * 100.0));
            section = section.child(setting("Shows", hints::STATS_SHOWS, shows)).child(opacity);
        }
        section.into_any_element()
    }

    /// Where the performance box goes, on a little screen: a tile in each corner, and an "off"
    /// mark in the middle. The chosen tile holds a miniature of the box.
    fn stats_placement(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = theme::palette();
        let stats = self.model.stats;
        let tile = |corner: Corner| {
            let chosen = stats.shown && stats.corner == corner;
            let miniature = chosen.then(|| {
                div().size_full().p(px(6.)).flex().flex_col().gap(px(3.)).children(
                    [0.9, 0.7, 0.8].map(|w| div().h(px(3.)).w(relative(w)).rounded(px(2.)).bg(pal.accent)),
                )
            });
            div()
                .id(("stats-corner", Corner::ALL.iter().position(|c| *c == corner).unwrap_or(0)))
                .absolute()
                .when(corner.is_left(), |d| d.left(px(8.)))
                .when(!corner.is_left(), |d| d.right(px(8.)))
                .when(corner.is_top(), |d| d.top(px(8.)))
                .when(!corner.is_top(), |d| d.bottom(px(8.)))
                .w(px(76.))
                .h(px(40.))
                .rounded(px(6.))
                .border_1()
                .cursor_pointer()
                .when(chosen, |d| d.bg(pal.accent_fill).border_color(pal.accent))
                .when(!chosen, |d| d.bg(pal.raised).border_color(pal.line).hover(|d| d.bg(pal.raised_hover).border_color(pal.accent_line)))
                .children(miniature)
                .on_click(cx.listener(move |this, _, _, cx| this.set_stats(StatsBox { shown: true, corner, ..this.model.stats }, cx)))
        };
        let off = div()
            .id("stats-off")
            .absolute()
            .top(relative(0.5))
            .left(relative(0.5))
            .mt(px(-18.))
            .ml(px(-18.))
            .size(px(36.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(if stats.shown { pal.muted } else { pal.danger })
            .hover(|d| d.bg(pal.wash).text_color(if stats.shown { pal.secondary } else { pal.danger }))
            .child(Icon::new(IconName::Ban).size_6())
            .on_click(cx.listener(|this, _, _, cx| this.set_stats(StatsBox { shown: false, ..this.model.stats }, cx)));
        div()
            .relative()
            .w_full()
            .h(px(150.))
            .rounded(px(10.))
            .bg(pal.deep)
            .border_1()
            .border_color(if stats.shown { pal.line } else { pal.danger.opacity(0.5) })
            .children(Corner::ALL.map(tile))
            .child(off)
            .into_any_element()
    }

    fn set_stats(&mut self, stats: StatsBox, cx: &mut Context<Self>) {
        if stats != self.model.stats {
            self.model.stats = stats;
            cx.emit(DrawerEvent::Stats(stats));
            cx.notify();
        }
    }

    fn colour_tab(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let pal = theme::palette();
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
            .text_color(if nav::focused("More") { pal.text } else { pal.secondary })
            .cursor_pointer()
            .hover(|d| d.text_color(pal.text))
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
                .gap(px(theme::spacing().gap))
                .child(slider_setting("Gamma", hints::GAMMA, &self.sliders.gamma, format!("{:.2}", a.gamma)))
                .child(slider_setting("Hue", hints::HUE, &self.sliders.hue, format!("{:+.0}°", a.hue)))
        });
        out.push(div().flex().flex_col().gap(px(theme::spacing().gap)).child(more).children(extra).into_any_element());
        out
    }

    fn audio_tab(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let delay = self.model.delay;
        let audio = &self.model.audio;
        let label = format!("{:+} ms", delay.av_sync_offset_ms);
        let warning = input_lag_warning(DelaySettings { buffering_frames: 0, ..delay }, self.model.frame_interval);
        let input = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(if self.model.microphone == Access::Granted {
                setting(
                    "Input",
                    hints::AUDIO_INPUT,
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(Select::new(&self.input_select).placeholder("No sound input").menu_max_h(rems(18.)))
                        .child(self.meter.render()),
                )
            } else {
                let names = audio.inputs.iter().map(|d| d.name.clone()).collect();
                setting("Input", hints::AUDIO_INPUT_LOCKED, locked_inputs(Privacy::Microphone, self.model.microphone, names, cx))
            })
            .children(audio.notice.clone().map(warn));
        let output = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(setting("Output", hints::AUDIO_OUTPUT, Select::new(&self.output_select).placeholder("System output").menu_max_h(rems(18.))))
            .children(audio.latency_hint.clone().map(warn));
        let pal = theme::palette();
        let muted = audio.muted;
        // Mute and Volume on one row: the speaker button, then the slider, which fades while muted.
        let mute = div()
            .id("mute")
            .flex_none()
            .size(px(theme::spacing().chip))
            .rounded(px(8.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .border_1()
            .when(muted, |d| d.bg(pal.accent_fill).border_color(pal.accent_line).text_color(pal.accent))
            .when(!muted, |d| d.bg(pal.raised).border_color(pal.line).text_color(pal.secondary).hover(|d| d.bg(pal.raised_hover).text_color(pal.text)))
            .child(Icon::new(if muted { IconName::VolumeX } else { IconName::Volume2 }).size_4())
            .on_click(cx.listener(|this, _, _, cx| {
                this.model.audio.muted = !this.model.audio.muted;
                cx.emit(DrawerEvent::Mute(this.model.audio.muted));
                cx.notify();
            }));
        let control = div()
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .child(mute)
            .child(div().flex_1().min_w_0().when(muted, |d| d.opacity(0.4)).child(Slider::new(&self.sliders.volume)))
            .child(
                div()
                    .w(px(44.))
                    .flex_none()
                    .text_xs()
                    .font_family(mono_family())
                    .text_right()
                    .text_color(pal.secondary)
                    .when(muted, |d| d.opacity(0.4))
                    .child(format!("{:.0}%", audio.volume * 100.0)),
            );
        let volume = setting("Volume", hints::VOLUME, control);
        vec![
            input.into_any_element(),
            output.into_any_element(),
            volume.into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(slider_setting("A/V sync offset", hints::AV_SYNC, &self.sliders.av_sync, label))
                .children(warning.map(warn))
                .into_any_element(),
        ]
    }

    fn reset_all(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = theme::palette();
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
                        .h(px(theme::spacing().chip))
                        .px_3()
                        .rounded(px(8.))
                        .flex()
                        .items_center()
                        .text_sm()
                        .cursor_pointer()
                        .bg(pal.danger)
                        .text_color(pal.on_danger)
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
                let colour = if nav::focused(label) { pal.text } else { pal.secondary };
                div().id(id).text_sm().text_color(colour).cursor_pointer().hover(|d| d.text_color(pal.text)).child(label)
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
            .border_color(pal.line)
            .flex()
            .flex_col()
            .gap_1()
            .child(body)
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
                    && self.model.window_mode == WindowMode::default()
                    && self.model.stats == StatsBox::default()
                    && !self.custom_ratio
            }
            Tab::Colour => p.colour_range == d.colour_range && p.colour_space == d.colour_space && p.adjustments == d.adjustments,
            Tab::Audio => {
                let a = &self.model.audio;
                self.model.delay.av_sync_offset_ms == 0 && a.volume == 1.0 && !a.muted && a.chosen_input.is_none() && a.output.is_none()
            }
            Tab::Interface => self.model.look == Look::default() && self.model.side == DrawerSide::default(),
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
                model.audio.volume = 1.0;
                model.audio.muted = false;
                model.audio.chosen_input = None;
                model.audio.output = None;
                events.extend([DrawerEvent::Volume(1.0), DrawerEvent::Mute(false), DrawerEvent::AudioInput(None), DrawerEvent::AudioOutput(None)]);
            }
            Tab::Interface => {
                model.look = Look::default();
                events.push(DrawerEvent::Look(model.look));
                if model.side != DrawerSide::default() {
                    model.side = DrawerSide::default();
                    events.push(DrawerEvent::Side(model.side));
                }
            }
        }
        self.set_model(model, window, cx);
        for event in events {
            cx.emit(event);
        }
    }

    /// The tab's title, with a Reset for just this tab.
    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = theme::palette();
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
            .when(idle, |d| d.text_color(pal.faint))
            .when(!idle, |d| {
                d.text_color(pal.secondary)
                    .cursor_pointer()
                    .hover(|d| d.bg(pal.raised).text_color(pal.text))
                    .on_click(cx.listener(|this, _, window, cx| this.reset_tab(window, cx)))
            })
            .child(Icon::new(IconName::RotateCcw).size_3p5())
            .child("Reset");
        // The page's title, set apart from its settings: a size up from their titles, and a
        // full-width line below (the lines between settings stop short at the padding).
        div()
            .flex_none()
            .px_5()
            .pt_5()
            .pb_4()
            .border_b_1()
            .border_color(pal.line)
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_xl().line_height(rems(1.75)).font_weight(FontWeight::SEMIBOLD).child(self.tab.label()))
                    // On the title, ←/→ switch tabs.
                    .when(nav::focused(self.tab.label()), |d| d.child(div().text_xs().text_color(pal.muted).child("‹ ›"))),
            )
            .child(reset)
            .into_any_element()
    }
}

impl Render for SettingsDrawer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = theme::palette();
        nav::set_focus(self.focus_for_render());
        let rows = match self.tab {
            Tab::Source => self.source_tab(cx),
            Tab::Display => self.display_tab(cx),
            Tab::Colour => self.colour_tab(cx),
            Tab::Audio => self.audio_tab(cx),
            Tab::Interface => self.interface_tab(cx),
        };
        let header = self.header(cx);
        let reset_all = self.reset_all(cx);
        let rail = self.rail(cx);
        // Each setting is a section, divided from the next by a hairline.
        let sections = rows.into_iter().enumerate().map(|(i, row)| {
            div().w_full().py(px(theme::spacing().section)).when(i > 0, |d| d.border_t_1().border_color(pal.line)).child(row)
        });
        let root = div().size_full().key_context(nav::KEY_CONTEXT).track_focus(&self.focus);
        self.nav_handlers(root, cx)
            .flex()
            // The rail on the outer edge: right of the content when the drawer is on the right.
            .when(self.model.side == DrawerSide::Right, |d| d.flex_row_reverse())
            .text_color(pal.text)
            .bg(pal.panel)
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
                            .track_scroll(&self.scroll)
                            .px_5()
                            .flex()
                            .flex_col()
                            .children(sections),
                    )
                    .child(reset_all),
            )
    }
}

fn hint(text: &'static str) -> Div {
    let pal = theme::palette();
    div().w_full().text_xs().line_height(px(16.)).text_color(pal.muted).child(text)
}

fn warn(text: String) -> Div {
    let pal = theme::palette();
    div().text_xs().font_family(mono_family()).text_color(pal.warn).child(text)
}

/// Title, control, hint: one setting.
fn setting(title: &'static str, hint_text: &'static str, control: impl IntoElement) -> AnyElement {
    let pal = theme::palette();
    let focused = nav::focused(title);
    div()
        // No fixed width: it spreads over the negative margins.
        // Room for the arrow-key highlight without moving anything: a wash and an accent edge
        // (2 px, taken off the left padding so the text stays put).
        .pl(px(6.))
        .pr_2()
        .mx(px(-8.))
        .py_1()
        .my(px(-4.))
        .rounded(px(8.))
        .border_l_2()
        .border_color(if focused { pal.accent } else { gpui_kit::transparent_black() })
        .when(focused, |d| d.bg(pal.wash))
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
        .child(control)
        .child(hint(hint_text))
        .into_any_element()
}

fn slider_setting(title: &'static str, hint_text: &'static str, state: &Entity<SliderState>, value: String) -> AnyElement {
    let pal = theme::palette();
    let control = div()
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .child(div().flex_1().min_w_0().child(Slider::new(state)))
        .child(div().w(px(56.)).flex_none().text_xs().font_family(mono_family()).text_right().text_color(pal.secondary).child(value));
    setting(title, hint_text, control)
}

/// How long a control takes to ease into its new state.
const EASE: Duration = Duration::from_millis(180);

/// `a` blended towards `b` by `t` (0 to 1), alpha included.
fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    let l = |x: f32, y: f32| x + (y - x) * t;
    Hsla::from(Rgba { r: l(a.r, b.r), g: l(a.g, b.g), b: l(a.b, b.b), a: l(a.a, b.a) })
}

/// The animation that eases a just-chosen control in. Its ID carries the chosen state, so it
/// plays when that changes (and once when the control first appears chosen).
fn ease_in(group: &'static str, index: usize, chosen: bool) -> (ElementId, Animation) {
    ((SharedString::from(format!("{group}-ease")), index * 2 + chosen as usize).into(), Animation::new(EASE).with_easing(ease_out_quint()))
}

/// A group of choices that wraps onto a second row instead of overflowing.
fn chips(id: &'static str, labels: &[&str], selected: usize, on_pick: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Div {
    let pal = theme::palette();
    let on_pick = std::rc::Rc::new(on_pick);
    div().w_full().flex().flex_wrap().gap(px(6.)).children(labels.iter().enumerate().map(|(i, label)| {
        let chosen = i == selected;
        let on_pick = on_pick.clone();
        let chip = div()
            .id((id, i))
            .h(px(theme::spacing().chip))
            .px_3()
            .rounded(px(8.))
            .flex()
            .items_center()
            .text_sm()
            .cursor_pointer()
            .border_1()
            .when(chosen, |d| d.bg(pal.accent_fill).border_color(pal.accent_line).text_color(pal.accent))
            .when(!chosen, |d| d.bg(pal.raised).border_color(pal.line).text_color(pal.secondary).hover(|d| d.bg(pal.raised_hover).text_color(pal.text)))
            .child(label.to_string())
            .on_click(move |_, window, cx| on_pick(&i, window, cx));
        // The newly chosen chip eases from plain into its accent fill.
        let (anim_id, animation) = ease_in(id, i, chosen);
        chip.with_animation(anim_id, animation, move |el, t| {
            if chosen { el.bg(mix(pal.raised, pal.accent_fill, t)).text_color(mix(pal.secondary, pal.accent, t)) } else { el }
        })
    }))
}

/// One choice out of a few, as radio rows: a label and a short line saying what it does.
fn radios(id: &'static str, items: &[(&str, &str)], selected: usize, on_pick: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Div {
    let pal = theme::palette();
    let on_pick = std::rc::Rc::new(on_pick);
    div().w_full().flex().flex_col().children(items.iter().enumerate().map(|(i, (label, detail))| {
        let chosen = i == selected;
        let on_pick = on_pick.clone();
        let dot = div()
            .flex_none()
            .size(px(16.))
            .rounded_full()
            .border_1()
            .flex()
            .items_center()
            .justify_center()
            .when(chosen, |d| {
                // The dot grows into place.
                let (anim_id, animation) = ease_in(id, i, true);
                d.border_color(pal.accent)
                    .child(div().size(px(8.)).rounded_full().bg(pal.accent).with_animation(anim_id, animation, |dot, t| dot.size(px(2. + 6. * t))))
            })
            .when(!chosen, |d| d.bg(pal.raised).border_color(pal.line));
        div()
            .id((id, i))
            .py(px(6.))
            .px_2()
            .mx(px(-8.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap_3()
            .cursor_pointer()
            .hover(|d| d.bg(pal.wash))
            .child(dot)
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_sm().text_color(if chosen { pal.text } else { pal.secondary }).child(label.to_string()))
                    .child(div().text_xs().text_color(pal.muted).child(detail.to_string())),
            )
            .on_click(move |_, window, cx| on_pick(&i, window, cx))
    }))
}

/// A list of checkbox rows, one per item; each switches on and off by itself. `cursor` is the row
/// under the arrow-key cursor.
fn checkboxes(id: &'static str, items: &[(&str, &str)], on: &[bool], cursor: Option<usize>, on_toggle: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Div {
    let pal = theme::palette();
    let on_toggle = std::rc::Rc::new(on_toggle);
    div().w_full().flex().flex_col().children(items.iter().enumerate().map(|(i, (label, detail))| {
        let checked = on.get(i).copied().unwrap_or(false);
        let on_toggle = on_toggle.clone();
        let mark = div().flex_none().size(px(16.)).rounded(px(4.)).border_1().flex().items_center().justify_center();
        let mark: AnyElement = if checked {
            // The box fills with the accent as it's ticked.
            let (anim_id, animation) = ease_in(id, i, true);
            mark.border_color(pal.accent)
                .text_color(if pal.dark { pal.deep } else { gpui_kit::white() })
                .child(Icon::new(IconName::Check).size_3())
                .with_animation(anim_id, animation, move |el, t| el.bg(mix(pal.raised, pal.accent, t)))
                .into_any_element()
        } else {
            mark.bg(pal.raised).border_color(pal.line).into_any_element()
        };
        div()
            .id((id, i))
            .py(px(6.))
            .px_2()
            .mx(px(-8.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap_3()
            .cursor_pointer()
            .hover(|d| d.bg(pal.wash))
            .when(cursor == Some(i), |d| d.bg(pal.wash))
            .child(mark)
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_sm().text_color(if checked { pal.text } else { pal.secondary }).child(label.to_string()))
                    .child(div().text_xs().text_color(pal.muted).child(detail.to_string())),
            )
            .on_click(move |_, window, cx| on_toggle(&i, window, cx))
    }))
}

/// Inputs xReveal may not use yet, for want of Camera or Microphone access (#113): each on its own
/// row, locked, with its own button. One grant unlocks them all (macOS asks per app, not per
/// device). Before macOS has asked, the button asks; once it has, it won't ask again, so the button
/// opens the privacy settings.
fn locked_inputs(privacy: Privacy, access: Access, names: Vec<String>, cx: &mut Context<SettingsDrawer>) -> AnyElement {
    let pal = theme::palette();
    let label = if access == Access::NotAsked { "Allow access" } else { "Open System Settings" };
    let what = match privacy {
        Privacy::Camera => "Inputs show here",
        Privacy::Microphone => "Sound inputs show here",
    };
    let names = if names.is_empty() { vec![format!("{what} once xReveal has access")] } else { names };
    let rows = names.into_iter().enumerate().map(|(i, name)| {
        let id = match privacy {
            Privacy::Camera => ("allow-camera", i),
            Privacy::Microphone => ("allow-microphone", i),
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(px(8.))
            .bg(pal.raised)
            .border_1()
            .border_color(pal.line)
            .child(Icon::new(IconName::Lock).size_4().text_color(pal.muted))
            .child(div().flex_1().min_w_0().truncate().text_sm().text_color(pal.secondary).child(name))
            .child(
                div()
                    .id(id)
                    .flex_none()
                    .px_2()
                    .py(px(2.))
                    .rounded(px(6.))
                    .text_xs()
                    .cursor_pointer()
                    .bg(pal.accent_fill)
                    .border_1()
                    .border_color(pal.accent_line)
                    .text_color(pal.accent)
                    .hover(|d| d.text_color(pal.text))
                    .child(label)
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(DrawerEvent::Allow(privacy)))),
            )
    });
    div().flex().flex_col().gap_1().children(rows).into_any_element()
}

fn quiet_button(id: &'static str, icon: IconName, label: &'static str) -> Stateful<Div> {
    let pal = theme::palette();
    div()
        .id(id)
        .h(px(theme::spacing().chip))
        .px_3()
        .rounded(px(8.))
        .flex()
        .items_center()
        .gap_2()
        .text_sm()
        .cursor_pointer()
        .bg(pal.raised)
        .border_1()
        .border_color(pal.line)
        .text_color(pal.secondary)
        .hover(|d| d.bg(pal.raised_hover).text_color(pal.text))
        .child(Icon::new(icon).size_4())
        .child(label)
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude has its own `test` attribute that would replace std's.
    use super::{DelaySettings, SLIDE_IN, SLIDE_OUT, Slide, TargetRatio, VideoFormat, input_label, input_lag_warning, preset_label, readout};
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
    fn controls_blend_from_plain_to_chosen() {
        use super::mix;
        use gpui_kit::{Hsla, rgb, rgba};
        let (from, to): (Hsla, Hsla) = (rgb(0x232325).into(), rgba(0x6aa9ff24).into());
        let close = |a: Hsla, b: Hsla| {
            let (a, b) = (a.to_rgb(), b.to_rgb());
            (a.r - b.r).abs() < 1e-3 && (a.g - b.g).abs() < 1e-3 && (a.b - b.b).abs() < 1e-3 && (a.a - b.a).abs() < 1e-3
        };
        assert!(close(mix(from, to, 0.0), from), "starts plain");
        assert!(close(mix(from, to, 1.0), to), "ends chosen");
        let half = mix(from, to, 0.5).to_rgb();
        assert!((half.a - (1.0 + to.a) / 2.0).abs() < 1e-3, "alpha blends too");
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
    fn the_cards_own_audio_is_marked_where_it_stays_visible() {
        assert_eq!(input_label("Digital Audio Interface (USB2 Digital Audio)", true), "Input's own audio · Digital Audio Interface (USB2 Digital Audio)");
        assert_eq!(input_label("Microphone (HyperX SoloCast)", false), "Microphone (HyperX SoloCast)");
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
