//! Self-rendering content for virtualized lists.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/render/renderable.rs (COPY)`

use std::ops::Range;
use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::WidgetRef;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::safe_buf::SafeBuf;

/// Per-item state supplied while a [`Renderable`] paints a virtualized list.
///
/// `match_ranges` are UTF-8 byte offsets in `search_text`. They deliberately
/// describe content rather than terminal columns: a custom renderable can map
/// them to its own styled or wrapped representation without guessing at wide
/// characters.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext<'a> {
    pub source_id: Option<usize>,
    pub search_text: &'a str,
    pub match_ranges: &'a [Range<usize>],
    pub selected: bool,
    pub hovered: bool,
    /// Theme-resolved base cell style, including list selection background.
    pub style: Style,
    /// Theme-resolved style for grapheme-safe search hits.
    pub match_style: Style,
}

/// Content that reports physical height at a width and paints a visible slice.
///
/// `line_offset` is a physical row within this renderable, while `area` is
/// already clipped to the viewport. Implementations must paint only the slice
/// beginning at `line_offset`; they must not allocate a full-height buffer just
/// to crop it. Plain [`str`], [`Span`], and [`Line`] remain single-line
/// renderables.
pub trait Renderable {
    fn render_slice(&self, area: Rect, buf: &mut Buffer, line_offset: usize);
    fn desired_height(&self, width: u16) -> usize;

    /// Paint a slice with list selection and search metadata.
    ///
    /// The default preserves the small `Renderable` contract. Rich content can
    /// override this method when it has an exact mapping from `search_text` to
    /// its styled output.
    fn render_slice_with_context(
        &self,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        _context: RenderContext<'_>,
    ) {
        self.render_slice(area, buf, line_offset);
    }
}

impl Renderable for () {
    fn render_slice(&self, _area: Rect, _buf: &mut Buffer, _line_offset: usize) {}

    fn desired_height(&self, _width: u16) -> usize {
        0
    }
}

impl Renderable for &str {
    fn render_slice(&self, area: Rect, buf: &mut Buffer, line_offset: usize) {
        if line_offset == 0 {
            self.render_ref(area, buf);
        }
    }

    fn render_slice_with_context(
        &self,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        context: RenderContext<'_>,
    ) {
        if line_offset == 0 {
            paint_highlighted_text(self, area, buf, 0, context.style, context);
        }
    }

    fn desired_height(&self, _width: u16) -> usize {
        1
    }
}

impl Renderable for String {
    fn render_slice(&self, area: Rect, buf: &mut Buffer, line_offset: usize) {
        self.as_str().render_slice(area, buf, line_offset);
    }

    fn render_slice_with_context(
        &self,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        context: RenderContext<'_>,
    ) {
        self.as_str()
            .render_slice_with_context(area, buf, line_offset, context);
    }

    fn desired_height(&self, _width: u16) -> usize {
        1
    }
}

impl<'a> Renderable for Span<'a> {
    fn render_slice(&self, area: Rect, buf: &mut Buffer, line_offset: usize) {
        if line_offset == 0 {
            self.render_ref(area, buf);
        }
    }

    fn render_slice_with_context(
        &self,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        context: RenderContext<'_>,
    ) {
        if line_offset == 0 {
            paint_highlighted_text(
                &self.content,
                area,
                buf,
                0,
                context.style.patch(self.style),
                context,
            );
        }
    }

    fn desired_height(&self, _width: u16) -> usize {
        1
    }
}

impl<'a> Renderable for Line<'a> {
    fn render_slice(&self, area: Rect, buf: &mut Buffer, line_offset: usize) {
        if line_offset == 0 {
            WidgetRef::render_ref(self, area, buf);
        }
    }

