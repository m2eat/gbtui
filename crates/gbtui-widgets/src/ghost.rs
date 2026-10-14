//! Ghost / autosuggest remainder after the cursor.
//!
//! State machine learned from `views/prompt_suggestion.rs` (REWRITE).
//! The host supplies the suggestion string; this type never fetches.
//!
//! ```text
//!  ❯ save the night theme
//!         ^^^^^^^^^^^^^^^ dim italic remainder
//! ```
//!
//! # Keys
//!
//! `→` accepts when the cursor is at end · empty-input `Esc` dismisses ·
//! `Tab` only if [`GhostText::accept_tab`] is set (off by default so Modal
//! tabs keep Tab).

use crossterm::event::{KeyCode, KeyEvent};
use gbtui_core::key;

/// Remainder of a suggestion after the current input prefix.
#[derive(Debug, Clone, Default)]
pub struct GhostText {
    full: String,
    dismissed: bool,
    /// When true, Tab accepts. Default false (Modal owns Tab).
    pub accept_tab: bool,
}

/// Result of [`GhostText::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhostAction {
    Accepted(String),
    Dismissed,
    Unhandled,
}

impl GhostText {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn accept_tab(mut self, on: bool) -> Self {
        self.accept_tab = on;
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.set(text);
        self
    }

    pub fn set(&mut self, text: impl Into<String>) {
        self.full = text.into();
        self.dismissed = false;
    }

    pub fn clear(&mut self) {
        self.full.clear();
        self.dismissed = false;
    }

    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    pub fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    /// Visible remainder when `text` is a proper prefix of the suggestion.
    pub fn ghost_for<'a>(&'a self, text: &str) -> Option<&'a str> {
        if self.dismissed || self.full.is_empty() {
            return None;
        }
        let rest = self.full.strip_prefix(text)?;
        if rest.is_empty() { None } else { Some(rest) }
    }

    pub fn accept(&mut self, text: &str) -> Option<String> {
        let rest = self.ghost_for(text)?.to_owned();
        self.clear();
        Some(rest)
    }

    /// `→` (cursor at end) · optional `Tab` · empty `Esc`.
    pub fn handle_key(&mut self, key: &KeyEvent, text: &str, cursor_at_end: bool) -> GhostAction {
        if self.ghost_for(text).is_none() {
            return GhostAction::Unhandled;
        }
        if cursor_at_end
            && (key.code == KeyCode::Right || (self.accept_tab && key!(Tab).matches(key)))
        {
            return match self.accept(text) {
                Some(rest) => GhostAction::Accepted(rest),
                None => GhostAction::Unhandled,
            };
        }
        if text.is_empty() && key!(Esc).matches(key) {
            self.dismiss();
            return GhostAction::Dismissed;
        }
        GhostAction::Unhandled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn empty_input_shows_full_suggestion() {
        let mut g = GhostText::new();
        g.set("run the tests");
        assert_eq!(g.ghost_for(""), Some("run the tests"));
    }

    #[test]
    fn matching_prefix_shrinks() {
        let mut g = GhostText::new();
        g.set("run the tests");
        assert_eq!(g.ghost_for("r"), Some("un the tests"));
        assert_eq!(g.ghost_for("run the"), Some(" tests"));
    }

    #[test]
    fn diverge_hides() {
        let mut g = GhostText::new();
        g.set("run the tests");
        assert_eq!(g.ghost_for("x"), None);
    }

    #[test]
    fn clear_then_empty_shows_again() {
        let mut g = GhostText::new();
        g.set("ab");
        g.dismiss();
        assert_eq!(g.ghost_for(""), None);
        g.set("ab");
        assert_eq!(g.ghost_for(""), Some("ab"));
        g.clear();
        g.set("ab");
        assert_eq!(g.ghost_for(""), Some("ab"));
    }

    #[test]
    fn accept_returns_remainder() {
        let mut g = GhostText::new();
        g.set("save the night theme");
        let rest = g.accept("save ").unwrap();
        assert_eq!(rest, "the night theme");
        assert_eq!(g.ghost_for("save "), None);
    }

    #[test]
    fn right_accepts_at_end_tab_does_not_by_default() {
        let mut g = GhostText::new();
        g.set("hello");
        let right = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        let tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(g.handle_key(&tab, "", true), GhostAction::Unhandled);
        g.set("hello");
        assert_eq!(
            g.handle_key(&right, "", true),
            GhostAction::Accepted("hello".into())
        );
    }
}
