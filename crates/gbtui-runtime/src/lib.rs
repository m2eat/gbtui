//! Application runtime: event loop, [`FrameCtx`], hit testing, animation ticks.
//!
//! This crate **must not** depend on `gbtui-widgets`.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/app/{event_loop,mouse,agent_view HitArea} (REWRITE)` — P3

#![forbid(unsafe_code)]

pub use gbtui_core as core;
pub use gbtui_input as input;
pub use gbtui_terminal as terminal;

mod app;
mod frame;
mod hitmap;
mod pointer;
mod pump;
mod run;

pub use app::{App, AppEvent, Outcome};
pub use frame::FrameCtx;
pub use hitmap::{HitMap, HitResult};
pub use pointer::{MULTI_CLICK_TIMEOUT_MS, Pointer};
pub use pump::{Pump, RESIZE_DEBOUNCE};
pub use run::{RuntimeTheme, run, run_blocking, run_inline, run_inline_blocking};

/// Crate version, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;
