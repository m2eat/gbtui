//! Semantic theme. Domain colors (skill/plan/remember) are not in this struct.
//!
//! Palettes STRIP'd from GrokNight / GrokDay
//! (`xai-grok-pager-render/src/theme/groknight.rs`, `grokday.rs`).

use ratatui::style::Color;

use crate::color_support::{self, ColorLevel};

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// Application-owned extra colors. The framework never reads this.
pub trait ThemeExt {
    fn get(&self, role: &str) -> Option<Color>;
}

/// Core semantic palette. Field whitelist is load-bearing — see `docs/plan.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub bg_base: Color,
    pub bg_elevated: Color,
    pub bg_dark: Color,
    pub bg_highlight: Color,
    pub bg_hover: Color,
    pub bg_visual: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub gray_dim: Color,
    pub gray: Color,
    pub gray_bright: Color,
    pub accent: Color,
    pub success: Color,
    pub error: Color,
    pub warning: Color,
    pub selection_border: Color,
    pub hover_border: Color,
    pub border: Color,
    pub scrollbar_bg: Color,
    pub scrollbar_fg: Color,
    pub button_fg: Color,
    pub button_bg: Color,
    pub button_hover_bg: Color,
    pub input_bg: Color,
    pub input_border: Color,
    pub input_border_active: Color,
    pub fuzzy_accent: Color,
    /// Inline code / fenced-block delimiter (GrokNight `md_code` / BLUE1).
    pub md_code: Color,
    /// Fenced code block background (GrokNight `md_code_bg`).
    pub md_code_bg: Color,
    /// File paths in tool headers (GrokNight `path` / orange).
    pub path: Color,
}

/// Enumerates the load-bearing palette slots once for palette adaptation and
/// semantic theme configuration.
macro_rules! theme_palette_colors {
    ($macro:ident) => {
        $macro! {
            BgBase => bg_base,
            BgElevated => bg_elevated,
            BgDark => bg_dark,
            BgHighlight => bg_highlight,
            BgHover => bg_hover,
            BgVisual => bg_visual,
            TextPrimary => text_primary,
            TextSecondary => text_secondary,
            GrayDim => gray_dim,
            Gray => gray,
            GrayBright => gray_bright,
            Accent => accent,
            Success => success,
            Error => error,
            Warning => warning,
            SelectionBorder => selection_border,
            HoverBorder => hover_border,
            Border => border,
            ScrollbarBg => scrollbar_bg,
            ScrollbarFg => scrollbar_fg,
            ButtonFg => button_fg,
            ButtonBg => button_bg,
            ButtonHoverBg => button_hover_bg,
            InputBg => input_bg,
            InputBorder => input_border,
            InputBorderActive => input_border_active,
            FuzzyAccent => fuzzy_accent,
            MdCode => md_code,
            MdCodeBg => md_code_bg,
            Path => path,
        }
    };
}

pub(crate) use theme_palette_colors;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemeKind {
    #[default]
    Night,
    Day,
    Auto,
}

impl ThemeKind {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "night" | "dark" | "groknight" => Some(Self::Night),
            "day" | "light" | "grokday" => Some(Self::Day),
            "auto" | "system" => Some(Self::Auto),
            _ => None,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Night => "night",
            Self::Day => "day",
            Self::Auto => "auto",
        }
    }
}

impl Theme {
    /// Neutral gray night palette (GrokNight, domain colors dropped).
    pub const fn night() -> Self {
        Self {
            bg_base: rgb(20, 20, 20),
            bg_elevated: rgb(36, 36, 36),
            bg_dark: rgb(28, 28, 28),
            bg_highlight: rgb(36, 36, 36),
            bg_hover: rgb(44, 44, 44),
            bg_visual: rgb(48, 48, 48),
            text_primary: rgb(225, 225, 225),
            text_secondary: rgb(200, 200, 200),
            gray_dim: rgb(88, 88, 88),
            gray: rgb(108, 108, 108),
            gray_bright: rgb(120, 120, 120),
            accent: rgb(122, 162, 247),
            success: rgb(158, 206, 106),
            error: rgb(247, 118, 142),
            warning: rgb(224, 175, 104),
            selection_border: rgb(60, 60, 65),
            hover_border: rgb(30, 30, 34),
            border: rgb(50, 50, 55),
            scrollbar_bg: rgb(17, 17, 17),
            scrollbar_fg: rgb(36, 36, 36),
            button_fg: rgb(225, 225, 225),
            button_bg: rgb(36, 36, 36),
            button_hover_bg: rgb(44, 44, 44),
            input_bg: rgb(20, 20, 20),
            input_border: rgb(50, 50, 55),
            input_border_active: rgb(80, 80, 88),
            fuzzy_accent: rgb(122, 162, 247),
            md_code: rgb(58, 149, 171),
            md_code_bg: rgb(28, 28, 28),
            path: rgb(255, 158, 100),
        }
    }

