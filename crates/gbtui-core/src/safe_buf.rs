//! Bounds-checked buffer writes. Bidirectional reorder is off by default.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/render/safe_buf.rs (STRIP)`

use ratatui::buffer::Buffer;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Extension trait for bounds-checked buffer writes.
pub trait SafeBuf {
    fn set_line_safe(&mut self, x: u16, y: u16, line: &Line<'_>, width: u16);
    fn set_span_safe(&mut self, x: u16, y: u16, span: &Span<'_>, width: u16);
    fn set_string_safe<S: AsRef<str>>(&mut self, x: u16, y: u16, string: S, style: Style);
}

impl SafeBuf for Buffer {
    #[inline]
    fn set_line_safe(&mut self, x: u16, y: u16, line: &Line<'_>, width: u16) {
        if y >= self.area.y && y < self.area.bottom() && x < self.area.right() {
            self.set_line(x, y, line, width);
        }
    }

    #[inline]
    fn set_span_safe(&mut self, x: u16, y: u16, span: &Span<'_>, width: u16) {
        if y >= self.area.y && y < self.area.bottom() && x < self.area.right() {
            self.set_span(x, y, span, width);
        }
    }

    #[inline]
    fn set_string_safe<S: AsRef<str>>(&mut self, x: u16, y: u16, string: S, style: Style) {
        if y >= self.area.y && y < self.area.bottom() && x < self.area.right() {
            self.set_string(x, y, string, style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    #[test]
    fn out_of_bounds_y_does_not_panic() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
        buf.set_string_safe(0, 99, "nope", Style::default());
        buf.set_line_safe(0, 99, &Line::from("nope"), 10);
        buf.set_span_safe(0, 99, &Span::raw("nope"), 10);
    }

    #[test]
    fn in_bounds_write_lands() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
        buf.set_string_safe(0, 0, "hi", Style::default());
        assert_eq!(buf[(0, 0)].symbol(), "h");
        assert_eq!(buf[(1, 0)].symbol(), "i");
    }
}
