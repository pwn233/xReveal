mod app;
mod platform;
mod video;

use gpui_kit::*;

use crate::app::XReveal;

fn main() {
    let _ = crate::video::LAUNCH.set(std::time::Instant::now());
    if std::env::var_os("XREVEAL_SMOKE_TEST").is_some() {
        watch_smoke_test();
    }

    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(None, size(px(1280.), px(720.)), cx)),
            titlebar: None,
            // The video surface sits under GPUI's view, so GPUI must not paint over it (ADR 0001).
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };

        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            window.set_window_title("xReveal");
            cx.new(|cx| XReveal::new(window, cx))
        })
        .expect("failed to open the main window");

        window
            .update(cx, |_, window, cx| {
                // The component theme paints the root with its background colour; the picture
                // must show through instead (ADR 0001). Root's own style is applied last.
                gpui_kit::base::Root::update(window, cx, |root, _, _| {
                    root.style().background = Some(transparent_black().into());
                });
                // Borderless fullscreen is the default presentation (docs/spec/v1.md §2.2). Wait for
                // the first frame: the window has to be on screen first. "Simple" fullscreen = a borderless
                // window covering the screen, not a separate macOS Space.
                window.on_next_frame(|window, _| window.toggle_simple_fullscreen());
            })
            .expect("the main window was just opened");

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
