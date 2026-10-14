//! One-row status bar: left / center / right. Display only — no hits.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/views/status_bar.rs (STRIP)`
//!
//! ```text
//!  Context: 5.2k          Turn 2/3          insert
//!  ^ left                 ^ center          ^ right
//! ```
//!
//! Widths use unicode display columns, not byte length. Center and right
//! are skipped when they would overwrite the left segment.

use gbtui_core::{SafeBuf, ThemeRole, ThemeState};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::paint::fill;

/// Left-aligned status row with optional centered and right-aligned segments.
pub struct StatusBar<'a> {
    pub left: &'a str,
    pub center: Option<&'a str>,
    pub right: Option<&'a str>,
}

impl<'a> StatusBar<'a> {
    pub fn new(left: &'a str) -> Self {
        Self {
            left,
            center: None,
            right: None,
        }
    }

    pub fn center(mut self, text: &'a str) -> Self {
        self.center = Some(text);
        self
    }

    pub fn right(mut self, text: &'a str) -> Self {
        self.right = Some(text);
        self
    }

    /// Paint one row using the resolved semantic frame theme.
    pub fn render(&self, area: Rect, buf: &mut Buffer, cx: &FrameCtx<'_>) {
        let style = cx
            .style(ThemeRole::Surface, ThemeState::Normal)
            .patch(cx.style(ThemeRole::Muted, ThemeState::Normal));
        self.render_with_style(area, buf, style);
    }

    fn render_with_style(&self, area: Rect, buf: &mut Buffer, style: Style) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        fill(buf, area, style);

        let left_w = self.left.width() as u16;
        if left_w > 0 {
            buf.set_string_safe(area.x, area.y, self.left, style);
        }
        let left_end = area.x.saturating_add(left_w.min(area.width));
        let gap = 2u16;

        if let Some(center) = self.center {
            let cw = center.width() as u16;
            if cw > 0 && cw <= area.width {
                let cx = area.x + area.width.saturating_sub(cw) / 2;
                let c_end = cx.saturating_add(cw);
                if cx >= left_end.saturating_add(gap) && c_end <= area.right() {
                    buf.set_string_safe(cx, area.y, center, style);
                    if let Some(right) = self.right {
                        paint_right(area, buf, style, right, left_end, gap, Some(c_end));
                    }
                    return;
                }
            }
        }

        if let Some(right) = self.right {
            paint_right(area, buf, style, right, left_end, gap, None);
        }
    }
}

fn paint_right(
    area: Rect,
    buf: &mut Buffer,
    style: Style,
    right: &str,
    left_end: u16,
    gap: u16,
    center_end: Option<u16>,
) {
    let rw = right.width() as u16;
    if rw == 0 || rw > area.width {
        return;
    }
    let rx = area.right().saturating_sub(rw);
    let floor = match center_end {
        Some(c) => left_end.max(c).saturating_add(gap),
        None => left_end.saturating_add(gap),
    };
    if rx >= floor {
        buf.set_string_safe(rx, area.y, right, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::HitMap;
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    use std::time::Instant;

    fn row(buf: &Buffer, area: Rect) -> String {
        (area.x..area.right())
            .map(|x| buf[(x, area.y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn left_uses_semantic_muted_style() {
        let theme = Theme::night();
        let mut anim = AnimationClock::new(Instant::now());
        let mut hits = HitMap::new();
        let cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        StatusBar::new("hello").render(Rect::new(0, 0, 40, 1), &mut buf, &cx);
        let expected = theme
            .style(ThemeRole::Surface, ThemeState::Normal)
            .patch(theme.style(ThemeRole::Muted, ThemeState::Normal));
        assert_eq!(buf[(0, 0)].fg, expected.fg.unwrap_or(Color::Reset));
        assert_eq!(buf[(0, 0)].bg, expected.bg.unwrap_or(Color::Reset));
        assert!(row(&buf, Rect::new(0, 0, 40, 1)).starts_with("hello"));
    }

    #[test]
    fn unicode_width_not_byte_len() {
        let theme = Theme::night();
        let mut anim = AnimationClock::new(Instant::now());
        let mut hits = HitMap::new();
        let cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        // Each CJK glyph is display width 2; byte-len placement would overlap.
        StatusBar::new("你好").center("中").right("右").render(
            Rect::new(0, 0, 20, 1),
            &mut buf,
            &cx,
        );
        assert_eq!(buf[(0, 0)].symbol(), "你");
        assert_eq!(buf[(2, 0)].symbol(), "好");
        // Center at (20 - 2) / 2 = 9.
        assert_eq!(buf[(9, 0)].symbol(), "中");
        assert_eq!(buf[(18, 0)].symbol(), "右");
    }

    #[test]
    fn center_and_right_do_not_cover_left() {
        let theme = Theme::night();
        let mut anim = AnimationClock::new(Instant::now());
        let mut hits = HitMap::new();
        let cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 1));
        StatusBar::new("LEFTLEFT")
            .center("CENTER")
            .right("RIGHT")
            .render(Rect::new(0, 0, 12, 1), &mut buf, &cx);
        let s = row(&buf, Rect::new(0, 0, 12, 1));
        assert!(s.starts_with("LEFTLEFT"), "left must stay intact: {s:?}");
        assert!(
            !s.contains("CENTER"),
            "center must yield when it would overlap left: {s:?}"
        );
        assert!(
            !s.contains("RIGHT"),
            "right must yield when it would overlap left: {s:?}"
        );
    }
}
