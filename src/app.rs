//! Root view: the live picture shows through it (it paints no background). On top: the Settings
//! drawer, a one-line status until the screens without a picture exist, the one-time colour tip,
//! and the performance box (F3).

use std::time::{Duration, Instant};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::drawer::{self, DrawerEvent, DrawerModel, SettingsDrawer, Tab};
use crate::permission::{self, Access, Privacy};
use crate::screens::{Actions, Screen};
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
        // macOS's own fullscreen shortcut: on a Mac keyboard F11 needs fn and defaults to Show Desktop.
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-cmd-f", ToggleFullscreen, Some(KEY_CONTEXT)),
        // Windows closes the window on Alt+F4 by itself, which quits too.
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    drawer::bind_keys(cx);
}

/// Switches the window between a normal window and fullscreen. On macOS that is macOS's own
/// fullscreen (a Space of its own): only there does macOS show the picture direct to display, about
/// a refresh sooner than a window stretched over the screen (#56). On Windows its fullscreen is
/// borderless. Call it once per change: the switch happens a moment later.
pub fn toggle_borderless(window: &Window) {
    #[cfg(target_os = "macos")]
    if let Some(handle) = crate::platform::window_handle(window) {
        crate::platform::toggle_fullscreen(handle);
    }
    #[cfg(not(target_os = "macos"))]
    window.toggle_fullscreen();
}

/// How long a fullscreen switch from [`toggle_borderless`] takes to happen: macOS slides into its
/// own Space.
const FULLSCREEN_SWITCH: Duration = Duration::from_millis(if cfg!(target_os = "macos") { 800 } else { 350 });

/// The longest macOS's fullscreen slide is waited for, from asking for it to its start and from its
/// start to its end. Past it the switch counts as finished, in case macOS never says so.
const SWITCH_LIMIT: Duration = Duration::from_secs(3);

/// macOS's fullscreen slide (#203). macOS drops a fullscreen toggle sent while it is sliding, so a
/// Window mode asked for meanwhile waits and is applied when the slide ends. On Windows, Fullscreen
/// is instant and this never runs.
#[derive(Debug, Default)]
struct FullscreenSwitch {
    /// A slide has been asked for or is running.
    running: bool,
    /// The Window mode changed during the slide: apply it when the slide ends.
    queued: bool,
    /// Counts starts and ends, so a time limit can tell whether its switch is still the current one.
    seq: u64,
}

impl FullscreenSwitch {
    /// A slide was asked for, or macOS started one. The number is for [`Self::timed_out`].
    fn start(&mut self) -> u64 {
        self.running = true;
        self.seq += 1;
        self.seq
    }

    /// The slide ended. Whether a Window mode is waiting to be applied.
    fn finish(&mut self) -> bool {
        self.running = false;
        self.seq += 1;
        std::mem::take(&mut self.queued)
    }

    /// Keeps a Window mode change for the end of the slide, if one is running. Whether it did.
    fn wait(&mut self) -> bool {
        self.queued |= self.running;
        self.running
    }

    /// The switch numbered `seq` by [`Self::start`] is still running, with nothing since.
    fn timed_out(&self, seq: u64) -> bool {
        self.running && self.seq == seq
    }
}

/// Borderless window mode on or off (docs/spec/v1.md §2.2), centred on screen `display` (else the
/// one it's on), after `delay`. Done outside GPUI's update, as the native call needs.
pub fn set_borderless(window: &Window, borderless: bool, display: Option<u64>, delay: Duration, cx: &mut App) {
    let Some(handle) = crate::platform::window_handle(window) else { return };
    cx.spawn(async move |cx| {
        cx.background_executor().timer(delay).await;
        crate::platform::set_borderless(handle, borderless, display);
    })
    .detach();
}

