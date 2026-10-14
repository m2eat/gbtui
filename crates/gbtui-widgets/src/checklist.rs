//! Status-icon checklist on top of the list pane.
//!
//! Learned from `views/todo_pane.rs` (REWRITE). No grok `TodoItem`.
//!
//! ```text
//!  □ write docs
//!  ▶ implement form
//!  ✓ ship P10
//! ```
//!
//! # Keys
//!
//! List navigation plus `Space` to cycle [`CheckStatus`].

use crate::list::{self, ListPaneState};
use crate::paint::fill;
use crossterm::event::{KeyCode, KeyEvent};
use gbtui_core::{HitId, SafeBuf, Theme, ThemeRole, ThemeState, line_utils};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;

pub const NS_CHECK: u32 = 0x434B;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckStatus {
    #[default]
    Pending,
    Active,
    Done,
    Cancelled,
}

impl CheckStatus {
    pub fn cycle(self) -> Self {
        match self {
            Self::Pending => Self::Active,
            Self::Active => Self::Done,
            Self::Done => Self::Cancelled,
            Self::Cancelled => Self::Pending,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Pending => "□",
            Self::Active => "▶",
            Self::Done => "✓",
            Self::Cancelled => "✗",
        }
    }

    fn role(self) -> ThemeRole {
        match self {
            Self::Pending => ThemeRole::Text,
            Self::Active => ThemeRole::Warning,
            Self::Done => ThemeRole::Success,
            Self::Cancelled => ThemeRole::Error,
        }
    }

    pub fn color(self, theme: &Theme) -> Color {
        theme
            .style(self.role(), ThemeState::Normal)
            .fg
            .unwrap_or(Color::Reset)
    }
}

#[derive(Debug, Clone)]
pub struct CheckItem {
    pub label: String,
    pub status: CheckStatus,
}

impl CheckItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            status: CheckStatus::Pending,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckAction {
    Unhandled,
    Changed,
    Cycled(usize),
}

pub fn handle_checklist_key(
    list: &mut ListPaneState,
    key: &KeyEvent,
    n: usize,
    page: usize,
) -> CheckAction {
    if n == 0 {
        return CheckAction::Unhandled;
    }
    if key.code == KeyCode::Char(' ') {
        return CheckAction::Cycled(list.selected.min(n - 1));
    }
    if list.handle_key(key, n, page) {
        CheckAction::Changed
    } else {
        CheckAction::Unhandled
    }
}

pub fn parse_row(id: HitId) -> Option<usize> {
    list::parse_row_id_ns(NS_CHECK, id)
}

pub fn render_checklist(
    state: &mut ListPaneState,
    items: &[CheckItem],
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> usize {
    if area.height == 0 || area.width == 0 {
        return 0;
    }
    let n = items.len();
    let page = area.height as usize;
    state.clamp(n, page);
    fill(buf, area, cx.style(ThemeRole::Surface, ThemeState::Normal));
    let start = state.offset;
    let end = (start + page).min(n);
    for (row, i) in (start..end).enumerate() {
        let y = area.y + row as u16;
        let r = Rect {
            x: area.x,
            y,
            width: area.width,
            height: 1,
        };
        cx.register(list::row_id_ns(NS_CHECK, i), r);
        let selected = i == state.selected;
        let hover = cx.hovered(r);
        let state = ThemeState::from_flags(false, false, selected, false, hover);
        let row_role = match state {
            ThemeState::Selected => ThemeRole::Selection,
            ThemeState::Hovered => ThemeRole::Hover,
            _ => ThemeRole::Text,
        };
        let item = &items[i];
        let icon_st = cx
            .style(ThemeRole::Surface, ThemeState::Normal)
            .patch(cx.style(row_role, state))
            .patch(cx.style(item.status.role(), ThemeState::Normal));
        let mut st = icon_st;
        if item.status == CheckStatus::Done {
            st = st.add_modifier(Modifier::DIM);
        }
        if item.status == CheckStatus::Cancelled {
            st = st.add_modifier(Modifier::CROSSED_OUT);
        }
        fill(buf, r, st);
        let icon = item.status.icon();
        buf.set_string_safe(r.x, y, icon, icon_st);
        buf.set_string_safe(
            r.x + 2,
            y,
            line_utils::truncate_str(&item.label, r.width.saturating_sub(2) as usize),
            st,
        );
    }
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn space_cycles_selected() {
        let mut list = ListPaneState::default();
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(
            handle_checklist_key(&mut list, &key, 3, 8),
            CheckAction::Cycled(0)
        );
    }

    #[test]
    fn cycle_order() {
        assert_eq!(CheckStatus::Pending.cycle(), CheckStatus::Active);
        assert_eq!(CheckStatus::Cancelled.cycle(), CheckStatus::Pending);
    }
}
