//! Shared buffer fills. Widgets must not pick colors here — callers pass Theme styles.

use gbtui_core::SafeBuf;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

pub fn fill(buf: &mut Buffer, area: Rect, style: Style) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            buf.set_string_safe(x, y, " ", style);
        }
    }
}
