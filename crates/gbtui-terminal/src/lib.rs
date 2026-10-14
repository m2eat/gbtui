//! Terminal capabilities and the inline-viewport `Terminal` fork.
//!
//! `SOURCE: crates/codegen/xai-ratatui-inline/** (COPY)` — P1b
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/terminal/** (STRIP)` — P2b
//!
//! The inline Terminal fork uses a small amount of `unsafe`. Do not add
//! new `unsafe` outside `inline/`.

pub use gbtui_core as core;

mod da2;
mod probe;
mod xtversion;

pub mod appearance;
pub mod brand;
pub mod glyphs;
pub mod host;
pub mod hyperlinks;
#[cfg(feature = "images")]
pub mod image;
pub mod inline;
pub mod keyboard;
pub mod kitty;
pub mod tmux;

pub use appearance::{
    BackgroundColor, SystemAppearance, detect as detect_appearance,
    detect_with_background as detect_appearance_with_probe,
};
pub use brand::{
    MultiplexerKind, TerminalContext, TerminalName, detect_multiplexer_from_env,
    detect_terminal_brand_from_env, terminal_context,
};
pub use da2::Da2Version;
pub use host::HostOs;
pub use hyperlinks::{HyperlinkCapabilities, Osc8Support, SchemeFilter, hyperlink_capabilities};
pub use inline::*;
pub use keyboard::{
    KeyboardCapabilities, ModifierDelivery, ModifierFate, keyboard_capabilities,
    keyboard_capabilities_for_host,
};
pub use kitty::{flags_for_context, negotiated_kitty_flags};
pub use probe::{
    ProbeEventDecoder, ProbeRead, StartupProbe, StartupProbeConfig, StartupProbeIo,
    StartupProbeOutcome, TerminalInfo,
};
#[cfg(unix)]
pub use probe::{StartupProbeSession, UnixInput};
pub use tmux::{passthrough_available, should_wrap_osc11, tmux_passthrough, tmux_passthrough_str};
pub use xtversion::XtVersion;

/// Crate version, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
