//! tmux DCS passthrough wrapping.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/terminal/tmux.rs (STRIP)`

use crate::brand::TerminalContext;

pub fn tmux_passthrough(sequence: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(sequence.len() + 16);
    out.extend_from_slice(b"\x1bPtmux;");
    for &byte in sequence {
        if byte == 0x1b {
            out.push(0x1b);
        }
        out.push(byte);
    }
    out.extend_from_slice(b"\x1b\\");
    out
}

pub fn tmux_passthrough_str(sequence: &str) -> String {
    let mut out = String::with_capacity(sequence.len() + 16);
    out.push_str("\x1bPtmux;");
    for ch in sequence.chars() {
        if ch == '\x1b' {
            out.push('\x1b');
        }
        out.push(ch);
    }
    out.push_str("\x1b\\");
    out
}

pub fn passthrough_available(ctx: &TerminalContext) -> bool {
    ctx.is_tmux()
}

pub fn should_wrap_osc11(ctx: &TerminalContext) -> bool {
    passthrough_available(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_esc_and_wraps() {
        assert_eq!(
            tmux_passthrough(b"\x1b]11;?\x07"),
            b"\x1bPtmux;\x1b\x1b]11;?\x07\x1b\\"
        );
    }

    #[test]
    fn plain_text_has_no_esc_to_double() {
        assert_eq!(tmux_passthrough(b"plain"), b"\x1bPtmux;plain\x1b\\");
    }
}
