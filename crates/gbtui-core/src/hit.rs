//! Stable identifier for a hit-testable region.
//!
//! Used by `gbtui-runtime` HitMap (P3). Defined here so widgets can name
//! targets without depending on the runtime.

/// Opaque hit-test id. Do not reuse an id that is still on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HitId(u64);

impl HitId {
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Pack a namespace + index (widget-internal row / tab / shortcut ids).
    pub const fn namespaced(ns: u32, index: u32) -> Self {
        Self(((ns as u64) << 32) | index as u64)
    }

    pub const fn namespace(self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub const fn index(self) -> u32 {
        self.0 as u32
    }

    /// Stable id from a `&'static str` (FNV-1a). Prefer this over ad-hoc numbers.
    pub const fn named(name: &'static str) -> Self {
        let mut hash = 0xcbf29ce484222325u64;
        let bytes = name.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            hash ^= bytes[i] as u64;
            hash = hash.wrapping_mul(0x100000001b3);
            i += 1;
        }
        Self(hash)
    }
}

impl From<u64> for HitId {
    fn from(raw: u64) -> Self {
        Self(raw)
    }
}
