//! Root view: the live picture shows through it (it paints no background), with a one-line status
//! until the Settings drawer and the screens without a picture exist.

use gpui_kit::*;

#[cfg(target_os = "macos")]
use crate::platform::macos::VideoPipeline;

pub struct XReveal {
    focus: FocusHandle,
    status: String,
    #[cfg(target_os = "macos")]
    _video: Option<VideoPipeline>,
}

impl XReveal {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);

        #[cfg(target_os = "macos")]
        {
            let (video, status) = match VideoPipeline::start(window) {
                Ok(video) => {
                    let status = video.description().to_owned();
                    (Some(video), status)
                }
                Err(error) => (None, error),
            };
            Self { focus, status, _video: video }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self { focus, status: "The live picture isn't implemented on this OS yet.".into() }
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, _cx: &mut Context<Self>) {
        // F11: windowed / fullscreen (docs/spec/v1.md §2.2; assumption to confirm, §8).
        if event.keystroke.key == "f11" {
            window.toggle_simple_fullscreen();
        }
    }
}

impl Render for XReveal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .items_end()
            .p_3()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(rgba(0x0000_0099))
                    .text_color(rgb(0xffffff))
                    .text_sm()
                    .child(self.status.clone()),
            )
    }
}
