//! Keyboard and pointer input normalization.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/input/** (STRIP)` — P2c
//!
//! macOS CoreGraphics probe is `unsafe` (FFI). Do not add new `unsafe`
//! outside `macos_modifiers`.

pub use gbtui_core as core;
pub use gbtui_terminal as terminal;

pub mod enter;
#[cfg(target_os = "macos")]
pub mod macos_modifiers;
pub mod normalizer;
pub mod scroll;
pub mod typeahead;

pub use enter::{is_apple_terminal_newline_modifier_held, is_mod_enter};
pub use normalizer::{KeyboardNormalizer, ModifierProbe, ModifierState, OsModifierProbe};
pub use scroll::{MouseScrollState, ScrollConfig, ScrollDirection, ScrollInputMode, ScrollUpdate};
pub use typeahead::{filter_startup_typeahead, is_typeahead_event};

/// Crate version, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