/// How close to the drawer's edge the pointer must be for the handle to appear.
const EDGE: f32 = 48.0;
/// The edge handle (assets/notch): its body is 24 wide and 96 tall; concave curves 10 tall above and
/// below flow it out of the edge, so the whole shape is 25 × 116 with 1 px over the edge.
const NOTCH_BODY: f32 = 24.0;
const NOTCH_FILLET: f32 = 10.0;
const NOTCH_HEIGHT: f32 = 96.0 + 2.0 * NOTCH_FILLET;
/// The widest a bar at the top centre gets before its text wraps.
const BAR_WIDTH: f32 = 480.0;
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
    /// The overlays that only show something (performance box, tip, status, edge handle) are drawn
    /// in a click-through window of their own, over the video surface that would hide them
    /// (Windows). `XREVEAL_OVERLAY_WINDOW=1` forces it elsewhere, for testing.
    overlays_in_own_window: bool,
    overlay_window: Option<(AnyWindowHandle, Entity<OverlayWindow>)>,
    /// The Window mode F11 goes back to from Fullscreen: Windowed or Borderless, whichever was last.
    before_fullscreen: WindowMode,
    /// macOS's fullscreen slide, and a Window mode waiting for it to end (#203).
    fullscreen_switch: FullscreenSwitch,
    _fullscreen_watch: Option<Task<()>>,
    /// Where the window and its content were when the drawer and overlay windows were last placed.
    /// GPUI also reports a bounds change whenever the window gains or loses focus; only a real move,
    /// resize or scale change places them again.
    placed: (Bounds<Pixels>, Size<Pixels>, f32),
    /// The pointer is near the drawer's edge: show the handle.
    near_edge: bool,
    /// The edge handle sliding in from the edge or back out.
    handle_slide: drawer::Slide,
    /// The side the drawer is drawn on: it follows the setting after sliding out on the old side.
    shown_side: DrawerSide,
    /// The one-time "Blacks look grey?" tip, while it is up.
    tip: Option<Task<()>>,
    /// Redraws the performance box while it is shown; dropping it stops the refresh.
    stats_refresh: Option<Task<()>>,
    /// What shows where the picture goes (docs/spec/v1.md §2.8), checked twice a second.
    screen: Screen,
    /// The remembered Source starts once camera access is given.
    pending_launch: bool,
    /// Camera and Microphone access as last seen, to notice them granted while xReveal runs (#113).
    access_seen: (Access, Access),
    /// The Drawer swipe in progress (#141).
    swipe: crate::swipe::Swipe,
    /// The picture has shown at least once this run: the update check waits for it (#145).
    picture_seen: bool,
    /// How the last update check this run went (it's running while `Checking`).
    update_check: crate::update::Check,
    /// The in-app update (#163).
    updater: crate::update::Updater,
    _update_watch: Option<Task<()>>,
    /// This version's What's new shows (#144): updated since it was last seen. The handle is its
    /// notes' scroll position, shared with the overlay window's copy on Windows.
    whats_new: Option<ScrollHandle>,
    /// A broken `setup.toml` was set aside: said once, over the Welcome screen.
    notice: Option<String>,
    /// The picture has been one flat colour for a while: a hint over it, never a screen (#108).
    blank_hint: Option<String>,
    /// The wanted card was unplugged: open it again as soon as it is back.
    unplugged: bool,
    /// When to try a busy card again.
    next_retry: Option<Instant>,
    /// The format another app switched the card to, while it differs from xReveal's choice.
    taken_over: Option<VideoFormat>,
    _signal_watch: Option<Task<()>>,
    /// Feeds the Input level meter while the drawer is open; dropping it stops that.
    meter_feed: Option<Task<()>>,
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

        // Without camera access (macOS), the permission screen first; the remembered Source starts
        // once access is given.
        let camera_ready = permission::access(Privacy::Camera) == Access::Granted;
        // Only a remembered input starts by itself; otherwise the no-input screen says to press Tab.
        let remembered_input = store.setup.source.is_some() && !store.first_run;
        let notice = store.notice.take();
        // What's new (#144): once after an update. A first install has nothing to compare with, so
        // it counts this version as seen.
        let whats_new = if store.first_run {
            store.update(|s| s.whats_new_seen = Some(crate::whats_new::VERSION.into()));
            false
        } else {
            store.setup.whats_new_seen.as_deref() != Some(crate::whats_new::VERSION) && !crate::whats_new::notes().is_empty()
        }
        .then(ScrollHandle::new);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let (video, status) = {
            let remembered = store.setup.source.clone();
            match VideoPipeline::start(window, remembered.as_ref().map(|s| (s, store.setup.video_format)), camera_ready && remembered_input) {
                Ok(video) => {
                    let status = video.description().to_owned();
                    (Some(video), status)
                }
                Err(error) => (None, error),
            }
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let status = String::from("The live picture isn't implemented on this OS yet.");

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
            overlays_in_own_window: cfg!(target_os = "windows") || std::env::var_os("XREVEAL_OVERLAY_WINDOW").is_some(),
            overlay_window: None,
            before_fullscreen: match store.setup.window_mode {
                WindowMode::Fullscreen => WindowMode::Windowed,
                mode => mode,
            },
            fullscreen_switch: FullscreenSwitch::default(),
            _fullscreen_watch: None,
            placed: (window.bounds(), window.viewport_size(), window.scale_factor()),
            near_edge: false,
            handle_slide: drawer::Slide::closed(),
            shown_side: store.setup.drawer_side,
            tip: None,
            stats_refresh: None,
            screen: Screen::Starting,
            pending_launch: !camera_ready && remembered_input,
            access_seen: (permission::access(Privacy::Camera), permission::access(Privacy::Microphone)),
            whats_new,
            swipe: Default::default(),
            picture_seen: false,
            update_check: Default::default(),
            updater: Default::default(),
            _update_watch: None,
            notice,
            blank_hint: None,
            unplugged: false,
            next_retry: None,
            taken_over: None,
            _signal_watch: None,
            meter_feed: None,
            sound: Default::default(),
            store,
            _subscriptions: vec![save_on_quit, on_drawer],
        };
        let follow = cx.observe_window_bounds(window, |this: &mut Self, window, cx| {
            let placed = (window.bounds(), window.viewport_size(), window.scale_factor());
            if placed == this.placed {
                // Only focus changed (the drawer window opening, say): reopening the drawer here
                // would close it at once.
                return;
            }
            let (_, _, was_scale) = std::mem::replace(&mut this.placed, placed);
            this.remember_placement(window, cx);
            // On another screen the pixel density may differ (Retina and 1×).
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            if let Some(video) = &this.video {
                video.set_scale(window.scale_factor());
            }
            if cfg!(target_os = "windows") && placed.2 == was_scale {
                // Moved or resized at the same density: the pop-ups move along with the window.
                // Reopening them on every step of a drag would flicker and replay the drawer's slide.
                this.place_popups(window, cx);
                return;
            }
            // The overlay first, so the drawer (opened after it) stays on top.
            if this.overlay_window.is_some() {
                this.close_overlay_window(cx);
                this.open_overlay_window(window, cx);
            }
            if this.drawer_window.is_some() {
                this.close_drawer_window(cx);
                this.open_drawer_window(window, cx);
            }
        });
        this._subscriptions.push(follow);
        if this.overlays_in_own_window {
            cx.defer_in(window, |this, window, cx| this.open_overlay_window(window, cx));
        }
        // The remembered look; with System it follows the OS's appearance as that changes.
        theme::apply(&this.store.setup.look, theme::appearance_is_dark(window.appearance()), cx);
        window.set_rem_size(px(theme::spacing().rem));
        let appearance = cx.observe_window_appearance(window, |this: &mut Self, window, cx| {
            theme::apply(&this.store.setup.look, theme::appearance_is_dark(window.appearance()), cx);
        });
        this._subscriptions.push(appearance);
        this.watch_fullscreen(window, cx);
        this.showing_changed(window, cx);
        this.watch_signal(window, cx);
        this.watch_for_updates(window, cx);
        // After an in-app update (#163): the previous version goes, off the main thread.
        cx.background_executor().spawn(async { crate::platform::clean_up_update() }).detach();
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
        // The driver's control channel, for unattended runs (#204).
        if crate::drive::wanted() {
            crate::drive::start(window, cx);
        }
        this
    }

    /// The driver's `state` (#204): one JSON line with what a script needs to see that a step has
    /// finished.
    pub fn drive_state(&self, window: &Window, cx: &App) -> String {
        let bounds = window.bounds();
        let stats = self.stats().unwrap_or_default();
        let tab = match &self.drawer_window {
            Some((_, host)) => host.read(cx).drawer.read(cx).tab(),
            None => self.drawer.read(cx).tab(),
        };
        serde_json::json!({
            "drawer_open": self.drawer_open,
            "drawer_tab": format!("{tab:?}"),
            "stats_shown": self.store.setup.stats.shown,
            "window_mode": format!("{:?}", self.store.setup.window_mode),
            "fullscreen": window.is_fullscreen(),
            // macOS is sliding into or out of Fullscreen (#203): `fullscreen` isn't settled yet.
            "transition_in_progress": self.fullscreen_switch.running,
            "bounds": [bounds.origin.x.as_f32(), bounds.origin.y.as_f32(), bounds.size.width.as_f32(), bounds.size.height.as_f32()],
            "monitor": self.store.setup.monitor,
            "screen": format!("{:?}", self.screen),
            "status": self.status,
            "audio_output": self.store.setup.audio.output,
            "fps": stats.fps,
            "dropped": stats.dropped,
            "late": stats.late,
            "mouse": [window.mouse_position().x.as_f32(), window.mouse_position().y.as_f32()],
            // How the last run ended: #205 fills it in.
            "last_run": null,
        })
        .to_string()
    }

    /// The drawer's own window while it's open there (Windows): it has xReveal's keys, not the main
    /// window. `None` when the main window has them.
    pub fn keys_window(&self) -> Option<AnyWindowHandle> {
        self.drawer_window.as_ref().filter(|_| self.drawer_open).map(|(handle, _)| *handle)
    }

    /// Where a pointer at `position` in this window's content goes: the drawer's own window and the
    /// point in it, when the drawer is open there and the point is over it. `None`: this window.
    pub fn pointer_target(&self, position: Point<Pixels>, window: &Window) -> Option<(AnyWindowHandle, Point<Pixels>)> {
        let handle = self.keys_window()?;
        let area = drawer_area(window.viewport_size(), self.store.setup.drawer_side);
        area.contains(&position).then(|| (handle, position - area.origin))
    }

    /// Works out what the Source is doing (docs/spec/v1.md §2.8) and reacts: reopens the card when
    /// it comes back or stops being busy, and notices another app changing its format.
    fn watch_signal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.watch_access(window, cx);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let screen = 'screen: {
            // The first screen stays until Camera is allowed and Microphone is allowed or skipped (#143).
            let camera = permission::access(Privacy::Camera);
            let microphone = permission::access(Privacy::Microphone);
            // Windows never asks: with its camera switch off it says so (#176), and the sound
            // never holds up the picture.
            if cfg!(target_os = "windows") && camera == Access::Denied {
                break 'screen Screen::Blocked;
            }
            if cfg!(target_os = "macos") && (camera != Access::Granted || (microphone != Access::Granted && !self.store.setup.sound_skipped)) {
                break 'screen Screen::Permission { camera, microphone };
            }
            let Some(video) = &mut self.video else { return };
            // The device's own settings window (Windows): its stream may be closed while it's open.
            if video.poll_device_settings() {
                self.status = video.description().to_owned();
                self.showing_changed(window, cx);
                return self.watch_signal(window, cx);
            }
            if video.settings_paused() {
                break 'screen Screen::Starting;
            }
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
                // Nothing on screen: chosen in settings (Tab); nothing starts by itself.
                None => Screen::no_input(self.store.setup.source.as_ref().map(|s| s.name.as_str())),
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
        // A stream restarted after a dropout or a device returning can meet the echo guard too.
        self.guard_echo(window, cx);
        if screen != self.screen {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            if let Some(video) = &self.video {
                // Windows' surface covers GPUI's screens, so it hides while one shows.
                video.set_surface_visible(matches!(screen, Screen::Picture | Screen::Starting));
            }
            self.screen = screen;
            self.picture_seen |= self.screen.shows_picture();
            cx.notify();
        }
        // A live picture that has stayed one flat colour: some cards send that with no signal.
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let blank_hint = self
            .video
            .as_ref()
            .filter(|video| self.screen == Screen::Picture && video.flat_for() >= crate::video::flat::HINT_AFTER)
            .and_then(|video| video.current().map(|(identity, _)| blank_hint(&identity.name)));
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let blank_hint = None;
        if blank_hint != self.blank_hint {
            self.blank_hint = blank_hint;
            cx.notify();
        }
        let _ = window;
    }

    /// Camera or Microphone access changed while xReveal runs (#113), e.g. granted from a locked
    /// input's **Allow access** button or in System Settings: unlock the inputs, and pick up what
    /// macOS only reveals with access.
    fn watch_access(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = (permission::access(Privacy::Camera), permission::access(Privacy::Microphone));
        let (camera_was, microphone_was) = std::mem::replace(&mut self.access_seen, now);
        if now == (camera_was, microphone_was) {
            return;
        }
        if now.0 == Access::Granted && camera_was != Access::Granted {
            crate::video::mark_access_granted();
            // The inputs were listed before access: macOS can list them without their formats, and
            // a remembered Video format must be looked for in the real list.
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            if let Some(video) = &mut self.video {
                video.refresh_sources();
                // Windows stopped the input when its switch went off (#176): open it again.
                #[cfg(target_os = "windows")]
                if camera_was == Access::Denied && video.wanted().is_some() && video.retry().is_ok() {
                    self.status = video.description().to_owned();
                }
            }
        }
        // Given: the sound starts. Taken away (Windows' switch, #176): the input it played from
        // fails, so it stops, without an error of its own; the access box says why.
        if now.1 != microphone_was {
            self.restart_sound(window, cx);
        }
        self.refresh_drawer(window, cx);
        cx.notify();
    }

    /// Plays the input on screen again with the current route (it changed, or the buffer grew).
    fn restart_sound(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some((identity, _)) = self.video.as_ref().and_then(|v| v.current().cloned()) {
            self.sound.start_for(Some(&identity), &self.store.setup);
        }
        self.guard_echo(window, cx);
        self.refresh_drawer(window, cx);
    }

    /// The echo guard muted the sound, or unmuted it on leaving an echoing route (#112,
    /// docs/spec/v1.md §2.7): remember the mute, so the Audio tab shows it and the person can undo
    /// it, and show why.
    fn guard_echo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(muted) = self.sound.take_mute_change() {
            self.store.update(|s| s.audio.muted = muted);
            self.sound.set_mix(&self.store.setup);
            self.refresh_drawer(window, cx);
            cx.notify();
        }
    }

    /// What the screens' buttons do.
    fn screen_actions(&self, cx: &mut Context<Self>) -> Actions {
        let this = cx.entity().downgrade();
        let b = this.clone();
        let c = this.clone();
        Actions {
            pick_another: std::rc::Rc::new(move |window, cx| {
                let _ = this.update(cx, |this, cx| this.pick_another_card(window, cx));
            }),
            allow: std::rc::Rc::new(move |privacy, window, cx| {
                permission::allow(privacy);
                let _ = b.update(cx, |this, cx| this.watch_signal(window, cx));
            }),
            open_settings: std::rc::Rc::new(|privacy, _, _| permission::open_settings(privacy)),
            skip_sound: std::rc::Rc::new(move |window, cx| {
                let _ = c.update(cx, |this, cx| {
                    this.store.update(|s| s.sound_skipped = true);
                    this.watch_signal(window, cx);
                });
            }),
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
        self.guard_echo(window, cx);
        self.refresh_drawer(window, cx);
    }

    fn drawer_model(&self, cx: &App) -> DrawerModel {
        let setup = &self.store.setup;
        // Screens, primary first, each by the name the OS gives it, told apart when two share one.
        let primary = cx.primary_display().map(|d| d.id());
        let mut displays = cx.displays();
        displays.sort_by_key(|d| Some(d.id()) != primary);
        let (ids, named): (Vec<String>, Vec<NamedScreen>) = displays
            .iter()
            .enumerate()
            .filter_map(|(i, d)| {
                let id = d.uuid().ok()?.to_string();
                let name = crate::platform::screen_name(u64::from(d.id())).unwrap_or_else(|| format!("Screen {}", i + 1));
                let size = d.bounds().size;
                Some((id, (name, (size.width.as_f32().round() as u32, size.height.as_f32().round() as u32))))
            })
            .unzip();
        let monitors = ids.into_iter().zip(screen_labels(&named)).collect();
        let mut model = DrawerModel {
            picture: setup.picture,
            delay: setup.delay,
            side: setup.drawer_side,
            swipe: setup.drawer_swipe,
            check_updates: setup.updates.check,
            update: setup.updates.offer(crate::whats_new::VERSION).map(|v| (v.to_owned(), setup.updates.notes.clone())),
            update_check: self.update_check,
            updater: self.updater.view(),
            window_mode: setup.window_mode,
            monitors,
            monitor: setup.monitor.clone(),
            stats: setup.stats,
            look: setup.look,
            audio: self.sound.model(setup),
            frame_interval: 1.0 / 60.0,
            camera: self.access_seen.0,
            microphone: self.access_seen.1,
            ..Default::default()
        };
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(video) = &self.video {
            model.sources = video.sources().to_vec();
            model.current = video.current().cloned();
            model.device_settings = video.has_device_settings();
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
        // The screen under the window's middle. GPUI's own idea of it can lag behind on Windows: after
        // a Borderless window moves to another screen it still names the old one.
        let middle = window.bounds().center();
        let display = cx.displays().into_iter().find(|d| d.bounds().contains(&middle)).or_else(|| window.display(cx));
        let monitor = display.and_then(|d| d.uuid().ok()).map(|u| u.to_string());
        let windowed = self.store.setup.window_mode == WindowMode::Windowed && !window.is_fullscreen();
        let b = window.window_bounds().get_bounds();
        let placement = saved_placement(b, window.viewport_size());
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
    /// Fullscreen it leaves fullscreen, moves, and covers the new screen; a Borderless window is
    /// centred on it.
    fn move_to_monitor(&mut self, uuid: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(display) = cx.displays().into_iter().find(|d| d.uuid().is_ok_and(|u| u.to_string() == uuid)) else { return };
        let id = u64::from(display.id());
        self.store.update(|s| s.monitor = Some(uuid));
        let Some(handle) = crate::platform::window_handle(window) else { return };
        // The move itself runs outside this update (see `platform::move_window`).
        match self.store.setup.window_mode {
            WindowMode::Windowed => cx.spawn(async move |_, _| crate::platform::move_window(handle, id)).detach(),
            WindowMode::Borderless => set_borderless(window, true, Some(id), Duration::ZERO, cx),
            WindowMode::Fullscreen => {
                // Fullscreen switches happen a moment after they're asked for: leave, move, come back.
                toggle_borderless(window);
                cx.spawn_in(window, async move |_, cx| {
                    cx.background_executor().timer(FULLSCREEN_SWITCH).await;
                    crate::platform::move_window(handle, id);
                    cx.background_executor().timer(FULLSCREEN_SWITCH).await;
                    let _ = cx.update(|window, _| toggle_borderless(window));
                })
                .detach();
            }
        }
    }

    /// Windowed, Borderless or Fullscreen (docs/spec/v1.md §2.2): remembered, and applied to the
    /// window.
    fn set_window_mode(&mut self, mode: WindowMode, window: &mut Window, cx: &mut Context<Self>) {
        let was = self.store.setup.window_mode;
        if mode == was {
            return;
        }
        self.store.update(|s| s.window_mode = mode);
        if mode != WindowMode::Fullscreen {
            self.before_fullscreen = mode;
        }
        // macOS would drop a toggle sent during its slide (#203): the mode asked for last is applied
        // when the slide ends.
        if !self.fullscreen_switch.wait() {
            // On macOS the window itself says whether it's fullscreen: the green button enters the
            // same fullscreen as the Window setting.
            let in_fullscreen = if cfg!(target_os = "macos") { window.is_fullscreen() } else { was == WindowMode::Fullscreen };
            self.apply_window_mode(in_fullscreen, window, cx);
        }
        self.refresh_drawer(window, cx);
        cx.notify();
    }

    /// Makes the window what the remembered Window mode says, from fullscreen or not.
    fn apply_window_mode(&mut self, in_fullscreen: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.store.setup.window_mode;
        if mode == WindowMode::Fullscreen {
            if !in_fullscreen {
                self.switch_fullscreen(window, cx);
            }
        } else if in_fullscreen {
            self.switch_fullscreen(window, cx);
            // Leaving fullscreen brings back the frame the window had before; then the one asked for.
            set_borderless(window, mode == WindowMode::Borderless, None, FULLSCREEN_SWITCH, cx);
        } else {
            set_borderless(window, mode == WindowMode::Borderless, None, Duration::ZERO, cx);
        }
    }

    /// Enters or leaves fullscreen. On macOS that starts a slide: until it ends, Window mode changes
    /// wait (#203).
    fn switch_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cfg!(target_os = "macos") {
            self.fullscreen_slide(true, window, cx);
        }
        toggle_borderless(window);
    }

    /// Follows macOS's fullscreen slides, whoever starts them: F11, the Window setting, the green
    /// button, or the launch into a remembered Fullscreen (#203).
    fn watch_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        if let Some(handle) = crate::platform::window_handle(window) {
            use futures::StreamExt as _;
            let (sender, mut slides) = futures::channel::mpsc::unbounded();
            crate::platform::watch_fullscreen(handle, move |started| {
                let _ = sender.unbounded_send(started);
            });
            self._fullscreen_watch = Some(cx.spawn_in(window, async move |this, cx| {
                while let Some(started) = slides.next().await {
                    if this.update_in(cx, |this, window, cx| this.fullscreen_slide(started, window, cx)).is_err() {
                        break;
                    }
                }
            }));
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (window, cx);
    }

    /// macOS's fullscreen slide started (or was asked for), or ended. At its end, a Window mode
    /// asked for during it is applied. A slide that never says it ended counts as ended after
    /// [`SWITCH_LIMIT`].
    fn fullscreen_slide(&mut self, started: bool, window: &mut Window, cx: &mut Context<Self>) {
        if started {
            let seq = self.fullscreen_switch.start();
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(SWITCH_LIMIT).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.fullscreen_switch.timed_out(seq) {
                        this.fullscreen_slide(false, window, cx);
                    }
                });
            })
            .detach();
        } else if self.fullscreen_switch.finish() {
            self.apply_window_mode(window.is_fullscreen(), window, cx);
        }
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
        let area = drawer_area(window.viewport_size(), self.store.setup.drawer_side);
        let bounds = Bounds { origin: content_origin(window.bounds(), window.viewport_size()) + area.origin, size: area.size };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: true,
            // Shown once it has lost the frame Windows gives it (`own_popup`): shown at once, Windows 11
            // draws its light border round it for a frame, a white line along the drawer's inner edge.
            show: false,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: window.display(cx).map(|d| d.id()),
            window_background: WindowBackgroundAppearance::Transparent,
            // Full frame rate while sliding out, after the keys have gone back to the main window:
            // GPUI otherwise draws a window without focus at 30 fps.
            inactive_frame_interval: None,
            ..Default::default()
        };
        let model = self.drawer_model(cx);
        // Back on the tab it was showing (a drawer window is a new one each time).
        let tab = self.drawer.read(cx).tab();
        let main_entity = cx.entity().downgrade();
        let main_window = window.window_handle();
        let side = self.store.setup.drawer_side;
        let owner = crate::platform::window_handle(window);
        match gpui_kit::open_window(options, cx, move |window, cx| {
            window.set_rem_size(px(theme::spacing().rem));
            cx.new(|cx| DrawerWindow::new(model, tab, side, main_entity, main_window, window, cx))
        }) {
            Ok((handle, host)) => {
                // A first reading for the Input level meter before it's shown (#180): a new
                // drawer's meter would otherwise say "No sound playing" until the first one, 50 ms
                // later, and the Audio tab's settings would jump up when that line goes.
                let peaks = self.sound.take_peaks();
                let drawer = host.read(cx).drawer.clone();
                drawer.update(cx, |d, cx| d.set_peaks(peaks, cx));
                let _ = handle.update(cx, |_, window, cx| {
                    gpui_kit::base::Root::update(window, cx, |root, _, _| root.style().background = Some(transparent_black().into()));
                    if let Some(owner) = owner {
                        crate::platform::own_popup(window, owner);
                    }
                    // Its first frame was drawn before the root was made see-through; draw it again.
                    window.refresh();
                    // Shows it, with the keys.
                    window.activate_window();
                });
                self.drawer_window = Some((handle, host));
                self.place_popups(window, cx);
            }
            Err(error) => self.status = format!("Couldn't open settings: {error}"),
        }
    }

    /// Opens the click-through overlay window over this one's content (see [`OverlayWindow`]).
    fn open_overlay_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = Bounds { origin: content_origin(window.bounds(), window.viewport_size()), size: window.viewport_size() };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: false,
            show: true,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: window.display(cx).map(|d| d.id()),
            window_background: WindowBackgroundAppearance::Transparent,
            // It never has focus; at GPUI's 30 fps for such windows, the edge handle and performance box
            // would lag the drawer.
            inactive_frame_interval: None,
            ..Default::default()
        };
        let Some(owner) = crate::platform::window_handle(window) else { return };
        let main_entity = cx.entity();
        let overlays = self.overlays();
        if let Ok((handle, host)) = gpui_kit::open_window(options, cx, move |window, cx| {
            window.set_rem_size(px(theme::spacing().rem));
            cx.new(|cx| OverlayWindow::new(&main_entity, overlays, cx))
        }) {
            let _ = handle.update(cx, |_, window, cx| {
                gpui_kit::base::Root::update(window, cx, |root, _, _| root.style().background = Some(transparent_black().into()));
                crate::platform::make_overlay(window, owner);
                // Its first frame was drawn before the root was made see-through; draw it again.
                window.refresh();
            });
            self.overlay_window = Some((handle, host));
            self.place_popups(window, cx);
        }
    }

    /// Puts the drawer's and the overlays' windows exactly over this window's content (Windows).
    fn place_popups(&self, window: &Window, cx: &mut Context<Self>) {
        let Some(owner) = crate::platform::window_handle(window) else { return };
        let (content, scale) = (window.viewport_size(), window.scale_factor());
        let overlay = self.overlay_window.as_ref().map(|(h, _)| (*h, Bounds { origin: Point::default(), size: content }));
        let drawer = self.drawer_window.as_ref().map(|(h, _)| (*h, drawer_area(content, self.store.setup.drawer_side)));
        for (handle, area) in overlay.into_iter().chain(drawer) {
            let Ok(Some(popup)) = handle.update(cx, |_, window, _| crate::platform::window_handle(window)) else { continue };
            // After this update: GPUI handles a window's move as it happens, which it can't mid-update.
            cx.spawn(async move |_, _| crate::platform::place_popup(popup, owner, area, scale)).detach();
        }
    }

    fn close_overlay_window(&mut self, cx: &mut Context<Self>) {
        if let Some((handle, _)) = self.overlay_window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    /// What the display-only overlays show right now (drawn here, or copied to the overlay window).
    fn overlays(&self) -> Overlays {
        let stats = self.store.setup.stats;
        Overlays {
            notice: self.notice.clone().filter(|_| matches!(self.screen, Screen::NoInput { .. })),
            // A one-line status only for what has no screen of its own (e.g. a failed switch).
            status: (!self.drawer_open && self.screen.shows_picture() && self.status.starts_with("Couldn't")).then(|| self.status.clone()),
            tip: self.tip.is_some(),
            blank_hint: self.blank_hint.clone().filter(|_| self.tip.is_none()),
            echo: self.screen.shows_picture() && self.sound.echo_muted(&self.store.setup),
            microphone_off: self.overlays_in_own_window && self.screen.shows_picture() && permission::access(Privacy::Microphone) == Access::Denied,
            stats: stats.shown.then(|| (stats, stats_rows(self.stats(), &self.status, crate::video::time_to_picture(crate::video::FIRST_PRESENT.get().copied(), crate::video::ACCESS_GRANTED.get().copied()), stats))),
            slide: self.slide,
            drawer_open: self.drawer_open,
            side: self.shown_side,
            handle: self.handle_slide,
            whats_new: self.whats_new.clone().filter(|_| self.overlays_in_own_window),
        }
    }
}

