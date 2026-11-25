//! Root view: the live picture shows through it (it paints no background). On top: the Settings
//! drawer, a one-line status until the screens without a picture exist, the one-time colour tip,
//! and the Stats overlay (F3).

use std::time::Duration;

use gpui_kit::*;

use crate::drawer::{self, DrawerEvent, DrawerModel, SettingsDrawer};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use crate::platform::VideoPipeline;
use crate::setup::{DrawerSide, Store};
use crate::video::timing::Summary;

actions!(xreveal, [ToggleDrawer, CloseDrawer, ToggleStats, ToggleFullscreen]);

/// The key context of xReveal's own bindings; deeper than GPUI Kit's root, so Tab is ours.
const KEY_CONTEXT: &str = "XReveal";

/// Keys (docs/spec/v1.md §2.2). Call once at startup.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", ToggleDrawer, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", CloseDrawer, Some(KEY_CONTEXT)),
        KeyBinding::new("f3", ToggleStats, Some(KEY_CONTEXT)),
        // Windowed / fullscreen: an assumption to confirm (docs/spec/v1.md §8).
        KeyBinding::new("f11", ToggleFullscreen, Some(KEY_CONTEXT)),
    ]);
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
    /// The pointer is near the drawer's edge: show the handle.
    near_edge: bool,
    /// The one-time "Blacks look grey?" tip, while it is up.
    tip: Option<Task<()>>,
    /// Redraws the Stats overlay while it is shown; dropping it stops the refresh.
    stats_refresh: Option<Task<()>>,
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

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let (video, mut status) = {
            let remembered = store.setup.source.clone();
            match VideoPipeline::start(window, remembered.as_ref().map(|s| (s, store.setup.video_format))) {
                Ok(video) => {
                    let status = video.description().to_owned();
                    (Some(video), status)
                }
                Err(error) => (None, error),
            }
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut status = String::from("The live picture isn't implemented on this OS yet.");
        if let Some(notice) = store.notice.take() {
            status = format!("{notice} {status}");
        }

        let drawer = cx.new(|cx| SettingsDrawer::new(DrawerModel::default(), cx));
        let on_drawer = cx.subscribe_in(&drawer, window, |this, _, event: &DrawerEvent, window, cx| this.apply(event.clone(), window, cx));
        let mut this = Self {
            focus,
            status,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            video,
            drawer,
            drawer_open: false,
            near_edge: false,
            tip: None,
            stats_refresh: None,
            store,
            _subscriptions: vec![save_on_quit, on_drawer],
        };
        this.showing_changed(window, cx);
        this
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
        self.refresh_drawer(window, cx);
    }

    fn drawer_model(&self) -> DrawerModel {
        let setup = &self.store.setup;
        let mut model = DrawerModel { picture: setup.picture, delay: setup.delay, side: setup.drawer_side, frame_interval: 1.0 / 60.0, ..Default::default() };
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

    fn refresh_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.drawer_model();
        self.drawer.update(cx, |drawer, cx| drawer.set_model(model, window, cx));
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
            }
            DrawerEvent::Side(side) => self.store.update(|s| s.drawer_side = side),
            DrawerEvent::Source(os_id, choice) => self.switch_source(os_id, choice, window, cx),
            DrawerEvent::ResetAll => self.reset_all_settings(window, cx),
        }
        cx.notify();
    }

    /// Reset all settings (docs/spec/v1.md §2.3): forget everything and start as on first run.
    fn reset_all_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.store.reset_all() {
            self.status = format!("Couldn't reset settings: {error}");
        }
        // Until the Welcome screen exists, keep showing the current Source with default settings.
        self.showing_changed(window, cx);
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
            self.refresh_drawer(window, cx);
        }
        self.drawer_open = open;
        self.near_edge = false;
        cx.notify();
    }

    fn toggle_stats(&mut self, cx: &mut Context<Self>) {
        self.stats_refresh = match self.stats_refresh.take() {
            Some(_) => None,
            None => Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(250)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            })),
        };
        cx.notify();
    }

    fn stats(&self) -> Option<Summary> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        return self.video.as_ref().map(VideoPipeline::stats);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        None
    }
}

