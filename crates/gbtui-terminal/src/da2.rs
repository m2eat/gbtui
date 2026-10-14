//! Secondary Device Attributes (DA2) response parsing for Alacritty.

/// DA2 query understood by Alacritty.
pub const QUERY: &[u8] = b"\x1b[>0c";

/// The largest valid packed `alacritty_terminal` version.
const MAX_PACKED_VERSION: u32 = 999_999;

/// The `alacritty_terminal` version reported by a DA2 response.
///
/// Alacritty packs `major * 10_000 + minor * 100 + patch`; this is the library
/// version, rather than the application release version.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Da2Version {
    packed: u32,
}

impl Da2Version {
    /// Parses Alacritty's exact `CSI > 0 ; Pv ; 1 c` reply.
    pub fn parse(reply: &[u8]) -> Option<Self> {
        let body = reply.strip_prefix(b"\x1b[>0;")?.strip_suffix(b";1c")?;
        if body.is_empty() || !body.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let packed = std::str::from_utf8(body).ok()?.parse().ok()?;
        Self::from_packed(packed)
    }

    /// Creates a version from Alacritty's documented packed representation.
    pub const fn from_packed(packed: u32) -> Option<Self> {
        if packed == 0 || packed > MAX_PACKED_VERSION {
            return None;
        }
        Some(Self { packed })
    }

    /// Returns Alacritty's original packed value.
    pub const fn packed(self) -> u32 {
        self.packed
    }

    /// Returns the unpacked major, minor, and patch fields.
    pub const fn components(self) -> (u32, u32, u32) {
        (
            self.packed / 10_000,
            (self.packed / 100) % 100,
            self.packed % 100,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_alacritty_da2_shape() {
        let version = Da2Version::parse(b"\x1b[>0;2401;1c").expect("Alacritty reply");
        assert_eq!(version.packed(), 2401);
        assert_eq!(version.components(), (0, 24, 1));

        for reply in [
            b"\x1b[>41;389;0c".as_slice(),
            b"\x1b[>0;+2401;1c",
            b"\x1b[>0;2401;0c",
            b"\x1b[>0;0;1c",
            b"\x1b[>0;1000000;1c",
            b"\x1b[>0;2401;1c trailing",
        ] {
            assert_eq!(Da2Version::parse(reply), None, "{reply:?}");
        }
    }
}
