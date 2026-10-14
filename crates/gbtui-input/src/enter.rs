//! OS-level rescue for the modified-Enter chord.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/input/terminal_support.rs (STRIP)`

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gbtui_terminal::{keyboard_capabilities_for_host, terminal_context};

/// `true` when the user is holding a modifier that should turn a bare
/// `Enter` into a newline, and the active terminal drops those flags.
pub fn is_apple_terminal_newline_modifier_held() -> bool {
    let ctx = terminal_context();
    if !keyboard_capabilities_for_host(ctx.brand, ctx.host).enter_needs_rescue() {
        return false;
    }
    os_any_newline_modifier_held()
}

/// Shift/Alt+Enter, or bare Enter while a newline modifier is held and the
/// terminal drops those flags. Always requires `KeyCode::Enter`.
pub fn is_mod_enter(key: &KeyEvent) -> bool {
    key.code == KeyCode::Enter
        && (key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT)
            || is_apple_terminal_newline_modifier_held())
}

#[cfg(target_os = "macos")]
fn os_any_newline_modifier_held() -> bool {
    let s = crate::macos_modifiers::snapshot();
    s.shift || s.option || s.command
}

#[cfg(not(target_os = "macos"))]
fn os_any_newline_modifier_held() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_mod_enter_requires_enter_code() {
        assert!(is_mod_enter(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT
        )));
        assert!(is_mod_enter(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::ALT
        )));
        assert!(!is_mod_enter(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        )));
        assert!(!is_mod_enter(&KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::SHIFT
        )));
        assert!(!is_mod_enter(&KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::SHIFT
        )));
    }
}
