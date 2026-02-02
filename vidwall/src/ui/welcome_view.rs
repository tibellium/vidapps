use std::path::PathBuf;

use gpui::{Context, IntoElement, Render, Window, div, prelude::*, px, rgb};

/**
    Event emitted when the user selects videos from the welcome screen.
*/
pub struct VideosSelected {
    pub paths: Vec<PathBuf>,
}

/**
    The welcome screen shown when the app starts without any paths.

    Displays a centered UI with a button to select videos or folders.
*/
pub struct WelcomeView {
    is_selecting: bool,
}

impl gpui::EventEmitter<VideosSelected> for WelcomeView {}

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
            directories: true,
            multiple: true,
            prompt: Some("Select videos or folders".into()),
        });

        cx.spawn(async move |this, cx| {
            match future.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => {
                    cx.update(|cx| {
                        // Activate the window to bring it to front after file picker closes
                        if let Some(window) = cx.active_window() {
                            window
                                .update(cx, |_, window, _cx| {
                                    window.activate_window();
                                })
                                .ok();
                        }

                        this.update(cx, |_this, cx| {
                            cx.emit(VideosSelected { paths });
                        })
                        .ok();
                    })
                    .ok();
                }
                _ => {
                    // Cancelled or error - reset selecting state and activate window
                    cx.update(|cx| {
                        // Activate window even on cancel to bring it back to front
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
            }
        })
        .detach();
    }
}

impl Render for WelcomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let button_label = if self.is_selecting {
            "Selecting..."
        } else {
            "Select Videos"
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
                    .child("Video Wall"),
            )
            .child(
                div()
                    .text_size(px(16.0))
                    .text_color(rgb(0x888888))
                    .child("Select videos or folders to create a dynamic video wall"),
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
