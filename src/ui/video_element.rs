use std::panic::Location;
use std::sync::Arc;

use gpui::{
    fill, prelude::*, Bounds, Corners, ElementId, GlobalElementId, InspectorElementId, LayoutId,
    Pixels, RenderImage, Window,
};

use crate::video::VideoPlayer;

/// A video element that renders frames from a VideoPlayer
pub struct VideoElement {
    player: Arc<VideoPlayer>,
    id: ElementId,
}

impl VideoElement {
    pub fn new(player: Arc<VideoPlayer>, id: impl Into<ElementId>) -> Self {
        Self {
            player,
            id: id.into(),
        }
    }
}

impl IntoElement for VideoElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for VideoElement {
    type RequestLayoutState = ();
    /// (current_image, old_image_to_drop)
    type PrepaintState = (Option<Arc<RenderImage>>, Option<Arc<RenderImage>>);

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    #[track_caller]
    fn source_location(&self) -> Option<&'static Location<'static>> {
        Some(Location::caller())
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // Create a style that fills the parent container
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.0).into();
        style.size.height = gpui::relative(1.0).into();

        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        // Get the cached RenderImage from the player
        // This only creates a new RenderImage when the frame actually changes
        self.player.get_render_image()
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut gpui::App,
    ) {
        let (current_image, old_image) = std::mem::take(prepaint);

        // Drop the old image from the sprite atlas to free memory
        if let Some(old) = old_image {
            let _ = window.drop_image(old);
        }

        if let Some(render_image) = current_image {
            // Paint the image scaled to fill bounds
            let _ = window.paint_image(
                bounds,
                Corners::default(),
                render_image,
                0, // frame index
                false, // grayscale
            );
        } else {
            // No frame available - draw black background
            window.paint_quad(fill(bounds, gpui::rgb(0x000000)));
        }

        // Request continuous animation for video playback
        window.request_animation_frame();
    }
}

/// Helper function to create a video element
pub fn video_element(player: Arc<VideoPlayer>, id: impl Into<ElementId>) -> VideoElement {
    VideoElement::new(player, id)
}
