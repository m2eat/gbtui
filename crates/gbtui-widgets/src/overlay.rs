//! Shared overlay pane visibility / focus / fullscreen state machine.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/views/overlay.rs (COPY)`
//!
//! ```text
//!   Hidden  --shortcut-->  Visible+Focused  --shortcut--> Hidden
//!                             |  Tab/Space
//!                             v
//!                       Visible+Unfocused
//! ```
//!
//! Suggested keys (the application may rebind): toggle shortcut · `Tab`
//! unfocus · `Space` unfocus-alt · `Esc`/`q` leave fullscreen then hide ·
//! `Ctrl-F` fullscreen. Domain names (`FocusScrollback` / `FocusPrompt`)
//! are [`OverlayAction::Unfocus`] / [`OverlayAction::UnfocusAlt`].

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What the caller should do after an overlay state change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAction {
    Ignored,
    Changed,
    /// Tab: unfocus, keep visible; caller moves focus to the main surface.
    Unfocus,
    /// Space: unfocus, keep visible; caller moves focus to an alternate surface.
    UnfocusAlt,
}

impl OverlayAction {
    pub fn consumed(self) -> bool {
        !matches!(self, Self::Ignored)
    }
}

/// Visibility / focus / fullscreen for a side pane. Not a widget tree.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverlayState {
    pub visible: bool,
    pub focused: bool,
    pub fullscreen: bool,
}

impl OverlayState {
    pub fn visible() -> Self {
        Self {
            visible: true,
            focused: false,
            fullscreen: false,
        }
    }

    pub fn hidden() -> Self {
        Self::default()
    }

    /// Hidden → show+focus; visible unfocused → focus; focused → hide.
    pub fn toggle(&mut self) -> OverlayAction {
        if !self.visible {
            self.visible = true;
            self.focused = true;
        } else if !self.focused {
            self.focused = true;
        } else {
            self.visible = false;
            self.fullscreen = false;
            self.focused = false;
        }
        OverlayAction::Changed
    }

    pub fn tab_out(&mut self) -> OverlayAction {
        self.fullscreen = false;
        self.focused = false;
        OverlayAction::Unfocus
    }

    pub fn escape(&mut self) -> OverlayAction {
        if self.fullscreen {
            self.fullscreen = false;
        } else {
            self.visible = false;
            self.focused = false;
        }
        OverlayAction::Changed
    }

    pub fn space(&mut self) -> OverlayAction {
        self.fullscreen = false;
        self.focused = false;
        OverlayAction::UnfocusAlt
    }

    pub fn toggle_fullscreen(&mut self) -> OverlayAction {
        self.fullscreen = !self.fullscreen;
        OverlayAction::Changed
    }

    pub fn hide(&mut self) -> OverlayAction {
        self.visible = false;
        self.fullscreen = false;
        self.focused = false;
        OverlayAction::Changed
    }

    pub fn show(&mut self) {
        self.visible = true;
    }
}

/// Ctrl-F toggles fullscreen even when an input bar is open.
pub fn handle_overlay_key(state: &mut OverlayState, key: &KeyEvent) -> Option<OverlayAction> {
    if key.code == KeyCode::Char('f') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(state.toggle_fullscreen());
    }
    None
}

/// Tab / Esc / q / Space when the overlay is focused and no input bar is open.
pub fn handle_overlay_nav_key(state: &mut OverlayState, key: &KeyEvent) -> Option<OverlayAction> {
    match key.code {
        KeyCode::Tab => Some(state.tab_out()),
        KeyCode::Esc => Some(state.escape()),
        KeyCode::Char('q') if key.modifiers == KeyModifiers::NONE => Some(state.escape()),
        KeyCode::Char(' ') => Some(state.space()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    #[test]
    fn toggle_three_states() {
        let mut s = OverlayState::hidden();
        s.toggle();
        assert!(s.visible && s.focused);
        s.tab_out();
        assert!(s.visible && !s.focused);
        s.toggle();
        assert!(s.focused);
        s.toggle();
        assert!(!s.visible);
    }

    #[test]
    fn escape_exits_fullscreen_first() {
        let mut s = OverlayState {
            visible: true,
            focused: true,
            fullscreen: true,
        };
        s.escape();
        assert!(s.visible && s.focused && !s.fullscreen);
        s.escape();
        assert!(!s.visible);
    }

    #[test]
    fn ctrl_f_toggles_fullscreen() {
        let mut s = OverlayState::visible();
        s.focused = true;
        let key = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL);
        assert_eq!(
            handle_overlay_key(&mut s, &key),
            Some(OverlayAction::Changed)
        );
        assert!(s.fullscreen);
    }
}
