//! Permission card. Layout learned from grok `permission_view.rs` (REWRITE).
//! No ACP options, bash pattern editor, or MCP scope.
//!
//! ```text
//! ┃ Run cargo test --workspace
//!   working tree is dirty
//!
//!   ▸ Allow
//!     Deny
//!     Allow for session
//! ```
//!
//! # Keys
//!
//! `↑`/`↓`/`j`/`k`/`Tab` move · `Enter`/`Space` activate · `Esc` Deny.
//!
//! Popup occludes the frame and docks above the bottom; Embedded fills the
//! given slot (grok paints the card in the prompt stack).

use crossterm::event::KeyEvent;
use gbtui::ModalKind;
use gbtui::core::{HitId, SafeBuf, wrapping::wrap_text};
use gbtui::key;
use gbtui::runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::paint::{fill, paint_accent};

pub const PERM_ALLOW: HitId = HitId::named("gbtui.agent.perm.allow");
pub const PERM_DENY: HitId = HitId::named("gbtui.agent.perm.deny");
pub const PERM_BODY: HitId = HitId::named("gbtui.agent.perm.body");
pub const NS_PERM_EXTRA: u32 = 0xA014;

pub const fn extra_id(index: u32) -> HitId {
    HitId::namespaced(NS_PERM_EXTRA, index)
}

pub fn parse_extra_id(id: HitId) -> Option<u32> {
    (id.namespace() == NS_PERM_EXTRA).then_some(id.index())
}

/// Result of [`PermissionCard::handle_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermAction {
    Allow,
    Deny,
    Extra(HitId),
    Unhandled,
}

/// Which button is focused: 0 Allow, 1 Deny, 2+ extra (in slice order).
#[derive(Debug, Clone)]
pub struct PermissionState {
    pub kind: ModalKind,
    pub focus: usize,
    pub extra: Vec<(String, HitId)>,
}

impl Default for PermissionState {
    fn default() -> Self {
        Self {
            kind: ModalKind::Popup,
            focus: 0,
            extra: Vec::new(),
        }
    }
}

impl PermissionState {
    pub fn popup() -> Self {
        Self::default()
    }

    pub fn embedded() -> Self {
        Self {
            kind: ModalKind::Embedded,
            ..Self::default()
        }
    }

    pub fn button_count(&self) -> usize {
        2 + self.extra.len()
    }

    fn clamp_focus(&mut self) {
        let n = self.button_count();
        if n == 0 {
            self.focus = 0;
        } else {
            self.focus = self.focus.min(n - 1);
        }
    }
}

/// Borrowed card content. Extra buttons live on [`PermissionState`].
pub struct PermissionCard<'a> {
    pub title: &'a str,
    pub body: &'a str,
}

impl<'a> PermissionCard<'a> {
    pub fn new(title: &'a str, body: &'a str) -> Self {
        Self { title, body }
    }

    pub fn handle_key(&self, key: &KeyEvent, state: &mut PermissionState) -> PermAction {
        state.clamp_focus();
        if key!(Esc).matches(key) {
            return PermAction::Deny;
        }
        if key!(Enter).matches(key) || key!(' ').matches(key) {
            return activate(state.focus, state);
        }
        let next = key!(Right).matches(key)
            || key!(Down).matches(key)
            || key!(Tab).matches(key)
            || key!('l').matches(key)
            || key!('j').matches(key);
        let prev = key!(Left).matches(key)
            || key!(Up).matches(key)
            || key!('h').matches(key)
            || key!('k').matches(key)
            || gbtui::core::is_shift_tab(key);
        if next {
            let n = state.button_count();
            if n > 0 {
                state.focus = (state.focus + 1) % n;
            }
            return PermAction::Unhandled;
        }
        if prev {
            let n = state.button_count();
            if n > 0 {
                state.focus = (state.focus + n - 1) % n;
            }
            return PermAction::Unhandled;
        }
        PermAction::Unhandled
    }

    /// Rows needed at `width` (title + body wrap + options).
    pub fn desired_height(&self, width: u16, state: &PermissionState) -> u16 {
        let inner = width.saturating_sub(3).max(8) as usize;
        1u16.saturating_add(wrap_text(self.body, inner).len() as u16)
            .saturating_add(1)
            .saturating_add(state.button_count() as u16)
            .max(4)
    }

    pub fn hit_action(&self, id: HitId, state: &PermissionState) -> Option<PermAction> {
        if id == PERM_ALLOW {
            return Some(PermAction::Allow);
        }
        if id == PERM_DENY {
            return Some(PermAction::Deny);
        }
        if state.extra.iter().any(|(_, hid)| *hid == id) || parse_extra_id(id).is_some() {
            return Some(PermAction::Extra(id));
        }
        None
    }

