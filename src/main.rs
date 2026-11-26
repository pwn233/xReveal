mod app;
mod drawer;
mod platform;
mod setup;
mod source;
mod theme;
mod video;

use gpui_kit::*;

use crate::app::XReveal;

fn main() {
    let _ = crate::video::LAUNCH.set(std::time::Instant::now());
    if std::env::var_os("XREVEAL_SMOKE_TEST").is_some() {
        watch_smoke_test();
    }

    gpui_kit::application().with_assets(theme::AppAssets).run(|cx| {
        gpui_kit::init(cx);
        // The remembered look is applied once the window exists (it may follow the OS appearance).
        theme::apply(&setup::Look::default(), true, cx);
        app::bind_keys(cx);

        let store = setup::Store::open();
        // The monitor to open on: the remembered one, recognised by the display's own identity,
        // else the primary (docs/spec/v1.md §2.1). On the remembered one, the window goes back
        // where it was.
        let (display_id, remembered) = launch_display(store.setup.monitor.as_deref(), cx);
        let window_bounds = match store.setup.window.filter(|p| remembered && p.usable()) {
            Some(p) => WindowBounds::Windowed(Bounds { origin: point(px(p.x), px(p.y)), size: size(px(p.width), px(p.height)) }),
            None => WindowBounds::centered(display_id, size(px(1280.), px(720.)), cx),
        };
        let fullscreen = store.setup.window_mode == setup::WindowMode::Fullscreen;
        let options = WindowOptions {
            window_bounds: Some(window_bounds),
            display_id,
            // A normal title bar: close, minimise, maximise, and dragging to move (docs/spec/v1.md §2.2).
            titlebar: Some(TitlebarOptions { title: Some("xReveal".into()), appears_transparent: false, traffic_light_position: None }),
            window_min_size: Some(size(px(setup::WindowPlacement::MIN.0), px(setup::WindowPlacement::MIN.1))),
            // The video surface sits under GPUI's view, so GPUI must not paint over it (ADR 0001).
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };

        let (window, _) = gpui_kit::open_window(options, cx, move |window, cx| {
            window.set_window_title("xReveal");
            cx.new(|cx| XReveal::new(window, cx, store))
        })
        .expect("failed to open the main window");

        window
            .update(cx, |_, window, cx| {
                // The component theme paints the root with its background colour; the picture
                // must show through instead (ADR 0001). Root's own style is applied last.
                gpui_kit::base::Root::update(window, cx, |root, _, _| {
                    root.style().background = Some(transparent_black().into());
                });
                // Fullscreen, if that's the remembered Window mode. Wait for the first frame: the
                // window has to be on screen first.
                if fullscreen {
                    window.on_next_frame(|window, _| app::toggle_borderless(window));
                }
            })
            .expect("the main window was just opened");

        // Closing the window (its close button, Alt+F4) quits, saving the setup on the way out.
        let main = window.window_id();
        cx.on_window_closed(move |cx, id| {
            if id == main {
                cx.quit();
            }
        })
        .detach();

        cx.activate(true);
    });
}

/// CI smoke launch (docs/spec/v1.md §5): exit 0 once the video surface has presented a frame, or
/// 1 if it hasn't within 20 s. Works with no capture card attached.
fn watch_smoke_test() {
    use std::time::{Duration, Instant};

    std::thread::spawn(|| {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(20) {
            if let Some(after_launch) = crate::video::FIRST_PRESENT.get() {
                println!("smoke test: video surface presented {after_launch:?} after launch");
                std::process::exit(0);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        eprintln!("smoke test: the video surface never presented a frame");
        std::process::exit(1);
    });
}

/// The display xReveal opens on: `remembered` (a display UUID) if it's connected, else the primary
/// display. Also says whether it is the remembered one.
fn launch_display(remembered: Option<&str>, cx: &App) -> (Option<DisplayId>, bool) {
    let mut displays = cx.displays();
    let primary = cx.primary_display().map(|d| d.id());
    // Primary first, so the fallback (index 0) is the primary display.
    displays.sort_by_key(|d| Some(d.id()) != primary);
    let identities: Vec<String> = displays.iter().map(|d| d.uuid().map(|u| u.to_string()).unwrap_or_default()).collect();
    let index = source::choose_monitor(remembered, &identities);
    let found = remembered.is_some_and(|r| identities.get(index).is_some_and(|i| i == r));
    (displays.get(index).map(|d| d.id()), found)
}
