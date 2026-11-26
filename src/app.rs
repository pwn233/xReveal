//! Root view: the live picture shows through it (it paints no background). On top: the Settings
//! drawer, a one-line status until the screens without a picture exist, the one-time colour tip,
//! and the performance box (F3).

use std::time::{Duration, Instant};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::drawer::{self, DrawerEvent, DrawerModel, SettingsDrawer, Tab};
use crate::permission::{self, Access, Privacy};
use crate::screens::{Actions, PickerInput, PickerKind, Screen};
use crate::source::VideoFormat;
use crate::video::signal;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use crate::platform::VideoPipeline;
use crate::setup::{DrawerSide, StatsBox, Store, WindowMode, WindowPlacement};
use crate::theme;
use crate::video::timing::Summary;

actions!(xreveal, [ToggleDrawer, CloseDrawer, ToggleStats, ToggleFullscreen, Quit]);

/// The key context of xReveal's own bindings; deeper than GPUI Kit's root, so Tab is ours.
const KEY_CONTEXT: &str = "XReveal";

/// Keys (docs/spec/v1.md §2.2). Call once at startup.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", ToggleDrawer, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", CloseDrawer, Some(KEY_CONTEXT)),
        KeyBinding::new("f3", ToggleStats, Some(KEY_CONTEXT)),
        KeyBinding::new("f11", ToggleFullscreen, Some(KEY_CONTEXT)),
        // Windows closes the window on Alt+F4 by itself, which quits too.
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    drawer::bind_keys(cx);
}

/// Switches the window between a normal window and borderless fullscreen. On macOS that is GPUI's
/// "simple" fullscreen (covering the screen, not a separate Space); on Windows its fullscreen is
/// already borderless. Call it once per change: the switch happens a moment later.
pub fn toggle_borderless(window: &Window) {
    #[cfg(target_os = "macos")]
    window.toggle_simple_fullscreen();
    #[cfg(not(target_os = "macos"))]
    window.toggle_fullscreen();
}

/// How close to the drawer's edge the pointer must be for the handle to appear.
const EDGE: f32 = 48.0;
/// How long the one-time colour tip stays up.
const TIP_FOR: Duration = Duration::from_secs(12);

pub struct XReveal {
    focus: FocusHandle,
    status: String,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    video: Option<VideoPipeline>,
    drawer: Entity<SettingsDrawer>,
    drawer_open: bool,
    /// The drawer sliding in or out (drawn in this window).
    slide: drawer::Slide,
    /// The drawer is drawn in its own transparent window (Windows: the video child window covers
    /// GPUI's content; docs/spec/v1.md §3.5). `XREVEAL_DRAWER_WINDOW=1` forces it elsewhere, for testing.
    drawer_in_own_window: bool,
    drawer_window: Option<(AnyWindowHandle, Entity<DrawerWindow>)>,
    /// The pointer is near the drawer's edge: show the handle.
    near_edge: bool,
    /// The one-time "Blacks look grey?" tip, while it is up.
    tip: Option<Task<()>>,
    /// Redraws the performance box while it is shown; dropping it stops the refresh.
    stats_refresh: Option<Task<()>>,
    /// What shows where the picture goes (docs/spec/v1.md §2.8), checked twice a second.
    screen: Screen,
    /// Which picker shows while nothing is wanted.
    picker: PickerKind,
    /// The remembered Source starts once camera access is given.
    pending_launch: bool,
    /// A broken `setup.toml` was set aside: said once, over the Welcome screen.
    notice: Option<String>,
    /// When the picker lists the inputs again (one may have been plugged in).
    next_listing: Option<Instant>,
    /// The wanted card was unplugged: open it again as soon as it is back.
    unplugged: bool,
    /// When to try a busy card again.
    next_retry: Option<Instant>,
    /// The format another app switched the card to, while it differs from xReveal's choice.
    taken_over: Option<VideoFormat>,
    _signal_watch: Option<Task<()>>,
    /// The sound for the input on screen (docs/spec/v1.md §2.7).
    sound: crate::audio::Sound,
    /// The Remembered setup (docs/spec/v1.md §3.7).
    store: Store,
    _subscriptions: Vec<Subscription>,
}

