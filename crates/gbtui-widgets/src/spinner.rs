//! Braille spinner and optional wave accent bar.
//!
//! Spinner: `cx.request_tick(Fast)` while running; idle demand drops when
//! `running` is false. Wave is not bound to a "running tool" semantic.
//!
//! ```text
//!  ⠋ working
//!  ▓▒░▒▓     ← WaveBar
//! ```

use std::time::Duration;

use gbtui_core::{SafeBuf, ThemeRole, ThemeState, TickDemand};
use gbtui_runtime::FrameCtx;
use gbtui_runtime::terminal::glyphs::braille_spinner_frames;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::paint::fill;

const SPINNER_PERIOD: Duration = Duration::from_millis(640);

/// Braille (or ASCII) spinner. Requests [`TickDemand::Fast`](gbtui_core::TickDemand::Fast) only while `running`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Spinner {
    pub running: bool,
}

impl Spinner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn running(mut self, running: bool) -> Self {
        self.running = running;
        self
    }

    /// Paint one cell. Returns the glyph used (empty when idle).
    pub fn render(&self, x: u16, y: u16, buf: &mut Buffer, cx: &mut FrameCtx<'_>) -> &'static str {
        if !self.running {
            return "";
        }
        cx.request_tick(TickDemand::Fast);
        let frames = braille_spinner_frames();
        let n = frames.len().max(1);
        let i = ((cx.anim().phase(SPINNER_PERIOD) * n as f32) as usize) % n;
        let glyph = frames[i];
        buf.set_string_safe(
            x,
            y,
            glyph,
            cx.style(ThemeRole::Surface, ThemeState::Normal)
                .patch(cx.style(ThemeRole::Accent, ThemeState::Normal))
                .add_modifier(Modifier::BOLD),
        );
        glyph
    }
}

/// Traveling wave along `area`. Requests [`TickDemand::Fast`] while painted.
pub fn render_wave_bar(area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    cx.request_tick(TickDemand::Fast);
    let style = cx
        .style(ThemeRole::Surface, ThemeState::Normal)
        .patch(cx.style(ThemeRole::Accent, ThemeState::Normal));
    fill(buf, area, style);
    for col in 0..area.width {
        let b = cx.anim().wave_brightness(col);
        let ch = if b > 0.75 {
            "█"
        } else if b > 0.45 {
            "▓"
        } else if b > 0.2 {
            "▒"
        } else {
            "░"
        };
        buf.set_string_safe(area.x + col, area.y, ch, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::HitMap;
    use std::time::Instant;

    #[test]
    fn idle_spinner_does_not_request_tick() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        Spinner::new().render(0, 0, &mut buf, &mut cx);
        assert_eq!(anim.demand(), TickDemand::None);
    }

    #[test]
    fn running_spinner_requests_fast() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        let g = Spinner::new().running(true).render(0, 0, &mut buf, &mut cx);
        assert!(!g.is_empty());
        assert_eq!(anim.demand(), TickDemand::Fast);
    }
}
