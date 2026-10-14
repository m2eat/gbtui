//! One- or two-row alert banner with a hide control.
//!
//! Layout learned from `views/announcements.rs` (REWRITE). No announcements
//! crate and no promo CTA.
//!
//! ```text
//! ! Title                                    [hide]
//!   Message…
//! ```
//!
//! # Mouse
//!
//! `[hide]` registers through `FrameCtx`. The application maps [`HIDE`].

use gbtui_core::{HitId, SafeBuf, ThemeRole, ThemeState, line_utils};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use unicode_width::UnicodeWidthStr;

use crate::paint::fill;

pub const HIDE: HitId = HitId::named("gbtui.banner.hide");
pub const BODY: HitId = HitId::named("gbtui.banner.body");

const HIDE_LABEL: &str = "[hide]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BannerKind {
    #[default]
    Info,
    Critical,
}

pub struct Banner<'a> {
    pub title: &'a str,
    pub message: Option<&'a str>,
    pub kind: BannerKind,
}

impl<'a> Banner<'a> {
    pub fn critical(title: &'a str) -> Self {
        Self {
            title,
            message: None,
            kind: BannerKind::Critical,
        }
    }

    pub fn info(title: &'a str) -> Self {
        Self {
            title,
            message: None,
            kind: BannerKind::Info,
        }
    }

    pub fn message(mut self, message: &'a str) -> Self {
        self.message = Some(message);
        self
    }

    pub fn height(&self) -> u16 {
        if self.message.is_some() { 2 } else { 1 }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) -> u16 {
        let h = self.height().min(area.height);
        if h == 0 || area.width < 8 {
            return 0;
        }
        let r = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: h,
        };
        let role = match self.kind {
            BannerKind::Critical => ThemeRole::Error,
            BannerKind::Info => ThemeRole::Warning,
        };
        let base = cx.style(ThemeRole::Elevated, ThemeState::Normal);
        let accent = base.patch(cx.style(role, ThemeState::Normal));
        fill(buf, r, base);
        cx.register(BODY, r);

        let hide_w = HIDE_LABEL.width() as u16;
        let hide = Rect {
            x: r.right().saturating_sub(hide_w + 1),
            y: r.y,
            width: hide_w,
            height: 1,
        };
        cx.register(HIDE, hide);
        let title_budget = hide.x.saturating_sub(r.x).saturating_sub(3) as usize;
        let prefix = "! ";
        let title_style = accent.add_modifier(Modifier::BOLD);
        buf.set_string_safe(r.x, r.y, prefix, title_style);
        buf.set_string_safe(
            r.x + 2,
            r.y,
            line_utils::truncate_str(self.title, title_budget),
            title_style,
        );
        let hide_state = ThemeState::from_flags(false, false, false, false, cx.hovered(hide));
        let hide_st = if hide_state == ThemeState::Hovered {
            base.patch(cx.style(ThemeRole::Hover, hide_state))
                .patch(cx.style(ThemeRole::Error, hide_state))
        } else {
            base.patch(cx.style(ThemeRole::Muted, hide_state))
        };
        buf.set_string_safe(hide.x, hide.y, HIDE_LABEL, hide_st);

        if let Some(msg) = self.message
            && h > 1
        {
            buf.set_string_safe(
                r.x + 2,
                r.y + 1,
                line_utils::truncate_str(msg, r.width.saturating_sub(2) as usize),
                base.patch(cx.style(ThemeRole::Text, ThemeState::Normal)),
            );
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::{HitMap, HitResult};
    use std::time::Instant;

    #[test]
    fn hide_hit_and_critical_color() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 2));
        Banner::critical("Heads up")
            .message("something broke")
            .render(Rect::new(0, 0, 40, 2), &mut buf, &mut cx);
        assert_eq!(buf[(0, 0)].fg, theme.error);
        assert_eq!(hits.hit(36, 0), HitResult::Hit(HIDE));
        assert!(buf[(2, 1)].symbol().contains('s') || buf[(2, 1)].symbol() == "s");
    }
}