impl XReveal {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, mut store: Store) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let save_on_quit = cx.on_app_quit(|this, _| {
            this.store.flush();
            async {}
        });

        // Remember the monitor xReveal opened on (by the display's own identity).
        if let Some(display) = window.display(cx)
            && let Ok(uuid) = display.uuid()
        {
            store.update(|s| s.monitor = Some(uuid.to_string()));
        }

        // First launch: the Welcome screen, not the first card. Without camera access (macOS), the
        // permission screen first; the remembered Source starts once access is given.
        let camera_ready = permission::access(Privacy::Camera) == Access::Granted;
        let first_run = store.first_run;
        let notice = store.notice.take();
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let (video, status) = {
            let remembered = store.setup.source.clone();
            match VideoPipeline::start(window, remembered.as_ref().map(|s| (s, store.setup.video_format)), camera_ready && !first_run) {
                Ok(video) => {
                    let status = video.description().to_owned();
                    (Some(video), status)
                }
                Err(error) => (None, error),
            }
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let status = String::from("The live picture isn't implemented on this OS yet.");
        let picker = match (&store.setup.source, first_run) {
            (_, true) => PickerKind::Welcome,
            (Some(remembered), false) => PickerKind::NotFound(remembered.name.clone()),
            (None, false) => PickerKind::Choose,
        };

        let drawer = cx.new(|cx| SettingsDrawer::new(DrawerModel::default(), window, cx));
        let on_drawer = cx.subscribe_in(&drawer, window, |this, _, event: &DrawerEvent, window, cx| this.apply(event.clone(), window, cx));
        let mut this = Self {
            focus,
            status,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            video,
            drawer,
            drawer_open: false,
            slide: drawer::Slide::closed(),
            drawer_in_own_window: cfg!(target_os = "windows") || std::env::var_os("XREVEAL_DRAWER_WINDOW").is_some(),
            drawer_window: None,
            near_edge: false,
            tip: None,
            stats_refresh: None,
            screen: Screen::Starting,
            picker,
            pending_launch: !camera_ready && !first_run,
            notice,
            next_listing: None,
            unplugged: false,
            next_retry: None,
            taken_over: None,
            _signal_watch: None,
            sound: Default::default(),
            store,
            _subscriptions: vec![save_on_quit, on_drawer],
        };
        let follow = cx.observe_window_bounds(window, |this: &mut Self, window, cx| {
            this.remember_placement(window, cx);
            if this.drawer_window.is_some() {
                this.close_drawer_window(cx);
                this.open_drawer_window(window, cx);
            }
        });
        this._subscriptions.push(follow);
        // The remembered look; with System it follows the OS's appearance as that changes.
        theme::apply(&this.store.setup.look, theme::appearance_is_dark(window.appearance()), cx);
        let appearance = cx.observe_window_appearance(window, |this: &mut Self, window, cx| {
            theme::apply(&this.store.setup.look, theme::appearance_is_dark(window.appearance()), cx);
        });
        this._subscriptions.push(appearance);
        this.showing_changed(window, cx);
        this.watch_signal(window, cx);
        this._signal_watch = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                if this.update_in(cx, |this, window, cx| this.watch_signal(window, cx)).is_err() {
                    break;
                }
            }
        }));
        let stats = this.store.setup.stats;
        this.stats_refresh = None;
        this.apply_stats(stats, cx);
        this
    }

    /// Works out what the Source is doing (docs/spec/v1.md §2.8) and reacts: reopens the card when
    /// it comes back or stops being busy, and notices another app changing its format.
    fn watch_signal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let screen = 'screen: {
            let camera = permission::access(Privacy::Camera);
            if camera != Access::Granted {
                break 'screen Screen::Permission { denied: camera == Access::Denied };
            }
            let Some(video) = &mut self.video else { return };
            if std::mem::take(&mut self.pending_launch) {
                let remembered = self.store.setup.source.clone();
                video.show_launch(remembered.as_ref().map(|s| (s, self.store.setup.video_format)));
                if video.wanted().is_some() {
                    self.status = video.description().to_owned();
                    self.showing_changed(window, cx);
                    return self.watch_signal(window, cx);
                }
            }
            if matches!(video.open_error(), Some(signal::OpenError::Blocked(_))) {
                if self.next_retry.is_none_or(|t| Instant::now() >= t) {
                    self.next_retry = Some(Instant::now() + signal::BUSY_RETRY);
                    if video.retry().is_ok() {
                        self.status = video.description().to_owned();
                        self.showing_changed(window, cx);
                        return self.watch_signal(window, cx);
                    }
                }
                break 'screen Screen::Blocked;
            }
            match video.observe() {
                None => {
                    // Nothing chosen yet: the picker, listing what's plugged in now (every 2 s).
                    if self.next_listing.is_none_or(|t| Instant::now() >= t) {
                        self.next_listing = Some(Instant::now() + Duration::from_secs(2));
                        video.refresh_sources();
                    }
                    let inputs = video
                        .sources()
                        .iter()
                        .filter_map(|s| {
                            let format = crate::source::default_choice(&s.formats)?;
                            Some(PickerInput { os_id: s.identity.os_id.clone(), name: s.identity.name.clone(), format: drawer::readout(format) })
                        })
                        .collect();
                    Screen::Picker { kind: self.picker.clone(), inputs }
                }
                Some(observed) => {
                    let mut observed = observed;
                    let mut reopened = false;
                    if !observed.connected {
                        // The card is gone: let go of it, and wait for the same Source.
                        if video.current().is_some() {
                            video.release();
                        }
                        self.unplugged = true;
                    } else if (self.unplugged || observed.busy) && self.next_retry.is_none_or(|t| Instant::now() >= t) {
                        // Back, or maybe no longer busy: open it again (at most every 2 s).
                        self.next_retry = Some(Instant::now() + signal::BUSY_RETRY);
                        reopened = video.retry().is_ok();
                        if reopened {
                            self.unplugged = false;
                        }
                        observed = video.observe().unwrap_or(observed);
                    }
                    if self.unplugged {
                        // Until it opens again, it is still the Card-unplugged screen.
                        observed.connected = false;
                    }
                    self.taken_over = video.taken_over();
                    let card = video.wanted().map(|(identity, _)| identity.name.clone()).unwrap_or_default();
                    let screen = if observed.connected && !observed.busy && video.current().is_none() {
                        // It's there but didn't open for another reason: say why.
                        Screen::no_card(video.description())
                    } else {
                        Screen::for_signal(signal::judge(observed), &card, observed.since_frame.is_some())
                    };
                    if reopened {
                        self.status = video.description().to_owned();
                        self.showing_changed(window, cx);
                    }
                    screen
                }
            }
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let screen = Screen::no_card(&self.status);
        // A dropout doubles that output's buffer, remembered for next time (docs/spec/v1.md §3.6).
        if let Some((device, frames)) = self.sound.watch(&self.store.setup) {
            self.store.update(|s| {
                s.audio_buffers.insert(device, frames);
            });
            self.restart_sound(window, cx);
        }
        if screen != self.screen {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            if let Some(video) = &self.video {
                // Windows' surface covers GPUI's screens, so it hides while one shows.
                video.set_surface_visible(matches!(screen, Screen::Picture | Screen::Starting));
            }
            self.screen = screen;
            cx.notify();
        }
        let _ = window;
    }

    /// Plays the input on screen again with the current route (it changed, or the buffer grew).
    fn restart_sound(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some((identity, _)) = self.video.as_ref().and_then(|v| v.current().cloned()) {
            self.sound.start_for(Some(&identity), &self.store.setup);
        }
        self.refresh_drawer(window, cx);
    }

    /// An input chosen on the Welcome screen or the picker: it starts, with the drawer closed.
    fn choose_input(&mut self, os_id: String, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(video) = &self.video
            && let Some(format) = video.sources().iter().find(|s| s.identity.os_id == os_id).and_then(|s| crate::source::default_choice(&s.formats))
        {
            self.notice = None;
            self.set_drawer_open(false, window, cx);
            self.switch_source(os_id, format, window, cx);
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = (os_id, window, cx);
    }

    /// What the screens' buttons do.
    fn screen_actions(&self, cx: &mut Context<Self>) -> Actions {
        let this = cx.entity().downgrade();
        let (a, b) = (this.clone(), this.clone());
        Actions {
            pick_another: std::rc::Rc::new(move |window, cx| {
                let _ = this.update(cx, |this, cx| this.pick_another_card(window, cx));
            }),
            choose: std::rc::Rc::new(move |os_id, window, cx| {
                let _ = a.update(cx, |this, cx| this.choose_input(os_id, window, cx));
            }),
            allow: std::rc::Rc::new(move |window, cx| {
                permission::request();
                let _ = b.update(cx, |this, cx| this.watch_signal(window, cx));
            }),
            open_settings: std::rc::Rc::new(|privacy, _, _| permission::open_settings(privacy)),
        }
    }

    /// "Restore": puts the card back to xReveal's own Video format after another app changed it.
    fn restore_format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(video) = &mut self.video {
            if let Err(error) = video.retry() {
                self.status = error;
            }
            self.taken_over = None;
        }
        self.watch_signal(window, cx);
        cx.notify();
    }

    /// "Pick another input": the drawer, on the Source tab.
    fn pick_another_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drawer.update(cx, |d, cx| d.show_tab(Tab::Source, cx));
        self.set_drawer_open(true, window, cx);
    }

    /// After the Source on screen changed: apply its remembered picture and delay, remember it,
    /// show its one-time colour tip, and refresh the drawer.
    fn showing_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(video) = &self.video
            && let Some((identity, choice)) = video.current().cloned()
        {
            video.set_picture(self.store.setup.picture_for(&identity));
            video.set_delay(self.store.setup.delay);
            let first_time = !self.store.setup.source_entry(&identity).tip_shown;
            self.store.update(|s| {
                s.source_entry(&identity).tip_shown = true;
                s.source = Some(identity);
                s.video_format = Some(choice);
            });
            if first_time {
                self.show_tip(cx);
            }
        }
        // The sound follows the input on screen.
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        match self.video.as_ref().and_then(|v| v.current().cloned()) {
            Some((identity, _)) => self.sound.start_for(Some(&identity), &self.store.setup),
            None => self.sound.stop(),
        }
        self.refresh_drawer(window, cx);
    }

    fn drawer_model(&self, cx: &App) -> DrawerModel {
        let setup = &self.store.setup;
        // Screens, primary first, each by the name the OS gives it.
        let primary = cx.primary_display().map(|d| d.id());
        let mut displays = cx.displays();
        displays.sort_by_key(|d| Some(d.id()) != primary);
        let monitors = displays
            .iter()
            .enumerate()
            .filter_map(|(i, d)| {
                let id = d.uuid().ok()?.to_string();
                let name = crate::platform::screen_name(u64::from(d.id())).unwrap_or_else(|| format!("Screen {}", i + 1));
                Some((id, name))
            })
            .collect();
        let mut model = DrawerModel {
            picture: setup.picture,
            delay: setup.delay,
            side: setup.drawer_side,
            window_mode: setup.window_mode,
            monitors,
            monitor: setup.monitor.clone(),
            stats: setup.stats,
            look: setup.look,
            audio: self.sound.model(setup),
            frame_interval: 1.0 / 60.0,
            ..Default::default()
        };
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(video) = &self.video {
            model.sources = video.sources().to_vec();
            model.current = video.current().cloned();
            if let Some((identity, _)) = &model.current {
                model.picture = setup.picture_for(identity);
            }
            if let Some(fps) = video.stats().fps {
                model.frame_interval = 1.0 / fps;
            }
        }
        model
    }

    /// Remembers which monitor the window is on and, in Windowed mode, where on it.
    fn remember_placement(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let monitor = window.display(cx).and_then(|d| d.uuid().ok()).map(|u| u.to_string());
        let windowed = self.store.setup.window_mode == WindowMode::Windowed && !window.is_fullscreen();
        // In fullscreen GPUI reports the bounds to go back to, which is just as good.
        let b = window.window_bounds().get_bounds();
        let placement = WindowPlacement { x: b.origin.x.as_f32(), y: b.origin.y.as_f32(), width: b.size.width.as_f32(), height: b.size.height.as_f32() };
        self.store.update(|s| {
            if monitor.is_some() {
                s.monitor = monitor;
            }
            if windowed && placement.usable() {
                s.window = Some(placement);
            }
        });
    }

    /// Moves xReveal to the screen with identity `uuid` (docs/spec/v1.md §2.3 Monitor). In
    /// Fullscreen it leaves fullscreen, moves, and covers the new screen.
    fn move_to_monitor(&mut self, uuid: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(display) = cx.displays().into_iter().find(|d| d.uuid().is_ok_and(|u| u.to_string() == uuid)) else { return };
        let id = u64::from(display.id());
        self.store.update(|s| s.monitor = Some(uuid));
        if self.store.setup.window_mode == WindowMode::Windowed {
            crate::platform::move_window(window, id);
            return;
        }
        // Fullscreen switches happen a moment after they're asked for: leave, move, come back.
        toggle_borderless(window);
        cx.spawn_in(window, async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(350)).await;
            let _ = cx.update(|window, _| crate::platform::move_window(window, id));
            cx.background_executor().timer(Duration::from_millis(350)).await;
            let _ = cx.update(|window, _| toggle_borderless(window));
        })
        .detach();
    }

    /// Windowed or Fullscreen (docs/spec/v1.md §2.2): remembered, and applied to the window.
    fn set_window_mode(&mut self, mode: WindowMode, window: &mut Window, cx: &mut Context<Self>) {
        if mode == self.store.setup.window_mode {
            return;
        }
        self.store.update(|s| s.window_mode = mode);
        // Fullscreen from the macOS green button is its own kind; leave it the same way.
        if cfg!(target_os = "macos") && window.is_fullscreen() {
            if mode == WindowMode::Windowed {
                window.toggle_fullscreen();
            }
        } else {
            toggle_borderless(window);
        }
        self.refresh_drawer(window, cx);
        cx.notify();
    }

    fn refresh_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.drawer_model(cx);
        if let Some((handle, host)) = self.drawer_window.clone() {
            let model = model.clone();
            let drawer = host.read(cx).drawer.clone();
            let _ = handle.update(cx, |_, window, cx| drawer.update(cx, |d, cx| d.set_model(model, window, cx)));
        }
        self.drawer.update(cx, |drawer, cx| drawer.set_model(model, window, cx));
    }

    /// Opens the drawer as its own borderless, transparent window along the drawer side of this one.
    fn open_drawer_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let main = window.bounds();
        // Over the content only, below the title bar if there is one.
        let content = window.viewport_size();
        let top = main.origin.y + (main.size.height - content.height).max(px(0.));
        let width = px(drawer::WIDTH);
        let x = match self.store.setup.drawer_side {
            DrawerSide::Right => main.origin.x + main.size.width - width,
            DrawerSide::Left => main.origin.x,
        };
        let bounds = Bounds { origin: point(x, top), size: size(width, content.height) };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: true,
            show: true,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: window.display(cx).map(|d| d.id()),
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };
        let model = self.drawer_model(cx);
        let main_entity = cx.entity().downgrade();
        let main_window = window.window_handle();
        let side = self.store.setup.drawer_side;
        match gpui_kit::open_window(options, cx, move |window, cx| cx.new(|cx| DrawerWindow::new(model, side, main_entity, main_window, window, cx))) {
            Ok((handle, host)) => {
                let _ = handle.update(cx, |_, window, cx| {
                    gpui_kit::base::Root::update(window, cx, |root, _, _| root.style().background = Some(transparent_black().into()));
                });
                self.drawer_window = Some((handle, host));
            }
            Err(error) => self.status = format!("Couldn't open settings: {error}"),
        }
    }

    /// Slides the drawer window's panel out, then closes the window.
    fn close_drawer_window(&mut self, cx: &mut Context<Self>) {
        if let Some((handle, host)) = self.drawer_window.take() {
            host.update(cx, |host, cx| {
                host.slide = host.slide.toward(false);
                cx.notify();
            });
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(drawer::SLIDE_OUT).await;
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            })
            .detach();
        }
    }

    /// Applies a change made in the drawer: live to the picture, and to the Remembered setup.
    fn apply(&mut self, event: DrawerEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            DrawerEvent::Picture(picture) => {
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                if let Some(video) = &self.video {
                    video.set_picture(picture);
                    if let Some((identity, _)) = video.current().cloned() {
                        self.store.update(|s| {
                            let entry = s.source_entry(&identity);
                            entry.colour_range = picture.colour_range;
                            entry.colour_space = picture.colour_space;
                        });
                    }
                }
                self.store.update(|s| {
                    s.picture.target_ratio = picture.target_ratio;
                    s.picture.fill_mode = picture.fill_mode;
                    s.picture.smart_stretch_strength = picture.smart_stretch_strength;
                    s.picture.adjustments = picture.adjustments;
                });
            }
            DrawerEvent::Delay(delay) => {
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                if let Some(video) = &self.video {
                    video.set_delay(delay);
                }
                self.store.update(|s| s.delay = delay);
                self.sound.set_mix(&self.store.setup);
            }
            DrawerEvent::AudioInput(input) => {
                self.store.update(|s| s.audio.input = input);
                self.restart_sound(window, cx);
            }
            DrawerEvent::AudioOutput(output) => {
                self.store.update(|s| s.audio.output = output);
                self.restart_sound(window, cx);
            }
            DrawerEvent::Volume(volume) => {
                self.store.update(|s| s.audio.volume = volume);
                self.sound.set_mix(&self.store.setup);
            }
            DrawerEvent::Mute(muted) => {
                self.store.update(|s| s.audio.muted = muted);
                self.sound.set_mix(&self.store.setup);
            }
            DrawerEvent::Side(side) => {
                self.store.update(|s| s.drawer_side = side);
                // A drawer in its own window moves to the other edge.
                if self.drawer_window.is_some() {
                    self.close_drawer_window(cx);
                    self.open_drawer_window(window, cx);
                }
            }
            DrawerEvent::Source(os_id, choice) => self.switch_source(os_id, choice, window, cx),
            DrawerEvent::WindowMode(mode) => self.set_window_mode(mode, window, cx),
            DrawerEvent::Monitor(uuid) => self.move_to_monitor(uuid, window, cx),
            DrawerEvent::Stats(stats) => self.apply_stats(stats, cx),
            DrawerEvent::Look(look) => {
                self.store.update(|s| s.look = look);
                theme::apply(&look, theme::appearance_is_dark(window.appearance()), cx);
            }
            DrawerEvent::Quit => cx.quit(),
            DrawerEvent::ResetAll => self.reset_all_settings(window, cx),
        }
        cx.notify();
    }

    /// Reset all settings (docs/spec/v1.md §2.3): forget everything and start as on first run.
    fn reset_all_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.store.setup.window_mode;
        if let Err(error) = self.store.reset_all() {
            self.status = format!("Couldn't reset settings: {error}");
        }
        // Back to a normal window, the default.
        if mode != self.store.setup.window_mode {
            self.store.update(|s| s.window_mode = mode);
            self.set_window_mode(WindowMode::default(), window, cx);
        }
        // Everything forgotten: back to the Welcome screen (docs/spec/v1.md §2.3).
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(video) = &mut self.video {
            video.forget();
        }
        self.sound.stop();
        self.picker = PickerKind::Welcome;
        self.unplugged = false;
        self.set_drawer_open(false, window, cx);
        self.refresh_drawer(window, cx);
        self.watch_signal(window, cx);
    }

    /// Switches to another Source or Video format: the picture restarts briefly, with a notice
    /// until the new format is running (docs/spec/v1.md §2.3).
    fn switch_source(&mut self, os_id: String, choice: crate::source::VideoFormat, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = (os_id, choice, window);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            self.status = "Restarting picture…".into();
            cx.notify();
            // Let the notice paint before opening the device (which blocks for a moment).
            cx.spawn_in(window, async move |this, cx| {
                let _ = this.update_in(cx, |this, window, cx| {
                    if let Some(video) = &mut this.video {
                        this.status = match video.switch_to(&os_id, choice) {
                            Ok(()) => video.description().to_owned(),
                            Err(error) => error,
                        };
                    }
                    this.showing_changed(window, cx);
                    this.watch_signal(window, cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn show_tip(&mut self, cx: &mut Context<Self>) {
        self.tip = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TIP_FOR).await;
            let _ = this.update(cx, |this, cx| {
                this.tip = None;
                cx.notify();
            });
        }));
    }

    fn set_drawer_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if open && !self.drawer_open {
            // Without a live picture, list the cards again: one may have been plugged in since.
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            if !self.screen.shows_picture()
                && let Some(video) = &mut self.video
            {
                video.refresh_sources();
            }
            self.refresh_drawer(window, cx);
        }
        if self.drawer_in_own_window {
            match open {
                true if self.drawer_window.is_none() => self.open_drawer_window(window, cx),
                false => self.close_drawer_window(cx),
                _ => {}
            }
        }
        // Also kept for a drawer in its own window: the performance box moves aside with it.
        if open != self.drawer_open {
            self.slide = self.slide.toward(open);
        }
        // The drawer takes the keys while it's open (its arrow keys), and gives them back after.
        if !self.drawer_in_own_window {
            if open {
                self.drawer.read(cx).focus_handle().focus(window, cx);
            } else {
                self.focus.focus(window, cx);
            }
        }
        self.drawer_open = open;
        self.near_edge = false;
        cx.notify();
    }

    /// F3: shows or hides the performance box, and remembers that.
    fn toggle_stats(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let stats = StatsBox { shown: !self.store.setup.stats.shown, ..self.store.setup.stats };
        self.apply_stats(stats, cx);
        self.refresh_drawer(window, cx);
    }

    /// Remembers the performance box's settings and redraws it four times a second while shown.
    fn apply_stats(&mut self, stats: StatsBox, cx: &mut Context<Self>) {
        self.store.update(|s| s.stats = stats);
        match (stats.shown, self.stats_refresh.is_some()) {
            (true, false) => {
                self.stats_refresh = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_millis(250)).await;
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                }));
            }
            (false, true) => self.stats_refresh = None,
            _ => {}
        }
        cx.notify();
    }

    fn stats(&self) -> Option<Summary> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        return self.video.as_ref().map(VideoPipeline::stats);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        None
    }
}

