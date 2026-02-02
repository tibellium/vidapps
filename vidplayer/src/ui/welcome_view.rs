use std::path::PathBuf;

use gpui::{Context, IntoElement, Render, Window, div, prelude::*, px, rgb};

pub struct VideoSelected {
    pub path: PathBuf,
}

pub struct WelcomeView {
    is_selecting: bool,
}

impl gpui::EventEmitter<VideoSelected> for WelcomeView {}

impl WelcomeView {
    pub fn new() -> Self {
        Self {
            is_selecting: false,
        }
    }

    fn handle_select_click(
        &mut self,
        _event: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_selecting {
            return;
        }

        self.is_selecting = true;
        cx.notify();

        let future = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select a video file".into()),
        });

        cx.spawn(async move |this, cx| match future.await {
            Ok(Ok(Some(paths))) if !paths.is_empty() => {
                cx.update(|cx| {
                    if let Some(window) = cx.active_window() {
                        window
                            .update(cx, |_, window, _cx| {
                                window.activate_window();
                            })
                            .ok();
                    }

                    this.update(cx, |_this, cx| {
                        cx.emit(VideoSelected {
                            path: paths[0].clone(),
                        });
                    })
                    .ok();
                })
                .ok();
            }
            _ => {
                cx.update(|cx| {
                    if let Some(window) = cx.active_window() {
                        window
                            .update(cx, |_, window, _cx| {
                                window.activate_window();
                            })
                            .ok();
                    }

                    this.update(cx, |this, cx| {
                        this.is_selecting = false;
                        cx.notify();
                    })
                    .ok();
                })
                .ok();
            }
        })
        .detach();
    }
}

impl Default for WelcomeView {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for WelcomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let button_label = if self.is_selecting {
            "Selecting..."
        } else {
            "Select Video"
        };

        div()
            .id("welcome")
            .size_full()
            .bg(rgb(0x111111))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(24.0))
            .child(
                div()
                    .text_size(px(36.0))
                    .text_color(rgb(0xffffff))
                    .child("Video Player"),
            )
            .child(
                div()
                    .text_size(px(16.0))
                    .text_color(rgb(0x888888))
                    .child("Select a video file to play"),
            )
            .child(
                div()
                    .id("select-button")
                    .mt(px(8.0))
                    .px(px(28.0))
                    .py(px(14.0))
                    .bg(rgb(0x3b82f6))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .when(!self.is_selecting, |el| el.hover(|el| el.bg(rgb(0x2563eb))))
                    .when(self.is_selecting, |el| {
                        el.bg(rgb(0x6b7280)).cursor_default()
                    })
                    .child(
                        div()
                            .text_size(px(16.0))
                            .text_color(rgb(0xffffff))
                            .child(button_label),
                    )
                    .when(!self.is_selecting, |el| {
                        el.on_click(cx.listener(Self::handle_select_click))
                    }),
            )
    }
}
