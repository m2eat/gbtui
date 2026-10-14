//! Terminal brand and multiplexer detection from an env map.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/terminal/mod.rs (STRIP)`

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::host::HostOs;

/// Known terminal emulator categories.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, strum::Display)]
pub enum TerminalName {
    #[strum(to_string = "Apple Terminal")]
    AppleTerminal,
    Ghostty,
    #[strum(to_string = "iTerm2")]
    Iterm2,
    #[strum(to_string = "Warp")]
    WarpTerminal,
    #[strum(to_string = "VS Code")]
    VsCode,
    Cursor,
    Windsurf,
    Zed,
    WezTerm,
    #[strum(to_string = "Kitty")]
    Kitty,
    Alacritty,
    Rio,
    #[strum(to_string = "foot")]
    Foot,
    #[strum(to_string = "JetBrains")]
    JetBrains,
    #[strum(to_string = "VTE")]
    Vte,
    Terminator,
    #[strum(to_string = "Windows Terminal")]
    WindowsTerminal,
    #[default]
    Unknown,
}

impl TerminalName {
    pub fn is_vscode_family(self) -> bool {
        matches!(
            self,
            Self::VsCode | Self::Cursor | Self::Windsurf | Self::Zed
        )
    }

    pub fn is_xtermjs_embed(self) -> bool {
        matches!(self, Self::VsCode | Self::Cursor | Self::Windsurf)
    }

