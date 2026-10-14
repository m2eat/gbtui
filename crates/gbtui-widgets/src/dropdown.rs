//! Borrowed label/detail dropdown. Geometry from completion_dropdown /
//! overlay_list (STRIP); no slash, @-files, or workspace FS.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/views/{completion_dropdown,overlay_list}.rs (STRIP)`
//!
//! ```text
//!  ┌──────────────────────────┐
//!  │ ▸ Night           dark   │
//!  │   Day             light  │
//!  └──────────────────────────┘
//! ```
//!
//! Open: `occlude` the outer rect, then register `BODY` and each row so
//! row clicks sit on top of the mask.
//!
//! # Keys
//!
//! `↑`/`↓`/`j`/`k` move · `Enter` confirm · `Esc` close · wheel scrolls.

use std::ops::Range;

use crossterm::event::{KeyCode, KeyEvent, MouseButton};
use gbtui_core::{HitId, SafeBuf, ThemeRole, ThemeState, key};
use gbtui_runtime::{FrameCtx, Pointer};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::paint::fill;

pub const NS_DROP: u32 = 0x4452;
pub const BODY: HitId = HitId::named("gbtui.dropdown.body");
pub const SCROLLBAR: HitId = HitId::named("gbtui.dropdown.scrollbar");

/// Default visible rows before the list scrolls.
pub const MAX_VISIBLE: u16 = 8;

/// Keep the selected row merely visible or center it when selection changes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DropdownScrollPolicy {
    #[default]
    Nearest,
    Centered,
}

/// Borrowed row. `detail` is optional secondary text. Highlights are UTF-8 byte
/// ranges supplied by a source such as [`crate::SearchQuery`].
#[derive(Debug, Clone, Copy)]
pub struct DropdownItem<'a> {
    pub label: &'a str,
    pub detail: Option<&'a str>,
    pub highlights: &'a [Range<usize>],
}

impl<'a> DropdownItem<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            detail: None,
            highlights: &[],
        }
    }

    pub fn detail(mut self, detail: &'a str) -> Self {
        self.detail = Some(detail);
        self
    }

    /// Highlight grapheme-safe byte ranges in the label.
    pub fn highlights(mut self, ranges: &'a [Range<usize>]) -> Self {
        self.highlights = ranges;
        self
    }
}

/// Open / selected / scroll offset.
#[derive(Debug, Clone)]
pub struct DropdownState {
    pub open: bool,
    pub selected: usize,
    pub offset: usize,
    visible_rows: u16,
    scroll_policy: DropdownScrollPolicy,
    scrollbar_dragging: bool,
    manual_offset: bool,
}

impl Default for DropdownState {
    fn default() -> Self {
        Self {
            open: false,
            selected: 0,
            offset: 0,
            visible_rows: MAX_VISIBLE,
            scroll_policy: DropdownScrollPolicy::Nearest,
            scrollbar_dragging: false,
            manual_offset: false,
        }
    }
}

/// Result of [`DropdownState::handle_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropdownAction {
    Closed,
    Confirmed(usize),
    Changed,
    Unhandled,
}

impl DropdownState {
    /// Configure this dropdown to show at most `visible_rows` rows.
    pub fn with_visible_rows(mut self, visible_rows: u16) -> Self {
        self.set_visible_rows(visible_rows);
        self
    }

    /// Configure where keyboard selection is kept in an overflowing popup.
    pub fn with_scroll_policy(mut self, policy: DropdownScrollPolicy) -> Self {
        self.set_scroll_policy(policy);
        self
    }

    pub fn set_visible_rows(&mut self, visible_rows: u16) {
        self.visible_rows = visible_rows.max(1);
    }

    pub fn visible_rows(&self) -> u16 {
        self.visible_rows
    }

    pub fn set_scroll_policy(&mut self, policy: DropdownScrollPolicy) {
        self.scroll_policy = policy;
    }

    pub fn scroll_policy(&self) -> DropdownScrollPolicy {
        self.scroll_policy
    }

    pub fn open_at(&mut self, selected: usize) {
        self.open = true;
        self.selected = selected;
        self.offset = 0;
        self.scrollbar_dragging = false;
        self.manual_offset = false;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.scrollbar_dragging = false;
        self.manual_offset = false;
    }

