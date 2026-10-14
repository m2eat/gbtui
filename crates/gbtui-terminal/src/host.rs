//! Host OS classification.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/host/mod.rs (STRIP)`

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum HostOs {
    Macos,
    Linux,
    Windows,
    #[default]
    Other,
}

impl HostOs {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Other
        }
    }
}
