//! Chrome glyphs with legacy ConHost fallbacks. No voice glyphs.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/glyphs.rs (STRIP)`

use crate::brand::{TerminalName, terminal_context};
use crate::host::HostOs;

pub fn is_legacy_windows_console() -> bool {
    HostOs::current() == HostOs::Windows
        && terminal_context().brand != TerminalName::WindowsTerminal
}

pub fn prompt_arrow() -> &'static str {
    if is_legacy_windows_console() {
        "> "
    } else {
        "\u{276F} "
    }
}

pub const PROMPT_ARROW_WIDTH: u16 = 2;

/// `"◆"` (U+25C6) normally, `"♦"` (U+2666) on legacy ConHost. Always 1 column.
///
/// Grok scrollback block bullet (tools, thinking, lifecycle). STRIP'd from
/// `xai-grok-pager-render/src/glyphs.rs`.
pub fn diamond_filled() -> &'static str {
    if is_legacy_windows_console() {
        "\u{2666}"
    } else {
        "\u{25C6}"
    }
}

/// `"◇"` (U+25C7) normally, `"○"` (U+25CB) on legacy ConHost. Always 1 column.
pub fn diamond_hollow() -> &'static str {
    if is_legacy_windows_console() {
        "\u{25CB}"
    } else {
        "\u{25C7}"
    }
}

pub fn ballot_x() -> &'static str {
    if is_legacy_windows_console() {
        "x"
    } else {
        "\u{2717}"
    }
}

pub fn check_mark() -> &'static str {
    if is_legacy_windows_console() {
        "\u{221A}"
    } else {
        "\u{2713}"
    }
}

pub fn collapsed_accent() -> &'static str {
    if is_legacy_windows_console() {
        "|"
    } else {
        "\u{2759}"
    }
}

/// Rotating braille spinner (`⠋⠙⠹⠸⠼⠴⠦⠧`); ASCII `|/-\\` on legacy ConHost.
pub fn braille_spinner_frames() -> &'static [&'static str] {
    const FANCY: &[&str] = &[
        "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}",
        "\u{2827}",
    ];
    const FALLBACK: &[&str] = &["|", "/", "-", "\\"];
    if is_legacy_windows_console() {
        FALLBACK
    } else {
        FANCY
    }
}

/// `"┃"` normally, `"│"` on legacy ConHost. Always 1 column.
pub fn accent_bar() -> &'static str {
    if is_legacy_windows_console() {
        "\u{2502}"
    } else {
        "\u{2503}"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_arrow_is_two_columns() {
        use unicode_width::UnicodeWidthStr;
        assert_eq!(prompt_arrow().width(), PROMPT_ARROW_WIDTH as usize);
    }

    #[test]
    fn diamonds_are_one_column() {
        use unicode_width::UnicodeWidthStr;
        assert_eq!(diamond_filled().width(), 1);
        assert_eq!(diamond_hollow().width(), 1);
    }
}
