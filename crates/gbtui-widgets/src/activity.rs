//! One-row activity chrome: spinner + label + timers + optional stop.
//!
//! Layout learned from `views/turn_status.rs` (REWRITE). No ACP types.
//!
//! ```text
//!  ⠋ replying  0.2s                         1m20s  [stop]
//! ```
//!
//! Hidden (`running == false`) paints nothing and does not request a tick.

use std::time::Duration;

use gbtui_core::{HitId, SafeBuf, ThemeRole, ThemeState, TickDemand};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::paint::fill;
use crate::spinner::Spinner;

pub const STOP: HitId = HitId::named("gbtui.activity.stop");

const STOP_LABEL: &str = "[stop]";

pub struct ActivityRow<'a> {
    pub label: &'a str,
    pub phase: Option<Duration>,
    pub total: Option<Duration>,
    pub running: bool,
    pub stop: bool,
}

impl<'a> ActivityRow<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            phase: None,
            total: None,
            running: true,
            stop: true,
        }
    }

    pub fn total(mut self, total: Duration) -> Self {
        self.total = Some(total);
        self
    }

    pub fn phase(mut self, phase: Duration) -> Self {
        self.phase = Some(phase);
        self
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) -> u16 {
        if !self.running || area.height == 0 || area.width < 8 {
            return 0;
        }
        cx.request_tick(TickDemand::Fast);
        let r = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        fill(buf, r, cx.style(ThemeRole::Surface, ThemeState::Normal));
        Spinner::new().running(true).render(r.x, r.y, buf, cx);
        let mut x = r.x.saturating_add(2);
        let mut st = cx.style(ThemeRole::Accent, ThemeState::Normal);
        buf.set_string_safe(x, r.y, self.label, st);
        x = x.saturating_add(self.label.width() as u16 + 1);
        if let Some(p) = self.phase {
            st = cx.style(ThemeRole::Muted, ThemeState::Normal);
            buf.set_string_safe(x, r.y, fmt_short(p), st);
        }

        let mut right_x = r.right();
        if self.stop {
            let w = STOP_LABEL.width() as u16;
            let stop = Rect {
                x: right_x.saturating_sub(w),
                y: r.y,
                width: w,
                height: 1,
            };
            cx.register(STOP, stop);
            let stop_state = ThemeState::from_flags(false, false, false, false, cx.hovered(stop));
            let stop_st = cx
                .style(ThemeRole::Surface, ThemeState::Normal)
                .patch(cx.style(ThemeRole::Error, stop_state));
            buf.set_string_safe(stop.x, stop.y, STOP_LABEL, stop_st);
            right_x = stop.x.saturating_sub(1);
        }
        if let Some(t) = self.total {
            let s = fmt_long(t);
            let w = s.width() as u16;
            if right_x.saturating_sub(w) > x {
                buf.set_string_safe(
                    right_x.saturating_sub(w),
                    r.y,
                    &s,
                    cx.style(ThemeRole::Muted, ThemeState::Normal),
                );
            }
        }
        1
    }
}

fn fmt_short(d: Duration) -> String {
    let s = d.as_secs_f32();
    if s < 10.0 {
        format!("{s:.1}s")
    } else {
        format!("{}s", d.as_secs())
    }
}

fn fmt_long(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::{HitMap, HitResult};
    use std::time::Instant;

    #[test]
    fn hidden_does_not_tick() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        let row = ActivityRow {
            running: false,
            ..ActivityRow::new("x")
        };
        assert_eq!(row.render(Rect::new(0, 0, 40, 1), &mut buf, &mut cx), 0);
        assert_eq!(anim.demand(), TickDemand::None);
    }

    #[test]
    fn running_registers_stop_and_fast_tick() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        ActivityRow::new("replying")
            .total(Duration::from_secs(80))
            .render(Rect::new(0, 0, 40, 1), &mut buf, &mut cx);
        assert_eq!(anim.demand(), TickDemand::Fast);
        assert_eq!(hits.hit(38, 0), HitResult::Hit(STOP));
    }
}