/// The performance box's rows (label, value), with only what `shows` asks for.
pub fn stats_rows(summary: Option<Summary>, description: &str, launch_to_picture: Option<Duration>, shows: StatsBox) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    let summary = summary.unwrap_or_default();
    let approx = if summary.present_time_estimated { "≈" } else { "" };
    let ms = |pair: Option<(f64, f64)>| match pair {
        Some((p50, p99)) => format!("{approx}{p50:.1} ms · p99 {p99:.1}"),
        None => "–".into(),
    };
    if shows.latency {
        rows.push(("capture → screen", ms(summary.capture_to_screen)));
        rows.push(("xReveal's share", ms(summary.arrival_to_screen)));
        if let Some(launch) = launch_to_picture {
            rows.push(("launch → picture", format!("{} ms", launch.as_millis())));
        }
    }
    if shows.fps {
        rows.push(("fps", summary.fps.map_or("–".into(), |fps| format!("{fps:.1}"))));
    }
    if shows.frames {
        rows.push(("dropped · late", format!("{} · {}", summary.dropped, summary.late)));
    }
    if shows.format {
        rows.push(("format", description.to_owned()));
    }
    rows
}

/// How far from the window's side the performance box sits: clear of the drawer when it is open
/// on that side (following its slide), and of the status line below it at the bottom left.
pub fn stats_inset(corner: crate::setup::Corner, drawer_side: DrawerSide, drawer_shown: f32, status_shown: bool) -> (f32, f32) {
    const MARGIN: f32 = 12.0;
    let same_side = corner.is_left() == (drawer_side == DrawerSide::Left);
    let side = MARGIN + if same_side { drawer::WIDTH * drawer_shown } else { 0.0 };
    let vertical = MARGIN + if status_shown && corner == crate::setup::Corner::BottomLeft { 40.0 } else { 0.0 };
    (side, vertical)
}

