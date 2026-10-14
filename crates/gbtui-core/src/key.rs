//! Key shortcut types and the `key!()` macro.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/input/key.rs (COPY)`
//!
//! ```
//! use gbtui_core::key;
//!
//! let q = key!('q');
//! let ctrl_c = key!('c', CONTROL);
//! let ctrl_shift_z = key!('z', CONTROL | SHIFT);
//!
//! use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
//! let event = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
//! assert!(ctrl_c.matches(&event));
//! ```

use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// A key + modifiers pair for matching against crossterm events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyShortcut {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyShortcut {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }.normalize_case()
    }

    pub fn key(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::NONE)
    }

    pub fn ctrl(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::CONTROL)
    }

    fn normalize_case(mut self) -> Self {
        let c = match self.code {
            KeyCode::Char(c) => c,
            _ => return self,
        };
        if c.is_ascii_uppercase() {
            self.modifiers.insert(KeyModifiers::SHIFT);
        } else if self.modifiers.contains(KeyModifiers::SHIFT) {
            self.code = KeyCode::Char(c.to_ascii_uppercase());
        }
        self
    }

    /// Check if a crossterm KeyEvent matches this shortcut.
    ///
    /// Both `Char('G') + NONE` and `Char('g') + SHIFT` match `key!('G')`.
    pub fn matches(&self, event: &KeyEvent) -> bool {
        if event.kind == KeyEventKind::Release {
            return false;
        }
        let normalized = Self::new(event.code, event.modifiers);
        self.code == normalized.code && self.modifiers == normalized.modifiers
    }

    pub fn to_key_event(self) -> KeyEvent {
        KeyEvent::new(self.code, self.modifiers)
    }

    pub fn display(&self) -> String {
        self.to_string()
    }

    pub fn modifiers_prefix(&self) -> String {
        let mut s = String::new();
        let _ = self.write_modifiers_prefix(&mut s);
        s
    }

    pub fn code_display(&self) -> String {
        let mut s = String::new();
        let _ = self.write_code_display(&mut s);
        s
    }

    fn write_modifiers_prefix(&self, f: &mut impl fmt::Write) -> fmt::Result {
        if self.modifiers.contains(KeyModifiers::SUPER) {
            f.write_str(if cfg!(target_os = "macos") {
                "Cmd+"
            } else {
                "Super+"
            })?;
        }
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            f.write_str("Ctrl+")?;
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            f.write_str(if cfg!(target_os = "macos") {
                "Opt+"
            } else {
                "Alt+"
            })?;
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            f.write_str("Shift+")?;
        }
        Ok(())
    }

    fn write_code_display(&self, f: &mut impl fmt::Write) -> fmt::Result {
        match self.code {
            KeyCode::Char(' ') => f.write_str("Space"),
            KeyCode::Char(c) => write!(f, "{}", c.to_ascii_lowercase()),
            KeyCode::Enter => f.write_str("Enter"),
            KeyCode::Esc => f.write_str("Esc"),
            KeyCode::Tab => f.write_str("Tab"),
            KeyCode::BackTab => f.write_str("Shift+Tab"),
            KeyCode::Backspace => f.write_str("Bsp"),
            KeyCode::Delete => f.write_str("Del"),
            KeyCode::Up => f.write_str("↑"),
            KeyCode::Down => f.write_str("↓"),
            KeyCode::Left => f.write_str("←"),
            KeyCode::Right => f.write_str("→"),
            KeyCode::Home => f.write_str("Home"),
            KeyCode::End => f.write_str("End"),
            KeyCode::PageUp => f.write_str("PgUp"),
            KeyCode::PageDown => f.write_str("PgDn"),
            KeyCode::F(n) => write!(f, "F{n}"),
            other => write!(f, "{other:?}"),
        }
    }

    pub fn is_letter_or_shift_letter(&self) -> bool {
        let KeyCode::Char(c) = self.code else {
            return false;
        };
        if !c.is_ascii_alphabetic() {
            return false;
        }
        let mods = self.modifiers;
        mods.is_empty() || mods == KeyModifiers::SHIFT
    }
}

pub fn is_paste_key(key: &KeyEvent) -> bool {
    if key!('v', CONTROL).matches(key) || key!('v', SUPER).matches(key) {
        return true;
    }
    #[cfg(target_os = "windows")]
    if key!('v', ALT).matches(key) {
        return true;
    }
    false
}

pub fn is_inline_paste_key(key: &KeyEvent) -> bool {
    key!('v', CONTROL | SHIFT).matches(key) || key!('v', SUPER | SHIFT).matches(key)
}

#[cfg(target_os = "windows")]
#[inline]
pub fn is_altgr(modifiers: KeyModifiers) -> bool {
    modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

#[cfg(not(target_os = "windows"))]
#[inline]
pub fn is_altgr(_modifiers: KeyModifiers) -> bool {
    false
}

pub fn shift_tab_keys() -> [KeyShortcut; 3] {
    [
        KeyShortcut::key(KeyCode::BackTab),
        KeyShortcut::new(KeyCode::BackTab, KeyModifiers::SHIFT),
        KeyShortcut::new(KeyCode::Tab, KeyModifiers::SHIFT),
    ]
}

pub fn is_shift_tab(key: &KeyEvent) -> bool {
    shift_tab_keys().iter().any(|k| k.matches(key))
}

/// One step of the Tab / Shift+Tab walk through a list of rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowWalk {
    Forward,
    Backward,
}

