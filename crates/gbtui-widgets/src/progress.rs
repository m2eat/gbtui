//! Unicode block progress bar at 1/8-cell resolution.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/views/progress_bar.rs (STRIP)`
//!
//! ```text
//! ███████▌░░
//! ```
//!
//! Consolas (legacy ConHost) is missing the narrow LEFT-fractional blocks, so
//! we substitute shade glyphs `░▒▓` there.

use gbtui_core::{ThemeRole, ThemeState};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use gbtui_runtime::terminal::glyphs::is_legacy_windows_console;

const BLOCKS: [&str; 9] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"];
const SHADES: [&str; 9] = ["", "░", "░", "░", "▒", "▒", "▓", "▓", "█"];

fn partial_blocks() -> &'static [&'static str; 9] {
    if is_legacy_windows_console() {
        &SHADES
    } else {
        &BLOCKS
    }
}

fn cell_breakdown(width: u16, value: f32) -> (u16, usize) {
    let value = value.clamp(0.0, 1.0);
    let total_eighths = (value * width as f32 * 8.0).round() as u16;
    let full = (total_eighths / 8).min(width);
    let remainder = (total_eighths % 8) as usize;
    (full, remainder)
}

fn bar_cells(width: u16, value: f32) -> impl Iterator<Item = (&'static str, bool)> {
    let (full, remainder) = cell_breakdown(width, value);
    let glyphs = partial_blocks();
    (0..width).map(move |i| {
        if i < full {
            (glyphs[8], true)
        } else if i == full && remainder > 0 {
            (glyphs[remainder], true)
        } else {
            (" ", false)
        }
    })
}

pub fn progress_bar_spans(width: u16, value: f32, fg: Color, bg: Color) -> Vec<Span<'static>> {
    let fg_style = Style::default().fg(fg).bg(bg);
    let bg_style = Style::default().bg(bg);
    bar_cells(width, value)
        .map(|(symbol, filled)| {
            let style = if filled { fg_style } else { bg_style };
            Span::styled(symbol.to_string(), style)
        })
        .collect()
}

pub fn render_progress_bar(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    value: f32,
    fg: Color,
    bg: Color,
) {
    let fg_style = Style::default().fg(fg).bg(bg);
    let bg_style = Style::default().bg(bg);
    for (i, (symbol, filled)) in bar_cells(width, value).enumerate() {
        let Some(cell) = buf.cell_mut((x + i as u16, y)) else {
            continue;
        };
        cell.set_symbol(symbol);
        cell.set_style(if filled { fg_style } else { bg_style });
    }
}

/// Theme-colored bar into `area` (one row) using the resolved frame theme.
pub fn render_progress(area: Rect, value: f32, buf: &mut Buffer, cx: &FrameCtx<'_>) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let style = cx
        .style(ThemeRole::Surface, ThemeState::Normal)
        .patch(cx.style(ThemeRole::Accent, ThemeState::Normal));
    render_progress_bar(
        buf,
        area.x,
        area.y,
        area.width,
        value,
        style.fg.unwrap_or(Color::Reset),
        style.bg.unwrap_or(Color::Reset),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_bar() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        render_progress_bar(&mut buf, 0, 0, 5, 0.0, Color::White, Color::Black);
        for i in 0..5u16 {
            assert_eq!(buf[(i, 0)].symbol(), " ");
        }
    }

    #[test]
    fn test_full_bar() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        render_progress_bar(&mut buf, 0, 0, 5, 1.0, Color::White, Color::Black);
        for i in 0..5u16 {
            assert_eq!(buf[(i, 0)].symbol(), "█");
        }
    }

    #[test]
    fn test_half_bar() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        render_progress_bar(&mut buf, 0, 0, 4, 0.5, Color::White, Color::Black);
        assert_eq!(buf[(0, 0)].symbol(), "█");
        assert_eq!(buf[(1, 0)].symbol(), "█");
        assert_eq!(buf[(2, 0)].symbol(), " ");
        assert_eq!(buf[(3, 0)].symbol(), " ");
    }

    #[test]
    fn test_partial_block() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        render_progress_bar(&mut buf, 0, 0, 4, 0.125, Color::White, Color::Black);
        assert_eq!(buf[(0, 0)].symbol(), "▌");
        assert_eq!(buf[(1, 0)].symbol(), " ");
    }

    #[test]
    fn cell_breakdown_keeps_eighths_resolution() {
        assert_eq!(cell_breakdown(4, 0.5), (2, 0));
        assert_eq!(cell_breakdown(4, 0.125), (0, 4));
        assert_eq!(cell_breakdown(5, 0.03), (0, 1));
        assert_eq!(cell_breakdown(4, 2.0), (4, 0));
    }

    #[test]
    fn shades_and_blocks_tables_match_in_length() {
        assert_eq!(BLOCKS.len(), SHADES.len());
        assert_eq!(BLOCKS[0], SHADES[0]);
        assert_eq!(BLOCKS[8], SHADES[8]);
    }
}
