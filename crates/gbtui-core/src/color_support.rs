//! Terminal color-depth detection and quantization.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/theme/color_support.rs (STRIP)`
//!
//! Brand-based truecolor upgrades live in `gbtui-terminal`. This crate only
//! reads `NO_COLOR`, `GBTUI_FORCE_COLOR_LEVEL`, and `supports-color`.

use std::sync::OnceLock;

use ratatui::style::Color;

use crate::color::{indexed_to_ansi16, nearest_indexed};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ColorLevel {
    None,
    Basic,
    Ansi256,
    TrueColor,
}

impl ColorLevel {
    pub fn has_color(self) -> bool {
        self >= Self::Basic
    }

    pub fn has_256(self) -> bool {
        self >= Self::Ansi256
    }

    pub fn has_truecolor(self) -> bool {
        self >= Self::TrueColor
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Basic => "basic",
            Self::Ansi256 => "256",
            Self::TrueColor => "truecolor",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "basic" | "16" => Some(Self::Basic),
            "256" | "ansi256" => Some(Self::Ansi256),
            "truecolor" | "24bit" | "16m" => Some(Self::TrueColor),
            _ => None,
        }
    }
}

static COLOR_LEVEL: OnceLock<ColorLevel> = OnceLock::new();

/// Detect and cache the terminal color level.
pub fn detect() -> ColorLevel {
    *COLOR_LEVEL.get_or_init(detect_uncached)
}

/// Override the cached level (tests / `GBTUI_FORCE_COLOR_LEVEL`).
pub fn set(level: ColorLevel) -> Result<(), ColorLevel> {
    COLOR_LEVEL.set(level)
}

pub fn get() -> ColorLevel {
    detect()
}

pub fn detect_uncached() -> ColorLevel {
    if std::env::var_os("NO_COLOR").is_some() {
        return ColorLevel::None;
    }
    if let Ok(forced) = std::env::var("GBTUI_FORCE_COLOR_LEVEL")
        && let Some(level) = ColorLevel::from_name(&forced)
    {
        return level;
    }
    match supports_color::on(supports_color::Stream::Stdout) {
        Some(level) if level.has_16m => ColorLevel::TrueColor,
        Some(level) if level.has_256 => ColorLevel::Ansi256,
        Some(level) if level.has_basic => ColorLevel::Basic,
        Some(_) | None => ColorLevel::TrueColor,
    }
}

pub fn quantize_color(color: Color, level: ColorLevel) -> Color {
    match level {
        ColorLevel::TrueColor => color,
        ColorLevel::Ansi256 => match color {
            Color::Rgb(r, g, b) => Color::Indexed(nearest_indexed(r, g, b)),
            other => other,
        },
        ColorLevel::Basic => match color {
            Color::Rgb(r, g, b) => indexed_to_ansi16(nearest_indexed(r, g, b)),
            Color::Indexed(n) => indexed_to_ansi16(n),
            other => other,
        },
        ColorLevel::None => Color::Reset,
    }
}

pub fn quantize(color: Color) -> Color {
    quantize_color(color, get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_does_not_emit_rgb() {
        let rgb = Color::Rgb(122, 162, 247);
        match quantize_color(rgb, ColorLevel::Basic) {
            Color::Rgb(_, _, _) => panic!("16-color quantize must not keep Rgb"),
            Color::Indexed(_) => panic!("16-color quantize must not keep Indexed"),
            _ => {}
        }
    }

    #[test]
    fn none_becomes_reset() {
        assert_eq!(
            quantize_color(Color::Rgb(1, 2, 3), ColorLevel::None),
            Color::Reset
        );
    }

    #[test]
    fn truecolor_passthrough() {
        let c = Color::Rgb(1, 2, 3);
        assert_eq!(quantize_color(c, ColorLevel::TrueColor), c);
    }

    #[test]
    fn ansi256_maps_rgb_to_indexed() {
        match quantize_color(Color::Rgb(255, 0, 0), ColorLevel::Ansi256) {
            Color::Indexed(_) => {}
            other => panic!("expected Indexed, got {other:?}"),
        }
    }
}