/// The Stats overlay's lines.
pub fn stats_lines(summary: Option<Summary>, description: &str, launch_to_picture: Option<Duration>) -> Vec<String> {
    let mut lines = Vec::new();
    let summary = summary.unwrap_or_default();
    let approx = if summary.present_time_estimated { "≈" } else { "" };
    let ms = |pair: Option<(f64, f64)>| match pair {
        Some((p50, p99)) => format!("{approx}{p50:.1} ms (p99 {p99:.1})"),
        None => "–".into(),
    };
    lines.push(format!("capture → screen  {}", ms(summary.capture_to_screen)));
    lines.push(format!("xReveal's share   {}", ms(summary.arrival_to_screen)));
    let fps = summary.fps.map_or("–".into(), |fps| format!("{fps:.1}"));
    lines.push(format!("{fps} fps · dropped {} · late {}", summary.dropped, summary.late));
    lines.push(description.to_owned());
    if let Some(launch) = launch_to_picture {
        lines.push(format!("launch → picture  {} ms", launch.as_millis()));
    }
    lines
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
        let panel = |child: Div| child.px_2().py_1().rounded_md().bg(rgba(0x0000_00aa)).text_color(rgb(0xffffff)).text_sm();
        let side = self.store.setup.drawer_side;
        let width = window.bounds().size.width.as_f32();

        let overlay = self.stats_refresh.is_some().then(|| {
            let lines = stats_lines(self.stats(), &self.status, crate::video::FIRST_PRESENT.get().copied());
            panel(div().absolute().top_3().left_3().font_family("Menlo").flex().flex_col())
                .children(lines.into_iter().map(|line| div().child(line)))
        });
        let tip = self.tip.is_some().then(|| {
            div().absolute().top_3().w_full().flex().justify_center().child(panel(div()).child(
                "Blacks look grey? Open settings (Tab) › Colour and set Range to Limited, or set your console's RGB range to Full.",
            ))
        });
        let drawer = self.drawer_open.then(|| {
            let shell = div()
                .id("drawer")
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(drawer::WIDTH))
                .border_color(rgba(drawer::LINE))
                .occlude()
                // Clicks inside the drawer must not reach the picture (which closes it).
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .child(self.drawer.clone());
            match side {
                DrawerSide::Right => shell.right_0().border_l_1(),
                DrawerSide::Left => shell.left_0().border_r_1(),
            }
        });
        let handle = (!self.drawer_open && self.near_edge).then(|| {
            let h = div()
                .id("drawer-handle")
                .absolute()
                .top(relative(0.5))
                .mt(px(-36.))
                .w(px(22.))
                .h(px(72.))
                .rounded_lg()
                .bg(rgba(drawer::GLASS))
                .border_1()
                .border_color(rgba(drawer::LINE))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.set_drawer_open(true, window, cx);
                }));
            match side {
                DrawerSide::Right => h.right_1(),
                DrawerSide::Left => h.left_1(),
            }
        });

        div()
            .size_full()
            .relative()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ToggleDrawer, window, cx| this.set_drawer_open(!this.drawer_open, window, cx)))
            .on_action(cx.listener(|this, _: &CloseDrawer, window, cx| this.set_drawer_open(false, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleStats, _, cx| this.toggle_stats(cx)))
            .on_action(cx.listener(|_, _: &ToggleFullscreen, window, _| window.toggle_simple_fullscreen()))
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
            .child(panel(div()).child(self.status.clone()))
            .children(tip)
            .children(overlay)
            .children(handle)
            .children(drawer)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude has its own `test` attribute that would replace std's.
    use super::{DrawerSide, Duration, Summary, near_drawer_edge, stats_lines};

    #[test]
    fn stats_lines_show_timings_or_dashes() {
        let empty = stats_lines(None, "No capture card found.", None);
        assert_eq!(empty[0], "capture → screen  –");
        assert_eq!(empty[2], "– fps · dropped 0 · late 0");
        assert_eq!(empty.len(), 4);

        let summary = Summary {
            capture_to_screen: Some((12.34, 18.1)),
            arrival_to_screen: Some((4.2, 6.0)),
            fps: Some(59.94),
            dropped: 3,
            late: 1,
            present_time_estimated: true,
        };
        let lines = stats_lines(Some(summary), "USB2 Video · 1280x720 @ 60 · 420v", Some(Duration::from_millis(480)));
        assert_eq!(lines[0], "capture → screen  ≈12.3 ms (p99 18.1)");
        assert_eq!(lines[1], "xReveal's share   ≈4.2 ms (p99 6.0)");
        assert_eq!(lines[2], "59.9 fps · dropped 3 · late 1");
        assert_eq!(lines[4], "launch → picture  480 ms");
    }

    #[test]
    fn edge_handle_follows_the_drawer_side() {
        assert!(near_drawer_edge(1900.0, 1920.0, DrawerSide::Right));
        assert!(!near_drawer_edge(20.0, 1920.0, DrawerSide::Right));
        assert!(near_drawer_edge(20.0, 1920.0, DrawerSide::Left));
        assert!(!near_drawer_edge(960.0, 1920.0, DrawerSide::Left));
    }
}