    pub fn height(&self, n: usize) -> u16 {
        if !self.open || n == 0 {
            0
        } else {
            self.visible_height(n)
        }
    }

    /// Number of rows a popup needs for `n` source items, independent of open
    /// state so callers can calculate a pending popup.
    pub fn visible_height(&self, n: usize) -> u16 {
        u16::try_from(n).unwrap_or(u16::MAX).min(self.visible_rows)
    }

    fn clamp(&mut self, n: usize, page: usize) {
        if n == 0 {
            self.selected = 0;
            self.offset = 0;
            self.manual_offset = false;
            return;
        }
        self.selected = self.selected.min(n - 1);
        let page = page.max(1).min(n);
        let max_off = n.saturating_sub(page);
        self.offset = self.offset.min(max_off);
        if self.manual_offset {
            return;
        }
        match self.scroll_policy {
            DropdownScrollPolicy::Nearest => {
                if self.selected < self.offset {
                    self.offset = self.selected;
                } else if self.selected >= self.offset + page {
                    self.offset = self.selected + 1 - page;
                }
            }
            DropdownScrollPolicy::Centered => {
                self.offset = self.selected.saturating_sub(page / 2).min(max_off);
            }
        }
    }

    pub fn handle_key(&mut self, key: &KeyEvent, n: usize) -> DropdownAction {
        if !self.open {
            return DropdownAction::Unhandled;
        }
        if key!(Esc).matches(key) {
            self.close();
            return DropdownAction::Closed;
        }
        if key!(Enter).matches(key) {
            if n == 0 {
                self.close();
                return DropdownAction::Closed;
            }
            let i = self.selected.min(n - 1);
            self.close();
            return DropdownAction::Confirmed(i);
        }
        let page = self.visible_rows as usize;
        if key!(Down).matches(key) || key!('j').matches(key) {
            if n > 0 {
                self.selected = (self.selected + 1).min(n - 1);
                self.manual_offset = false;
                self.clamp(n, page);
            }
            return DropdownAction::Changed;
        }
        if key!(Up).matches(key) || key!('k').matches(key) {
            self.selected = self.selected.saturating_sub(1);
            self.manual_offset = false;
            self.clamp(n, page);
            return DropdownAction::Changed;
        }
        if key.code == KeyCode::PageDown {
            if n > 0 {
                self.selected = (self.selected + page).min(n - 1);
                self.manual_offset = false;
                self.clamp(n, page);
            }
            return DropdownAction::Changed;
        }
        if key.code == KeyCode::PageUp {
            self.selected = self.selected.saturating_sub(page);
            self.manual_offset = false;
            self.clamp(n, page);
            return DropdownAction::Changed;
        }
        DropdownAction::Unhandled
    }

    pub fn handle_scroll(&mut self, delta_lines: i32, n: usize) {
        if !self.open || n == 0 || delta_lines == 0 {
            return;
        }
        if delta_lines > 0 {
            self.selected = (self.selected + delta_lines as usize).min(n - 1);
        } else {
            self.selected = self
                .selected
                .saturating_sub(delta_lines.unsigned_abs() as usize);
        }
        self.manual_offset = false;
        self.clamp(n, self.visible_rows as usize);
    }

    /// Consume pointer capture for the overflow scrollbar. The `area` is the
    /// exact popup area returned by [`popup_area`].
    pub fn handle_scrollbar_pointer(&mut self, pointer: Pointer, area: Rect, n: usize) -> bool {
        if !self.open || n <= area.height as usize || area.height == 0 {
            return false;
        }
        match pointer {
            Pointer::Down {
                button: MouseButton::Left,
                row,
                ..
            } => {
                self.scrollbar_dragging = true;
                self.set_offset_from_scrollbar(row, area, n);
                true
            }
            Pointer::Drag {
                button: MouseButton::Left,
                row,
                ..
            } if self.scrollbar_dragging => {
                self.set_offset_from_scrollbar(row, area, n);
                true
            }
            Pointer::Up {
                button: MouseButton::Left,
                row,
                ..
            } if self.scrollbar_dragging => {
                self.set_offset_from_scrollbar(row, area, n);
                self.scrollbar_dragging = false;
                true
            }
            Pointer::Cancel { .. } if self.scrollbar_dragging => {
                self.scrollbar_dragging = false;
                true
            }
            _ => false,
        }
    }

