use std::sync::Arc;

use gpui::{Context, Entity, IntoElement, Render, Window, div, prelude::*, rgb};

use crate::playback::VideoPlayer;

use super::grid_view::GridView;

/// The root view of the application.
///
/// Contains the video grid. Keyboard shortcuts are handled at the app level.
pub struct RootView {
    grid: Entity<GridView>,
}

impl RootView {
    /// Create a new root view with the given initial players.
    pub fn new(players: [Arc<VideoPlayer>; 4], cx: &mut Context<Self>) -> Self {
        let grid = cx.new(|cx| GridView::new(players, cx));
        Self { grid }
    }

    /// Get the grid view entity.
    pub fn grid(&self) -> &Entity<GridView> {
        &self.grid
    }
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("root")
            .size_full()
            .bg(rgb(0x000000))
            .child(self.grid.clone())
    }
}