impl XReveal {
    /// Update available (#145): every minute, once the picture has shown, ask GitHub when a day has
    /// passed since it last answered (sooner when `now`, the switch just turned on).
    fn watch_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._update_watch = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(60)).await;
                if this.update_in(cx, |this, window, cx| this.check_for_update(false, window, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    /// The in-app update (#163): downloads the offered version, checks it and gets it ready, while
    /// the update button's ring follows the download. Nothing installs until Restart is pressed.
    fn download_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(version) = self.store.setup.updates.offer(crate::whats_new::VERSION).map(str::to_owned) else { return };
        if matches!(self.updater, crate::update::Updater::Downloading(_) | crate::update::Updater::Ready(_)) {
            return;
        }
        let progress = std::sync::Arc::new(crate::update::Progress::default());
        self.updater = crate::update::Updater::Downloading(progress.clone());
        self.refresh_drawer(window, cx);
        let fetching = progress.clone();
        let done = cx.background_executor().spawn(async move { crate::update::fetch_update(&version, &fetching) });
        // The ring follows the download while it runs.
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(150)).await;
                let downloading = this.update_in(cx, |this, window, cx| {
                    let downloading = matches!(this.updater, crate::update::Updater::Downloading(_));
                    if downloading {
                        this.refresh_drawer(window, cx);
                    }
                    downloading
                });
                match downloading {
                    Ok(true) => {}
                    Ok(false) => break,
                    // xReveal closed mid-download: stop it.
                    Err(_) => {
                        progress.cancel();
                        break;
                    }
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            let result = done.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.updater = match result {
                    Ok(prepared) => crate::update::Updater::Ready(prepared),
                    Err(why) => crate::update::Updater::Failed(why),
                };
                this.refresh_drawer(window, cx);
            });
        })
        .detach();
    }

    fn check_for_update(&mut self, now: bool, window: &mut Window, cx: &mut Context<Self>) {
        let updates = &self.store.setup.updates;
        let checking = self.update_check == crate::update::Check::Checking;
        // By hand (`now`: the update button, or the switch turned on) it always asks; by itself
        // only with the switch on, once the picture has shown, and once a day.
        if checking || !(now || (updates.check && self.picture_seen && updates.due(crate::update::now()))) {
            return;
        }
        self.update_check = crate::update::Check::Checking;
        self.refresh_drawer(window, cx);
        let answer = cx.background_executor().spawn(async { crate::platform::latest_release() });
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _ = this.update_in(cx, |this, window, cx| {
                // No usable answer (offline, a proxy, rate-limited): nothing shows but the button's
                // tip, and it's tried again later.
                this.update_check = match answer {
                    Some(answer) => {
                        this.store.update(|s| s.updates.answered(crate::update::now(), answer));
                        crate::update::Check::Answered
                    }
                    None => crate::update::Check::Failed,
                };
                this.refresh_drawer(window, cx);
            });
        })
        .detach();
    }

    /// One scroll event of a possible Drawer swipe (#141): fingers away from the drawer's side
    /// open it, back towards it close it.
    fn on_swipe(&mut self, phase: TouchPhase, fingers: (f32, f32), window: &mut Window, cx: &mut Context<Self>) {
        if !self.store.setup.drawer_swipe {
            return;
        }
        let Some(went) = self.swipe.feed(phase, fingers) else { return };
        let away = match self.shown_side {
            DrawerSide::Right => crate::swipe::Fingers::Left,
            DrawerSide::Left => crate::swipe::Fingers::Right,
        };
        let open = went == away;
        if open != self.drawer_open {
            self.set_drawer_open(open, window, cx);
        }
    }
}