    fn set_offset_from_scrollbar(&mut self, row: u16, area: Rect, n: usize) {
        let page = area.height as usize;
        let max_offset = n.saturating_sub(page);
        let thumb = scrollbar_thumb(area, n, self.offset);
        let travel = area.height.saturating_sub(thumb.height) as usize;
        let row = row
            .saturating_sub(area.y)
            .min(area.height.saturating_sub(1)) as usize;
        let thumb_top = row.saturating_sub((thumb.height / 2) as usize).min(travel);
        self.offset = if travel == 0 {
            0
        } else {
            (thumb_top * max_offset + travel / 2) / travel
        };
        self.manual_offset = true;
    }
}

pub fn row_id(i: usize) -> HitId {
    HitId::namespaced(NS_DROP, i as u32)
}

pub fn parse_row(id: HitId) -> Option<usize> {
    (id.namespace() == NS_DROP).then_some(id.index() as usize)
}

pub fn is_scrollbar(id: HitId) -> bool {
    id == SCROLLBAR
}

/// Place the popup under `anchor` when there is room, otherwise where more rows
/// fit above it. The geometry uses the state's configured row cap.
pub fn popup_area(anchor: Rect, frame: Rect, n: usize, state: &DropdownState) -> Rect {
    if frame.width == 0 || frame.height == 0 {
        return Rect::default();
    }
    let desired_height = state.visible_height(n.max(1));
    let anchor_top = anchor.y.clamp(frame.y, frame.bottom());
    let anchor_bottom = anchor.bottom().clamp(frame.y, frame.bottom());
    let below = frame.bottom().saturating_sub(anchor_bottom);
    let above = anchor_top.saturating_sub(frame.y);
    let height = desired_height.min(below.max(above));
    let y = if below >= desired_height || below >= above {
        anchor_bottom.min(frame.bottom().saturating_sub(height))
    } else {
        anchor_top.saturating_sub(height).max(frame.y)
    };
    let width = anchor.width.max(16).min(frame.width);
    Rect::new(
        anchor.x.clamp(frame.x, frame.right().saturating_sub(width)),
        y,
        width,
        height,
    )
}

/// Paint an open dropdown into `area`. No-op when closed.
pub fn render_dropdown<'a>(
    state: &mut DropdownState,
    items: impl ExactSizeIterator<Item = DropdownItem<'a>>,
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    let n = items.len();
    if !state.open || n == 0 || area.height == 0 {
        return;
    }
    if area.width < 4 {
        // A narrow resize cannot draw useful columns, but the popup must still
        // occlude its plane rather than exposing controls underneath it.
        cx.occlude(area);
        cx.register(BODY, area);
        cx.register_scroll(BODY, area);
        return;
    }
    let page = area.height.min(state.visible_height(n)) as usize;
    state.clamp(n, page);

    cx.occlude(area);
    cx.register(BODY, area);
    fill(buf, area, cx.style(ThemeRole::Elevated, ThemeState::Normal));

    let has_scrollbar = n > page;
    let row_width = area.width.saturating_sub(u16::from(has_scrollbar));
    let start = state.offset;
    for (row, item) in items.skip(start).take(page).enumerate() {
        let i = start + row;
        let y = area.y + row as u16;
        let r = Rect {
            x: area.x,
            y,
            width: row_width,
            height: 1,
        };
        cx.register(row_id(i), r);
        let selected = i == state.selected;
        let row_state = ThemeState::from_flags(false, false, selected, false, cx.hovered(r));
        let row_role = match row_state {
            ThemeState::Selected => ThemeRole::Selection,
            ThemeState::Hovered => ThemeRole::Hover,
            _ => ThemeRole::Text,
        };
        let mut st = cx
            .style(ThemeRole::Elevated, ThemeState::Normal)
            .patch(cx.style(row_role, row_state));
        if selected {
            st = st.add_modifier(Modifier::BOLD);
        }
        fill(buf, r, st);
        let prefix = if selected { "▸ " } else { "  " };
        buf.set_string_safe(r.x, y, prefix, st);
        let content_x = r.x.saturating_add(prefix.width() as u16);
        let content_width = r.right().saturating_sub(content_x);
        let (label_width, detail_x, detail_width) =
            dropdown_columns(content_x, content_width, item.detail);
        render_label(
            buf,
            Rect::new(content_x, y, label_width, 1),
            item.label,
            item.highlights,
            st,
            st.patch(cx.style(ThemeRole::Fuzzy, ThemeState::Normal))
                .add_modifier(Modifier::BOLD),
        );
        if let (Some(detail), Some(detail_x)) = (item.detail, detail_x) {
            render_plain(
                buf,
                detail_x,
                y,
                detail,
                detail_width,
                st.patch(cx.style(ThemeRole::Muted, ThemeState::Normal)),
            );
        }
    }
    if has_scrollbar {
        let scrollbar = Rect::new(area.right().saturating_sub(1), area.y, 1, area.height);
        paint_scrollbar(scrollbar, n, state.offset, buf, cx);
        // Register after rows so an overflow scrollbar owns down/drag capture.
        cx.register(SCROLLBAR, scrollbar);
    }
    // Keep wheel streams on the popup even while rows and the scrollbar recycle.
    cx.register_scroll(BODY, area);
}

