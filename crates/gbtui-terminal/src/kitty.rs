//! Kitty keyboard enhancement flags.
//!
//! Alacritty before its fixed `alacritty_terminal` 0.24.2 implementation
//! reports a legacy duplicate press for several releases when event types are
//! requested. It still benefits from disambiguation, so only that flag is
//! negotiated for a positively identified affected version.

use std::sync::atomic::{AtomicU8, Ordering};

use crossterm::event::KeyboardEnhancementFlags;

use crate::brand::TerminalContext;

/// The last Alacritty library version with duplicate release encoding.
pub const ALACRITTY_BROKEN_EVENT_TYPES_MAX_PACKED: u32 = 2401;

static PUSHED: AtomicU8 = AtomicU8::new(0);

/// Negotiates keyboard flags after the startup DA2 probe.
///
/// Alacritty with no trustworthy DA2 version is deliberately conservative:
/// accepting duplicate release bytes as presses is worse than withholding
/// release events. Other brands do not use DA2 and retain the normal flags.
pub fn negotiated_kitty_flags(
    skip_reason: Option<&str>,
    is_alacritty: bool,
    alacritty_da2_packed: Option<u32>,
) -> KeyboardEnhancementFlags {
    if skip_reason.is_some() {
        return KeyboardEnhancementFlags::empty();
    }
    let mut flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
    let withhold_event_types = is_alacritty
        && alacritty_da2_packed
            .is_none_or(|packed| packed <= ALACRITTY_BROKEN_EVENT_TYPES_MAX_PACKED);
    if !withhold_event_types {
        flags |= KeyboardEnhancementFlags::REPORT_EVENT_TYPES;
    }
    flags
}

/// Resolves keyboard flags for a context using the DA2 result collected before
/// terminal protocols and the event stream start.
pub fn flags_for_context(
    ctx: &TerminalContext,
    alacritty_da2_packed: Option<u32>,
) -> KeyboardEnhancementFlags {
    negotiated_kitty_flags(
        ctx.kitty_skip_reason(),
        ctx.brand == crate::brand::TerminalName::Alacritty,
        alacritty_da2_packed,
    )
}

pub fn set_pushed_kitty_flags(flags: KeyboardEnhancementFlags) {
    PUSHED.store(flags.bits(), Ordering::Relaxed);
}

/// Clear the observable pushed flags after a matching successful pop.
///
/// The comparison keeps a stale cleanup with different flags from erasing a
/// newer protocol state.
pub fn clear_pushed_kitty_flags(flags: KeyboardEnhancementFlags) {
    let _ = PUSHED.compare_exchange(flags.bits(), 0, Ordering::Relaxed, Ordering::Relaxed);
}

pub fn pushed_kitty_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::from_bits_truncate(PUSHED.load(Ordering::Relaxed))
}

pub fn kitty_flags_pushed() -> bool {
    !pushed_kitty_flags().is_empty()
}

/// Whether the actually pushed protocol reports release events.
pub fn kitty_releases_reported() -> bool {
    pushed_kitty_flags().contains(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
}

/// Whether the Alacritty workaround pushed keyboard support without release
/// reporting. This differs from an entirely skipped protocol.
pub fn kitty_event_types_withheld() -> bool {
    let flags = pushed_kitty_flags();
    !flags.is_empty() && !flags.contains(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_and_unknown_alacritty_withhold_releases_but_new_is_enabled() {
        let disambiguate = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
        let release_types = KeyboardEnhancementFlags::REPORT_EVENT_TYPES;
        assert!(negotiated_kitty_flags(Some("multiplexer"), true, Some(2402)).is_empty());
        assert_eq!(negotiated_kitty_flags(None, true, Some(2401)), disambiguate);
        assert_eq!(negotiated_kitty_flags(None, true, None), disambiguate);
        assert_eq!(
            negotiated_kitty_flags(None, true, Some(2402)),
            disambiguate | release_types
        );
        assert_eq!(
            negotiated_kitty_flags(None, false, None),
            disambiguate | release_types
        );
    }

    #[test]
    fn withheld_event_types_are_distinct_from_skipping_protocol() {
        let withheld = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
        set_pushed_kitty_flags(withheld);
        assert!(kitty_event_types_withheld());
        assert!(!kitty_releases_reported());
        clear_pushed_kitty_flags(withheld);

        assert!(!kitty_event_types_withheld());
        assert!(!kitty_releases_reported());
    }

    #[test]
    fn matching_pop_clears_pushed_state_without_clobbering_other_flags() {
        let pushed = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
        let other = KeyboardEnhancementFlags::REPORT_EVENT_TYPES;
        set_pushed_kitty_flags(pushed);
        clear_pushed_kitty_flags(other);
        assert_eq!(pushed_kitty_flags(), pushed);
        clear_pushed_kitty_flags(pushed);
        assert!(!kitty_flags_pushed());
    }
}