/// Hears every scroll in the window before anything else does (a capture-phase listener, so a
/// list or a dialog that keeps a scroll to itself can't hide a swipe), and hands trackpad scrolls
/// to [`XReveal::on_swipe`]. Only listens; never stops an event.
fn swipe_listener(app: WeakEntity<XReveal>) -> impl IntoElement {
    swipe_canvas("main", move |phase, fingers, window, cx| {
        let _ = app.update(cx, |app, cx| app.on_swipe(phase, fingers, window, cx));
    })
}

/// The same in the drawer's own window (Windows), handing the swipe to the main window's view.
fn drawer_swipe_listener(host: WeakEntity<DrawerWindow>) -> impl IntoElement {
    swipe_canvas("drawer", move |phase, fingers, _, cx| {
        let _ = host.update(cx, |host, cx| host.with_main(cx, move |main, window, cx| main.on_swipe(phase, fingers, window, cx)));
    })
}

/// An empty element that, each frame, listens for scrolls in its window in the capture phase and
/// passes precise (trackpad) ones on as finger movement.
fn swipe_canvas(which: &'static str, on_scroll: impl Fn(TouchPhase, (f32, f32), &mut Window, &mut App) + Clone + 'static) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let on_scroll = on_scroll.clone();
            window.on_mouse_event(move |event: &ScrollEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    log_scroll(which, event);
                }
                // Precise scrolling only: a mouse wheel's line steps are never a swipe.
                let (DispatchPhase::Capture, ScrollDelta::Pixels(delta)) = (phase, event.delta) else { return };
                let sign = if crate::platform::scroll_follows_fingers() { 1. } else { -1. };
                on_scroll(event.touch_phase, (sign * delta.x.as_f32(), sign * delta.y.as_f32()), window, cx);
            });
        },
    )
    .absolute()
    .size_0()
}

