//! One-row ephemeral toast. Visible while TTL remains; Slow tick while shown.
//!
//! Layout learned from `views/welcome/toast.rs` (REWRITE). No welcome flow,
//! consent, or logo. Colors are `accent` / `bg_elevated` only.
//!
//! ```text
//!                    Saved Night  [✗]
//!                    ^ bottom-right of area
//! ```
//!
//! # Keys
//!
//! None — the application maps `CLOSE` clicks. Esc is the caller's job.

use std::time::{Duration, Instant};

use gbtui_core::{HitId, SafeBuf, ThemeRole, ThemeState, TickDemand, line_utils};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use unicode_width::UnicodeWidthStr;

use crate::paint::fill;

/// Clickable dismiss control.
pub const CLOSE: HitId = HitId::named("gbtui.toast.close");
/// Body so clicks on the message are not treated as a miss.
pub const BODY: HitId = HitId::named("gbtui.toast.body");

/// Default time-to-live.
pub const DEFAULT_TTL: Duration = Duration::from_millis(3000);

const CLOSE_LABEL: &str = "[✗]";

/// Ephemeral one-line notice.
#[derive(Debug, Clone)]
pub struct Toast {
    message: String,
    shown_at: Option<Instant>,
    ttl: Duration,
}

impl Default for Toast {
    fn default() -> Self {
        Self {
            message: String::new(),
            shown_at: None,
            ttl: DEFAULT_TTL,
        }
    }
}

impl Toast {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn show(&mut self, message: impl Into<String>, now: Instant) {
        self.message = message.into();
        self.shown_at = Some(now);
    }

    pub fn dismiss(&mut self) {
        self.shown_at = None;
        self.message.clear();
    }

    pub fn is_visible(&self, now: Instant) -> bool {
        self.shown_at
            .is_some_and(|t| now.saturating_duration_since(t) < self.ttl)
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// Paint bottom-right inside `area`. Requests [`TickDemand::Slow`] while
    /// visible so the TTL can expire without a 30fps spin.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) -> Option<Rect> {
        let now = cx.anim().now();
        if !self.is_visible(now) {
            if self.shown_at.is_some() {
                self.dismiss();
            }
            return None;
        }
        if area.height == 0 || area.width < 6 {
            return None;
        }
        cx.request_tick(TickDemand::Slow);

        let close_w = CLOSE_LABEL.width() as u16;
        let pad = 1u16;
        let budget =
            area.width
                .saturating_sub(close_w.saturating_add(pad.saturating_mul(3))) as usize;
        if budget == 0 {
            return None;
        }
        let body = line_utils::truncate_str(&self.message, budget);
        let text = format!(" {body} {CLOSE_LABEL} ");
        let w = (text.width() as u16).min(area.width);
        let x = area.right().saturating_sub(w);
        let y = area.bottom().saturating_sub(1);
        let rect = Rect {
            x,
            y,
            width: w,
            height: 1,
        };

        let style = cx
            .style(ThemeRole::Elevated, ThemeState::Normal)
            .patch(cx.style(ThemeRole::Accent, ThemeState::Normal))
            .add_modifier(Modifier::BOLD);
        fill(buf, rect, style);
        cx.register(BODY, rect);

        let close = Rect {
            x: rect.right().saturating_sub(close_w.saturating_add(1)),
            y,
            width: close_w,
            height: 1,
        };
        cx.register(CLOSE, close);
        let close_state = ThemeState::from_flags(false, false, false, false, cx.hovered(close));
        let close_style = if close_state == ThemeState::Hovered {
            cx.style(ThemeRole::Elevated, ThemeState::Normal)
                .patch(cx.style(ThemeRole::Hover, close_state))
                .patch(cx.style(ThemeRole::Error, close_state))
        } else {
            style
        };
        buf.set_string_safe(rect.x, y, format!(" {body} "), style);
        buf.set_string_safe(close.x, y, CLOSE_LABEL, close_style);
        Some(rect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::HitMap;
    use std::time::Duration;

    fn cx<'a>(
        theme: &'a Theme,
        mouse: Option<(u16, u16)>,
        anim: &'a mut AnimationClock,
        hits: &'a mut HitMap,
    ) -> FrameCtx<'a> {
        FrameCtx::new(theme, mouse, anim, hits)
    }

    #[test]
    fn visible_requests_slow_tick() {
        let theme = Theme::night();
        let t0 = Instant::now();
        let mut anim = AnimationClock::new(t0);
        let mut hits = HitMap::new();
        let mut cx = cx(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
        let mut toast = Toast::new();
        toast.show("ok", t0);
        toast.render(Rect::new(0, 0, 40, 4), &mut buf, &mut cx);
        assert_eq!(anim.demand(), TickDemand::Slow);
        let row: String = (0..40).map(|x| buf[(x, 3)].symbol().to_string()).collect();
        assert!(row.contains("ok"), "{row:?}");
        assert_eq!(buf[(38, 3)].fg, theme.accent);
        assert_eq!(buf[(38, 3)].bg, theme.bg_elevated);
    }

    #[test]
    fn expired_does_not_request_tick() {
        let theme = Theme::night();
        let t0 = Instant::now();
        let mut anim = AnimationClock::new(t0 + Duration::from_secs(10));
        let mut hits = HitMap::new();
        let mut cx = cx(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
        let mut toast = Toast::new();
        toast.show("ok", t0);
        assert!(
            toast
                .render(Rect::new(0, 0, 40, 4), &mut buf, &mut cx)
                .is_none()
        );
        assert_eq!(anim.demand(), TickDemand::None);
        assert!(!toast.is_visible(t0 + Duration::from_secs(10)));
    }

    #[test]
    fn close_hit_is_registered() {
        let theme = Theme::night();
        let t0 = Instant::now();
        let mut anim = AnimationClock::new(t0);
        let mut hits = HitMap::new();
        let mut cx = cx(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
        let mut toast = Toast::new();
        toast.show("ok", t0);
        let rect = toast
            .render(Rect::new(0, 0, 40, 4), &mut buf, &mut cx)
            .unwrap();
        assert_eq!(
            hits.hit(rect.right().saturating_sub(2), rect.y),
            gbtui_runtime::HitResult::Hit(CLOSE)
        );
        assert_eq!(
            hits.hit(rect.x + 1, rect.y),
            gbtui_runtime::HitResult::Hit(BODY)
        );
    }
}
