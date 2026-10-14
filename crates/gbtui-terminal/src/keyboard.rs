//! Per-terminal keyboard delivery classification.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/terminal/keyboard.rs (STRIP)`

use crate::brand::TerminalName;
use crate::host::HostOs;

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum ModifierFate {
    Native,
    Dropped,
    Unrecoverable,
    #[default]
    Unknown,
}

impl ModifierFate {
    pub fn benefits_from_rescue(self) -> bool {
        matches!(self, Self::Dropped)
    }
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct ModifierDelivery {
    pub cmd: ModifierFate,
    pub opt: ModifierFate,
}

impl ModifierDelivery {
    pub fn new_for_test(cmd: ModifierFate, opt: ModifierFate) -> Self {
        Self { cmd, opt }
    }

    pub fn benefits_from_rescue(self) -> bool {
        self.cmd.benefits_from_rescue() || self.opt.benefits_from_rescue()
    }
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct KeyboardCapabilities {
    pub modifier_delivery: ModifierDelivery,
    pub enter_modifier: ModifierFate,
}

impl KeyboardCapabilities {
    pub fn enter_needs_rescue(&self) -> bool {
        matches!(self.enter_modifier, ModifierFate::Dropped)
    }
}

pub fn keyboard_capabilities(brand: TerminalName) -> KeyboardCapabilities {
    keyboard_capabilities_for_host(brand, HostOs::current())
}

pub fn keyboard_capabilities_for_host(brand: TerminalName, host: HostOs) -> KeyboardCapabilities {
    match host {
        HostOs::Macos => macos_capabilities(brand),
        HostOs::Linux | HostOs::Windows | HostOs::Other => KeyboardCapabilities::default(),
    }
}

fn macos_capabilities(brand: TerminalName) -> KeyboardCapabilities {
    use ModifierFate::*;
    let (cmd, opt, enter) = match brand {
        TerminalName::Ghostty | TerminalName::Kitty | TerminalName::Foot => {
            (Native, Native, Native)
        }
        TerminalName::Iterm2
        | TerminalName::VsCode
        | TerminalName::Cursor
        | TerminalName::Windsurf
        | TerminalName::Zed => (Native, Native, Native),
        TerminalName::WezTerm => (Dropped, Native, Native),
        TerminalName::Alacritty | TerminalName::Rio => (Native, Dropped, Native),
        TerminalName::WarpTerminal => (Dropped, Dropped, Native),
        TerminalName::AppleTerminal => (Unrecoverable, Dropped, Dropped),
        TerminalName::Vte | TerminalName::Terminator => (Unknown, Unknown, Unknown),
        TerminalName::JetBrains => (Unknown, Unknown, Unknown),
        TerminalName::WindowsTerminal | TerminalName::Unknown => (Unknown, Unknown, Unknown),
    };
    KeyboardCapabilities {
        modifier_delivery: ModifierDelivery { cmd, opt },
        enter_modifier: enter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghostty_native_on_macos() {
        let c = keyboard_capabilities_for_host(TerminalName::Ghostty, HostOs::Macos);
        assert_eq!(c.modifier_delivery.cmd, ModifierFate::Native);
        assert!(!c.modifier_delivery.benefits_from_rescue());
    }

    #[test]
    fn wezterm_drops_cmd_on_macos() {
        let c = keyboard_capabilities_for_host(TerminalName::WezTerm, HostOs::Macos);
        assert_eq!(c.modifier_delivery.cmd, ModifierFate::Dropped);
        assert!(c.modifier_delivery.benefits_from_rescue());
    }

    #[test]
    fn linux_stays_unknown() {
        let c = keyboard_capabilities_for_host(TerminalName::Ghostty, HostOs::Linux);
        assert_eq!(c.modifier_delivery.cmd, ModifierFate::Unknown);
    }
}