/// Whether the pointer at `x` is near the drawer's edge of a window `width` wide.
pub fn near_drawer_edge(x: f32, width: f32, side: DrawerSide) -> bool {
    match side {
        DrawerSide::Right => x >= width - EDGE,
        DrawerSide::Left => x <= EDGE,
    }
}

impl Render for XReveal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = theme::palette();
        let panel = |child: Div| child.px_2().py_1().rounded(px(8.)).bg(pal.panel).border_1().border_color(pal.line).text_color(pal.text).text_sm();
        let side = self.store.setup.drawer_side;
        let width = window.bounds().size.width.as_f32();

        let overlay = self.store.setup.stats.shown.then(|| {
            let stats = self.store.setup.stats;
            let rows = stats_rows(self.stats(), &self.status, crate::video::FIRST_PRESENT.get().copied(), stats);
            if self.slide.moving() {
                window.request_animation_frame();
            }
            let shown = 1.0 + self.slide.offset().as_f32() / drawer::WIDTH;
            let (side_inset, vertical_inset) = stats_inset(stats.corner, side, shown, !self.drawer_open);
            let rows = rows.into_iter().map(|(label, value)| {
                div()
                    .flex()
                    .justify_between()
                    .gap_4()
                    .child(div().text_color(pal.muted).child(label))
                    .child(div().text_color(pal.text).child(value))
            });
            let corner = stats.corner;
            div()
                .absolute()
                .when(corner.is_left(), |d| d.left(px(side_inset)))
                .when(!corner.is_left(), |d| d.right(px(side_inset)))
                .when(corner.is_top(), |d| d.top(px(vertical_inset)))
                .when(!corner.is_top(), |d| d.bottom(px(vertical_inset)))
                .px_3()
                .py_2()
                .rounded(px(10.))
                .bg(pal.panel)
                .border_1()
                .border_color(pal.line)
                .font_family(theme::mono_family())
                .text_xs()
                .line_height(px(18.))
                .flex()
                .flex_col()
                .children(rows)
        });
        let tip = self.tip.is_some().then(|| {
            div().absolute().top_3().w_full().flex().justify_center().child(panel(div()).child(
                "Blacks look grey? Open settings (Tab) › Colour and set Range to Limited, or set your console's RGB range to Full.",
            ))
        });
        let drawer = (!self.drawer_in_own_window && self.slide.visible()).then(|| {
            if self.slide.moving() {
                window.request_animation_frame();
            }
            let offset = self.slide.offset();
            let shell = div()
                .id("drawer")
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(drawer::WIDTH))
                .border_color(pal.line)
                .occlude()
                // Clicks inside the drawer must not reach the picture (which closes it).
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .child(self.drawer.clone());
            match side {
                DrawerSide::Right => shell.right(offset).border_l_1(),
                DrawerSide::Left => shell.left(offset).border_r_1(),
            }
        });
        let handle = (!self.drawer_open && self.near_edge).then(|| {
            let h = div()
                .id("drawer-handle")
                .absolute()
                .top(relative(0.5))
                .mt(px(-36.))
                .w(px(20.))
                .h(px(64.))
                .rounded(px(10.))
                .bg(pal.panel)
                .border_1()
                .border_color(pal.line)
                .flex()
                .items_center()
                .justify_center()
                .text_color(pal.secondary)
                .hover(|d| d.text_color(pal.text))
                .cursor_pointer()
                .child(
                    gpui_kit::component::Icon::new(match side {
                        DrawerSide::Right => gpui_kit::assets::IconName::ChevronLeft,
                        DrawerSide::Left => gpui_kit::assets::IconName::ChevronRight,
                    })
                    .size_4(),
                )
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.set_drawer_open(true, window, cx);
                }));
            match side {
                DrawerSide::Right => h.right_1(),
                DrawerSide::Left => h.left_1(),
            }
        });

        // Screens without a picture sit on black, so they always use the dark palette.
        let screen_palette = theme::palette_for(&crate::setup::Look { theme: crate::setup::ThemeChoice::Dark, ..self.store.setup.look }, true);
        let backdrop = self.screen.backdrop();
        let actions = self.screen_actions(cx);
        let screen = self.screen.render(screen_palette, &actions);
        // A one-time notice (a broken setup set aside), and macOS microphone access turned off.
        let microphone_off = self.screen.shows_picture() && permission::access(Privacy::Microphone) == Access::Denied;
        let notice = self.notice.clone().filter(|_| matches!(self.screen, Screen::Picker { .. })).map(|n| panel(div()).child(n));
        let microphone = microphone_off.then(|| {
            panel(div().flex().items_center().gap_3()).child("Microphone access is off, so xReveal can't play sound").child(
                div()
                    .id("open-microphone-settings")
                    .text_color(pal.accent)
                    .cursor_pointer()
                    .hover(|d| d.text_color(pal.text))
                    .child("Open System Settings")
                    .on_click(|_, _, _| permission::open_settings(Privacy::Microphone)),
            )
        });
        let top_notices = (notice.is_some() || microphone.is_some())
            .then(|| div().absolute().top_3().w_full().flex().flex_col().items_center().gap_2().children(notice).children(microphone));
        let taken_over = self.taken_over.map(|format| {
            let card = self.status.split(" · ").next().unwrap_or("The input").to_owned();
            div().absolute().top_3().w_full().flex().justify_center().child(
                panel(div().flex().items_center().gap_3())
                    .child(format!("Another app changed {card} to {}", drawer::readout(format)))
                    .child(
                        div()
                            .id("restore-format")
                            .text_color(pal.accent)
                            .cursor_pointer()
                            .hover(|d| d.text_color(pal.text))
                            .child("Restore")
                            .on_click(cx.listener(|this, _, window, cx| this.restore_format(window, cx))),
                    ),
            )
        });
        // A one-line status only for what has no screen of its own (e.g. a failed switch).
        let status = (!self.drawer_open && self.screen.shows_picture() && self.status.starts_with("Couldn't")).then(|| panel(div()).child(self.status.clone()));

        div()
            .size_full()
            .relative()
            // Without a live picture nothing is drawn under this view: black, or the last frame dimmed.
            .when_some(backdrop, |d, colour| d.bg(colour))
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ToggleDrawer, window, cx| this.set_drawer_open(!this.drawer_open, window, cx)))
            .on_action(cx.listener(|this, _: &CloseDrawer, window, cx| this.set_drawer_open(false, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleStats, window, cx| this.toggle_stats(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleFullscreen, window, cx| {
                let mode = match this.store.setup.window_mode {
                    WindowMode::Windowed => WindowMode::Fullscreen,
                    WindowMode::Fullscreen => WindowMode::Windowed,
                };
                this.set_window_mode(mode, window, cx);
            }))
            // Right-click on the picture toggles the drawer; a click outside it closes it.
            .on_mouse_down(MouseButton::Right, cx.listener(|this, _, window, cx| this.set_drawer_open(!this.drawer_open, window, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                if this.drawer_open {
                    this.set_drawer_open(false, window, cx);
                }
            }))
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                let near = near_drawer_edge(event.position.x.as_f32(), width, this.store.setup.drawer_side);
                if near != this.near_edge {
                    this.near_edge = near;
                    cx.notify();
                }
            }))
            .flex()
            .items_end()
            .p_3()
            .children(screen)
            .children(top_notices)
            .children(status)
            .children(taken_over)
            .children(tip)
            .children(overlay)
            .children(handle)
            .children(drawer)
    }
}


