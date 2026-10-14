//! Optional generic action registry (keymap + shortcut hints).
//!
//! Applications define their own action id and context types. This crate
//! must not ship product enums (`SendPrompt`, …).
//!
//! Depends only on [`gbtui_core`] (plus crossterm for `KeyEvent`) so it
//! cannot form a cycle with widgets.
//!
//! # Extraction
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/actions/ (structure only, NEVER copy ActionId)` — P6
//!
//! ```text
//! // bubbling is written at the call site, not inside lookup:
//! if let Some(id) = registry.lookup(&key, Ctx::Overlay) { … }
//! else if let Some(id) = registry.lookup(&key, Ctx::Home) { … }
//! else if let Some(id) = registry.lookup(&key, Ctx::Always) { … }
//! ```

#![forbid(unsafe_code)]

pub use gbtui_core as core;

mod registry;

pub use registry::{ActionDef, ActionRegistry, UnreliableKey};

/// Crate version, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles() {
        assert!(!super::VERSION.is_empty());
    }
}
