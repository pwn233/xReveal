use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode, TitleBar, h_flex, v_flex};
use gpui_kit::*;

/// Root view of the application.
pub struct XReveal {
    clicks: usize,
}

impl XReveal {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self { clicks: 0 }
    }

    fn increment(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.clicks += 1;
        cx.notify();
    }

    fn reset(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.clicks = 0;
        cx.notify();
    }

    fn toggle_theme(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if cx.theme().is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        Theme::change(mode, Some(window), cx);
        cx.notify();
    }
}

impl Render for XReveal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let theme_label = if theme.is_dark() {
            "Light mode"
        } else {
            "Dark mode"
        };

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .pr_2()
                        .justify_between()
                        .child("xReveal")
                        .child(
                            Button::new("toggle-theme")
                                .ghost()
                                .label(theme_label)
                                .on_click(cx.listener(Self::toggle_theme)),
                        ),
                ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .gap_4()
                    .items_center()
                    .justify_center()
                    .child(div().text_2xl().child("Hello from xReveal"))
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(format!("Button clicked {} times", self.clicks)),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("increment")
                                    .primary()
                                    .label("Click me")
                                    .on_click(cx.listener(Self::increment)),
                            )
                            .child(
                                Button::new("reset")
                                    .outline()
                                    .label("Reset")
                                    .on_click(cx.listener(Self::reset)),
                            ),
                    ),
            )
    }
}
