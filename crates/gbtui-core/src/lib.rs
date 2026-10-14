//! Core primitives for the gbtui terminal UI runtime.
//!
//! This crate is the **dependency root**. It must not depend on any other
//! `gbtui-*` crate.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/{input/key.rs, render/*, theme/*} (STRIP)`

#![forbid(unsafe_code)]

pub mod color;
pub mod color_support;
pub mod hit;
pub mod key;
pub mod line_utils;
pub mod renderable;
pub mod safe_buf;
pub mod search;
pub mod theme;
pub mod theme_system;
pub mod tick;
pub mod wrapping;

/// The event types used by gbtui, also resolved through `$crate` in [`key!`].
pub use crossterm;
/// The rendering types used by gbtui.
pub use ratatui;

pub use color_support::{ColorLevel, quantize, quantize_color};
pub use hit::HitId;
pub use key::{
    KeyShortcut, RowWalk, is_inline_paste_key, is_paste_key, is_shift_tab, is_text_input_key,
};
pub use renderable::Renderable;
pub use safe_buf::SafeBuf;
pub use search::{TextMatcher, next_index_after, prev_index_before};
pub use theme::{Theme, ThemeExt, ThemeKind};
#[cfg(feature = "theme-config")]
pub use theme_system::ThemeConfigError;
pub use theme_system::{
    BackgroundPolicy, ResolvedTheme, ThemeAppearance, ThemeColor, ThemeContext, ThemeDefinition,
    ThemeRegistry, ThemeResolver, ThemeRole, ThemeState,
};
pub use tick::{AnimationClock, TickDemand, pulse_brightness, wave_brightness};

/// Crate version, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