    /// Paint the card. `frame` is the full terminal area for Popup centering.
    pub fn render(
        &self,
        frame: Rect,
        buf: &mut Buffer,
        cx: &mut FrameCtx<'_>,
        state: &mut PermissionState,
    ) -> Option<Rect> {
        if frame.width < 24 || frame.height < 6 {
            return None;
        }
        state.clamp_focus();
        let h = self.desired_height(frame.width, state).min(frame.height);
        let card = match state.kind {
            ModalKind::Popup => {
                let w = frame.width;
                Rect {
                    x: frame.x,
                    y: frame.bottom().saturating_sub(h),
                    width: w,
                    height: h,
                }
            }
            ModalKind::Embedded => Rect {
                x: frame.x,
                y: frame.y,
                width: frame.width,
                height: frame.height.min(h.max(frame.height)),
            },
        };

        if state.kind == ModalKind::Popup {
            cx.occlude(frame);
        } else {
            cx.occlude(card);
        }
        cx.register(PERM_BODY, card);

        Clear.render(card, buf);
        let bg = cx.theme.bg_elevated;
        fill(buf, card, Style::default().bg(bg).fg(cx.theme.text_primary));
        paint_accent(card, cx.theme.warning, false, bg, buf, cx);

        let inner = Rect {
            x: card.x.saturating_add(2),
            y: card.y,
            width: card.width.saturating_sub(2),
            height: card.height,
        };
        if inner.height == 0 || inner.width < 8 {
            return Some(card);
        }

        buf.set_string_safe(
            inner.x,
            inner.y,
            gbtui::core::line_utils::truncate_str(self.title, inner.width as usize),
            Style::default()
                .fg(cx.theme.text_primary)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        );

        let wrap_w = inner.width.max(1) as usize;
        let lines = wrap_text(self.body, wrap_w);
        let mut y = inner.y.saturating_add(1);
        for line in &lines {
            if y + 1 >= inner.bottom() {
                break;
            }
            buf.set_string_safe(inner.x, y, line, Style::default().fg(cx.theme.gray).bg(bg));
            y = y.saturating_add(1);
        }
        y = y.saturating_add(1);

        let mut opts: Vec<(&str, HitId)> = vec![("Allow", PERM_ALLOW), ("Deny", PERM_DENY)];
        for (label, id) in &state.extra {
            opts.push((label.as_str(), *id));
        }
        for (i, (label, id)) in opts.iter().enumerate() {
            if y >= inner.bottom() {
                break;
            }
            let r = Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            };
            paint_option(label, *id, state.focus == i, r, bg, buf, cx);
            y = y.saturating_add(1);
        }
        Some(card)
    }
}

fn activate(focus: usize, state: &PermissionState) -> PermAction {
    match focus {
        0 => PermAction::Allow,
        1 => PermAction::Deny,
        i => state
            .extra
            .get(i - 2)
            .map(|(_, id)| PermAction::Extra(*id))
            .unwrap_or(PermAction::Deny),
    }
}

fn paint_option(
    label: &str,
    id: HitId,
    focused: bool,
    area: Rect,
    panel_bg: ratatui::style::Color,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    cx.register(id, area);
    let hot = focused || cx.hovered(area);
    let bg = if hot { cx.theme.bg_visual } else { panel_bg };
    fill(buf, area, Style::default().bg(bg).fg(cx.theme.text_primary));
    let prefix = if focused { "▸ " } else { "  " };
    let st = if focused {
        Style::default()
            .fg(cx.theme.text_primary)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(cx.theme.text_secondary).bg(bg)
    };
    buf.set_string_safe(area.x, area.y, format!("{prefix}{label}"), st);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui::core::{AnimationClock, Theme};
    use gbtui::runtime::{HitMap, HitResult};
    use std::time::Instant;

    fn key_esc() -> KeyEvent {
        KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        )
    }

    #[test]
    fn esc_is_deny() {
        let card = PermissionCard::new("Run cargo test?", "dirty tree");
        let mut state = PermissionState::popup();
        assert_eq!(card.handle_key(&key_esc(), &mut state), PermAction::Deny);
    }

    #[test]
    fn allow_deny_hits_and_occlude() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        let behind = HitId::named("behind");
        cx.register(behind, Rect::new(0, 0, 80, 24));
        let card = PermissionCard::new("Run cargo test?", "working tree is dirty");
        let mut state = PermissionState::popup();
        let painted = card
            .render(Rect::new(0, 0, 80, 24), &mut buf, &mut cx, &mut state)
            .expect("card");
        assert_eq!(hits.hit(0, 0), HitResult::Occluded);
        assert_eq!(
            hits.hit(painted.x + 2, painted.y + 1),
            HitResult::Hit(PERM_BODY)
        );
        let mut saw_allow = false;
        let mut saw_deny = false;
        let mut saw_warning = false;
        for y in painted.y..painted.bottom() {
            for x in painted.x..painted.right() {
                let id = hits.hit(x, y);
                if id == HitResult::Hit(PERM_ALLOW) {
                    saw_allow = true;
                }
                if id == HitResult::Hit(PERM_DENY) {
                    saw_deny = true;
                }
                if buf[(x, y)].fg == theme.warning {
                    saw_warning = true;
                }
            }
        }
        assert!(saw_allow && saw_deny, "buttons must register hits");
        assert!(saw_warning, "border uses theme.warning");
    }
}
