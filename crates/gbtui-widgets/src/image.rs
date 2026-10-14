//! Image widget with an always-visible text fallback.
//!
//! The widget never owns a frame pipeline. It paints `alt` into Ratatui's cell
//! buffer every frame, then conditionally registers its prepared image through
//! the host's [`FrameCtx`].

use std::sync::Arc;

use gbtui_runtime::{
    FrameCtx,
    terminal::image::{ImagePlacement, PreparedImage, fit_image_to_cells},
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Paragraph, Widget, Wrap},
};

/// An inline image with a retained-terminal-graphics registration and text
/// fallback.
///
/// The host owns stable nonzero image IDs. Leaving the ID at its default `0`
/// paints the fallback but deliberately does not register graphics.
///
/// ```no_run
/// use std::sync::Arc;
///
/// use gbtui_runtime::terminal::image::PreparedImage;
/// use gbtui_widgets::Image;
///
/// fn screenshot_widget(
///     png_bytes: &[u8],
/// ) -> Result<Image<'static>, Box<dyn std::error::Error>> {
///     let image = Arc::new(PreparedImage::from_encoded(png_bytes)?);
///     Ok(Image::new(image, "Screenshot").id(42))
/// }
/// ```
pub struct Image<'a> {
    id: u32,
    image: Arc<PreparedImage>,
    alt: &'a str,
}

impl<'a> Image<'a> {
    /// Create an image widget. Call [`Self::id`] with a stable, nonzero host ID
    /// to opt into terminal graphics when the live frame supports them.
    pub fn new(image: Arc<PreparedImage>, alt: &'a str) -> Self {
        Self { id: 0, image, alt }
    }

    /// Set the host-assigned placement ID.
    ///
    /// `0` remains fallback-only, which is useful while a parent has not yet
    /// assigned persistent identity to this image.
    pub const fn id(mut self, id: u32) -> Self {
        self.id = id;
        self
    }

    /// Paint the complete available text fallback, then register a centered,
    /// aspect-preserving image placement for a graphics-capable live frame.
    pub fn render(&self, area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        cx.occlude(area);
        Paragraph::new(self.alt)
            .wrap(Wrap { trim: false })
            .render(area, buf);

        if self.id == 0 || !cx.supports_images() {
            return;
        }
        let (width, height) = fit_image_to_cells(
            self.image.width(),
            self.image.height(),
            area.width,
            area.height,
        );
        if width == 0 || height == 0 {
            return;
        }

        let placement_area = Rect::new(
            area.x.saturating_add((area.width - width) / 2),
            area.y.saturating_add((area.height - height) / 2),
            width,
            height,
        );
        cx.register_image(ImagePlacement {
            id: self.id,
            image: Arc::clone(&self.image),
            area: placement_area,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::HitMap;
    use ratatui::layout::Rect;
    use std::time::Instant;

    #[test]
    fn fallback_paints_when_no_graphics_id_is_assigned() {
        let image = Arc::new(
            PreparedImage::from_rgba8(1, 1, &[12, 34, 56, 255])
                .expect("a valid RGBA pixel prepares"),
        );
        let mut animation = AnimationClock::new(Instant::now());
        let mut hits = HitMap::default();
        let theme = Theme::night();
        let mut cx = FrameCtx::new(&theme, None, &mut animation, &mut hits);
        let area = Rect::new(0, 0, 12, 2);
        let mut buffer = Buffer::empty(area);

        Image::new(image, "Alt text").render(area, &mut buffer, &mut cx);

        assert_eq!(buffer[(0, 0)].symbol(), "A");
        assert_eq!(buffer[(1, 0)].symbol(), "l");
        assert_eq!(buffer[(2, 0)].symbol(), "t");
    }

    #[test]
    fn image_fallback_occludes_underlying_native_links() {
        let image = Arc::new(PreparedImage::from_rgba8(1, 1, &[0, 0, 0, 255]).unwrap());
        let mut animation = AnimationClock::new(Instant::now());
        let mut hits = HitMap::default();
        let theme = Theme::night();
        let mut cx = FrameCtx::new(&theme, None, &mut animation, &mut hits)
            .with_hyperlink_capabilities(gbtui_runtime::terminal::HyperlinkCapabilities::native());
        let area = Rect::new(0, 0, 12, 2);
        cx.register_link(area, Arc::from("https://example.com"), Some(1));
        Image::new(image, "Alt text").render(area, &mut Buffer::empty(area), &mut cx);
        assert!(
            cx.frame_links().is_empty(),
            "covered text must not remain a native link"
        );
    }
}
