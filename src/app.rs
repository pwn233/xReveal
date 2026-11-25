//! Root view: the live picture shows through it (it paints no background), with a one-line status
//! until the Settings drawer and the screens without a picture exist, and the Stats overlay (F3).

use std::time::Duration;

use gpui_kit::*;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use crate::platform::VideoPipeline;
use crate::video::timing::Summary;

pub struct XReveal {
    focus: FocusHandle,
    status: String,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    video: Option<VideoPipeline>,
    /// Redraws the Stats overlay while it is shown; dropping it stops the refresh.
    stats_refresh: Option<Task<()>>,
}

impl XReveal {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let (video, status) = match VideoPipeline::start(window) {
                Ok(video) => {
                    let status = video.description().to_owned();
                    (Some(video), status)
                }
                Err(error) => (None, error),
            };
            Self { focus, status, video, stats_refresh: None }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Self { focus, status: "The live picture isn't implemented on this OS yet.".into(), stats_refresh: None }
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            // Windowed / fullscreen (docs/spec/v1.md §2.2; assumption to confirm, §8).
            "f11" => window.toggle_simple_fullscreen(),
            // Stats overlay: off by default, not remembered (docs/spec/v1.md §3.3).
            "f3" => self.toggle_stats(cx),
            _ => {}
        }
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

    /// Switches to another Source or Video format: the picture restarts briefly, with a notice
    /// until the new format is running (docs/spec/v1.md §2.3).
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[expect(dead_code, reason = "called from the Settings drawer's Source tab (its own slice)")]
    pub fn switch_source(&mut self, os_id: String, choice: crate::source::VideoFormat, cx: &mut Context<Self>) {
        self.status = "Restarting picture…".into();
        cx.notify();
        // Let the notice paint before opening the device (which blocks for a moment).
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| {
                if let Some(video) = &mut this.video {
                    this.status = match video.switch_to(&os_id, choice) {
                        Ok(()) => video.description().to_owned(),
                        Err(error) => error,
                    };
                }
                cx.notify();
            });
        })
        .detach();
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

impl Render for XReveal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = |child: Div| child.px_2().py_1().rounded_md().bg(rgba(0x0000_00aa)).text_color(rgb(0xffffff)).text_sm();
        let overlay = self.stats_refresh.is_some().then(|| {
            let lines = stats_lines(self.stats(), &self.status, crate::video::FIRST_PRESENT.get().copied());
            panel(div().absolute().top_3().left_3().font_family("Menlo").flex().flex_col())
                .children(lines.into_iter().map(|line| div().child(line)))
        });

        div()
            .size_full()
            .relative()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .items_end()
            .p_3()
            .child(panel(div()).child(self.status.clone()))
            .children(overlay)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude has its own `test` attribute that would replace std's.
    use super::{Duration, Summary, stats_lines};

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
}
