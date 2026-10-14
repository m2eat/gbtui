//! Horizontal tab strip. Hover / selected styles go through `FrameCtx`.
//!
//! Extracted from Modal `paint_tabs` (P9c). Hit namespace stays [`NS_TAB`]
//! so Modal behavior matches P4a.
//!
//! ```text
//!  List  Picker  Choices  Prompt
//!  ^^^^ selected (accent + highlight)
//! ```
//!
//! # Keys
//!
//! `Tab` / `→` next · `Shift+Tab` / `←` previous (wraps).

use crossterm::event::{KeyCode, KeyEvent};
use gbtui_core::{HitId, SafeBuf, ThemeRole, ThemeState, is_shift_tab, key};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

/// Same namespace Modal used in P4a.
pub const NS_TAB: u32 = 0x4D01;

pub fn tab_id(i: usize) -> HitId {
    HitId::namespaced(NS_TAB, i as u32)
}

pub fn parse_tab(id: HitId) -> Option<usize> {
    (id.namespace() == NS_TAB).then_some(id.index() as usize)
}

/// Borrowed labels plus the active index.
pub struct Tabs<'a> {
    labels: &'a [&'a str],
    active: usize,
}

impl<'a> Tabs<'a> {
    pub fn new(labels: &'a [&'a str]) -> Self {
        Self { labels, active: 0 }
    }

    pub fn active(mut self, active: usize) -> Self {
        self.active = active;
        self
    }

    pub fn cycle_next(active: usize, n: usize) -> usize {
        if n == 0 { 0 } else { (active + 1) % n }
    }

    pub fn cycle_prev(active: usize, n: usize) -> usize {
        if n == 0 {
            0
        } else if active > 0 {
            active - 1
        } else {
            n - 1
        }
    }

    /// `Some(new_index)` when the key cycles tabs.
    pub fn handle_key(key: &KeyEvent, active: usize, n: usize) -> Option<usize> {
        if n == 0 {
            return None;
        }
        if key!(Tab).matches(key) || key.code == KeyCode::Right {
            return Some(Self::cycle_next(active, n));
        }
        if is_shift_tab(key) || key.code == KeyCode::Left {
            return Some(Self::cycle_prev(active, n));
        }
        None
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) {
        if area.height == 0 || area.width == 0 || self.labels.is_empty() {
            return;
        }
        let mut x = area.x;
        for (i, label) in self.labels.iter().enumerate() {
            let text = format!(" {label} ");
            let w = text.width() as u16;
            if x.saturating_add(w) > area.right() {
                break;
            }
            let r = Rect {
                x,
                y: area.y,
                width: w,
                height: 1,
            };
            cx.register(tab_id(i), r);
            let state =
                ThemeState::from_flags(false, false, i == self.active, false, cx.hovered(r));
            let role = match state {
                ThemeState::Selected => ThemeRole::Selection,
                ThemeState::Hovered => ThemeRole::Hover,
                _ => ThemeRole::Muted,
            };
            let style = cx
                .style(ThemeRole::Surface, ThemeState::Normal)
                .patch(cx.style(role, state));
            buf.set_string_safe(x, area.y, &text, style);
            x = x.saturating_add(w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::HitMap;
    use ratatui::style::Color;
    use std::time::Instant;

    #[test]
    fn tab_and_right_cycle() {
        let tab = KeyEvent::new(KeyCode::Tab, crossterm::event::KeyModifiers::NONE);
        let right = KeyEvent::new(KeyCode::Right, crossterm::event::KeyModifiers::NONE);
        assert_eq!(Tabs::handle_key(&tab, 0, 2), Some(1));
        assert_eq!(Tabs::handle_key(&right, 1, 2), Some(0));
    }

    #[test]
    fn selected_uses_semantic_selection_style() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        Tabs::new(&["List", "Picker"])
            .active(1)
            .render(Rect::new(0, 0, 40, 1), &mut buf, &mut cx);
        // " List " is 6 cols; " Picker " starts at x=6.
        let expected = theme
            .style(ThemeRole::Surface, ThemeState::Normal)
            .patch(theme.style(ThemeRole::Selection, ThemeState::Selected));
        assert_eq!(buf[(7, 0)].fg, expected.fg.unwrap_or(Color::Reset));
        assert_eq!(buf[(7, 0)].bg, expected.bg.unwrap_or(Color::Reset));
        assert_eq!(hits.hit(7, 0), gbtui_runtime::HitResult::Hit(tab_id(1)));
        assert_eq!(hits.hit(1, 0), gbtui_runtime::HitResult::Hit(tab_id(0)));
    }

    #[test]
    fn hover_uses_semantic_hover_style_on_inactive() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, Some((1, 0)), &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        Tabs::new(&["List", "Picker"])
            .active(1)
            .render(Rect::new(0, 0, 40, 1), &mut buf, &mut cx);
        let expected = theme
            .style(ThemeRole::Surface, ThemeState::Normal)
            .patch(theme.style(ThemeRole::Hover, ThemeState::Hovered));
        assert_eq!(buf[(1, 0)].fg, expected.fg.unwrap_or(Color::Reset));
        assert_eq!(buf[(1, 0)].bg, expected.bg.unwrap_or(Color::Reset));
    }
}