/// The drawer in its own window (see [`XReveal::open_drawer_window`]). Changes go to the main
/// window's [`XReveal`]; its keys and losing focus close the drawer.
pub struct DrawerWindow {
    drawer: Entity<SettingsDrawer>,
    /// Which edge of the screen it sits on (it is opened again when that changes).
    side: DrawerSide,
    slide: drawer::Slide,
    focus: FocusHandle,
    main: WeakEntity<XReveal>,
    main_window: AnyWindowHandle,
    _subscriptions: Vec<Subscription>,
}

impl DrawerWindow {
    fn new(model: DrawerModel, side: DrawerSide, main: WeakEntity<XReveal>, main_window: AnyWindowHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let drawer = cx.new(|cx| SettingsDrawer::new(model, window, cx));
        let focus = cx.focus_handle();
        // The drawer itself takes the keys, so its arrow keys work; this window's keys still reach
        // the shell around it.
        drawer.read(cx).focus_handle().focus(window, cx);
        let forward = cx.subscribe(&drawer, |this: &mut Self, _, event: &DrawerEvent, cx| {
            let event = event.clone();
            this.with_main(cx, move |main, window, cx| main.apply(event, window, cx));
        });
        // Losing focus to another app closes the drawer, so the pop-up never floats over other
        // apps. A click on the picture moves focus to the main window, which closes it anyway.
        let blur = cx.observe_window_activation(window, |this: &mut Self, window, cx| {
            if window.is_window_active() {
                return;
            }
            let (main, main_window) = (this.main.clone(), this.main_window);
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                let _ = main_window.update(cx, |_, window, cx| {
                    if !window.is_window_active()
                        && let Some(main) = main.upgrade()
                    {
                        main.update(cx, |main, cx| main.set_drawer_open(false, window, cx));
                    }
                });
            })
            .detach();
        });
        let slide = drawer::Slide::closed().toward(true);
        Self { drawer, side, slide, focus, main, main_window, _subscriptions: vec![forward, blur] }
    }

    /// Runs `f` on the main window's view, in the main window.
    fn with_main(&self, cx: &mut App, f: impl FnOnce(&mut XReveal, &mut Window, &mut Context<XReveal>) + 'static) {
        let main = self.main.clone();
        let _ = self.main_window.update(cx, move |_, window, cx| {
            if let Some(main) = main.upgrade() {
                main.update(cx, |main, cx| f(main, window, cx));
            }
        });
    }
}

