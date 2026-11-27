// Release builds on Windows are GUI apps: no console window next to xReveal. (`--report` attaches
// to the terminal that started it.)
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod audio;
mod drawer;
mod permission;
mod platform;
mod report;
mod screens;
mod setup;
mod source;
mod theme;
mod video;
mod whats_new;

use gpui_kit::*;

use crate::app::XReveal;

fn main() {
    if std::env::args().any(|a| a == "--report") {
        report::run();
    }
    let _ = crate::video::LAUNCH.set(std::time::Instant::now());
    if std::env::var_os("XREVEAL_SMOKE_TEST").is_some() {
        watch_smoke_test();
    }

    gpui_kit::application().with_assets(theme::AppAssets).run(|cx| {
        gpui_kit::init(cx);
        // The remembered look is applied once the window exists (it may follow the OS appearance).
        theme::load_fonts(cx);
        theme::apply(&setup::Look::default(), true, cx);
        app::bind_keys(cx);

        let store = setup::Store::open();
        // The monitor to open on: the remembered one, recognised by the display's own identity,
        // else the primary (docs/spec/v1.md §2.1). On the remembered one, the window goes back
        // where it was, fitted onto the screen in case it hangs off it or the screen has shrunk.
        let (display_id, remembered, area) = launch_display(store.setup.monitor.as_deref(), cx);
        let window_bounds = match store.setup.window.filter(|p| remembered && p.usable()).map(|p| p.fit_into(area)) {
            Some(p) => WindowBounds::Windowed(Bounds { origin: point(px(p.x), px(p.y)), size: size(px(p.width), px(p.height)) }),
            None => WindowBounds::centered(display_id, size(px(1280.), px(720.)), cx),
        };
        let mode = store.setup.window_mode;
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
                // Fullscreen or Borderless, if that's the remembered Window mode. Wait for the first
                // frame: the window has to be on screen first.
                match mode {
                    setup::WindowMode::Fullscreen => window.on_next_frame(|window, _| app::toggle_borderless(window)),
                    setup::WindowMode::Borderless => window.on_next_frame(|window, cx| app::set_borderless(window, true, None, std::time::Duration::ZERO, cx)),
                    setup::WindowMode::Windowed => {}
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
/// display. Also says whether it is the remembered one, and gives its visible area (x, y, width,
/// height, without the taskbar).
fn launch_display(remembered: Option<&str>, cx: &App) -> (Option<DisplayId>, bool, (f32, f32, f32, f32)) {
    let mut displays = cx.displays();
    let primary = cx.primary_display().map(|d| d.id());
    // Primary first, so the fallback (index 0) is the primary display.
    displays.sort_by_key(|d| Some(d.id()) != primary);
    let identities: Vec<String> = displays.iter().map(|d| d.uuid().map(|u| u.to_string()).unwrap_or_default()).collect();
    let index = source::choose_monitor(remembered, &identities);
    let found = remembered.is_some_and(|r| identities.get(index).is_some_and(|i| i == r));
    let display = displays.get(index);
    let area = display.map(|d| d.visible_bounds()).map_or((0.0, 0.0, f32::MAX, f32::MAX), |b| {
        (b.origin.x.as_f32(), b.origin.y.as_f32(), b.size.width.as_f32(), b.size.height.as_f32())
    });
    (display.map(|d| d.id()), found, area)
}