    /// Neutral gray day palette (GrokDay, domain colors dropped).
    pub const fn day() -> Self {
        Self {
            bg_base: rgb(238, 238, 238),
            bg_elevated: rgb(222, 222, 222),
            bg_dark: rgb(228, 228, 228),
            bg_highlight: rgb(222, 222, 222),
            bg_hover: rgb(208, 208, 208),
            bg_visual: rgb(198, 198, 198),
            text_primary: rgb(38, 38, 38),
            text_secondary: rgb(68, 68, 68),
            gray_dim: rgb(165, 165, 165),
            gray: rgb(118, 118, 118),
            gray_bright: rgb(98, 98, 98),
            accent: rgb(47, 100, 210),
            success: rgb(55, 142, 35),
            error: rgb(205, 48, 72),
            warning: rgb(162, 118, 18),
            selection_border: rgb(185, 185, 190),
            hover_border: rgb(212, 212, 216),
            border: rgb(200, 200, 205),
            scrollbar_bg: rgb(234, 234, 234),
            scrollbar_fg: rgb(222, 222, 222),
            button_fg: rgb(38, 38, 38),
            button_bg: rgb(222, 222, 222),
            button_hover_bg: rgb(208, 208, 208),
            input_bg: rgb(238, 238, 238),
            input_border: rgb(200, 200, 205),
            input_border_active: rgb(165, 165, 175),
            fuzzy_accent: rgb(47, 100, 210),
            md_code: rgb(47, 111, 168),
            md_code_bg: rgb(228, 228, 228),
            path: rgb(180, 90, 40),
        }
    }

    /// Quantize every palette slot for a terminal color capability.
    ///
    /// For the built-in palettes, prefer [`Self::adapted`] so Basic-color
    /// terminals retain chrome contrast after quantization.
    pub fn quantized(self, level: ColorLevel) -> Self {
        let q = |c: Color| color_support::quantize_color(c, level);
        macro_rules! quantize_palette {
            ($($variant:ident => $field:ident,)*) => {
                Self {
                    $($field: q(self.$field),)*
                }
            };
        }
        theme_palette_colors!(quantize_palette)
    }

    /// Adapt this palette for a terminal color capability.
    ///
    /// This quantizes every color, then pins Basic-color chrome to a compact
    /// ANSI16 hierarchy so background, hover, selection, borders, text, and
    /// scrollbar thumbs remain distinguishable. `None` deliberately leaves
    /// every slot as [`Color::Reset`].
    ///
    /// Built-in theme definitions use this after choosing their palette.
    /// Explicit runtime palettes deliberately retain full ownership and are not
    /// implicitly adapted.
    pub fn adapted(self, level: ColorLevel) -> Self {
        if level == ColorLevel::Basic {
            self.adapted_for(level, self.is_dark())
        } else {
            self.quantized(level)
        }
    }

    pub(crate) fn adapted_for(self, level: ColorLevel, dark: bool) -> Self {
        let quantized = self.quantized(level);
        if level != ColorLevel::Basic {
            return quantized;
        }
        let mut adapted = quantized.ansi16_chrome_overrides(dark);
        // An explicit terminal-default channel must survive contrast adaptation.
        macro_rules! preserve_resets {
            ($($variant:ident => $field:ident,)*) => {
                $(if self.$field == Color::Reset {
                    adapted.$field = Color::Reset;
                })*
            };
        }
        theme_palette_colors!(preserve_resets);
        adapted
    }

