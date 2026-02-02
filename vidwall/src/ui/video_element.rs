use std::panic::Location;
use std::sync::Arc;

use gpui::{
    Bounds, Corners, ElementId, GlobalElementId, InspectorElementId, LayoutId, Pixels, Point,
    RenderImage, Size, Window, fill, prelude::*, px,
};

use crate::playback::VideoPlayer;

/**
    A video element that renders frames from a VideoPlayer with crop-to-fill scaling.
*/
pub struct VideoElement {
    player: Arc<VideoPlayer>,
    /// The original aspect ratio of the video (width / height)
    video_aspect_ratio: f32,
    id: ElementId,
}

impl VideoElement {
    pub fn new(
        player: Arc<VideoPlayer>,
        video_aspect_ratio: f32,
        id: impl Into<ElementId>,
    ) -> Self {
        Self {
            player,
            video_aspect_ratio,
            id: id.into(),
        }
    }

    /**
        Calculate bounds for crop-to-fill scaling.

        Given the video's aspect ratio and the cell bounds,
        returns the bounds at which to paint the image so that it
        fills the cell completely while maintaining aspect ratio.
        Overflow will be clipped by the parent's overflow_hidden.

        Values are rounded to avoid sub-pixel flickering at cell edges.
    */
    fn calculate_fill_bounds(&self, cell_bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        let cell_x: f32 = cell_bounds.origin.x.into();
        let cell_y: f32 = cell_bounds.origin.y.into();
        let cell_width: f32 = cell_bounds.size.width.into();
        let cell_height: f32 = cell_bounds.size.height.into();
        let cell_aspect = cell_width / cell_height;
        let video_aspect = self.video_aspect_ratio;

        if (video_aspect - cell_aspect).abs() < 0.001 {
            // Aspect ratios match, no adjustment needed
            // Still round to pixel boundaries to avoid edge flickering
            return Bounds {
                origin: Point {
                    x: px(cell_x.round()),
                    y: px(cell_y.round()),
                },
                size: Size {
                    width: px(cell_width.round()),
                    height: px(cell_height.round()),
                },
            };
        }

        let (paint_width, paint_height) = if video_aspect > cell_aspect {
            // Video is wider than cell - expand width to fill, crop sides
            let height = cell_height;
            let width = height * video_aspect;
            (width, height)
        } else {
            // Video is taller than cell - expand height to fill, crop top/bottom
            let width = cell_width;
            let height = width / video_aspect;
            (width, height)
        };

        // Center the image in the cell, rounding to pixel boundaries
        let x_offset = (cell_width - paint_width) / 2.0;
        let y_offset = (cell_height - paint_height) / 2.0;

        Bounds {
            origin: Point {
                x: px((cell_x + x_offset).round()),
                y: px((cell_y + y_offset).round()),
            },
            size: Size {
                width: px(paint_width.round()),
                height: px(paint_height.round()),
            },
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
    /**
        (current_image, old_image_to_drop)
    */
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
            // Calculate fill bounds (may extend beyond cell, will be clipped by overflow_hidden)
            let fill_bounds = self.calculate_fill_bounds(bounds);

            // Paint the image scaled to fill bounds
            let _ = window.paint_image(
                fill_bounds,
                Corners::default(),
                render_image,
                0,     // frame index
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

/**
    Helper function to create a video element
*/
pub fn video_element(
    player: Arc<VideoPlayer>,
    video_aspect_ratio: f32,
    id: impl Into<ElementId>,
) -> VideoElement {
    VideoElement::new(player, video_aspect_ratio, id)
}