    pub fn supports_truecolor(self) -> bool {
        matches!(
            self,
            Self::Iterm2
                | Self::Ghostty
                | Self::Kitty
                | Self::WezTerm
                | Self::Alacritty
                | Self::Rio
                | Self::WarpTerminal
                | Self::VsCode
                | Self::Cursor
                | Self::Windsurf
                | Self::WindowsTerminal
                | Self::Foot
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum MultiplexerKind {
    Tmux,
    Screen,
    Zellij,
    Herdr,
    Cmux,
    #[default]
    Undetected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ByobuBackend {
    Tmux,
    Screen,
}

fn env_get<'a>(env: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    env.get(key).map(|v| v.as_str()).filter(|v| !v.is_empty())
}

fn terminal_name_from_term_program(value: &str) -> Option<TerminalName> {
    let normalized: String = value
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_' | '.'))
        .map(|c| c.to_ascii_lowercase())
        .collect();
    match normalized.as_str() {
        "appleterminal" => Some(TerminalName::AppleTerminal),
        "ghostty" => Some(TerminalName::Ghostty),
        "iterm" | "iterm2" | "itermapp" => Some(TerminalName::Iterm2),
        "warp" | "warpterminal" => Some(TerminalName::WarpTerminal),
        "vscode" => Some(TerminalName::VsCode),
        "wezterm" => Some(TerminalName::WezTerm),
        "kitty" => Some(TerminalName::Kitty),
        "alacritty" => Some(TerminalName::Alacritty),
        "rio" => Some(TerminalName::Rio),
        "terminator" => Some(TerminalName::Terminator),
        "zed" => Some(TerminalName::Zed),
        "windowsterminal" => Some(TerminalName::WindowsTerminal),
        _ => None,
    }
}

pub fn detect_terminal_brand_from_env(env: &HashMap<String, String>) -> TerminalName {
    if env_get(env, "CURSOR_TRACE_ID").is_some() {
        return TerminalName::Cursor;
    }
    if let Some(askpass) = env_get(env, "VSCODE_GIT_ASKPASS_MAIN") {
        let askpass_lower = askpass.to_ascii_lowercase();
        if askpass_lower.contains("cursor") {
            return TerminalName::Cursor;
        }
        if askpass_lower.contains("windsurf") {
            return TerminalName::Windsurf;
        }
        return TerminalName::VsCode;
    }
    if let Some(term_program) = env_get(env, "TERM_PROGRAM")
        && let Some(name) = terminal_name_from_term_program(term_program)
    {
        return name;
    }
    if let Some(te) = env_get(env, "TERMINAL_EMULATOR") {
        let te_lower = te.to_ascii_lowercase();
        if te_lower.contains("jetbrains") || te_lower.contains("jediterm") {
            return TerminalName::JetBrains;
        }
    }
    if env_get(env, "WEZTERM_VERSION").is_some() {
        return TerminalName::WezTerm;
    }
    if env_get(env, "ITERM_SESSION_ID").is_some()
        || env_get(env, "ITERM_PROFILE").is_some()
        || env_get(env, "LC_TERMINAL").is_some_and(|v| v.eq_ignore_ascii_case("iterm2"))
    {
        return TerminalName::Iterm2;
    }
    if env_get(env, "TERM_SESSION_ID").is_some() {
        return TerminalName::AppleTerminal;
    }
    if env_get(env, "KITTY_WINDOW_ID").is_some() {
        return TerminalName::Kitty;
    }
    if let Some(term) = env_get(env, "TERM")
        && term.contains("kitty")
    {
        return TerminalName::Kitty;
    }
    if env_get(env, "ALACRITTY_SOCKET").is_some() {
        return TerminalName::Alacritty;
    }
    if let Some(term) = env_get(env, "TERM")
        && term == "alacritty"
    {
        return TerminalName::Alacritty;
    }
    if let Some(term) = env_get(env, "TERM")
        && term == "rio"
    {
        return TerminalName::Rio;
    }
    if let Some(term) = env_get(env, "TERM")
        && matches!(term, "foot" | "foot-extra" | "foot-direct")
    {
        return TerminalName::Foot;
    }
    if env_get(env, "TERMINATOR_UUID").is_some() {
        return TerminalName::Terminator;
    }
    if env_get(env, "VTE_VERSION").is_some() {
        return TerminalName::Vte;
    }
    if env_get(env, "WT_SESSION").is_some() {
        return TerminalName::WindowsTerminal;
    }
    TerminalName::Unknown
}

fn detect_byobu_from_env(env: &HashMap<String, String>) -> Option<ByobuBackend> {
    let has_backend = env_get(env, "BYOBU_BACKEND").is_some();
    let has_config = env_get(env, "BYOBU_CONFIG_DIR").is_some();
    let has_distro = env_get(env, "BYOBU_DISTRO").is_some();
    if !has_backend && !has_config && !has_distro {
        return None;
    }
    if let Some(backend) = env_get(env, "BYOBU_BACKEND") {
        return match backend.to_ascii_lowercase().as_str() {
            "tmux" => Some(ByobuBackend::Tmux),
            "screen" => Some(ByobuBackend::Screen),
            _ => infer_byobu(env),
        };
    }
    infer_byobu(env)
}

fn infer_byobu(env: &HashMap<String, String>) -> Option<ByobuBackend> {
    match (
        env_get(env, "TMUX").is_some(),
        env_get(env, "STY").is_some(),
    ) {
        (true, _) => Some(ByobuBackend::Tmux),
        (false, true) => Some(ByobuBackend::Screen),
        (false, false) => None,
    }
}

pub fn detect_multiplexer_from_env(env: &HashMap<String, String>) -> MultiplexerKind {
    if let Some(backend) = detect_byobu_from_env(env) {
        match backend {
            ByobuBackend::Tmux => return MultiplexerKind::Tmux,
            ByobuBackend::Screen => return MultiplexerKind::Screen,
        }
    }
    if env_get(env, "TMUX").is_some() {
        return MultiplexerKind::Tmux;
    }
    if env_get(env, "ZELLIJ").is_some() || env_get(env, "ZELLIJ_SESSION_NAME").is_some() {
        return MultiplexerKind::Zellij;
    }
    if env_get(env, "STY").is_some() {
        return MultiplexerKind::Screen;
    }
    if env_get(env, "HERDR_ENV").is_some() {
        return MultiplexerKind::Herdr;
    }
    if env_get(env, "CMUX_SOCKET_PATH").is_some()
        || env_get(env, "CMUX_PANEL_ID").is_some()
        || env_get(env, "CMUX_BUNDLE_ID").is_some()
    {
        return MultiplexerKind::Cmux;
    }
    MultiplexerKind::Undetected
}

fn collect_env() -> HashMap<String, String> {
    std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect()
}

/// Snapshot of the process terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalContext {
    pub brand: TerminalName,
    pub multiplexer: MultiplexerKind,
    pub host: HostOs,
}

impl TerminalContext {
    pub fn from_env(env: &HashMap<String, String>, host: HostOs) -> Self {
        let mut brand = detect_terminal_brand_from_env(env);
        if brand == TerminalName::Unknown && host == HostOs::Windows {
            brand = TerminalName::WindowsTerminal;
        }
        Self {
            brand,
            multiplexer: detect_multiplexer_from_env(env),
            host,
        }
    }

    pub fn is_tmux(&self) -> bool {
        self.multiplexer == MultiplexerKind::Tmux
    }

    /// Kitty keyboard is skipped under multiplexers that intercept CSI (v1).
    pub fn kitty_skip_reason(&self) -> Option<&'static str> {
        match self.multiplexer {
            MultiplexerKind::Tmux | MultiplexerKind::Screen | MultiplexerKind::Zellij => {
                Some("multiplexer")
            }
            _ => None,
        }
    }

    /// Simplified stand-in for Grok's `ctrl_dot_unreliable`: Ctrl+. often
    /// never arrives without Kitty keyboard (tmux/screen, Apple Terminal,
    /// unknown brand, Windows).
    pub fn ctrl_dot_unreliable(self) -> bool {
        self.kitty_skip_reason().is_some()
            || self.host == HostOs::Windows
            || matches!(
                self.brand,
                TerminalName::Unknown | TerminalName::AppleTerminal
            )
    }
}

static CONTEXT: OnceLock<TerminalContext> = OnceLock::new();

pub fn terminal_context() -> TerminalContext {
    *CONTEXT.get_or_init(|| TerminalContext::from_env(&collect_env(), HostOs::current()))
}

#[cfg(test)]
pub(crate) fn env_from(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_program_ghostty() {
        let env = env_from(&[("TERM_PROGRAM", "ghostty")]);
        assert_eq!(detect_terminal_brand_from_env(&env), TerminalName::Ghostty);
    }

    #[test]
    fn kitty_window_id() {
        let env = env_from(&[("KITTY_WINDOW_ID", "1")]);
        assert_eq!(detect_terminal_brand_from_env(&env), TerminalName::Kitty);
    }

    #[test]
    fn tmux_wins_over_zellij() {
        let env = env_from(&[("TMUX", "1"), ("ZELLIJ", "1")]);
        assert_eq!(detect_multiplexer_from_env(&env), MultiplexerKind::Tmux);
    }

    #[test]
    fn cursor_askpass_beats_vscode_term_program() {
        let env = env_from(&[("TERM_PROGRAM", "vscode"), ("CURSOR_TRACE_ID", "abc")]);
        assert_eq!(detect_terminal_brand_from_env(&env), TerminalName::Cursor);
    }

    #[test]
    fn ctrl_dot_unreliable_on_tmux_and_apple_terminal() {
        let tmux = TerminalContext {
            brand: TerminalName::Ghostty,
            multiplexer: MultiplexerKind::Tmux,
            host: HostOs::Macos,
        };
        assert!(tmux.ctrl_dot_unreliable());
        let apple = TerminalContext {
            brand: TerminalName::AppleTerminal,
            multiplexer: MultiplexerKind::Undetected,
            host: HostOs::Macos,
        };
        assert!(apple.ctrl_dot_unreliable());
        let ghostty = TerminalContext {
            brand: TerminalName::Ghostty,
            multiplexer: MultiplexerKind::Undetected,
            host: HostOs::Macos,
        };
        assert!(!ghostty.ctrl_dot_unreliable());
    }
}