impl RowWalk {
    pub fn from_key(key: &KeyEvent) -> Option<Self> {
        if is_shift_tab(key) {
            Some(Self::Backward)
        } else if KeyShortcut::key(KeyCode::Tab).matches(key) {
            Some(Self::Forward)
        } else {
            None
        }
    }

    #[must_use]
    pub fn step(self, idx: usize, len: usize) -> usize {
        let Some(last) = len.checked_sub(1) else {
            return 0;
        };
        let cur = idx.min(last);
        match self {
            Self::Forward => {
                if cur == last {
                    0
                } else {
                    cur + 1
                }
            }
            Self::Backward => {
                if cur == 0 {
                    last
                } else {
                    cur - 1
                }
            }
        }
    }
}

/// Printable typing (and AltGr), not chords.
pub fn is_text_input_key(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char(_))
        && (key.modifiers.is_empty()
            || key.modifiers == KeyModifiers::SHIFT
            || is_altgr(key.modifiers))
}

impl From<KeyEvent> for KeyShortcut {
    fn from(key: KeyEvent) -> Self {
        Self::new(key.code, key.modifiers)
    }
}

impl fmt::Display for KeyShortcut {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.write_modifiers_prefix(f)?;
        self.write_code_display(f)
    }
}

/// Ergonomic constructor for [`KeyShortcut`].
///
/// ```ignore
/// key!(Enter)
/// key!('q')
/// key!('c', CONTROL)
/// key!('z', CONTROL | SHIFT)
/// key!(F(5))
/// ```
#[macro_export]
macro_rules! key {
    ($char:literal $(, $($mod:ident)|+)? $(,)?) => {
        $crate::key::KeyShortcut::new(
            $crate::crossterm::event::KeyCode::Char($char),
            $crate::crossterm::event::KeyModifiers::NONE
                $($(| $crate::crossterm::event::KeyModifiers::$mod)+)?,
        )
    };
    ($code:ident $(, $($mod:ident)|+)? $(,)?) => {
        $crate::key::KeyShortcut::new(
            $crate::crossterm::event::KeyCode::$code,
            $crate::crossterm::event::KeyModifiers::NONE
                $($(| $crate::crossterm::event::KeyModifiers::$mod)+)?,
        )
    };
    ($code:ident ($($arg:tt)*) $(, $($mod:ident)|+)? $(,)?) => {
        $crate::key::KeyShortcut::new(
            $crate::crossterm::event::KeyCode::$code($($arg)*),
            $crate::crossterm::event::KeyModifiers::NONE
                $($(| $crate::crossterm::event::KeyModifiers::$mod)+)?,
        )
    };
}

pub use key;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_char() {
        let k = key!('q');
        assert_eq!(k.code, KeyCode::Char('q'));
        assert_eq!(k.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn ctrl_modifier() {
        let k = key!('c', CONTROL);
        assert_eq!(k.code, KeyCode::Char('c'));
        assert_eq!(k.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn ctrl_shift_combined() {
        let k = key!('z', CONTROL | SHIFT);
        assert!(k.modifiers.contains(KeyModifiers::CONTROL));
        assert!(k.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn shift_normalizes_uppercase() {
        let k = key!('G');
        assert_eq!(k.code, KeyCode::Char('G'));
        assert!(k.modifiers.contains(KeyModifiers::SHIFT));
        let event = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::SHIFT);
        assert!(k.matches(&event));
    }

    #[test]
    fn shift_tab_all_encodings() {
        assert!(is_shift_tab(&KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::NONE
        )));
        assert!(is_shift_tab(&KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT
        )));
        assert!(is_shift_tab(&KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::SHIFT
        )));
        assert!(!is_shift_tab(&KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE
        )));
        let mut release = KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert!(!is_shift_tab(&release));
    }

    #[test]
    fn row_walk_wraps_at_both_ends() {
        assert_eq!(RowWalk::Forward.step(0, 3), 1);
        assert_eq!(RowWalk::Forward.step(2, 3), 0);
        assert_eq!(RowWalk::Backward.step(0, 3), 2);
        assert_eq!(RowWalk::Forward.step(0, 0), 0);
    }

    #[test]
    fn special_keys() {
        assert_eq!(key!(Enter).code, KeyCode::Enter);
        assert_eq!(key!(Esc).code, KeyCode::Esc);
        assert_eq!(key!(F(5)).code, KeyCode::F(5));
    }

    #[test]
    fn matches_key_event() {
        let ctrl_c = key!('c', CONTROL);
        assert!(ctrl_c.matches(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(!ctrl_c.matches(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)));
    }

    #[test]
    fn is_paste_key_ctrl_v() {
        let ev = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL);
        assert!(is_paste_key(&ev));
    }

    #[test]
    fn text_input_rejects_chords() {
        assert!(is_text_input_key(&KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE
        )));
        assert!(!is_text_input_key(&KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL
        )));
    }
}