fn dropdown_columns(
    content_x: u16,
    content_width: u16,
    detail: Option<&str>,
) -> (u16, Option<u16>, u16) {
    if detail.is_none() || content_width < 5 {
        return (content_width, None, 0);
    }
    let label_width = (content_width / 2).max(2);
    let detail_x = content_x.saturating_add(label_width).saturating_add(1);
    let detail_width = content_width.saturating_sub(label_width.saturating_add(1));
    (label_width, Some(detail_x), detail_width)
}

fn render_label(
    buf: &mut Buffer,
    area: Rect,
    text: &str,
    ranges: &[Range<usize>],
    style: Style,
    highlight_style: Style,
) {
    let mut used = 0;
    let mut range_index = 0;
    for (start, grapheme) in text.grapheme_indices(true) {
        let grapheme_width = grapheme.width() as u16;
        if grapheme_width > area.width.saturating_sub(used) {
            break;
        }
        let end = start + grapheme.len();
        while ranges
            .get(range_index)
            .is_some_and(|range| range.end <= start)
        {
            range_index += 1;
        }
        let matched = ranges
            .get(range_index)
            .is_some_and(|range| range.start < end);
        buf.set_string_safe(
            area.x.saturating_add(used),
            area.y,
            grapheme,
            if matched { highlight_style } else { style },
        );
        used = used.saturating_add(grapheme_width);
    }
}

fn render_plain(buf: &mut Buffer, x: u16, y: u16, text: &str, width: u16, style: Style) {
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let grapheme_width = grapheme.width() as u16;
        if grapheme_width > width.saturating_sub(used) {
            break;
        }
        buf.set_string_safe(x.saturating_add(used), y, grapheme, style);
        used = used.saturating_add(grapheme_width);
    }
}

fn scrollbar_thumb(area: Rect, n: usize, offset: usize) -> Rect {
    let page = area.height as usize;
    let thumb_height = (page.saturating_mul(page).div_ceil(n)).max(1).min(page) as u16;
    let travel = area.height.saturating_sub(thumb_height) as usize;
    let max_offset = n.saturating_sub(page);
    let top = if max_offset == 0 {
        0
    } else {
        offset.min(max_offset) * travel / max_offset
    } as u16;
    Rect::new(area.x, area.y.saturating_add(top), area.width, thumb_height)
}

