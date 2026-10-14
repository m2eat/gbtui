//! Keyboard cheatsheet body: caller-supplied headers and hints.
//!
//! Structure learned from `views/shortcuts_help.rs`. No grok `ActionId` /
//! `Category` enums — the application groups [`HintItem`]s itself (typically
//! from `ActionRegistry::all()`).
//!
//! ```text
//!  /filter
//!  ── Home ──
//!  ▸ o    overlay     Toggle the overlay pane
//!    ?    help        Show this cheatsheet
//!  ── Always ──
//!    q    quit        Quit (press twice)
//! ```
//!
//! Drawn with a search row plus [`crate::list`] rows. Chrome is a [`crate::Modal`].
//!
//! # Keys
//!
//! Printable characters filter · `↑`/`↓`/`j`/`k` move · `Backspace` deletes
//! the query. Esc/close stay on the Modal.

use crate::line_editor::{LineEditOutcome, LineEditor};
use crate::list::{self, ListPaneState};
use crate::paint::fill;
use crate::shortcuts::HintItem;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gbtui_core::{HitId, SafeBuf, ThemeRole, ThemeState, key};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use unicode_width::UnicodeWidthStr;

pub const NS_CHEAT: u32 = 0x4353;

/// One cheatsheet row. Headers are section dividers.
#[derive(Debug, Clone)]
pub enum CheatsheetEntry {
    Header(&'static str),
    Hint {
        item: HintItem,
        long_help: Option<&'static str>,
    },
}

impl CheatsheetEntry {
    pub fn hint(item: HintItem) -> Self {
        Self::Hint {
            item,
            long_help: None,
        }
    }

    pub fn with_help(item: HintItem, long_help: &'static str) -> Self {
        Self::Hint {
            item,
            long_help: Some(long_help),
        }
    }

    fn matches(&self, q: &str) -> bool {
        if q.is_empty() {
            return true;
        }
        let q = q.to_ascii_lowercase();
        match self {
            Self::Header(label) => label.to_ascii_lowercase().contains(&q),
            Self::Hint { item, long_help } => {
                if item.label.to_ascii_lowercase().contains(&q) {
                    return true;
                }
                if item
                    .description
                    .as_ref()
                    .is_some_and(|d| d.to_ascii_lowercase().contains(&q))
                {
                    return true;
                }
                if item
                    .keys
                    .iter()
                    .any(|k| k.to_string().to_ascii_lowercase().contains(&q))
                {
                    return true;
                }
                if item
                    .custom_display
                    .is_some_and(|d| d.to_ascii_lowercase().contains(&q))
                {
                    return true;
                }
                long_help.is_some_and(|h| h.to_ascii_lowercase().contains(&q))
            }
        }
    }
}

/// Query editor plus list navigation.
#[derive(Debug, Clone, Default)]
pub struct CheatsheetState {
    pub list: ListPaneState,
    pub editor: LineEditor,
}

impl CheatsheetState {
    pub fn query(&self) -> &str {
        self.editor.text()
    }

    pub fn visual_len(&self, entries: &[CheatsheetEntry]) -> usize {
        flatten(entries, self.query()).len()
    }

    pub fn handle_key(&mut self, key: &KeyEvent, entries: &[CheatsheetEntry], page: usize) -> bool {
        let n = flatten(entries, self.query()).len();
        if key!('n', CONTROL).matches(key) {
            let mut k = *key;
            k.code = KeyCode::Down;
            k.modifiers = KeyModifiers::NONE;
            return self.list.handle_key(&k, n, page);
        }
        if key!('p', CONTROL).matches(key) {
            let mut k = *key;
            k.code = KeyCode::Up;
            k.modifiers = KeyModifiers::NONE;
            return self.list.handle_key(&k, n, page);
        }
        let empty = self.query().is_empty();
        if empty
            && let KeyCode::Char(c) = key.code
            && key.modifiers.is_empty()
            && matches!(c, 'j' | 'k' | 'g' | 'G')
        {
            return self.list.handle_key(key, n, page);
        }
        match self.editor.handle_key(key) {
            LineEditOutcome::Unhandled => self.list.handle_key(key, n, page),
            LineEditOutcome::HandledNoChange | LineEditOutcome::CursorChanged => true,
            LineEditOutcome::TextChanged => {
                self.list.selected = 0;
                self.list.offset = 0;
                true
            }
        }
    }
}

struct FlatRow {
    label: String,
    right: String,
    is_header: bool,
    source: usize,
}

fn flatten(entries: &[CheatsheetEntry], query: &str) -> Vec<FlatRow> {
    let q = query.trim();
    let mut out = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        match &entries[i] {
            CheatsheetEntry::Header(label) => {
                let start = i;
                i += 1;
                let mut kids = Vec::new();
                while i < entries.len() && !matches!(entries[i], CheatsheetEntry::Header(_)) {
                    if entries[i].matches(q) {
                        kids.push(i);
                    }
                    i += 1;
                }
                let header_hit = q.is_empty() || CheatsheetEntry::Header(label).matches(q);
                if kids.is_empty() && !header_hit {
                    continue;
                }
                if q.is_empty() || header_hit || !kids.is_empty() {
                    out.push(FlatRow {
                        label: format!("── {label} ──"),
                        right: String::new(),
                        is_header: true,
                        source: start,
                    });
                }
                for k in kids {
                    out.push(flat_hint(k, &entries[k]));
                }
            }
            CheatsheetEntry::Hint { .. } => {
                if entries[i].matches(q) {
                    out.push(flat_hint(i, &entries[i]));
                }
                i += 1;
            }
        }
    }
    out
}

