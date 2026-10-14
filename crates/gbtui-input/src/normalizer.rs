//! Rescue modifiers the terminal dropped.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/input/keyboard_normalizer.rs (STRIP)`

use std::borrow::Cow;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use gbtui_terminal::{ModifierDelivery, terminal_context};

/// Snapshot of physically-held modifier keys.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct ModifierState {
    pub command: bool,
    pub option: bool,
    pub shift: bool,
    pub control: bool,
}

pub trait ModifierProbe {
    fn snapshot(&self) -> ModifierState;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OsModifierProbe;

impl ModifierProbe for OsModifierProbe {
    #[cfg(target_os = "macos")]
    fn snapshot(&self) -> ModifierState {
        crate::macos_modifiers::snapshot()
    }

    #[cfg(not(target_os = "macos"))]
    fn snapshot(&self) -> ModifierState {
        ModifierState::default()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct KeyboardNormalizer<P: ModifierProbe = OsModifierProbe> {
    probe: P,
    delivery: ModifierDelivery,
}

impl<P: ModifierProbe> KeyboardNormalizer<P> {
    pub fn new(probe: P, delivery: ModifierDelivery) -> Self {
        Self { probe, delivery }
    }

    /// Upgrade a key event when a held modifier is missing from the encoding.
    pub fn rescue_key(&self, key: KeyEvent) -> Option<KeyEvent> {
        if key.code == KeyCode::Char('\u{0002}') && key.modifiers.is_empty() {
            let mut out = key;
            out.code = KeyCode::Char('b');
            out.modifiers = KeyModifiers::CONTROL;
            return Some(out);
        }
        if !self.delivery.benefits_from_rescue() {
            return None;
        }
        if !key.modifiers.is_empty() {
            return None;
        }
        if !matches!(key.code, KeyCode::Backspace | KeyCode::Delete) {
            return None;
        }
        let state = self.probe.snapshot();
        let added = match (
            state.command && self.delivery.cmd.benefits_from_rescue(),
            state.option && self.delivery.opt.benefits_from_rescue(),
        ) {
            (true, _) => KeyModifiers::SUPER,
            (false, true) => KeyModifiers::ALT,
            _ => return None,
        };
        let mut out = key;
        out.modifiers |= added;
        Some(out)
    }

    pub fn rescue<'a>(&self, ev: &'a Event) -> Cow<'a, Event> {
        if let Event::Key(k) = ev
            && let Some(upgraded) = self.rescue_key(*k)
        {
            return Cow::Owned(Event::Key(upgraded));
        }
        Cow::Borrowed(ev)
    }
}

impl KeyboardNormalizer<OsModifierProbe> {
    pub fn from_terminal_context() -> Self {
        Self {
            probe: OsModifierProbe,
            delivery: gbtui_terminal::keyboard_capabilities(terminal_context().brand)
                .modifier_delivery,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{is_text_input_key, key};
    use gbtui_terminal::ModifierFate;

    #[derive(Debug, Default, Clone, Copy)]
    struct MockProbe(ModifierState);

    impl ModifierProbe for MockProbe {
        fn snapshot(&self) -> ModifierState {
            self.0
        }
    }

    fn drops_both() -> ModifierDelivery {
        ModifierDelivery::new_for_test(ModifierFate::Dropped, ModifierFate::Dropped)
    }

    fn make(state: ModifierState, delivery: ModifierDelivery) -> KeyboardNormalizer<MockProbe> {
        KeyboardNormalizer::new(MockProbe(state), delivery)
    }

    fn bare(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn raw_ctrl_b_canonicalizes_without_modifier_probe() {
        let native = ModifierDelivery::new_for_test(ModifierFate::Native, ModifierFate::Native);
        let n = make(ModifierState::default(), native);
        let raw = KeyEvent::new(KeyCode::Char('\u{0002}'), KeyModifiers::NONE);
        let out = n.rescue_key(raw).expect("raw Ctrl+B must canonicalize");
        assert_eq!(out.code, KeyCode::Char('b'));
        assert_eq!(out.modifiers, KeyModifiers::CONTROL);
        assert!(!is_text_input_key(&out));
        assert!(key!('b', CONTROL).matches(&out));
    }

    #[test]
    fn cmd_backspace_upgrades_to_super() {
        let n = make(
            ModifierState {
                command: true,
                ..Default::default()
            },
            drops_both(),
        );
        let out = n.rescue_key(bare(KeyCode::Backspace)).unwrap();
        assert_eq!(out.modifiers, KeyModifiers::SUPER);
    }

    #[test]
    fn opt_backspace_upgrades_to_alt() {
        let n = make(
            ModifierState {
                option: true,
                ..Default::default()
            },
            drops_both(),
        );
        let out = n.rescue_key(bare(KeyCode::Backspace)).unwrap();
        assert_eq!(out.modifiers, KeyModifiers::ALT);
    }

    #[test]
    fn no_modifier_held_skips_rescue() {
        let n = make(ModifierState::default(), drops_both());
        assert!(n.rescue_key(bare(KeyCode::Backspace)).is_none());
    }

    #[test]
    fn already_modified_event_skips_rescue() {
        let n = make(
            ModifierState {
                command: true,
                ..Default::default()
            },
            drops_both(),
        );
        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::SHIFT);
        assert!(n.rescue_key(key).is_none());
    }

    #[test]
    fn non_deletion_keys_skip_rescue() {
        let n = make(
            ModifierState {
                command: true,
                option: true,
                ..Default::default()
            },
            drops_both(),
        );
        for code in [KeyCode::Char('a'), KeyCode::Enter, KeyCode::Esc] {
            assert!(n.rescue_key(bare(code)).is_none());
        }
    }

    #[test]
    fn rescue_only_adds_modifier_for_dropped_axis() {
        let opt_only = ModifierDelivery::new_for_test(ModifierFate::Native, ModifierFate::Dropped);
        let n = make(
            ModifierState {
                command: true,
                ..Default::default()
            },
            opt_only,
        );
        assert!(n.rescue_key(bare(KeyCode::Backspace)).is_none());
        let n = make(
            ModifierState {
                option: true,
                ..Default::default()
            },
            opt_only,
        );
        assert_eq!(
            n.rescue_key(bare(KeyCode::Backspace)).unwrap().modifiers,
            KeyModifiers::ALT
        );
    }
}