/// With `XREVEAL_LOG_SCROLL` set, every scroll a window hears goes to `scroll.log` next to
/// `setup.toml`: which window, the phase, the delta (pixels or lines), where, and whether the
/// delta follows the fingers. For checking the Drawer swipe on a device (#140, #160).
fn log_scroll(which: &str, event: &ScrollEvent) {
    use std::io::Write;
    static LOG: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let log = LOG.get_or_init(|| {
        std::env::var_os("XREVEAL_LOG_SCROLL")?;
        let path = crate::setup::default_path()?.with_file_name("scroll.log");
        std::fs::OpenOptions::new().create(true).append(true).open(path).ok().map(std::sync::Mutex::new)
    });
    let Some(file) = log else { return };
    let delta = match event.delta {
        ScrollDelta::Pixels(d) => format!("pixels {:.1},{:.1}", d.x.as_f32(), d.y.as_f32()),
        ScrollDelta::Lines(d) => format!("lines {:.2},{:.2}", d.x, d.y),
    };
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0., |d| d.as_secs_f64());
    if let Ok(mut file) = file.lock() {
        let _ = writeln!(
            file,
            "{at:.3} {which} {:?} {delta} at {:.0},{:.0} follows-fingers={}",
            event.touch_phase,
            event.position.x.as_f32(),
            event.position.y.as_f32(),
            crate::platform::scroll_follows_fingers()
        );
    }
}