fn paint_scrollbar(area: Rect, n: usize, offset: usize, buf: &mut Buffer, cx: &FrameCtx<'_>) {
    let style = cx.style(ThemeRole::Scrollbar, ThemeState::Normal);
    fill(buf, area, style);
    for y in area.y..area.bottom() {
        buf.set_string_safe(area.x, y, "│", style);
    }
    let thumb = scrollbar_thumb(area, n, offset);
    for y in thumb.y..thumb.bottom() {
        buf.set_string_safe(thumb.x, y, "█", style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::{HitMap, HitResult};
    use std::time::Instant;

    fn items() -> [DropdownItem<'static>; 3] {
        [
            DropdownItem::new("Night").detail("dark"),
            DropdownItem::new("Day").detail("light"),
            DropdownItem::new("Auto").detail("system"),
        ]
    }

    #[test]
    fn enter_confirms_and_closes() {
        let mut s = DropdownState {
            open: true,
            selected: 1,
            ..Default::default()
        };
        let key = KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
        assert_eq!(s.handle_key(&key, 3), DropdownAction::Confirmed(1));
        assert!(!s.open);
    }

    #[test]
    fn occlude_then_rows_still_hit() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
        let mut state = DropdownState {
            open: true,
            ..Default::default()
        };
        let area = Rect::new(2, 2, 20, 3);
        render_dropdown(&mut state, items().into_iter(), area, &mut buf, &mut cx);
        assert_eq!(hits.hit(3, 2), HitResult::Hit(row_id(0)));
        assert_eq!(hits.hit(3, 3), HitResult::Hit(row_id(1)));
        // Inside the popup but after the last row: BODY (registered before rows,
        // so a later row would win; empty cells still hit BODY via later-on-top
        // only if BODY is the topmost — BODY is under rows. Point on row 2:
        assert_eq!(hits.hit(3, 4), HitResult::Hit(row_id(2)));
        // Outside the popup.
        assert_eq!(hits.hit(0, 0), HitResult::Miss);
    }

    #[test]
    fn jk_move_selection() {
        let mut s = DropdownState {
            open: true,
            ..Default::default()
        };
        let j = KeyEvent::new(KeyCode::Char('j'), crossterm::event::KeyModifiers::NONE);
        let k = KeyEvent::new(KeyCode::Char('k'), crossterm::event::KeyModifiers::NONE);
        assert_eq!(s.handle_key(&j, 3), DropdownAction::Changed);
        assert_eq!(s.selected, 1);
        assert_eq!(s.handle_key(&k, 3), DropdownAction::Changed);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn configured_centering_uses_the_visible_row_cap() {
        let mut state = DropdownState::default()
            .with_visible_rows(3)
            .with_scroll_policy(DropdownScrollPolicy::Centered);
        state.open_at(0);
        let down = KeyEvent::new(KeyCode::Down, crossterm::event::KeyModifiers::NONE);
        for _ in 0..6 {
            state.handle_key(&down, 12);
        }
        assert_eq!(state.height(12), 3);
        assert_eq!(state.offset, 5);
        assert_eq!(
            popup_area(Rect::new(4, 2, 12, 1), Rect::new(0, 0, 30, 10), 12, &state),
            Rect::new(4, 3, 16, 3)
        );
    }

    #[test]
    fn stale_anchor_stays_inside_a_nonzero_frame() {
        let state = DropdownState::default().with_visible_rows(3);
        let frame = Rect::new(40, 10, 20, 5);
        assert_eq!(
            popup_area(Rect::new(0, 0, 18, 1), frame, 12, &state),
            Rect::new(40, 10, 18, 3)
        );
        assert_eq!(
            popup_area(Rect::new(200, 200, 18, 1), frame, 12, &state),
            Rect::new(42, 12, 18, 3)
        );
    }

    #[test]
    fn overflow_scrollbar_owns_drag_and_preserves_bounded_columns() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 16, 4));
        let mut state = DropdownState::default().with_visible_rows(4);
        state.open_at(0);
        let items: Vec<_> = (0..12)
            .map(|_| DropdownItem::new("long label that must not hide detail").detail("detail"))
            .collect();
        let area = Rect::new(0, 0, 16, 4);
        render_dropdown(&mut state, items.iter().copied(), area, &mut buf, &mut cx);

        assert_eq!(hits.hit(15, 0), HitResult::Hit(SCROLLBAR));
        assert_eq!(hits.scroll_hit(15, 0), HitResult::Hit(BODY));
        assert_eq!(buf[(9, 0)].symbol(), "d");
        assert!(state.handle_scrollbar_pointer(
            Pointer::Down {
                button: MouseButton::Left,
                col: 15,
                row: 3,
                clicks: 1,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            area,
            items.len(),
        ));
        assert!(state.offset > 0);
        assert!(state.handle_scrollbar_pointer(
            Pointer::Drag {
                button: MouseButton::Left,
                col: 15,
                row: 0,
            },
            area,
            items.len(),
        ));
        assert_eq!(state.offset, 0);
        assert!(state.handle_scrollbar_pointer(
            Pointer::Cancel {
                button: MouseButton::Left,
                col: 15,
                row: 0,
            },
            area,
            items.len(),
        ));
    }
}
