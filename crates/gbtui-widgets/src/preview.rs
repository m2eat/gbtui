//! Multiline preview popup: first N lines, optional `⋮ (k more)`, last N.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/render/preview_overlay.rs (STRIP)`
//!
//! ```text
//!  ╭────────────────────╮
//!  │ first line         │
//!  │ ⋮ (3 more)         │
//!  │ last line          │
//!  ╰─ enter to expand ─╯
//! ```

use gbtui_core::{HitId, SafeBuf, ThemeRole, ThemeState, line_utils};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Clear, Widget};

use crate::paint::fill;

pub const BODY: HitId = HitId::named("gbtui.preview.body");

#[derive(Debug, Clone)]
pub struct PreviewConfig {
    pub preview_lines: usize,
    pub width_ratio: f32,
    pub min_width: u16,
    pub min_height: u16,
    pub hint: Option<&'static str>,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            preview_lines: 3,
            width_ratio: 0.75,
            min_width: 20,
            min_height: 5,
            hint: None,
        }
    }
}

/// Paint a preview anchored at the bottom of `area`. Returns the box rect.
pub fn render_preview(
    content: &str,
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
    config: &PreviewConfig,
) -> Option<Rect> {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    if total == 0 || area.height < config.min_height || area.width < config.min_width {
        return None;
    }
    let n = config.preview_lines.max(1);
    let needs_dots = total > n * 2;
    let content_rows = if needs_dots { n * 2 + 1 } else { total };
    let box_height = (content_rows as u16 + 2).min(area.height);
    let box_width = ((area.width as f32) * config.width_ratio) as u16;
    let box_width = box_width.max(config.min_width).min(area.width);
    let box_area = Rect {
        x: area.x + area.width.saturating_sub(box_width) / 2,
        y: area.bottom().saturating_sub(box_height),
        width: box_width,
        height: box_height,
    };
    cx.occlude(box_area);
    cx.register(BODY, box_area);
    // Fill with spaces after Clear so a Reset background cannot expose text
    // from the prior frame beneath a transparent overlay.
    Clear.render(box_area, buf);
    let base = cx.style(ThemeRole::Elevated, ThemeState::Normal);
    fill(buf, box_area, base);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(base.patch(cx.style(ThemeRole::Border, ThemeState::Normal)))
        .style(base);
    let inner = block.inner(box_area);
    block.render(box_area, buf);

    let text_st = base.patch(cx.style(ThemeRole::Text, ThemeState::Normal));
    let dim_st = base
        .patch(cx.style(ThemeRole::Muted, ThemeState::Normal))
        .add_modifier(Modifier::ITALIC);
    let mut row = 0u16;
    if needs_dots {
        for line in lines.iter().take(n) {
            if row >= inner.height {
                break;
            }
            buf.set_string_safe(
                inner.x,
                inner.y + row,
                line_utils::truncate_str(line, inner.width as usize),
                text_st,
            );
            row += 1;
        }
        if row < inner.height {
            let skipped = total.saturating_sub(n * 2);
            buf.set_string_safe(
                inner.x,
                inner.y + row,
                line_utils::truncate_str(&format!("⋮ ({skipped} more)"), inner.width as usize),
                dim_st,
            );
            row += 1;
        }
        for line in lines
            .iter()
            .rev()
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            if row >= inner.height {
                break;
            }
            buf.set_string_safe(
                inner.x,
                inner.y + row,
                line_utils::truncate_str(line, inner.width as usize),
                text_st,
            );
            row += 1;
        }
    } else {
        for line in &lines {
            if row >= inner.height {
                break;
            }
            buf.set_string_safe(
                inner.x,
                inner.y + row,
                line_utils::truncate_str(line, inner.width as usize),
                text_st,
            );
            row += 1;
        }
    }

    if let Some(hint) = config.hint {
        let y = box_area.bottom().saturating_sub(1);
        let t = format!(" {hint} ");
        if (t.len() as u16) + 4 < box_area.width {
            buf.set_string_safe(
                box_area.x + 2,
                y,
                &t,
                base.patch(cx.style(ThemeRole::Muted, ThemeState::Normal)),
            );
        }
    }
    Some(box_area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::{HitMap, HitResult};
    use std::time::Instant;

    #[test]
    fn truncates_with_more_marker() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 16));
        let content = (0..10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let cfg = PreviewConfig {
            preview_lines: 2,
            ..Default::default()
        };
        let rect = render_preview(&content, Rect::new(0, 0, 40, 16), &mut buf, &mut cx, &cfg);
        assert!(rect.is_some());
        let mut blob = String::new();
        for y in 0..16u16 {
            for x in 0..40u16 {
                blob.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(blob.contains("line0"), "{blob}");
        assert!(blob.contains("more"), "{blob}");
        assert_eq!(hits.hit(20, 12), HitResult::Hit(BODY));
    }
}
