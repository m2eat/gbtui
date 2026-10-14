//! Bounded XTVERSION response parsing.

use crate::brand::{MultiplexerKind, TerminalContext, TerminalName};

/// XTVERSION query.
pub const QUERY: &[u8] = b"\x1b[>0q";

/// Maximum accepted self-report payload, excluding DCS framing.
pub const MAX_PAYLOAD_LEN: usize = 128;

/// A sanitized terminal self-report from XTVERSION.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XtVersion(String);

impl XtVersion {
    /// Parses exactly one DCS `>|payload ST` reply.
    ///
    /// The payload is deliberately ASCII-only and restricted to the characters
    /// used by terminal brand/version strings. This keeps a terminal reply from
    /// becoming a control-sequence transport.
    pub fn parse(reply: &[u8]) -> Option<Self> {
        let payload = reply.strip_prefix(b"\x1bP>|")?.strip_suffix(b"\x1b\\")?;
        if payload.is_empty() || payload.len() > MAX_PAYLOAD_LEN {
            return None;
        }
        let payload = std::str::from_utf8(payload).ok()?;
        if !payload.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b' ' | b'.' | b'_' | b'-' | b'(' | b')' | b'+')
        }) {
            return None;
        }
        let payload = payload.trim();
        (!payload.is_empty()).then(|| Self(payload.to_owned()))
    }

    /// Returns the sanitized terminal brand/version text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// XTVERSION is sent only to brands with a documented clean reply, directly
/// connected to the process. Unknown terminals and every known multiplexer are
/// intentionally excluded.
pub fn probe_is_safe(ctx: &TerminalContext) -> bool {
    if ctx.multiplexer != MultiplexerKind::Undetected {
        return false;
    }
    matches!(
        ctx.brand,
        TerminalName::Kitty
            | TerminalName::Ghostty
            | TerminalName::Iterm2
            | TerminalName::WezTerm
            | TerminalName::Rio
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brand::TerminalContext;
    use crate::host::HostOs;

    #[test]
    fn parses_only_safe_dcs_payloads() {
        let version = XtVersion::parse(b"\x1bP>|kitty 0.35.2\x1b\\").expect("valid reply");
        assert_eq!(version.as_str(), "kitty 0.35.2");

        for reply in [
            b"\x1bP>|\x1b\\".as_slice(),
            b"\x1bP>|kitty\x07\x1b\\",
            b"\x1bP>|kitty 0.35.2\x07",
            b"\x1bP>|\xff\x1b\\",
        ] {
            assert_eq!(XtVersion::parse(reply), None, "{reply:?}");
        }
    }
    #[test]
    fn only_direct_known_brands_are_queried() {
        let direct_kitty = TerminalContext {
            brand: TerminalName::Kitty,
            multiplexer: MultiplexerKind::Undetected,
            host: HostOs::Macos,
        };
        assert!(probe_is_safe(&direct_kitty));

        let muxed_kitty = TerminalContext {
            multiplexer: MultiplexerKind::Tmux,
            ..direct_kitty
        };
        assert!(!probe_is_safe(&muxed_kitty));
        assert!(!probe_is_safe(&TerminalContext {
            brand: TerminalName::Unknown,
            ..direct_kitty
        }));
    }
}
