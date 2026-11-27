//! The Settings drawer (docs/spec/v1.md §2.3): a panel over the live picture with a side rail of
//! four tabs. Every setting has an always-visible hint line and applies live.
//!
//! The drawer only shows and edits a snapshot ([`DrawerModel`]); every change goes out as a
//! [`DrawerEvent`] and the app applies it to the video pipeline and the Remembered setup. That
//! keeps it hostable in its own window too (Windows draws it in a separate transparent window).

use gpui_kit::assets::IconName;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::{Duration, Instant};

mod chooser;
mod lock;
mod update_button;
use lock::locked;
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
    /// The Drawer swipe is on (#141).
    pub swipe: bool,
    /// **Check for updates** is on (#145).
    pub check_updates: bool,
    /// A newer release to offer, and its notes (#145).
    pub update: Option<(String, Vec<String>)>,
    /// How the last check this run went (#145).
    pub update_check: crate::update::Check,
    /// The in-app update: downloading, ready to restart, or failed (#163).
    pub updater: crate::update::UpdaterView,
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
    /// The Drawer swipe turned on or off (#141).
    Swipe(bool),
    /// **Check for updates** turned on or off (#145).
    CheckUpdates(bool),
    /// The update button asked to check now (#145).
    CheckNow,
    /// The update button asked to download the newer version, or to restart into it (#163).
    DownloadUpdate,
    RestartToUpdate,
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
    pub const CHECK_UPDATES: &str = "Once a day, xReveal asks GitHub whether there's a newer version. Nothing else is sent.";
    pub const SWIPE: &str = if cfg!(target_os = "macos") {
        "Swipe two fingers sideways on the trackpad: away from this panel's side to open it, back to close it. Start in the middle of the trackpad; a swipe from its right edge opens Notification Center."
    } else {
        "Swipe two fingers sideways on the touchpad: away from this panel's side to open it, back to close it."
    };
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
    /// A locked setting's tip (#146).
    pub const NEEDS_CAMERA: &str = "Allow Camera access to change this.";
    pub const NEEDS_MICROPHONE: &str = "Allow Microphone access to change this.";
    #[cfg(not(target_os = "windows"))]
    pub const CARD_LOCKED: &str = "xReveal needs Camera access to show any input. macOS asks once; after that it's in System Settings › Privacy & Security › Camera.";
    #[cfg(target_os = "windows")]
    pub const CARD_LOCKED: &str = "Windows is blocking camera access for desktop apps. Turn on \"Let desktop apps access your camera\" in Settings › Privacy & security › Camera.";
    #[cfg(not(target_os = "windows"))]
    pub const AUDIO_INPUT_LOCKED: &str =
        "xReveal needs Microphone access to play the input's sound. macOS asks once; after that it's in System Settings › Privacy & Security › Microphone.";
    #[cfg(target_os = "windows")]
    pub const AUDIO_INPUT_LOCKED: &str =
        "Windows is blocking microphone access for desktop apps. Turn on \"Let desktop apps access your microphone\" in Settings › Privacy & security › Microphone.";
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
    custom_ratio: bool,
    sliders: Sliders,
    /// The selectors: Input (card), Video format, sound Input, Output (see [`chooser`]).
    choosers: [chooser::Chooser; chooser::COUNT],
    /// The selector whose list is open.
    open_chooser: Option<usize>,
    /// The pointer is over the update button, or over its tip (#145); whether the tip shows, and
    /// the pending close that lets the pointer cross from the button to the tip.
    update_hover: (bool, bool),
    update_tip_open: bool,
    update_tip_closing: bool,
    /// The download ring as shown, easing after the real progress (`update_button::Ring`).
    update_ring: std::cell::Cell<update_button::Ring>,
    update_tip_close: Option<Task<()>>,
    /// Where the update tip's notes are scrolled to, for their fades.
    update_notes_scroll: ScrollHandle,
    /// The lock warning under the pointer, and where each one is, with the window's width (see [`lock`]).
    hovered_lock: Option<&'static str>,
    /// A lock warning the pointer just left: its tip is fading out.
    fading_lock: Option<&'static str>,
    lock_bounds: lock::Spots,
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
        subscriptions.push(cx.subscribe(&sliders.opacity, |this: &mut Self, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(SliderValue::Single(value)) = event {
                this.set_look(Look { opacity: value.round() / 100.0, ..this.model.look }, cx);
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
            custom_ratio,
            sliders,
            choosers: Default::default(),
            open_chooser: None,
            update_hover: (false, false),
            update_tip_open: false,
            update_tip_closing: false,
            update_ring: Default::default(),
            update_tip_close: None,
            update_notes_scroll: ScrollHandle::new(),
            hovered_lock: None,
            fading_lock: None,
            lock_bounds: Default::default(),
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
        self.choosers[chooser::CARD].set(names, card_index);

        // That card's Video formats, with the one on screen selected.
        let info = current_id.as_ref().and_then(|id| model.sources.iter().find(|s| &s.identity.os_id == id));
        self.formats = info.map(|i| source::choices(&i.formats)).unwrap_or_default();
        let labels: Vec<String> = self.formats.iter().copied().map(readout).collect();
        let format_index = model.current.as_ref().and_then(|(_, f)| self.formats.iter().position(|c| c == f));
        self.choosers[chooser::FORMAT].set(labels, format_index);

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
        self.choosers[chooser::SOUND_INPUT].set(input_names, input_index);
        let output_names: Vec<String> = std::iter::once("System output".to_owned()).chain(audio.outputs.iter().map(|d| d.name.clone())).collect();
        let output_index = match &audio.output {
            None => Some(0),
            Some(id) => audio.outputs.iter().position(|d| &d.id == id).map(|i| i + 1),
        };
        self.choosers[chooser::OUTPUT].set(output_names, output_index);

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
                    .child(div().min_w_0().child(identity.name.clone())),
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
            setting("Input", hints::CARD, self.chooser(chooser::CARD, "Input", "No input", cx))
        } else {
            setting("Input", hints::CARD_LOCKED, locked_inputs(Privacy::Camera, self.model.camera, cx))
        };
        // Without Camera access, nothing about the picture can be set (#146).
        let why = (self.model.camera != Access::Granted).then_some(hints::NEEDS_CAMERA);
        let format = self.chooser(chooser::FORMAT, "Video format", "No formats", cx);
        let buffering = div().flex().flex_col().gap_2().child(locked("lock-buffering", why, buffering.into_any_element())).children(warning.map(warn));
        let mut rows = vec![
            self.live(),
            input,
            setting_with("Video format", hints::VIDEO_FORMAT, locked("lock-format", why, format), self.lock_badge("badge-format", why, cx)),
            setting_with("Buffering", hints::BUFFERING, buffering, self.lock_badge("badge-buffering", why, cx)),
        ];
        if self.model.device_settings {
            let open = quiet_button("device-settings", IconName::Settings, "Device settings…").on_click(cx.listener(|_, _, _, cx| cx.emit(DrawerEvent::DeviceSettings)));
            let device = locked("lock-device-settings", why, div().flex().child(open).into_any_element());
            rows.push(setting_with("Device settings…", hints::DEVICE_SETTINGS, device, self.lock_badge("badge-device-settings", why, cx)));
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
        // A selector: "Borderless" and "Fullscreen" don't fit a third of the drawer each.
        let mode_labels: Vec<String> = mode_labels.into_iter().map(str::to_owned).collect();
        let window_mode = self.chooser_of(chooser::WINDOW, "Window", "Windowed", &mode_labels, modes.iter().position(|m| *m == self.model.window_mode), cx);
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
        // A selector, like Window: the three names don't fit a third of the drawer each.
        let density_labels = ["Compact", "Default", "Spacious"].map(str::to_owned);
        let density = self.chooser_of(chooser::DENSITY, "Density", "Default", &density_labels, densities.iter().position(|d| *d == look.density), cx);
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
        let swipe = chips(
            "swipe",
            &["On", "Off"],
            if self.model.swipe { 0 } else { 1 },
            cx.listener(|this, i: &usize, _, cx| {
                let on = *i == 0;
                if on != this.model.swipe {
                    this.model.swipe = on;
                    cx.emit(DrawerEvent::Swipe(on));
                    cx.notify();
                }
            }),
        );
        let check_updates = chips(
            "check-updates",
            &["On", "Off"],
            if self.model.check_updates { 0 } else { 1 },
            cx.listener(|this, i: &usize, _, cx| {
                let on = *i == 0;
                if on != this.model.check_updates {
                    this.model.check_updates = on;
                    cx.emit(DrawerEvent::CheckUpdates(on));
                    cx.notify();
                }
            }),
        );
        vec![
            setting("Theme", hints::THEME, theme_chips),
            setting("Accent", hints::ACCENT, swatches),
            slider_setting("Panel opacity", hints::OPACITY, &self.sliders.opacity, format!("{:.0}%", look.opacity() * 100.0)),
            setting("Density", hints::DENSITY, density),
            setting("Drawer side", hints::DRAWER_SIDE, side),
            setting("Swipe to open settings", hints::SWIPE, swipe),
            setting("Check for updates", hints::CHECK_UPDATES, check_updates),
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
        // Without Microphone access, nothing about the sound can be set (#146).
        let why = (self.model.microphone != Access::Granted).then_some(hints::NEEDS_MICROPHONE);
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
                        .child(self.chooser(chooser::SOUND_INPUT, "Input", "No sound input", cx))
                        .child(self.meter.render()),
                )
            } else {
                setting("Input", hints::AUDIO_INPUT_LOCKED, locked_inputs(Privacy::Microphone, self.model.microphone, cx))
            })
            .children(audio.notice.clone().map(warn));
        let output = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(setting_with("Output", hints::AUDIO_OUTPUT, locked("lock-output", why, self.chooser(chooser::OUTPUT, "Output", "System output", cx)), self.lock_badge("badge-output", why, cx)))
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
            .child(div().flex_1().min_w_0().mx(px(-KNOB_OVERHANG)).px(px(KNOB_OVERHANG)).cursor_pointer().when(muted, |d| d.opacity(0.4)).child(Slider::new(&self.sliders.volume)))
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
        let volume = setting_with("Volume", hints::VOLUME, locked("lock-volume", why, control.into_any_element()), self.lock_badge("badge-volume", why, cx));
        let av_sync = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(setting_with("A/V sync offset", hints::AV_SYNC, locked("lock-av-sync", why, slider_control(&self.sliders.av_sync, label).into_any_element()), self.lock_badge("badge-av-sync", why, cx)))
            .children(warning.map(warn))
            .into_any_element();
        vec![input.into_any_element(), output.into_any_element(), volume, av_sync]
    }

    /// The footer, one row (#129, #179): the version, from Cargo.toml, opening its release page;
    /// then Quit, the repository and the update button (#145).
    fn footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = theme::palette();
        let quit = div()
            .id("quit")
            .text_sm()
            .text_color(pal.secondary)
            .cursor_pointer()
            .hover(|d| d.text_color(pal.text))
            .child("Quit")
            .on_click(cx.listener(|_, _, _, cx| cx.emit(DrawerEvent::Quit)));
        div()
            .flex_none()
            .px_5()
            .py_4()
            .border_t_1()
            .border_color(pal.line)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .id("version")
                            .text_xs()
                            .font_family(mono_family())
                            .text_color(pal.muted)
                            .cursor_pointer()
                            .hover(|d| d.text_color(pal.text))
                            .child(concat!("xReveal ", env!("CARGO_PKG_VERSION")))
                            .on_click(|_, _, cx| cx.open_url(crate::whats_new::RELEASE_PAGE)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(quit)
                            .child(
                                div()
                                    .id("repository")
                                    .group("repository")
                                    .cursor_pointer()
                                    .child(svg().path("xreveal/github-mark.svg").size(px(14.)).text_color(pal.muted).group_hover("repository", |s| s.text_color(pal.text)))
                                    .on_click(|_, _, cx| cx.open_url(crate::whats_new::REPOSITORY)),
                            )
                            .child(self.update_button(cx)),
                    ),
            )
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
            Tab::Interface => self.model.look == Look::default() && self.model.side == DrawerSide::default() && self.model.swipe && self.model.check_updates,
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
                if !model.swipe {
                    model.swipe = true;
                    events.push(DrawerEvent::Swipe(true));
                }
                if !model.check_updates {
                    model.check_updates = true;
                    events.push(DrawerEvent::CheckUpdates(true));
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
        let footer = self.footer(cx);
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
                            // A sideways swipe is the Drawer swipe (#141), never a scroll of this list.
                            .restrict_scroll_to_axis()
                            .track_scroll(&self.scroll)
                            .px_5()
                            .flex()
                            .flex_col()
                            .children(sections),
                    )
                    .child(footer),
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
    setting_with(title, hint_text, control, None)
}