impl Render for DrawerWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = theme::palette();
        if self.slide.moving() {
            window.request_animation_frame();
        }
        let offset = self.slide.offset();
        let shell = div().absolute().top_0().bottom_0().w(px(drawer::WIDTH)).border_color(pal.line).child(self.drawer.clone());
        let shell = match self.side {
            DrawerSide::Right => shell.right(offset).border_l_1(),
            DrawerSide::Left => shell.left(offset).border_r_1(),
        };
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ToggleDrawer, _, cx| this.with_main(cx, |m, w, cx| m.set_drawer_open(false, w, cx))))
            .on_action(cx.listener(|this, _: &CloseDrawer, _, cx| this.with_main(cx, |m, w, cx| m.set_drawer_open(false, w, cx))))
            .on_action(cx.listener(|this, _: &ToggleStats, _, cx| this.with_main(cx, |m, w, cx| m.toggle_stats(w, cx))))
            .on_action(cx.listener(|this, _: &ToggleFullscreen, _, cx| {
                this.with_main(cx, |m, w, cx| {
                    m.set_drawer_open(false, w, cx);
                    w.dispatch_action(Box::new(ToggleFullscreen), cx);
                })
            }))
            .child(shell)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude has its own `test` attribute that would replace std's.
    use super::{DrawerSide, Duration, Summary, near_drawer_edge, stats_inset, stats_rows};
    use crate::setup::{Corner, StatsBox};

    fn value<'a>(rows: &'a [(&str, String)], label: &str) -> &'a str {
        rows.iter().find(|(l, _)| *l == label).map(|(_, v)| v.as_str()).unwrap_or_else(|| panic!("no {label} row"))
    }

    #[test]
    fn stats_rows_show_timings_or_dashes() {
        let all = StatsBox::default();
        let empty = stats_rows(None, "No input found.", None, all);
        assert_eq!(value(&empty, "capture → screen"), "–");
        assert_eq!(value(&empty, "fps"), "–");
        assert_eq!(value(&empty, "dropped · late"), "0 · 0");
        assert_eq!(empty.len(), 5);

        let summary = Summary {
            capture_to_screen: Some((12.34, 18.1)),
            arrival_to_screen: Some((4.2, 6.0)),
            fps: Some(59.94),
            dropped: 3,
            late: 1,
            present_time_estimated: true,
        };
        let rows = stats_rows(Some(summary), "USB2 Video · 1280x720 @ 60 · 420v", Some(Duration::from_millis(480)), all);
        assert_eq!(value(&rows, "capture → screen"), "≈12.3 ms · p99 18.1");
        assert_eq!(value(&rows, "xReveal's share"), "≈4.2 ms · p99 6.0");
        assert_eq!(value(&rows, "launch → picture"), "480 ms");
        assert_eq!(value(&rows, "fps"), "59.9");
        assert_eq!(value(&rows, "dropped · late"), "3 · 1");

        let just_fps = StatsBox { latency: false, frames: false, format: false, ..all };
        let rows = stats_rows(Some(summary), "", Some(Duration::from_millis(480)), just_fps);
        assert_eq!(rows, vec![("fps", "59.9".to_string())]);
    }

    #[test]
    fn performance_box_moves_clear_of_the_drawer_and_status_line() {
        assert_eq!(stats_inset(Corner::TopRight, DrawerSide::Right, 1.0, false), (392.0, 12.0), "drawer open on its side");
        assert_eq!(stats_inset(Corner::TopRight, DrawerSide::Right, 0.5, false), (202.0, 12.0), "follows the slide");
        assert_eq!(stats_inset(Corner::TopLeft, DrawerSide::Right, 1.0, false), (12.0, 12.0), "drawer on the other side");
        assert_eq!(stats_inset(Corner::BottomLeft, DrawerSide::Right, 0.0, true), (12.0, 52.0), "above the status line");
    }

    #[test]
    fn edge_handle_follows_the_drawer_side() {
        assert!(near_drawer_edge(1900.0, 1920.0, DrawerSide::Right));
        assert!(!near_drawer_edge(20.0, 1920.0, DrawerSide::Right));
        assert!(near_drawer_edge(20.0, 1920.0, DrawerSide::Left));
        assert!(!near_drawer_edge(960.0, 1920.0, DrawerSide::Left));
    }
}