    fn render_slice_with_context(
        &self,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        context: RenderContext<'_>,
    ) {
        if line_offset != 0 || area.width == 0 || area.height == 0 {
            return;
        }
        let mut x = area.x;
        let mut offset = 0;
        let base = context.style.patch(self.style);
        for span in &self.spans {
            if !paint_highlighted_text_at(
                &span.content,
                area,
                buf,
                &mut x,
                offset,
                base.patch(span.style),
                context,
            ) {
                break;
            }
            offset = offset.saturating_add(span.content.len());
        }
    }

    fn desired_height(&self, _width: u16) -> usize {
        1
    }
}

impl<R: Renderable> Renderable for Option<R> {
    fn render_slice(&self, area: Rect, buf: &mut Buffer, line_offset: usize) {
        if let Some(renderable) = self {
            renderable.render_slice(area, buf, line_offset);
        }
    }

    fn render_slice_with_context(
        &self,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        context: RenderContext<'_>,
    ) {
        if let Some(renderable) = self {
            renderable.render_slice_with_context(area, buf, line_offset, context);
        }
    }

    fn desired_height(&self, width: u16) -> usize {
        self.as_ref().map(|r| r.desired_height(width)).unwrap_or(0)
    }
}

impl<R: Renderable> Renderable for Arc<R> {
    fn render_slice(&self, area: Rect, buf: &mut Buffer, line_offset: usize) {
        self.as_ref().render_slice(area, buf, line_offset);
    }

    fn render_slice_with_context(
        &self,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        context: RenderContext<'_>,
    ) {
        self.as_ref()
            .render_slice_with_context(area, buf, line_offset, context);
    }

    fn desired_height(&self, width: u16) -> usize {
        self.as_ref().desired_height(width)
    }
}

impl<R: Renderable + ?Sized> Renderable for Box<R> {
    fn render_slice(&self, area: Rect, buf: &mut Buffer, line_offset: usize) {
        self.as_ref().render_slice(area, buf, line_offset);
    }

    fn render_slice_with_context(
        &self,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        context: RenderContext<'_>,
    ) {
        self.as_ref()
            .render_slice_with_context(area, buf, line_offset, context);
    }

    fn desired_height(&self, width: u16) -> usize {
        self.as_ref().desired_height(width)
    }
}

/// Paint one physical line with UTF-8 match ranges expanded to graphemes by
/// the query layer. Returns false once the terminal row is full or a newline
/// starts a subsequent physical row.
fn paint_highlighted_text_at(
    text: &str,
    area: Rect,
    buf: &mut Buffer,
    x: &mut u16,
    text_offset: usize,
    style: Style,
    context: RenderContext<'_>,
) -> bool {
    for (offset, grapheme) in text.grapheme_indices(true) {
        if grapheme == "\n" {
            return false;
        }
        let width = grapheme.width().min(u16::MAX as usize) as u16;
        if width > 0 && x.saturating_add(width) > area.right() {
            return false;
        }
        let range = text_offset.saturating_add(offset)
            ..text_offset
                .saturating_add(offset)
                .saturating_add(grapheme.len());
        let matched = context
            .match_ranges
            .iter()
            .any(|hit| hit.start < range.end && range.start < hit.end);
        buf.set_string_safe(
            *x,
            area.y,
            grapheme,
            if matched { context.match_style } else { style },
        );
        *x = x.saturating_add(width);
    }
    true
}

fn paint_highlighted_text(
    text: &str,
    area: Rect,
    buf: &mut Buffer,
    text_offset: usize,
    style: Style,
    context: RenderContext<'_>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let mut x = area.x;
    paint_highlighted_text_at(text, area, buf, &mut x, text_offset, style, context);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_has_zero_height() {
        assert_eq!(().desired_height(80), 0);
    }

    #[test]
    fn str_has_height_one() {
        assert_eq!("hello".desired_height(80), 1);
    }

    #[test]
    fn option_none_has_zero_height() {
        let opt: Option<&str> = None;
        assert_eq!(opt.desired_height(80), 0);
    }
}
