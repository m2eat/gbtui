//! Per-terminal hyperlink (OSC 8) capabilities.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/terminal/hyperlinks.rs (STRIP)`
//!
//! v1 Link widget uses hover + click. True OSC 8 emission stays in the
//! inline Terminal (feature-gate at the call site).

use crate::brand::TerminalName;

/// Whether the terminal supports OSC 8 hyperlink sequences.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum Osc8Support {
    Native,
    /// Apple Terminal garbles unknown OSC sequences.
    HostileParser,
    Unsupported,
    #[default]
    Unknown,
}

/// Which URL schemes the terminal supports in OSC 8 links.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum SchemeFilter {
    #[default]
    Standard,
    EditorExtended,
}

impl SchemeFilter {
    pub fn allows(self, scheme: &str) -> bool {
        match self {
            Self::Standard => matches!(scheme, "http" | "https" | "mailto"),
            Self::EditorExtended => matches!(
                scheme,
                "http" | "https" | "mailto" | "file" | "vscode" | "cursor" | "idea" | "zed"
            ),
        }
    }
}

impl HyperlinkCapabilities {
    /// Whether this terminal can safely receive OSC 8 sequences.
    pub const fn supports_osc8(self) -> bool {
        matches!(self.osc8, Osc8Support::Native)
    }

    /// A conservative native OSC 8 profile for embedded terminal hosts that
    /// have positively probed support.
    pub const fn native() -> Self {
        Self {
            osc8: Osc8Support::Native,
            id_param: true,
            scheme_filter: SchemeFilter::Standard,
            osc22_cursor: false,
            native_link_hover: false,
            native_plain_url_open: false,
        }
    }

    /// A profile for terminals known to misparse OSC sequences.
    pub const fn hostile_parser() -> Self {
        Self {
            osc8: Osc8Support::HostileParser,
            id_param: false,
            scheme_filter: SchemeFilter::Standard,
            osc22_cursor: false,
            native_link_hover: false,
            native_plain_url_open: false,
        }
    }

    /// Whether `url` is safe and supported for OSC 8 emission.
    ///
    /// URLs with C0/C1 controls are rejected rather than rewritten: an OSC
    /// payload must never be allowed to terminate or inject another escape
    /// sequence. A scheme is required and is filtered by terminal policy.
    pub fn allows_url(self, url: &str) -> bool {
        self.supports_osc8()
            && !url.chars().any(char::is_control)
            && url_scheme(url).is_some_and(|scheme| self.scheme_filter.allows(scheme))
    }
}

fn url_scheme(url: &str) -> Option<&str> {
    let colon = url.find(':')?;
    let scheme = &url[..colon];
    let mut chars = scheme.bytes();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic()
        || !chars.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
    {
        return None;
    }
    Some(scheme)
}

/// Per-terminal hyperlink capabilities.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct HyperlinkCapabilities {
    pub osc8: Osc8Support,
    pub id_param: bool,
    pub scheme_filter: SchemeFilter,
    pub osc22_cursor: bool,
    /// Terminal styles link hover natively — skip app Cmd/Ctrl+hover paint.
    pub native_link_hover: bool,
    pub native_plain_url_open: bool,
}

pub fn hyperlink_capabilities(brand: TerminalName) -> HyperlinkCapabilities {
    use Osc8Support::*;
    match brand {
        TerminalName::AppleTerminal => HyperlinkCapabilities {
            osc8: HostileParser,
            id_param: false,
            scheme_filter: SchemeFilter::Standard,
            osc22_cursor: false,
            native_link_hover: false,
            native_plain_url_open: false,
        },
        TerminalName::Iterm2 | TerminalName::Ghostty | TerminalName::Kitty => {
            HyperlinkCapabilities {
                osc8: Native,
                id_param: true,
                scheme_filter: SchemeFilter::Standard,
                osc22_cursor: true,
                native_link_hover: false,
                native_plain_url_open: false,
            }
        }
        TerminalName::Alacritty
        | TerminalName::Rio
        | TerminalName::Foot
        | TerminalName::WezTerm => HyperlinkCapabilities {
            osc8: Native,
            id_param: true,
            scheme_filter: SchemeFilter::Standard,
            osc22_cursor: false,
            native_link_hover: false,
            native_plain_url_open: false,
        },
        TerminalName::VsCode
        | TerminalName::Cursor
        | TerminalName::Windsurf
        | TerminalName::Zed => HyperlinkCapabilities {
            osc8: Native,
            id_param: true,
            scheme_filter: SchemeFilter::Standard,
            osc22_cursor: false,
            native_link_hover: true,
            native_plain_url_open: false,
        },
        TerminalName::WarpTerminal => HyperlinkCapabilities {
            osc8: Unsupported,
            id_param: false,
            scheme_filter: SchemeFilter::Standard,
            osc22_cursor: false,
            native_link_hover: false,
            native_plain_url_open: true,
        },
        TerminalName::Vte | TerminalName::Terminator => HyperlinkCapabilities {
            osc8: Native,
            id_param: true,
            scheme_filter: SchemeFilter::Standard,
            osc22_cursor: false,
            native_link_hover: false,
            native_plain_url_open: false,
        },
        TerminalName::WindowsTerminal => HyperlinkCapabilities {
            osc8: Native,
            id_param: true,
            scheme_filter: SchemeFilter::Standard,
            osc22_cursor: false,
            native_link_hover: false,
            native_plain_url_open: false,
        },
        TerminalName::JetBrains | TerminalName::Unknown => HyperlinkCapabilities {
            osc8: Unknown,
            id_param: false,
            scheme_filter: SchemeFilter::Standard,
            osc22_cursor: false,
            native_link_hover: false,
            native_plain_url_open: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_terminal_is_hostile() {
        assert_eq!(
            hyperlink_capabilities(TerminalName::AppleTerminal).osc8,
            Osc8Support::HostileParser
        );
    }

    #[test]
    fn vscode_family_has_native_hover() {
        assert!(hyperlink_capabilities(TerminalName::VsCode).native_link_hover);
    }

    #[test]
    fn standard_schemes() {
        assert!(SchemeFilter::Standard.allows("https"));
        assert!(!SchemeFilter::Standard.allows("file"));
        assert!(SchemeFilter::EditorExtended.allows("file"));
    }

    #[test]
    fn osc8_url_policy_rejects_unknown_hostile_and_control_payloads() {
        let native = HyperlinkCapabilities {
            osc8: Osc8Support::Native,
            ..Default::default()
        };
        assert!(native.allows_url("https://example.com/path"));
        assert!(!native.allows_url("javascript:alert(1)"));
        assert!(!native.allows_url("https://example.com/\x1b]52;c;payload"));

        let hostile = HyperlinkCapabilities {
            osc8: Osc8Support::HostileParser,
            ..Default::default()
        };
        assert!(!hostile.allows_url("https://example.com"));
    }
}