/// A copy of what the display-only overlays show: the overlay window draws from it, never from
/// [`XReveal`] itself (which may be mid-update when that window draws).
#[derive(Clone)]
struct Overlays {
    notice: Option<String>,
    status: Option<String>,
    tip: bool,
    blank_hint: Option<String>,
    /// The echo guard muted the sound (#112).
    echo: bool,
    /// Microphone access is off while the picture shows: a bar here where the overlays have their
    /// own window (Windows, #176), since that window can't take the link's click; the main window
    /// shows it, with the link, otherwise.
    microphone_off: bool,
    stats: Option<(StatsBox, Vec<(&'static str, String)>)>,
    slide: drawer::Slide,
    drawer_open: bool,
    side: DrawerSide,
    handle: drawer::Slide,
    /// Draw What's new, on this scroll position (only where the overlays have their own window).
    whats_new: Option<ScrollHandle>,
}

impl Overlays {
    /// The performance box, the bars at the top centre (the one-time notice, the echo and blank
    /// picture hints, the colour tip, and `extra` from the main window), the one-line status and the
    /// edge handle.
    fn render(&self, window: &mut Window, extra: Vec<AnyElement>) -> Vec<AnyElement> {
        let pal = theme::palette();
        let panel = |child: Div| child.px_2().py_1().rounded(px(8.)).bg(pal.panel).border_1().border_color(pal.line).text_color(pal.text).text_sm();
        let bar = |text: SharedString| panel(div().max_w(px(BAR_WIDTH))).child(text).into_any_element();
        let side = self.side;
        let mut out = Vec::new();
        if let Some(scroll) = &self.whats_new {
            // Shown only: the main window's invisible twin takes the clicks and the scrolling.
            out.push(crate::whats_new::dialog(pal, &crate::whats_new::notes(), scroll, None));
        }
        if let Some(status) = self.status.clone() {
            out.push(panel(div()).child(status).into_any_element());
        }
        // The bars at the top centre, one under another when more than one shows.
        let mut bars: Vec<AnyElement> = Vec::new();
        bars.extend(self.notice.clone().map(|n| bar(n.into())));
        if self.microphone_off {
            bars.push(bar("Microphone access is off, so xReveal can't play sound. Settings (Tab) › Audio turns it back on.".into()));
        }
        if self.echo {
            bars.push(bar(crate::audio::echo::MUTED_NOTICE.into()));
        }
        bars.extend(self.blank_hint.clone().map(|hint| bar(hint.into())));
        if self.tip {
            bars.push(bar("Blacks look grey? Open settings (Tab) › Colour and set Range to Limited, or set your console's RGB range to Full.".into()));
        }
        bars.extend(extra);
        let bar_column = |bars: Vec<AnyElement>| div().flex().flex_col().items_center().gap_2().children(bars);
        let mut bars = Some(bars).filter(|b| !b.is_empty());
        if let Some((stats, rows)) = self.stats.clone() {
            if self.slide.moving() {
                window.request_animation_frame();
            }
            let shown = 1.0 + self.slide.offset().as_f32() / drawer::width();
            let (side_inset, vertical_inset) = stats_inset(stats.corner, side, shown, !self.drawer_open);
            let corner = stats.corner;
            let stats_box = || div()
                .flex_none()
                .px_3()
                .py_2()
                .rounded(px(10.))
                // Its own opacity (Display › Performance box), not the drawer's.
                .bg(Hsla { a: stats.opacity(), ..pal.panel })
                .border_1()
                .border_color(pal.line)
                .font_family(theme::mono_family())
                .text_xs()
                .line_height(px(18.))
                .flex()
                .flex_col()
                .children(rows.iter().map(|(label, value)| {
                    div()
                        .flex()
                        .justify_between()
                        .gap_4()
                        .child(div().text_color(pal.muted).child(*label))
                        .child(div().text_color(pal.text).child(value.clone()))
                }));
            if corner.is_top() {
                // In a top corner the box and the bars share the top row, with an invisible copy of
                // the box on the other side: the bars stay centred in the window and never run
                // under the box (#114).
                let area = div().flex_1().min_w_0().flex().justify_center().children(bars.take().map(bar_column));
                // The box keeps its own inset (it slides aside with the drawer on its side).
                let slid = px(side_inset - STATS_MARGIN);
                let row = div().absolute().top(px(vertical_inset)).left(px(STATS_MARGIN)).right(px(STATS_MARGIN)).flex().items_start().gap_3();
                let twin = stats_box().invisible();
                let row = if corner.is_left() {
                    row.child(stats_box().ml(slid)).child(area).child(twin)
                } else {
                    row.child(twin).child(area).child(stats_box().mr(slid))
                };
                out.push(row.into_any_element());
            } else {
                out.push(
                    stats_box()
                        .absolute()
                        .when(corner.is_left(), |d| d.left(px(side_inset)))
                        .when(!corner.is_left(), |d| d.right(px(side_inset)))
                        .bottom(px(vertical_inset))
                        .into_any_element(),
                );
            }
        }
        if let Some(bars) = bars {
            let edge = px(STATS_MARGIN);
            out.push(div().absolute().top(edge).left(edge).right(edge).flex().justify_center().child(bar_column(bars)).into_any_element());
        }
        // The edge handle. A click on it lands on the window's own mouse handler (which opens the
        // drawer), also when it's drawn in the click-through overlay window.
        // The notch: a tab attached to the edge (the window's, or the drawer's while it's open).
        // Closed, it slides out as the pointer nears the edge and opens the drawer; open, it
        // rides on the drawer's inner edge and closes it. Its clicks land on the main window.
        if let Some(h) = self.notch(window) {
            // Towards the inside to open; back towards the edge to close.
            let points_right = (side == DrawerSide::Left) != self.drawer_open;
            let (fill, line) = match side {
                DrawerSide::Right => ("xreveal/notch/fill-right.svg", "xreveal/notch/line-right.svg"),
                DrawerSide::Left => ("xreveal/notch/fill-left.svg", "xreveal/notch/line-left.svg"),
            };
            let chevron = div()
                .absolute()
                .top(px(NOTCH_FILLET))
                .bottom(px(NOTCH_FILLET))
                .w(px(NOTCH_BODY))
                .when(side == DrawerSide::Right, |d| d.left_0())
                .when(side == DrawerSide::Left, |d| d.right_0())
                .flex()
                .items_center()
                .justify_center()
                .text_color(pal.secondary)
                .child(
                    gpui_kit::component::Icon::new(if points_right { gpui_kit::assets::IconName::ChevronRight } else { gpui_kit::assets::IconName::ChevronLeft })
                        .size_4(),
                );
            let h = h
                // Exactly the drawer's panel, with the outline running on into the drawer's edge.
                .child(svg().path(fill).absolute().inset_0().size_full().text_color(pal.panel))
                .child(svg().path(line).absolute().inset_0().size_full().text_color(pal.line))
                .child(chevron);
            out.push(h.into_any_element());
        }
        out
    }

    /// Where the notch is, empty, with the hand cursor; `None` while it's out of sight. Where the
    /// overlays have their own click-through window (Windows), that window draws the notch, which
    /// can't set the cursor there, and the main window, which takes its clicks, places an empty copy.
    fn notch(&self, window: &mut Window) -> Option<Div> {
        if !self.handle.visible() && !self.slide.visible() {
            return None;
        }
        if self.handle.moving() || self.slide.moving() {
            window.request_animation_frame();
        }
        let drawer_edge = if self.slide.visible() { drawer::width() * self.slide.shown() } else { 0.0 };
        let inset = px(drawer_edge - NOTCH_BODY * (1.0 - self.handle.shown()) - 1.0);
        let h = div().absolute().top(relative(0.5)).mt(px(-NOTCH_HEIGHT / 2.0)).w(px(NOTCH_BODY + 1.0)).h(px(NOTCH_HEIGHT)).cursor_pointer();
        Some(match self.side {
            DrawerSide::Right => h.right(inset),
            DrawerSide::Left => h.left(inset),
        })
    }
}

impl XReveal {
    /// Slides the drawer window's panel out, then closes the window. The tab it shows is kept for the
    /// next one.
    fn close_drawer_window(&mut self, cx: &mut Context<Self>) {
        if let Some((handle, host)) = self.drawer_window.take() {
            let tab = host.read(cx).drawer.read(cx).tab();
            self.drawer.update(cx, |drawer, cx| drawer.show_tab(tab, cx));
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
                self.sound.mute_chosen(muted);
                self.store.update(|s| s.audio.muted = muted);
                self.sound.set_mix(&self.store.setup);
                // The echo guard's notice goes once the mute is the person's.
                self.refresh_drawer(window, cx);
            }
            DrawerEvent::Swipe(on) => self.store.update(|s| s.drawer_swipe = on),
            DrawerEvent::CheckNow => self.check_for_update(true, window, cx),
            DrawerEvent::DownloadUpdate => self.download_update(window, cx),
            DrawerEvent::RestartToUpdate => {
                if let crate::update::Updater::Ready(prepared) = &self.updater {
                    match crate::platform::install_update(prepared) {
                        Ok(()) => cx.quit(),
                        Err(why) => {
                            self.updater = crate::update::Updater::Failed(why);
                            self.refresh_drawer(window, cx);
                        }
                    }
                }
            }
            DrawerEvent::CheckUpdates(on) => {
                self.store.update(|s| if on { s.updates.check = true } else { s.updates.turn_off() });
                // Switching it on checks at once; off hides an update already found.
                if on {
                    self.check_for_update(true, window, cx);
                }
                self.refresh_drawer(window, cx);
            }
            DrawerEvent::Side(side) => {
                self.store.update(|s| s.drawer_side = side);
                if self.drawer_window.is_some() {
                    // A drawer in its own window slides out, then a new one slides in on the
                    // other edge.
                    self.close_drawer_window(cx);
                    self.shown_side = side;
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(drawer::SLIDE_OUT).await;
                        let _ = this.update_in(cx, |this, window, cx| {
                            if this.drawer_open && this.drawer_window.is_none() {
                                this.open_drawer_window(window, cx);
                            }
                        });
                    })
                    .detach();
                } else if self.drawer_open {
                    // Slide out on the old side, then in on the new one; the drawer's rail moves
                    // across while it's out of sight.
                    self.slide = self.slide.toward(false);
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(drawer::SLIDE_OUT).await;
                        let _ = this.update_in(cx, |this, window, cx| {
                            this.shown_side = side;
                            this.refresh_drawer(window, cx);
                            if this.drawer_open {
                                this.slide = this.slide.toward(true);
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                } else {
                    self.shown_side = side;
                    self.refresh_drawer(window, cx);
                }
            }
            DrawerEvent::Source(os_id, choice) => self.switch_source(os_id, choice, window, cx),
            DrawerEvent::WindowMode(mode) => self.set_window_mode(mode, window, cx),
            DrawerEvent::Monitor(uuid) => self.move_to_monitor(uuid, window, cx),
            DrawerEvent::NoInput => {
                // "None" in the Input list: show nothing, and remember that.
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                if let Some(video) = &mut self.video {
                    video.forget();
                }
                self.unplugged = false;
                self.store.update(|s| {
                    s.source = None;
                    s.video_format = None;
                });
                self.showing_changed(window, cx);
                self.watch_signal(window, cx);
            }
            DrawerEvent::DeviceSettings => {
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                if let Some(video) = &mut self.video {
                    video.open_device_settings(window);
                }
            }
            DrawerEvent::Stats(stats) => self.apply_stats(stats, cx),
            DrawerEvent::Look(look) => {
                let density_changed = look.density != self.store.setup.look.density;
                self.store.update(|s| s.look = look);
                theme::apply(&look, theme::appearance_is_dark(window.appearance()), cx);
                // Density sets every window's base text size; a drawer window also changes width.
                let rem = px(theme::spacing().rem);
                window.set_rem_size(rem);
                for handle in self.drawer_window.iter().map(|(h, _)| *h).chain(self.overlay_window.iter().map(|(h, _)| *h)) {
                    let _ = handle.update(cx, |_, w, _| w.set_rem_size(rem));
                }
                if density_changed && self.drawer_window.is_some() {
                    self.close_drawer_window(cx);
                    self.open_drawer_window(window, cx);
                }
            }
            DrawerEvent::Allow(privacy) => {
                permission::allow(privacy);
                self.watch_access(window, cx);
            }
        }
        cx.notify();
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
                false => {
                    let had_keys = self.drawer_window.as_ref().is_some_and(|(handle, _)| handle.update(cx, |_, window, _| window.is_window_active()).unwrap_or(false));
                    self.close_drawer_window(cx);
                    // The keys come back here at once: left with the drawer's window while it slides
                    // out, the next Tab would be lost, and Windows may give them to another app after.
                    if had_keys && let Some(main) = crate::platform::window_handle(window) {
                        cx.spawn(async move |_, _| crate::platform::activate(main)).detach();
                    }
                }
                _ => {}
            }
        }
        // The Input level meter reads the sound about 20 times a second, only while it can be seen.
        self.meter_feed = open.then(|| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(50)).await;
                    let fed = this.update(cx, |this, cx| {
                        let peaks = this.sound.take_peaks();
                        this.drawer.update(cx, |d, cx| d.set_peaks(peaks, cx));
                        if let Some((_, host)) = &this.drawer_window {
                            let drawer = host.read(cx).drawer.clone();
                            drawer.update(cx, |d, cx| d.set_peaks(peaks, cx));
                        }
                    });
                    if fed.is_err() {
                        break;
                    }
                }
            })
        });
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
        // The notch stays while the drawer is open (to close it), and tucks away after it closes.
        self.handle_slide = if open { drawer::Slide::opened() } else { drawer::Slide::opened().toward(false) };
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
pub fn stats_rows(summary: Option<Summary>, description: &str, to_picture: Option<(&'static str, Duration)>, shows: StatsBox) -> Vec<(&'static str, String)> {
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
        if let Some((label, took)) = to_picture {
            rows.push((label, format!("{} ms", took.as_millis())));
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
/// How far the performance box and the top bars keep from the window's edges.
const STATS_MARGIN: f32 = 12.0;

pub fn stats_inset(corner: crate::setup::Corner, drawer_side: DrawerSide, drawer_shown: f32, status_shown: bool) -> (f32, f32) {
    const MARGIN: f32 = STATS_MARGIN;
    let same_side = corner.is_left() == (drawer_side == DrawerSide::Left);
    let side = MARGIN + if same_side { drawer::width() * drawer_shown } else { 0.0 };
    let vertical = MARGIN + if status_shown && corner == crate::setup::Corner::BottomLeft { 40.0 } else { 0.0 };
    (side, vertical)
}

/// A screen's OS name and its size in logical pixels.
pub type NamedScreen = (String, (u32, u32));

/// Labels for the Monitor setting from each screen's name and size, primary first. Two screens of
/// the same model share a name, so those get their size too ("Mi monitor · 3440 × 1440"), and a
/// number if that's the same as well.
pub fn screen_labels(screens: &[NamedScreen]) -> Vec<String> {
    fn repeated(labels: &[String], label: &str) -> bool {
        labels.iter().filter(|l| l.eq_ignore_ascii_case(label)).count() > 1
    }
    let names: Vec<String> = screens.iter().map(|(name, _)| name.clone()).collect();
    let sized: Vec<String> =
        screens.iter().map(|(name, (w, h))| if repeated(&names, name) { format!("{name} · {w} × {h}") } else { name.clone() }).collect();
    sized.iter().enumerate().map(|(i, label)| if repeated(&sized, label) { format!("{label} ({})", i + 1) } else { label.clone() }).collect()
}

/// The hint over a picture from `card` that has stayed one flat colour (#108): a question, because
/// a dark game scene looks the same as a card sending black with no signal.
pub fn blank_hint(card: &str) -> String {
    format!("Only a blank picture from {card}? Check the console is on, its HDMI cable and any HDMI switch.")
}

/// The placement to remember for a window with outer frame `frame` and content area `content`:
/// the frame's position with the content's size. GPUI reopens a window with that size as its
/// content area, so saving the frame's size would grow the window by its title bar every launch.
pub fn saved_placement(frame: Bounds<Pixels>, content: Size<Pixels>) -> WindowPlacement {
    WindowPlacement { x: frame.origin.x.as_f32(), y: frame.origin.y.as_f32(), width: content.width.as_f32(), height: content.height.as_f32() }
}

/// Where the content area `content` of a window with outer frame `frame` starts on the screen:
/// below the title bar if there is one.
pub fn content_origin(frame: Bounds<Pixels>, content: Size<Pixels>) -> Point<Pixels> {
    point(frame.origin.x, frame.origin.y + (frame.size.height - content.height).max(px(0.)))
}

/// The drawer window's part of the content area `content`, from its top-left: the full height,
/// along the `side` edge.
pub fn drawer_area(content: Size<Pixels>, side: DrawerSide) -> Bounds<Pixels> {
    let width = px(drawer::width());
    let x = match side {
        DrawerSide::Right => content.width - width,
        DrawerSide::Left => px(0.),
    };
    Bounds { origin: point(x, px(0.)), size: size(width, content.height) }
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
        let side = self.shown_side;
        let width = window.bounds().size.width.as_f32();
        // The pointer left the window from near the edge (#178): no mouse move says so, only the
        // window's hover state (Windows redraws when it changes), so hide the handle here.
        if cfg!(target_os = "windows") && self.near_edge && !window.is_window_hovered() {
            cx.defer_in(window, |this, window, cx| {
                if this.near_edge && !window.is_window_hovered() {
                    this.near_edge = false;
                    if !this.drawer_open {
                        this.handle_slide = this.handle_slide.toward(false);
                    }
                    cx.notify();
                }
            });
        }

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
                .w(px(drawer::width()))
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
        // Screens without a picture sit on black, so they always use the dark palette.
        let screen_palette = theme::palette_for(&crate::setup::Look { theme: crate::setup::ThemeChoice::Dark, ..self.store.setup.look }, true);
        let backdrop = self.screen.backdrop();
        let actions = self.screen_actions(cx);
        let screen = self.screen.render(screen_palette, &actions);
        // Microphone access turned off, with a link (where the overlays have their own window, a bar
        // there says it instead: see `Overlays::microphone_off`).
        let microphone_off = !self.overlays_in_own_window && self.screen.shows_picture() && permission::access(Privacy::Microphone) == Access::Denied;
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
        let taken_over = self.taken_over.map(|format| {
            let card = self.status.split(" · ").next().unwrap_or("The input").to_owned();
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
                )
        });
        // The notices with buttons live in this window (they're macOS's: microphone access, another
        // app's format), and join the overlays' bars at the top centre.
        let notices: Vec<AnyElement> = microphone.into_iter().chain(taken_over).map(IntoElement::into_any_element).collect();
        let passive = if self.overlays_in_own_window {
            let edge = px(STATS_MARGIN);
            let column = (!notices.is_empty()).then(|| div().absolute().top(edge).left(edge).right(edge).flex().flex_col().items_center().gap_2().children(notices).into_any_element());
            // An empty copy of the notch, for its hand cursor (see `Overlays::notch`).
            let notch = self.overlays().notch(window).map(IntoElement::into_any_element);
            column.into_iter().chain(notch).collect()
        } else {
            self.overlays().render(window, notices)
        };
        // What's new (#144). Its buttons work here; where the overlays have their own click-through
        // window (Windows), that window draws the card and this one is an invisible twin at the
        // same place, which the clicks reach.
        let whats_new = self.whats_new.as_ref().map(|scroll| {
            let (this, that) = (cx.entity().downgrade(), cx.entity().downgrade());
            let actions = crate::whats_new::Actions {
                got_it: Box::new(move |_, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.whats_new = None;
                        this.store.update(|s| s.whats_new_seen = Some(crate::whats_new::VERSION.into()));
                        cx.notify();
                    });
                }),
                github: Box::new(|_, cx| cx.open_url(crate::whats_new::RELEASE_PAGE)),
                // Redraw the fades (and the overlay window's copy, which follows this view).
                scrolled: Box::new(move |_, cx| {
                    let _ = that.update(cx, |_, cx| cx.notify());
                }),
            };
            let dialog = crate::whats_new::dialog(pal, &crate::whats_new::notes(), scroll, Some(actions));
            if self.overlays_in_own_window { div().absolute().inset_0().opacity(0.).child(dialog).into_any_element() } else { dialog }
        });

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
                    WindowMode::Fullscreen => this.before_fullscreen,
                    _ => WindowMode::Fullscreen,
                };
                this.set_window_mode(mode, window, cx);
            }))
            // Right-click on the picture toggles the drawer; a click outside it closes it.
            .on_mouse_down(MouseButton::Right, cx.listener(|this, _, window, cx| this.set_drawer_open(!this.drawer_open, window, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                if this.drawer_open {
                    this.set_drawer_open(false, window, cx);
                } else if this.near_edge {
                    // The edge handle (which lets clicks through to here).
                    this.set_drawer_open(true, window, cx);
                }
            }))
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                let near = near_drawer_edge(event.position.x.as_f32(), width, this.shown_side);
                if near != this.near_edge {
                    this.near_edge = near;
                    if !this.drawer_open {
                        this.handle_slide = this.handle_slide.toward(near);
                    }
                    cx.notify();
                }
            }))
            .flex()
            .items_end()
            .p_3()
            .children(screen)
            .children(drawer)
            // After the drawer, so the notch can cover the drawer's border where they meet.
            .children(passive)
            .child(swipe_listener(cx.entity().downgrade()))
            .children(whats_new)
    }
}