/// A setting with `badge` after its title (a locked setting's warning, #146).
fn setting_with(title: &'static str, hint_text: &'static str, control: impl IntoElement, badge: Option<AnyElement>) -> AnyElement {
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
        .child(div().flex().items_center().gap(px(6.)).child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title)).children(badge))
        .child(control)
        .child(hint(hint_text))
        .into_any_element()
}

fn slider_setting(title: &'static str, hint_text: &'static str, state: &Entity<SliderState>, value: String) -> AnyElement {
    setting(title, hint_text, slider_control(state, value))
}

/// A slider and its value, without the title and hint.
fn slider_control(state: &Entity<SliderState>, value: String) -> Div {
    let pal = theme::palette();
    div()
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        // GPUI Kit's slider sets no cursor of its own. The hand also covers the knob, which hangs
        // [`KNOB_OVERHANG`] past either end of the bar.
        .child(div().flex_1().min_w_0().mx(px(-KNOB_OVERHANG)).px(px(KNOB_OVERHANG)).cursor_pointer().child(Slider::new(state)))
        .child(div().w(px(56.)).flex_none().text_xs().font_family(mono_family()).text_right().text_color(pal.secondary).child(value))
}

/// How far GPUI Kit's slider knob (16 px, centred on the value) hangs past either end of its bar.
const KNOB_OVERHANG: f32 = 8.;

