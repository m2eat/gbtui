//! Dismissible label chip: `label [✗]`.
//!
//! Does not exist as a type in Grok (REWRITE). Theme colors only.
//!
//! ```text
//!  Night [✗]
//!  ^label ^close (hover → error)
//! ```
//!
//! # Mouse
//!
//! Body and close register through `FrameCtx`. The application maps
//! [`Chip::close_id`] clicks; this widget does not match mouse kinds.

use gbtui_core::{HitId, SafeBuf, ThemeRole, ThemeState};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::paint::fill;

const CLOSE_LABEL: &str = "[✗]";

/// Compact labeled tag, optionally dismissible.
pub struct Chip<'a> {
    id: HitId,
    close_id: HitId,
    label: &'a str,
    dismissible: bool,
}

impl<'a> Chip<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            id: HitId::from_raw(0),
            close_id: HitId::from_raw(0),
            label,
            dismissible: true,
        }
    }

    pub fn id(mut self, id: HitId) -> Self {
        self.id = id;
        self
    }

    pub fn close_id(mut self, id: HitId) -> Self {
        self.close_id = id;
        self
    }

    pub fn dismissible(mut self, on: bool) -> Self {
        self.dismissible = on;
        self
    }

    pub fn hit_id(&self) -> HitId {
        self.id
    }

    pub fn close_hit_id(&self) -> HitId {
        self.close_id
    }

    /// Columns needed for the chip (label, padding, optional close).
    pub fn width(&self) -> u16 {
        let mut w = self.label.width() as u16 + 2;
        if self.dismissible {
            w = w.saturating_add(1 + CLOSE_LABEL.width() as u16);
        }
        w
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) -> u16 {
        if area.height == 0 || area.width == 0 {
            return 0;
        }
        let w = self.width().min(area.width);
        let r = Rect {
            x: area.x,
            y: area.y,
            width: w,
            height: 1,
        };
        cx.register(self.id, r);
        let state = ThemeState::from_flags(false, false, false, false, cx.hovered(r));
        let role = if state == ThemeState::Hovered {
            ThemeRole::Hover
        } else {
            ThemeRole::Muted
        };
        let style = cx
            .style(ThemeRole::Elevated, ThemeState::Normal)
            .patch(cx.style(role, state));
        fill(buf, r, style);
        buf.set_string_safe(r.x, r.y, format!(" {} ", self.label), style);

        if self.dismissible {
            let cw = CLOSE_LABEL.width() as u16;
            if w > cw {
                let close = Rect {
                    x: r.right().saturating_sub(cw),
                    y: r.y,
                    width: cw,
                    height: 1,
                };
                cx.register(self.close_id, close);
                let close_style = if cx.hovered(close) {
                    cx.style(ThemeRole::Hover, ThemeState::Hovered)
                        .patch(cx.style(ThemeRole::Error, ThemeState::Hovered))
                } else {
                    style
                };
                buf.set_string_safe(close.x, close.y, CLOSE_LABEL, close_style);
            }
        }
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::{HitMap, HitResult};
    use std::time::Instant;

    const BODY: HitId = HitId::named("chip.body");
    const CLOSE: HitId = HitId::named("chip.close");

    #[test]
    fn close_hover_uses_error() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let w = Chip::new("Night").id(BODY).close_id(CLOSE).width();
        let mut cx = FrameCtx::new(&theme, Some((w - 1, 0)), &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        Chip::new("Night").id(BODY).close_id(CLOSE).render(
            Rect::new(0, 0, 20, 1),
            &mut buf,
            &mut cx,
        );
        assert_eq!(buf[(w - 1, 0)].fg, theme.error);
        assert_eq!(hits.hit(w - 1, 0), HitResult::Hit(CLOSE));
        assert_eq!(hits.hit(1, 0), HitResult::Hit(BODY));
    }

    #[test]
    fn occlude_after_register_swallows_clicks() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        let area = Rect::new(0, 0, 20, 1);
        Chip::new("Night")
            .id(BODY)
            .close_id(CLOSE)
            .render(area, &mut buf, &mut cx);
        cx.occlude(area);
        assert_eq!(hits.hit(1, 0), HitResult::Occluded);
        assert_eq!(
            hits.hit(Chip::new("Night").width() - 1, 0),
            HitResult::Occluded
        );
    }
}