/// The overlays that only show something, in a click-through window of their own over the main
/// one (see [`XReveal::open_overlay_window`]): on Windows the video surface covers anything drawn
/// in the main window itself. Redrawn whenever the main view changes.
pub struct OverlayWindow {
    overlays: Overlays,
    _observe: Subscription,
}

impl OverlayWindow {
    fn new(main: &Entity<XReveal>, overlays: Overlays, cx: &mut Context<Self>) -> Self {
        // Read after each of the main view's updates has finished, never while it's running.
        let observe = cx.observe(main, |this, main, cx| {
            this.overlays = main.read(cx).overlays();
            cx.notify();
        });
        Self { overlays, _observe: observe }
    }
}

impl Render for OverlayWindow {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().relative().flex().items_end().p_3().children(self.overlays.render(window, Vec::new()))
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
    fn new(model: DrawerModel, tab: Tab, side: DrawerSide, main: WeakEntity<XReveal>, main_window: AnyWindowHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let drawer = cx.new(|cx| {
            let mut drawer = SettingsDrawer::new(model, window, cx);
            drawer.show_tab(tab, cx);
            drawer
        });
        let focus = cx.focus_handle();
        // The drawer itself takes the keys, so its arrow keys work; this window's keys still reach
        // the shell around it.
        drawer.read(cx).focus_handle().focus(window, cx);
        let forward = cx.subscribe(&drawer, |this: &mut Self, _, event: &DrawerEvent, cx| {
            let event = event.clone();
            this.with_main(cx, move |main, window, cx| main.apply(event, window, cx));
        });
        let slide = drawer::Slide::closed().toward(true);
        Self { drawer, side, slide, focus, main, main_window, _subscriptions: vec![forward] }
    }

