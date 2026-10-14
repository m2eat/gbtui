//! Local fill / accent-rail helpers. Widgets crate `paint` is private.

use gbtui::TickDemand;
use gbtui::core::SafeBuf;
use gbtui::runtime::FrameCtx;
use gbtui::terminal::glyphs::accent_bar;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

pub fn fill(buf: &mut Buffer, area: Rect, style: Style) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(style);
                cell.set_symbol(" ");
            }
        }
    }
}

/// Left accent column (`┃`), grok scrollback rail. `animated` waves while a
/// tool is running (Fast tick). Colors stay on the Theme whitelist.
pub fn paint_accent(
    area: Rect,
    color: Color,
    animated: bool,
    bg: Color,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let bar = accent_bar();
    if animated {
        cx.request_tick(TickDemand::Fast);
    }
    for row in 0..area.height {
        let y = area.y + row;
        let fg = if animated && cx.anim().wave_brightness(row) < 0.45 {
            cx.theme.gray_dim
        } else {
            color
        };
        buf.set_string_safe(area.x, y, bar, Style::default().fg(fg).bg(bg));
    }
}