/// The longest label that still fits a third of the drawer ("2 frames", "Limited", "BT.709").
const SHORT_CHIP: usize = 8;

/// How long a control takes to ease into its new state.
const EASE: Duration = Duration::from_millis(180);

/// `a` blended towards `b` by `t` (0 to 1), alpha included.
fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    let l = |x: f32, y: f32| x + (y - x) * t;
    Hsla::from(Rgba { r: l(a.r, b.r), g: l(a.g, b.g), b: l(a.b, b.b), a: l(a.a, b.a) })
}

thread_local! {
    /// When something in the drawer was last chosen, by click or by key.
    static PICKED_AT: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// Notes that a choice was just made, so the control taking it eases in (see [`ease_in`]).
pub(crate) fn note_pick() {
    PICKED_AT.with(|p| p.set(Some(Instant::now())));
}

/// The animation that eases a just-chosen control in. Its ID carries the chosen state, so it
/// starts when that changes. It only shows right after a choice (#128): controls that appear
/// already chosen (the drawer opening, another tab) are drawn in their final state at once,
/// instead of every chosen control flashing its fill in.
fn ease_in(group: &'static str, index: usize, chosen: bool) -> (ElementId, Animation) {
    let fresh = PICKED_AT.with(|p| p.get()).is_some_and(|at| at.elapsed() < EASE * 2);
    let animation = Animation::new(EASE);
    let animation = if fresh { animation.with_easing(ease_out_quint()) } else { animation.with_easing(|_| 1.0) };
    ((SharedString::from(format!("{group}-ease")), index * 2 + chosen as usize).into(), animation)
}

/// A group of choices that wraps onto a second row instead of overflowing.
/// Three or fewer short labels share the row in equal widths; otherwise (more of them, or a long
/// one such as a monitor's name) they wrap, each as wide as its label.
fn chips(id: &'static str, labels: &[&str], selected: usize, on_pick: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Div {
    let pal = theme::palette();
    let on_pick = std::rc::Rc::new(on_pick);
    let even = labels.len() <= 3 && labels.iter().all(|l| l.chars().count() <= SHORT_CHIP);
    div().w_full().flex().when(!even, |d| d.flex_wrap()).gap(px(6.)).children(labels.iter().enumerate().map(|(i, label)| {
        let chosen = i == selected;
        let on_pick = on_pick.clone();
        let chip = div()
            .id((id, i))
            .h(px(theme::spacing().chip))
            .when(even, |d| d.flex_1().min_w_0().px_2().justify_center().whitespace_nowrap())
            .when(!even, |d| d.px_3())
            .rounded(px(8.))
            .flex()
            .items_center()
            .text_sm()
            .cursor_pointer()
            .border_1()
            .when(chosen, |d| d.bg(pal.accent_fill).border_color(pal.accent_line).text_color(pal.accent))
            .when(!chosen, |d| d.bg(pal.raised).border_color(pal.line).text_color(pal.secondary).hover(|d| d.bg(pal.raised_hover).text_color(pal.text)))
            .child(label.to_string())
            .on_click(move |_, window, cx| {
                note_pick();
                on_pick(&i, window, cx)
            });
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
            .on_click(move |_, window, cx| {
                note_pick();
                on_pick(&i, window, cx)
            })
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
            .on_click(move |_, window, cx| {
                note_pick();
                on_toggle(&i, window, cx)
            })
    }))
}

/// The Input setting while xReveal may not use the inputs yet, for want of Camera or Microphone
/// access: one box with one button (#113, #146), not one per input. Access is per app, so one
/// grant unlocks every input. Before macOS has asked, the button asks; once it has, it won't ask
/// again, so the button opens the privacy settings.
fn locked_inputs(privacy: Privacy, access: Access, cx: &mut Context<SettingsDrawer>) -> AnyElement {
    let pal = theme::palette();
    let (icon, title) = match (privacy, access) {
        (Privacy::Camera, Access::Denied) => (IconName::Camera, "Camera access is off"),
        (Privacy::Camera, _) => (IconName::Camera, "Camera access needed"),
        (Privacy::Microphone, Access::Denied) => (IconName::Mic, "Microphone access is off"),
        (Privacy::Microphone, _) => (IconName::Mic, "Microphone access needed"),
    };
    let label = match access {
        Access::Denied if cfg!(target_os = "windows") => "Open Windows settings",
        Access::Denied => "Open System Settings",
        _ => "Allow access",
    };
    let id = match privacy {
        Privacy::Camera => "allow-camera",
        Privacy::Microphone => "allow-microphone",
    };
    div()
        .p_3()
        .rounded(px(10.))
        .bg(pal.raised)
        .border_1()
        .border_color(pal.line)
        .flex()
        .flex_col()
        .gap_3()
        .child(div().flex().items_center().gap_3().child(Icon::new(icon).size_5().text_color(pal.secondary)).child(div().flex_1().text_sm().text_color(pal.text).child(title)))
        .child(
            div()
                .id(id)
                .w_full()
                .flex()
                .justify_center()
                .py(px(6.))
                .rounded(px(6.))
                .text_sm()
                .cursor_pointer()
                .bg(pal.accent_fill)
                .border_1()
                .border_color(pal.accent_line)
                .text_color(pal.accent)
                .hover(|d| d.text_color(pal.text))
                .child(label)
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(DrawerEvent::Allow(privacy)))),
        )
        .into_any_element()
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
