mod app;

use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::app::XReveal;

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(None, size(px(960.), px(640.)), cx)),
            ..TitleBar::window_options()
        };

        gpui_kit::open_window(options, cx, |window, cx| {
            window.set_window_title("xReveal");
            cx.new(|cx| XReveal::new(window, cx))
        })
        .expect("failed to open the main window");

        cx.activate(true);
    });
}