    fn is_dark(self) -> bool {
        let (r, g, b) = match self.bg_base {
            Color::Reset => return true,
            Color::Black => (0, 0, 0),
            Color::Red => (128, 0, 0),
            Color::Green => (0, 128, 0),
            Color::Yellow => (128, 128, 0),
            Color::Blue => (0, 0, 128),
            Color::Magenta => (128, 0, 128),
            Color::Cyan => (0, 128, 128),
            Color::Gray => (192, 192, 192),
            Color::DarkGray => (128, 128, 128),
            Color::LightRed => (255, 0, 0),
            Color::LightGreen => (0, 255, 0),
            Color::LightYellow => (255, 255, 0),
            Color::LightBlue => (0, 0, 255),
            Color::LightMagenta => (255, 0, 255),
            Color::LightCyan => (0, 255, 255),
            Color::White => (255, 255, 255),
            Color::Indexed(index) => crate::color::indexed_to_rgb(index),
            Color::Rgb(r, g, b) => (r, g, b),
        };
        299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b) < 128_000
    }

    fn ansi16_chrome_overrides(self, dark: bool) -> Self {
        let canvas = if dark { Color::Black } else { Color::White };
        let elevated = if dark { Color::DarkGray } else { Color::Gray };
        let text = if dark { Color::White } else { Color::Black };
        let muted = if dark { Color::Gray } else { Color::DarkGray };
        let hovered = if dark {
            Color::Blue
        } else {
            Color::LightYellow
        };
        let selected = if dark {
            Color::LightBlue
        } else {
            Color::LightCyan
        };
        let dim = if dark { Color::DarkGray } else { Color::Gray };
        let red = if dark { Color::LightRed } else { Color::Red };
        let green = if dark {
            Color::LightGreen
        } else {
            Color::Green
        };
        let yellow = if dark {
            Color::LightYellow
        } else {
            Color::Yellow
        };
        let blue = if dark { Color::LightBlue } else { Color::Blue };
        let cyan = if dark { Color::LightCyan } else { Color::Cyan };

        Self {
            bg_base: canvas,
            bg_elevated: elevated,
            bg_dark: canvas,
            bg_highlight: elevated,
            bg_hover: hovered,
            bg_visual: selected,
            text_primary: text,
            text_secondary: muted,
            gray_dim: dim,
            gray: muted,
            gray_bright: muted,
            accent: blue,
            success: green,
            error: red,
            warning: yellow,
            selection_border: text,
            hover_border: muted,
            border: muted,
            scrollbar_bg: canvas,
            scrollbar_fg: muted,
            button_fg: text,
            button_bg: elevated,
            button_hover_bg: hovered,
            input_bg: canvas,
            input_border: muted,
            input_border_active: text,
            fuzzy_accent: blue,
            md_code: cyan,
            md_code_bg: canvas,
            path: yellow,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::night()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(theme: Theme) -> [Color; 30] {
        [
            theme.bg_base,
            theme.bg_elevated,
            theme.bg_dark,
            theme.bg_highlight,
            theme.bg_hover,
            theme.bg_visual,
            theme.text_primary,
            theme.text_secondary,
            theme.gray_dim,
            theme.gray,
            theme.gray_bright,
            theme.accent,
            theme.success,
            theme.error,
            theme.warning,
            theme.selection_border,
            theme.hover_border,
            theme.border,
            theme.scrollbar_bg,
            theme.scrollbar_fg,
            theme.button_fg,
            theme.button_bg,
            theme.button_hover_bg,
            theme.input_bg,
            theme.input_border,
            theme.input_border_active,
            theme.fuzzy_accent,
            theme.md_code,
            theme.md_code_bg,
            theme.path,
        ]
    }

    #[test]
    fn basic_adaptation_keeps_builtin_chrome_readable() {
        let night = Theme::night().adapted(ColorLevel::Basic);
        assert_eq!(night.bg_base, Color::Black);
        assert_ne!(night.bg_hover, night.bg_elevated);
        assert_ne!(night.bg_visual, night.bg_hover);
        assert_eq!(night.text_primary, Color::White);
        assert_ne!(night.bg_base, night.bg_visual);
        assert_ne!(night.bg_base, night.bg_hover);
        assert_ne!(night.bg_base, night.border);
        assert_ne!(night.bg_base, night.scrollbar_fg);
        assert_ne!(night.text_primary, night.bg_visual);

        let day = Theme::day().adapted(ColorLevel::Basic);
        assert_eq!(day.bg_base, Color::White);
        assert_ne!(day.bg_hover, day.bg_elevated);
        assert_ne!(day.bg_visual, day.bg_hover);
        assert_eq!(day.text_primary, Color::Black);
        assert_ne!(day.bg_base, day.bg_visual);
        assert_ne!(day.bg_base, day.bg_hover);
        assert_ne!(day.bg_base, day.border);
        assert_ne!(day.bg_base, day.scrollbar_fg);
        assert_ne!(day.text_primary, day.bg_visual);

        for color in colors(night).into_iter().chain(colors(day)) {
            assert!(
                !matches!(color, Color::Rgb(_, _, _) | Color::Indexed(_)),
                "Basic adaptation emitted non-ANSI color {color:?}"
            );
        }
    }

    #[test]
    fn no_color_adaptation_resets_every_palette_slot() {
        for theme in [Theme::night(), Theme::day()] {
            for color in colors(theme.adapted(ColorLevel::None)) {
                assert_eq!(color, Color::Reset);
            }
        }
    }

    #[test]
    fn truecolor_adaptation_preserves_the_palette() {
        assert_eq!(
            Theme::night().adapted(ColorLevel::TrueColor),
            Theme::night()
        );
    }

    #[test]
    fn kind_parses_aliases() {
        assert_eq!(ThemeKind::from_name("dark"), Some(ThemeKind::Night));
        assert_eq!(ThemeKind::from_name("light"), Some(ThemeKind::Day));
        assert_eq!(ThemeKind::from_name("auto"), Some(ThemeKind::Auto));
    }
}