fn flat_hint(source: usize, entry: &CheatsheetEntry) -> FlatRow {
    match entry {
        CheatsheetEntry::Header(label) => FlatRow {
            label: format!("── {label} ──"),
            right: String::new(),
            is_header: true,
            source,
        },
        CheatsheetEntry::Hint { item, .. } => {
            let keys = if let Some(d) = item.custom_display {
                d.to_string()
            } else {
                item.keys
                    .iter()
                    .map(|k| k.to_string())
                    .collect::<Vec<_>>()
                    .join(" / ")
            };
            let right = item
                .description
                .as_deref()
                .unwrap_or(item.label.as_ref())
                .to_string();
            FlatRow {
                label: format!("{keys}  {}", item.label),
                right,
                is_header: false,
                source,
            }
        }
    }
}

pub fn parse_row(id: HitId) -> Option<usize> {
    list::parse_row_id_ns(NS_CHEAT, id)
}

/// Source index of the selected visual row, if it is a hint.
pub fn selected_hint_index(state: &CheatsheetState, entries: &[CheatsheetEntry]) -> Option<usize> {
    let rows = flatten(entries, state.query());
    let row = rows.get(state.list.selected)?;
    if row.is_header {
        None
    } else {
        Some(row.source)
    }
}

/// Paint search + rows into `area`. Returns the list page height.
pub fn render_cheatsheet(
    state: &mut CheatsheetState,
    entries: &[CheatsheetEntry],
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> usize {
    if area.height == 0 || area.width == 0 {
        return 0;
    }
    fill(buf, area, cx.style(ThemeRole::Surface, ThemeState::Normal));
    let search = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    let q = format!("/{}", state.query());
    buf.set_string_safe(
        search.x,
        search.y,
        gbtui_core::line_utils::truncate_str(&q, search.width as usize),
        cx.style(ThemeRole::Surface, ThemeState::Normal)
            .patch(cx.style(ThemeRole::Accent, ThemeState::Focused))
            .add_modifier(Modifier::BOLD),
    );
    if search.width > 1 {
        let vp = state
            .editor
            .viewport(search.width.saturating_sub(1) as usize);
        let col = 1u16.saturating_add(vp.cursor_display_column as u16);
        cx.set_cursor(search.x + col.min(search.width.saturating_sub(1)), search.y);
    }

    let list_area = Rect {
        x: area.x,
        y: area.y.saturating_add(1),
        width: area.width,
        height: area.height.saturating_sub(1),
    };
    if list_area.height == 0 {
        return 0;
    }
    let rows = flatten(entries, state.query());
    let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
    let page = list::render_list_ns(&mut state.list, &labels, NS_CHEAT, list_area, buf, cx);
    // Overlay description on the right of hint rows.
    let vis = state.list.visible_indices(labels.iter().copied());
    let start = state.list.offset;
    let end = (start + list_area.height as usize).min(vis.len());
    for (row, vis_i) in vis.get(start..end).unwrap_or(&[]).iter().enumerate() {
        let Some(flat) = rows.get(*vis_i) else {
            continue;
        };
        if flat.right.is_empty() {
            continue;
        }
        let y = list_area.y + row as u16;
        let rw = flat.right.width() as u16;
        if rw + 1 >= list_area.width {
            continue;
        }
        let dx = list_area.right().saturating_sub(rw + 1);
        let row = Rect::new(list_area.x, y, list_area.width, 1);
        let row_state = ThemeState::from_flags(
            false,
            false,
            vis.get(state.list.selected) == Some(vis_i),
            false,
            cx.hovered(row),
        );
        let row_role = match row_state {
            ThemeState::Selected => ThemeRole::Selection,
            ThemeState::Hovered => ThemeRole::Hover,
            _ => ThemeRole::Muted,
        };
        buf.set_string_safe(
            dx,
            y,
            &flat.right,
            cx.style(ThemeRole::Surface, ThemeState::Normal)
                .patch(cx.style(row_role, row_state)),
        );
    }
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, KeyShortcut, Theme};
    use gbtui_runtime::HitMap;
    use std::time::Instant;

    fn sample() -> Vec<CheatsheetEntry> {
        vec![
            CheatsheetEntry::Header("Home"),
            CheatsheetEntry::with_help(
                HintItem::new(
                    KeyShortcut::from(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)),
                    "overlay",
                ),
                "Toggle the overlay pane",
            ),
            CheatsheetEntry::Header("Always"),
            CheatsheetEntry::hint(HintItem::new(
                KeyShortcut::from(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
                "quit",
            )),
        ]
    }

    #[test]
    fn filter_keeps_matching_section() {
        let entries = sample();
        let rows = flatten(&entries, "quit");
        assert!(
            rows.iter().any(|r| r.label.contains("quit")),
            "{:?}",
            rows.iter().map(|r| &r.label).collect::<Vec<_>>()
        );
        assert!(
            rows.iter()
                .any(|r| r.is_header && r.label.contains("Always")),
            "header of the matching section must remain"
        );
        assert!(
            !rows.iter().any(|r| r.label.contains("overlay")),
            "non-matching hints must drop"
        );
    }

    #[test]
    fn render_registers_rows() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 10));
        let mut state = CheatsheetState::default();
        let entries = sample();
        render_cheatsheet(
            &mut state,
            &entries,
            Rect::new(0, 0, 60, 10),
            &mut buf,
            &mut cx,
        );
        assert!(matches!(hits.hit(2, 2), gbtui_runtime::HitResult::Hit(_)));
    }
}