    /// Runs `f` on the main window's view, in the main window. Deferred until this window's own
    /// update is over: `f` may reach back into this window (closing the drawer, refreshing it), and
    /// GPUI can't update an entity while it is already being updated.
    fn with_main(&self, cx: &mut App, f: impl FnOnce(&mut XReveal, &mut Window, &mut Context<XReveal>) + 'static) {
        let (main, main_window) = (self.main.clone(), self.main_window);
        cx.defer(move |cx| {
            let _ = main_window.update(cx, move |_, window, cx| {
                if let Some(main) = main.upgrade() {
                    main.update(cx, |main, cx| f(main, window, cx));
                }
            });
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
        // The inner edge's line, open where the notch joins it: the notch is drawn in the overlay
        // window, under this one, and its outline runs on into the line from either end.
        let edge = div()
            .absolute()
            .top_0()
            .bottom_0()
            .w(px(1.))
            .flex()
            .flex_col()
            .child(div().flex_1().bg(pal.line))
            .child(div().h(px(NOTCH_HEIGHT)))
            .child(div().flex_1().bg(pal.line));
        let edge = match self.side {
            DrawerSide::Right => edge.left_0(),
            DrawerSide::Left => edge.right_0(),
        };
        let shell = div().absolute().top_0().bottom_0().w(px(drawer::width())).child(self.drawer.clone()).child(edge);
        let shell = match self.side {
            DrawerSide::Right => shell.right(offset).pl(px(1.)),
            DrawerSide::Left => shell.left(offset).pr(px(1.)),
        };
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .child(drawer_swipe_listener(cx.entity().downgrade()))
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
    use super::{DrawerSide, Duration, FullscreenSwitch, Summary, content_origin, drawer_area, near_drawer_edge, saved_placement, screen_labels, stats_inset, stats_rows};
    use crate::setup::{Corner, StatsBox};

    #[test]
    fn a_window_mode_asked_for_during_the_fullscreen_slide_waits_for_its_end() {
        let mut switch = FullscreenSwitch::default();
        assert!(!switch.wait(), "no slide: applied at once");
        switch.start();
        assert!(switch.wait(), "during the slide: waits");
        assert!(switch.wait(), "a second change waits too");
        assert!(switch.finish(), "the slide ends: apply the mode asked for last");
        assert!(!switch.finish(), "applied once");
        assert!(!switch.wait());
    }

    #[test]
    fn a_slide_that_never_ends_times_out_only_if_nothing_happened_since() {
        let mut switch = FullscreenSwitch::default();
        let asked = switch.start();
        let started = switch.start();
        assert!(!switch.timed_out(asked), "macOS started the slide since: that one counts");
        assert!(switch.timed_out(started));
        switch.finish();
        assert!(!switch.timed_out(started), "it ended");
    }

    #[test]
    fn screens_of_the_same_model_are_told_apart() {
        let screen = |name: &str, w, h| (name.to_string(), (w, h));
        let different = [screen("DELL U2720Q", 3840, 2160), screen("Mi monitor", 3440, 1440)];
        assert_eq!(screen_labels(&different), ["DELL U2720Q", "Mi monitor"], "names alone when they differ");
        // Seen on the Windows test PC: two Xiaomi screens, named with different capitals.
        let same_model = [screen("Mi monitor", 3440, 1440), screen("Mi Monitor", 2560, 1440)];
        assert_eq!(screen_labels(&same_model), ["Mi monitor · 3440 × 1440", "Mi Monitor · 2560 × 1440"]);
        let twins = [screen("LG", 2560, 1440), screen("LG", 2560, 1440), screen("Mi monitor", 3440, 1440)];
        assert_eq!(screen_labels(&twins), ["LG · 2560 × 1440 (1)", "LG · 2560 × 1440 (2)", "Mi monitor"]);
    }

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
        let rows = stats_rows(Some(summary), "USB2 Video · 1280x720 @ 60 · 420v", crate::video::time_to_picture(Some(Duration::from_millis(480)), None), all);
        assert_eq!(value(&rows, "capture → screen"), "≈12.3 ms · p99 18.1");
        assert_eq!(value(&rows, "xReveal's share"), "≈4.2 ms · p99 6.0");
        assert_eq!(value(&rows, "launch → picture"), "480 ms");
        assert_eq!(value(&rows, "fps"), "59.9");
        assert_eq!(value(&rows, "dropped · late"), "3 · 1");

        let just_fps = StatsBox { latency: false, frames: false, format: false, ..all };
        let rows = stats_rows(Some(summary), "", crate::video::time_to_picture(Some(Duration::from_millis(480)), None), just_fps);
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
    fn the_window_keeps_its_size_across_launches() {
        use gpui_kit::{Bounds, point, px, size};
        // A 1280 × 917 content area under a 32 px title bar.
        let frame = Bounds { origin: point(px(164.), px(33.)), size: size(px(1280.), px(949.)) };
        let placement = saved_placement(frame, size(px(1280.), px(917.)));
        assert_eq!((placement.x, placement.y, placement.width, placement.height), (164.0, 33.0, 1280.0, 917.0), "the content's size, not the frame's");
    }

    #[test]
    fn edge_handle_follows_the_drawer_side() {
        assert!(near_drawer_edge(1900.0, 1920.0, DrawerSide::Right));
        assert!(!near_drawer_edge(20.0, 1920.0, DrawerSide::Right));
        assert!(near_drawer_edge(20.0, 1920.0, DrawerSide::Left));
        assert!(!near_drawer_edge(960.0, 1920.0, DrawerSide::Left));
    }

    #[test]
    fn pop_ups_cover_the_content_below_the_title_bar() {
        use gpui_kit::{Bounds, point, px, size};
        // A 1280 × 917 content area under a 32 px title bar.
        let frame = Bounds { origin: point(px(164.), px(33.)), size: size(px(1280.), px(949.)) };
        let content = size(px(1280.), px(917.));
        assert_eq!(content_origin(frame, content), point(px(164.), px(65.)));
        // No title bar (Windows reports the content area as the window's bounds).
        assert_eq!(content_origin(Bounds { origin: frame.origin, size: content }, content), point(px(164.), px(33.)));
        let right = drawer_area(content, DrawerSide::Right);
        let left = drawer_area(content, DrawerSide::Left);
        assert_eq!((right.right(), right.top(), right.bottom()), (px(1280.), px(0.), px(917.)), "along the right edge, full height");
        assert_eq!((left.left(), left.size), (px(0.), right.size));
    }
}
