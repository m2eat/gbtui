//! System dark/light detection and safe OSC 11 background parsing.

/// A true-color terminal background reported by OSC 11.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackgroundColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl BackgroundColor {
    /// Parses a complete OSC 11 response terminated by BEL or ST.
    pub fn parse_osc11_reply(reply: &[u8]) -> Option<Self> {
        let payload = reply.strip_prefix(b"\x1b]11;")?;
        let payload = payload
            .strip_suffix(b"\x07")
            .or_else(|| payload.strip_suffix(b"\x1b\\"))?;
        if let Some(rgb) = payload.strip_prefix(b"rgb:") {
            let mut components = rgb.split(|byte| *byte == b'/');
            let color = Self {
                red: parse_osc_component(components.next()?)?,
                green: parse_osc_component(components.next()?)?,
                blue: parse_osc_component(components.next()?)?,
            };
            return components.next().is_none().then_some(color);
        }
        let hex = payload.strip_prefix(b"#")?;
        if hex.len() != 6 {
            return None;
        }
        Some(Self {
            red: hex_pair(&hex[..2])?,
            green: hex_pair(&hex[2..4])?,
            blue: hex_pair(&hex[4..])?,
        })
    }

    /// Returns the contrast-preserving appearance implied by this background.
    pub const fn appearance(self) -> SystemAppearance {
        let luminance = 299 * self.red as u32 + 587 * self.green as u32 + 114 * self.blue as u32;
        if luminance < 128_000 {
            SystemAppearance::Dark
        } else {
            SystemAppearance::Light
        }
    }
}

fn parse_osc_component(component: &[u8]) -> Option<u8> {
    if component.is_empty() || component.len() > 4 {
        return None;
    }
    let mut value = 0_u32;
    for byte in component {
        value = value.checked_mul(16)? + u32::from(hex_value(*byte)?);
    }
    let scale = (1_u32 << (4 * component.len())) - 1;
    Some(((value * 255 + scale / 2) / scale) as u8)
}

fn hex_pair(bytes: &[u8]) -> Option<u8> {
    Some(hex_value(*bytes.first()?)? * 16 + hex_value(*bytes.get(1)?)?)
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemAppearance {
    Light,
    Dark,
}

impl SystemAppearance {
    pub const fn as_env_value(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

/// Chooses appearance in priority order: explicit environment override, a
/// terminal background obtained during startup probing, then the OS setting.
pub fn select(
    env_override: Option<&str>,
    probed_background: Option<BackgroundColor>,
    os_appearance: Option<SystemAppearance>,
) -> Option<SystemAppearance> {
    if let Some(appearance) = env_override.and_then(parse_override) {
        return Some(appearance);
    }
    probed_background
        .map(BackgroundColor::appearance)
        .or(os_appearance)
}

/// Detects appearance using the process environment and OS setting, without a
/// terminal query. See [`detect_with_background`] after startup probing.
pub fn detect() -> Option<SystemAppearance> {
    detect_with_background(None)
}

/// Detects appearance using the process environment, a parsed OSC 11
/// background, and the OS setting in that order.
pub fn detect_with_background(
    probed_background: Option<BackgroundColor>,
) -> Option<SystemAppearance> {
    select(
        std::env::var("GBTUI_APPEARANCE").ok().as_deref(),
        probed_background,
        None,
    )
    .or_else(detect_os)
}

fn parse_override(value: &str) -> Option<SystemAppearance> {
    match value.trim().to_ascii_lowercase().as_str() {
        "dark" => Some(SystemAppearance::Dark),
        "light" => Some(SystemAppearance::Light),
        _ => None,
    }
}

fn detect_os() -> Option<SystemAppearance> {
    match dark_light::detect() {
        Ok(dark_light::Mode::Dark) => Some(SystemAppearance::Dark),
        Ok(dark_light::Mode::Light) => Some(SystemAppearance::Light),
        Ok(dark_light::Mode::Unspecified) | Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc11_accepts_bounded_rgb_and_rejects_controls() {
        assert_eq!(
            BackgroundColor::parse_osc11_reply(b"\x1b]11;rgb:0000/8000/ffff\x1b\\"),
            Some(BackgroundColor {
                red: 0,
                green: 128,
                blue: 255,
            })
        );
        assert_eq!(
            BackgroundColor::parse_osc11_reply(b"\x1b]11;#123456\x07"),
            Some(BackgroundColor {
                red: 0x12,
                green: 0x34,
                blue: 0x56,
            })
        );
        assert_eq!(
            BackgroundColor::parse_osc11_reply(b"\x1b]11;rgb:0/0/\x1b\x07"),
            None
        );
    }

    #[test]
    fn explicit_override_outranks_probe_then_os() {
        let light_background = BackgroundColor {
            red: 255,
            green: 255,
            blue: 255,
        };
        assert_eq!(
            select(
                Some(" dark "),
                Some(light_background),
                Some(SystemAppearance::Light)
            ),
            Some(SystemAppearance::Dark)
        );
        assert_eq!(
            select(None, Some(light_background), Some(SystemAppearance::Dark)),
            Some(SystemAppearance::Light)
        );
        assert_eq!(
            select(None, None, Some(SystemAppearance::Dark)),
            Some(SystemAppearance::Dark)
        );
    }

    #[test]
    fn env_value_roundtrip() {
        assert_eq!(SystemAppearance::Dark.as_env_value(), "dark");
        assert_eq!(SystemAppearance::Light.as_env_value(), "light");
    }
}
