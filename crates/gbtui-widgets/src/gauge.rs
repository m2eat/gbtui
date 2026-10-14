//! Compact usage readout that becomes a same-width progress bar on hover.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/views/context_bar.rs (STRIP)`
//!
//! ```text
//!  8.5K / 1.0M     ← idle
//!  █████ 42.0%     ← hover, same columns
//! ```
//!
//! Breakpoint colors: `text_primary` → `accent` → `warning` → `error`.

use gbtui_core::{SafeBuf, Theme, ThemeRole, ThemeState};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use unicode_width::UnicodeWidthStr;

use crate::progress::render_progress_bar;

/// Format a count as at most ~4 characters (`999`, `1.2K`, `12K`, `1.2M`).
pub fn fmt_compact(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 10_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else if n < 1_000_000 {
        format!("{}K", n / 1_000)
    } else if n < 10_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else {
        format!("{}M", n / 1_000_000)
    }
}

fn gauge_role(fraction: f32) -> ThemeRole {
    let pct = (fraction * 100.0) as i32;
    if pct >= 90 {
        ThemeRole::Error
    } else if pct >= 70 {
        ThemeRole::Warning
    } else if pct >= 40 {
        ThemeRole::Accent
    } else {
        ThemeRole::Text
    }
}

pub fn gauge_color(theme: &Theme, fraction: f32) -> Color {
    theme
        .style(gauge_role(fraction), ThemeState::Normal)
        .fg
        .unwrap_or(Color::Reset)
}

/// `(current, max)` compact usage. Hover paints a bar of the idle width.
pub struct UsageGauge {
    pub current: u64,
    pub max: u64,
}

impl UsageGauge {
    pub fn new(current: u64, max: u64) -> Self {
        Self { current, max }
    }

    pub fn fraction(&self) -> f32 {
        if self.max == 0 {
            0.0
        } else {
            (self.current as f32 / self.max as f32).clamp(0.0, 1.0)
        }
    }

    pub fn idle_label(&self) -> String {
        format!("{} / {}", fmt_compact(self.current), fmt_compact(self.max))
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) -> u16 {
        if area.height == 0 || area.width == 0 {
            return 0;
        }
        let label = self.idle_label();
        let w = (label.width() as u16).max(6).min(area.width);
        let r = Rect {
            x: area.x + area.width.saturating_sub(w),
            y: area.y,
            width: w,
            height: 1,
        };
        let frac = self.fraction();
        let state = ThemeState::from_flags(false, false, false, false, cx.hovered(r));
        let gauge_style = cx
            .style(ThemeRole::Surface, ThemeState::Normal)
            .patch(cx.style(gauge_role(frac), ThemeState::Normal));
        let gauge_style = if state == ThemeState::Hovered {
            cx.style(ThemeRole::Surface, ThemeState::Normal)
                .patch(cx.style(ThemeRole::Hover, state))
                .patch(cx.style(gauge_role(frac), ThemeState::Normal))
        } else {
            gauge_style
        };
        let fg = gauge_style.fg.unwrap_or(Color::Reset);
        let bg = gauge_style.bg.unwrap_or(Color::Reset);
        if state == ThemeState::Hovered {
            let pct = format!("{:.0}%", frac * 100.0);
            let bar_w = w.saturating_sub(pct.width() as u16 + 1);
            render_progress_bar(buf, r.x, r.y, bar_w, frac, fg, bg);
            buf.set_string_safe(r.x + bar_w + 1, r.y, &pct, gauge_style);
        } else {
            buf.set_string_safe(r.x, r.y, &label, gauge_style);
        }
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::HitMap;
    use std::time::Instant;

    #[test]
    fn compact_format() {
        assert_eq!(fmt_compact(12), "12");
        assert_eq!(fmt_compact(1_200), "1.2K");
        assert_eq!(fmt_compact(12_000), "12K");
        assert_eq!(fmt_compact(1_200_000), "1.2M");
    }

    #[test]
    fn idle_paints_label_hover_paints_percent() {
        let theme = Theme::night();
        let g = UsageGauge::new(42, 100);
        let label = g.idle_label();
        let w = label.width() as u16;
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        g.render(Rect::new(0, 0, 20, 1), &mut buf, &mut cx);
        let idle: String = (0..20).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(idle.contains('/'), "{idle}");

        let x = 20 - w / 2;
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, Some((x, 0)), &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        g.render(Rect::new(0, 0, 20, 1), &mut buf, &mut cx);
        let hover: String = (0..20).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(hover.contains('%'), "{hover}");
    }
}
