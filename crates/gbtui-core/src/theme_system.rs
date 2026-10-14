//! Resolved semantic themes and terminal-aware palette policy.

use std::collections::BTreeMap;

use ratatui::style::{Color, Modifier, Style};

use crate::color::indexed_to_rgb;
use crate::color_support::{self, ColorLevel};
use crate::theme::{Theme, ThemeKind, theme_palette_colors};

/// How palette surfaces interact with the terminal's own background.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BackgroundPolicy {
    #[default]
    Solid,
    TerminalDefault,
    Transparent,
}

/// Appearance supplied by the host terminal or operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeAppearance {
    Light,
    Dark,
}

/// Terminal facts used to resolve a theme definition. Unknown appearance falls
/// back to the dark palette for [`ThemeKind::Auto`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeContext {
    pub appearance: Option<ThemeAppearance>,
    pub color_level: ColorLevel,
    pub terminal_foreground: Option<Color>,
    pub terminal_background: Option<Color>,
}

impl Default for ThemeContext {
    fn default() -> Self {
        Self {
            appearance: None,
            color_level: ColorLevel::TrueColor,
            terminal_foreground: None,
            terminal_background: None,
        }
    }
}

/// A semantic style role independent of a widget implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum ThemeRole {
    Surface,
    Elevated,
    Text,
    Muted,
    Border,
    Input,
    Button,
    Selection,
    Hover,
    Scrollbar,
    Accent,
    Success,
    Warning,
    Error,
    Link,
    InlineCode,
    CodeBlock,
    DiffAdded,
    DiffRemoved,
    DiffHeader,
    Path,
    Fuzzy,
}

impl ThemeRole {
    pub const ALL: &'static [Self] = &[
        Self::Surface,
        Self::Elevated,
        Self::Text,
        Self::Muted,
        Self::Border,
        Self::Input,
        Self::Button,
        Self::Selection,
        Self::Hover,
        Self::Scrollbar,
        Self::Accent,
        Self::Success,
        Self::Warning,
        Self::Error,
        Self::Link,
        Self::InlineCode,
        Self::CodeBlock,
        Self::DiffAdded,
        Self::DiffRemoved,
        Self::DiffHeader,
        Self::Path,
        Self::Fuzzy,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Surface => "surface",
            Self::Elevated => "elevated",
            Self::Text => "text",
            Self::Muted => "muted",
            Self::Border => "border",
            Self::Input => "input",
            Self::Button => "button",
            Self::Selection => "selection",
            Self::Hover => "hover",
            Self::Scrollbar => "scrollbar",
            Self::Accent => "accent",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Link => "link",
            Self::InlineCode => "inline_code",
            Self::CodeBlock => "code_block",
            Self::DiffAdded => "diff_added",
            Self::DiffRemoved => "diff_removed",
            Self::DiffHeader => "diff_header",
            Self::Path => "path",
            Self::Fuzzy => "fuzzy",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim() {
            "surface" => Some(Self::Surface),
            "elevated" => Some(Self::Elevated),
            "text" => Some(Self::Text),
            "muted" => Some(Self::Muted),
            "border" => Some(Self::Border),
            "input" => Some(Self::Input),
            "button" => Some(Self::Button),
            "selection" => Some(Self::Selection),
            "hover" => Some(Self::Hover),
            "scrollbar" => Some(Self::Scrollbar),
            "accent" => Some(Self::Accent),
            "success" => Some(Self::Success),
            "warning" => Some(Self::Warning),
            "error" => Some(Self::Error),
            "link" => Some(Self::Link),
            "inline_code" => Some(Self::InlineCode),
            "code_block" => Some(Self::CodeBlock),
            "diff_added" => Some(Self::DiffAdded),
            "diff_removed" => Some(Self::DiffRemoved),
            "diff_header" => Some(Self::DiffHeader),
            "path" => Some(Self::Path),
            "fuzzy" => Some(Self::Fuzzy),
            _ => None,
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::from_name(name)
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// The state of a semantic role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum ThemeState {
    Normal,
    Hovered,
    Focused,
    Selected,
    Invalid,
    Disabled,
}

impl ThemeState {
    pub const ALL: &'static [Self] = &[
        Self::Normal,
        Self::Hovered,
        Self::Focused,
        Self::Selected,
        Self::Invalid,
        Self::Disabled,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Hovered => "hovered",
            Self::Focused => "focused",
            Self::Selected => "selected",
            Self::Invalid => "invalid",
            Self::Disabled => "disabled",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim() {
            "normal" => Some(Self::Normal),
            "hovered" => Some(Self::Hovered),
            "focused" => Some(Self::Focused),
            "selected" => Some(Self::Selected),
            "invalid" => Some(Self::Invalid),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::from_name(name)
    }

    /// Maps widget flags to one unambiguous style state.
    pub const fn from_flags(
        disabled: bool,
        invalid: bool,
        selected: bool,
        focused: bool,
        hovered: bool,
    ) -> Self {
        if disabled {
            Self::Disabled
        } else if invalid {
            Self::Invalid
        } else if selected {
            Self::Selected
        } else if focused {
            Self::Focused
        } else if hovered {
            Self::Hovered
        } else {
            Self::Normal
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

const ROLE_COUNT: usize = ThemeRole::ALL.len();
const STATE_COUNT: usize = ThemeState::ALL.len();
const STYLE_COUNT: usize = ROLE_COUNT * STATE_COUNT;

macro_rules! define_theme_color {
    ($($variant:ident => $field:ident,)*) => {
        /// A configurable field in [`Theme`].
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(usize)]
        pub enum ThemeColor {
            $($variant,)*
        }

        impl ThemeColor {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];
            pub const COUNT: usize = Self::ALL.len();

            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => stringify!($field),)*
                }
            }

            pub fn from_name(name: &str) -> Option<Self> {
                let name = name.trim();
                $(if name == stringify!($field) {
                    return Some(Self::$variant);
                })*
                None
            }

            pub fn parse(name: &str) -> Option<Self> {
                Self::from_name(name)
            }

            pub const fn get(self, palette: &Theme) -> Color {
                match self {
                    $(Self::$variant => palette.$field,)*
                }
            }

            pub fn set(self, palette: &mut Theme, color: Color) {
                match self {
                    $(Self::$variant => palette.$field = color,)*
                }
            }

        }
    };
}

theme_palette_colors!(define_theme_color);

/// A named palette source plus sparse color and semantic-style overrides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeDefinition {
    name: String,
    kind: ThemeKind,
    palette: Option<Theme>,
    background: BackgroundPolicy,
    colors: Vec<(ThemeColor, Color)>,
    styles: Vec<(ThemeRole, ThemeState, Style)>,
}

impl ThemeDefinition {
    pub fn new(name: impl Into<String>, kind: ThemeKind) -> Self {
        Self {
            name: name.into(),
            kind,
            palette: None,
            background: BackgroundPolicy::Solid,
            colors: Vec::new(),
            styles: Vec::new(),
        }
    }

    /// Starts a definition from an application-owned palette. It is adapted
    /// for the active terminal context, unlike [`ThemeResolver::explicit`].
    pub fn from_palette(name: impl Into<String>, palette: Theme) -> Self {
        Self {
            name: name.into(),
            kind: ThemeKind::Night,
            palette: Some(palette),
            background: BackgroundPolicy::Solid,
            colors: Vec::new(),
            styles: Vec::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn kind(&self) -> ThemeKind {
        self.kind
    }

    pub const fn background(&self) -> BackgroundPolicy {
        self.background
    }

    pub fn with_background(mut self, background: BackgroundPolicy) -> Self {
        self.background = background;
        self
    }

    pub fn with_color(mut self, color: ThemeColor, value: Color) -> Self {
        if let Some((_, current)) = self.colors.iter_mut().find(|(key, _)| *key == color) {
            *current = value;
        } else {
            self.colors.push((color, value));
        }
        self
    }

    pub fn with_style(mut self, role: ThemeRole, state: ThemeState, style: Style) -> Self {
        if let Some((_, _, current)) = self
            .styles
            .iter_mut()
            .find(|(r, s, _)| *r == role && *s == state)
        {
            *current = style;
        } else {
            self.styles.push((role, state, style));
        }
        self
    }
}

impl From<ThemeKind> for ThemeDefinition {
    fn from(kind: ThemeKind) -> Self {
        Self::new(kind.display_name(), kind)
    }
}

impl Theme {
    /// Returns the semantic fallback style for a raw palette.
    pub fn style(&self, role: ThemeRole, state: ThemeState) -> Style {
        semantic_style(self, role, state)
    }
}

/// A fully quantized theme. Resolving is done only on configuration changes;
/// style lookup during a frame is an array access and a `Copy` return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTheme {
    palette: Theme,
    styles: [Style; STYLE_COUNT],
    version: u64,
    name: String,
    background: BackgroundPolicy,
    context: ThemeContext,
    appearance: ThemeAppearance,
}

impl ResolvedTheme {
    pub fn palette(&self) -> &Theme {
        &self.palette
    }

    pub fn style(&self, role: ThemeRole, state: ThemeState) -> Style {
        self.styles[style_index(role, state)]
    }

    /// A monotonically increasing generation scoped to the resolver that
    /// created this value. Compare full resolved themes across resolver owners.
    pub const fn version(&self) -> u64 {
        self.version
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn background(&self) -> BackgroundPolicy {
        self.background
    }

    pub const fn context(&self) -> &ThemeContext {
        &self.context
    }

    /// Effective theme appearance, before transparency or color quantization.
    /// Explicit day/night definitions take precedence over terminal facts.
    pub const fn appearance(&self) -> ThemeAppearance {
        self.appearance
    }

    /// Computes an estimated WCAG contrast ratio for the resolved style. ANSI
    /// colors use the standard RGB approximation. A terminal `Reset`
    /// foreground or background has no stable RGB value without the matching
    /// value in [`ThemeContext`], and then returns `None`.
    pub fn contrast(&self, role: ThemeRole, state: ThemeState) -> Option<f64> {
        let style = self.style(role, state);
        let foreground = style.fg.unwrap_or(self.palette.text_primary);
        let background = style.bg.unwrap_or(self.palette.bg_base);
        let foreground = color_rgb(foreground, self.context.terminal_foreground)?;
        let background = color_rgb(background, self.context.terminal_background)?;
        let foreground_luminance = relative_luminance(foreground);
        let background_luminance = relative_luminance(background);
        let lighter = foreground_luminance.max(background_luminance);
        let darker = foreground_luminance.min(background_luminance);
        Some((lighter + 0.05) / (darker + 0.05))
    }

    fn same_output(&self, other: &Self) -> bool {
        self.palette == other.palette
            && self.styles == other.styles
            && self.name == other.name
            && self.background == other.background
            && self.context == other.context
            && self.appearance == other.appearance
    }
}

#[derive(Clone, Debug)]
enum ThemeSource {
    Definition(ThemeDefinition),
    Explicit(Theme),
}

/// Caches terminal-aware semantic styles for one application owner.
#[derive(Clone, Debug)]
pub struct ThemeResolver {
    source: ThemeSource,
    context: ThemeContext,
    resolved: ResolvedTheme,
}

impl ThemeResolver {
    pub fn new(definition: ThemeDefinition, context: ThemeContext) -> Self {
        let source = ThemeSource::Definition(definition);
        let resolved = resolve_source(&source, context, 1);
        Self {
            source,
            context,
            resolved,
        }
    }

    /// Creates a resolver for the historical explicit-palette path. This keeps
    /// every palette color raw, regardless of terminal color depth.
    pub fn explicit(palette: Theme) -> Self {
        let context = ThemeContext::default();
        let source = ThemeSource::Explicit(palette);
        let resolved = resolve_source(&source, context, 1);
        Self {
            source,
            context,
            resolved,
        }
    }

    pub fn resolved(&self) -> &ResolvedTheme {
        &self.resolved
    }

    /// Replaces the definition and returns whether the effective output changed.
    pub fn set_definition(&mut self, definition: ThemeDefinition) -> bool {
        self.source = ThemeSource::Definition(definition);
        self.refresh()
    }

    /// Replaces the terminal context and returns whether the effective output changed.
    pub fn set_context(&mut self, context: ThemeContext) -> bool {
        self.context = context;
        self.refresh()
    }

    /// Replaces an explicit raw palette and returns whether the effective output changed.
    pub fn set_explicit(&mut self, palette: Theme) -> bool {
        self.source = ThemeSource::Explicit(palette);
        self.refresh()
    }

    fn refresh(&mut self) -> bool {
        let mut candidate = resolve_source(&self.source, self.context, self.resolved.version);
        if self.resolved.same_output(&candidate) {
            return false;
        }
        candidate.version = self
            .resolved
            .version
            .checked_add(1)
            .expect("theme resolver generation overflow");
        self.resolved = candidate;
        true
    }
}

/// Named definitions bundled with the built-in night, day, and auto choices.
#[derive(Clone, Debug)]
pub struct ThemeRegistry {
    definitions: BTreeMap<String, ThemeDefinition>,
}

impl ThemeRegistry {
    pub fn new() -> Self {
        let mut definitions = BTreeMap::new();
        for kind in [ThemeKind::Night, ThemeKind::Day, ThemeKind::Auto] {
            let definition = ThemeDefinition::from(kind);
            definitions.insert(definition.name.clone(), definition);
        }
        Self { definitions }
    }

    pub fn insert(&mut self, definition: ThemeDefinition) -> Option<ThemeDefinition> {
        self.definitions.insert(definition.name.clone(), definition)
    }

    pub fn get(&self, name: &str) -> Option<&ThemeDefinition> {
        self.definitions.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.definitions.keys().map(String::as_str)
    }
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn resolve_source(source: &ThemeSource, context: ThemeContext, version: u64) -> ResolvedTheme {
    match source {
        ThemeSource::Definition(definition) => resolve_definition(definition, context, version),
        ThemeSource::Explicit(palette) => resolve_explicit(*palette, context, version),
    }
}

fn resolve_definition(
    definition: &ThemeDefinition,
    context: ThemeContext,
    version: u64,
) -> ResolvedTheme {
    let base = definition
        .palette
        .unwrap_or_else(|| builtin_palette(definition.kind, context));
    let appearance = if definition.palette.is_some() {
        palette_appearance(&base, context)
    } else {
        match definition.kind {
            ThemeKind::Night => ThemeAppearance::Dark,
            ThemeKind::Day => ThemeAppearance::Light,
            ThemeKind::Auto => context.appearance.unwrap_or(ThemeAppearance::Dark),
        }
    };
    let mut palette = base.adapted_for(context.color_level, appearance == ThemeAppearance::Dark);

    for &(color, value) in &definition.colors {
        color.set(
            &mut palette,
            color_support::quantize_color(value, context.color_level),
        );
    }
    apply_background_policy(&mut palette, definition.background);

    let mut styles = default_styles(&palette);
    for &(role, state, override_style) in &definition.styles {
        if state == ThemeState::Normal {
            let override_style = quantize_style(override_style, context.color_level);
            for &state in ThemeState::ALL {
                let index = style_index(role, state);
                styles[index] = styles[index].patch(override_style);
            }
        }
    }
    for &(role, state, override_style) in &definition.styles {
        if state != ThemeState::Normal {
            let index = style_index(role, state);
            styles[index] =
                styles[index].patch(quantize_style(override_style, context.color_level));
        }
    }

    ResolvedTheme {
        palette,
        styles,
        version,
        name: definition.name.clone(),
        background: definition.background,
        context,
        appearance,
    }
}

fn resolve_explicit(palette: Theme, context: ThemeContext, version: u64) -> ResolvedTheme {
    ResolvedTheme {
        styles: default_styles(&palette),
        palette,
        version,
        name: "explicit".to_owned(),
        background: BackgroundPolicy::Solid,
        context,
        appearance: palette_appearance(&palette, context),
    }
}

fn builtin_palette(kind: ThemeKind, context: ThemeContext) -> Theme {
    match kind {
        ThemeKind::Night => Theme::night(),
        ThemeKind::Day => Theme::day(),
        ThemeKind::Auto => match context.appearance {
            Some(ThemeAppearance::Light) => Theme::day(),
            Some(ThemeAppearance::Dark) | None => Theme::night(),
        },
    }
}

fn palette_appearance(palette: &Theme, context: ThemeContext) -> ThemeAppearance {
    match color_rgb(palette.bg_base, context.terminal_background) {
        Some((r, g, b))
            if 299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b) >= 128_000 =>
        {
            ThemeAppearance::Light
        }
        Some(_) => ThemeAppearance::Dark,
        None => context.appearance.unwrap_or(ThemeAppearance::Dark),
    }
}

fn apply_background_policy(palette: &mut Theme, policy: BackgroundPolicy) {
    match policy {
        BackgroundPolicy::Solid => {}
        BackgroundPolicy::TerminalDefault | BackgroundPolicy::Transparent => {
            palette.bg_base = Color::Reset;
            palette.bg_dark = Color::Reset;
            palette.input_bg = Color::Reset;
            palette.md_code_bg = Color::Reset;
            palette.scrollbar_bg = Color::Reset;
            if policy == BackgroundPolicy::Transparent {
                palette.bg_elevated = Color::Reset;
                palette.button_bg = Color::Reset;
            }
        }
    }
}

fn default_styles(palette: &Theme) -> [Style; STYLE_COUNT] {
    let mut styles = [Style::default(); STYLE_COUNT];
    for role in ThemeRole::ALL {
        for state in ThemeState::ALL {
            styles[style_index(*role, *state)] = semantic_style(palette, *role, *state);
        }
    }
    styles
}

fn semantic_style(palette: &Theme, role: ThemeRole, state: ThemeState) -> Style {
    let style = match role {
        ThemeRole::Surface => Style::default()
            .fg(palette.text_primary)
            .bg(palette.bg_base),
        ThemeRole::Elevated => Style::default()
            .fg(palette.text_primary)
            .bg(palette.bg_elevated),
        ThemeRole::Text => Style::default().fg(palette.text_primary),
        ThemeRole::Muted => Style::default().fg(palette.gray),
        ThemeRole::Border => Style::default().fg(palette.border),
        ThemeRole::Input => Style::default()
            .fg(palette.text_primary)
            .bg(palette.input_bg),
        ThemeRole::Button => Style::default().fg(palette.button_fg).bg(palette.button_bg),
        ThemeRole::Selection => Style::default()
            .fg(palette.text_primary)
            .bg(palette.bg_visual),
        ThemeRole::Hover => Style::default()
            .fg(palette.text_primary)
            .bg(palette.bg_hover),
        ThemeRole::Scrollbar => Style::default()
            .fg(palette.scrollbar_fg)
            .bg(palette.scrollbar_bg),
        ThemeRole::Accent => Style::default().fg(palette.accent),
        ThemeRole::Success => Style::default().fg(palette.success),
        ThemeRole::Warning => Style::default().fg(palette.warning),
        ThemeRole::Error => Style::default().fg(palette.error),
        ThemeRole::Link => Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::UNDERLINED),
        ThemeRole::InlineCode => Style::default().fg(palette.md_code),
        ThemeRole::CodeBlock => Style::default()
            .fg(palette.text_primary)
            .bg(palette.md_code_bg),
        ThemeRole::DiffAdded => Style::default().fg(palette.success),
        ThemeRole::DiffRemoved => Style::default().fg(palette.error),
        ThemeRole::DiffHeader => Style::default()
            .fg(palette.gray)
            .add_modifier(Modifier::BOLD),
        ThemeRole::Path => Style::default().fg(palette.path),
        ThemeRole::Fuzzy => Style::default()
            .fg(palette.fuzzy_accent)
            .add_modifier(Modifier::BOLD),
    };

    match state {
        ThemeState::Normal => style,
        ThemeState::Hovered => {
            style
                .bg(palette.bg_hover)
                .add_modifier(if palette.bg_hover == Color::Reset {
                    Modifier::UNDERLINED
                } else {
                    Modifier::empty()
                })
        }
        ThemeState::Focused => style.bg(palette.bg_highlight).add_modifier(Modifier::BOLD),
        ThemeState::Selected => {
            style
                .bg(palette.bg_visual)
                .add_modifier(if palette.bg_visual == Color::Reset {
                    Modifier::REVERSED
                } else {
                    Modifier::BOLD
                })
        }
        ThemeState::Invalid => {
            style
                .fg(palette.error)
                .add_modifier(if palette.error == Color::Reset {
                    Modifier::BOLD | Modifier::UNDERLINED
                } else {
                    Modifier::BOLD
                })
        }
        ThemeState::Disabled => style.fg(palette.gray_dim).add_modifier(Modifier::DIM),
    }
}

fn quantize_style(mut style: Style, level: ColorLevel) -> Style {
    style.fg = style
        .fg
        .map(|color| color_support::quantize_color(color, level));
    style.bg = style
        .bg
        .map(|color| color_support::quantize_color(color, level));
    style
}

const fn style_index(role: ThemeRole, state: ThemeState) -> usize {
    role.index() * STATE_COUNT + state.index()
}

fn color_rgb(color: Color, reset: Option<Color>) -> Option<(u8, u8, u8)> {
    match color {
        Color::Reset => reset.and_then(|known| match known {
            Color::Reset => None,
            known => color_rgb(known, None),
        }),
        Color::Black => Some((0, 0, 0)),
        Color::Red => Some((128, 0, 0)),
        Color::Green => Some((0, 128, 0)),
        Color::Yellow => Some((128, 128, 0)),
        Color::Blue => Some((0, 0, 128)),
        Color::Magenta => Some((128, 0, 128)),
        Color::Cyan => Some((0, 128, 128)),
        Color::Gray => Some((192, 192, 192)),
        Color::DarkGray => Some((128, 128, 128)),
        Color::LightRed => Some((255, 0, 0)),
        Color::LightGreen => Some((0, 255, 0)),
        Color::LightYellow => Some((255, 255, 0)),
        Color::LightBlue => Some((0, 0, 255)),
        Color::LightMagenta => Some((255, 0, 255)),
        Color::LightCyan => Some((0, 255, 255)),
        Color::White => Some((255, 255, 255)),
        Color::Indexed(index) => Some(indexed_to_rgb(index)),
        Color::Rgb(red, green, blue) => Some((red, green, blue)),
    }
}

fn relative_luminance((red, green, blue): (u8, u8, u8)) -> f64 {
    fn channel(value: u8) -> f64 {
        let value = f64::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    0.2126 * channel(red) + 0.7152 * channel(green) + 0.0722 * channel(blue)
}

#[cfg(feature = "theme-config")]
mod config;
#[cfg(feature = "theme-config")]
pub use config::ThemeConfigError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_reset_canvas_uses_known_light_terminal_for_basic_chrome() {
        let mut palette = Theme::day();
        palette.bg_base = Color::Reset;
        let resolver = ThemeResolver::new(
            ThemeDefinition::from_palette("terminal-light", palette),
            ThemeContext {
                appearance: Some(ThemeAppearance::Light),
                color_level: ColorLevel::Basic,
                ..ThemeContext::default()
            },
        );
        assert_eq!(resolver.resolved().palette().bg_base, Color::Reset);
        assert_eq!(resolver.resolved().palette().text_primary, Color::Black);
        assert_eq!(resolver.resolved().appearance(), ThemeAppearance::Light);
    }

    #[test]
    fn monochrome_controls_keep_each_interaction_state_distinguishable() {
        let resolver = ThemeResolver::new(
            ThemeDefinition::from(ThemeKind::Auto),
            ThemeContext {
                color_level: ColorLevel::None,
                ..ThemeContext::default()
            },
        );
        for (index, &state) in ThemeState::ALL.iter().enumerate() {
            for &other in &ThemeState::ALL[index + 1..] {
                assert_ne!(
                    resolver.resolved().style(ThemeRole::Button, state),
                    resolver.resolved().style(ThemeRole::Button, other),
                    "{state:?} and {other:?} need non-color feedback",
                );
            }
        }
    }

    #[test]
    fn state_overrides_take_precedence_after_normal_role_overrides() {
        let definition = ThemeDefinition::new("priority", ThemeKind::Night)
            .with_style(
                ThemeRole::Text,
                ThemeState::Normal,
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::ITALIC),
            )
            .with_style(
                ThemeRole::Text,
                ThemeState::Selected,
                Style::default().fg(Color::Green),
            );
        let resolved = ThemeResolver::new(definition, ThemeContext::default());
        let style = resolved
            .resolved()
            .style(ThemeRole::Text, ThemeState::Selected);

        assert_eq!(style.fg, Some(Color::Green));
        assert_eq!(style.bg, Some(Theme::night().bg_visual));
        assert!(style.add_modifier.contains(Modifier::ITALIC));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn state_flags_use_documented_precedence() {
        assert_eq!(
            ThemeState::from_flags(true, true, true, true, true),
            ThemeState::Disabled
        );
        assert_eq!(
            ThemeState::from_flags(false, true, true, true, true),
            ThemeState::Invalid
        );
        assert_eq!(
            ThemeState::from_flags(false, false, true, true, true),
            ThemeState::Selected
        );
        assert_eq!(
            ThemeState::from_flags(false, false, false, true, true),
            ThemeState::Focused
        );
        assert_eq!(
            ThemeState::from_flags(false, false, false, false, true),
            ThemeState::Hovered
        );
    }

    #[test]
    fn custom_definition_and_style_overrides_are_adapted() {
        let definition = ThemeDefinition::from_palette("custom", Theme::night())
            .with_color(ThemeColor::Accent, Color::Rgb(1, 2, 3))
            .with_style(
                ThemeRole::Text,
                ThemeState::Normal,
                Style::default().fg(Color::Rgb(4, 5, 6)),
            );
        let resolved = ThemeResolver::new(
            definition,
            ThemeContext {
                color_level: ColorLevel::Basic,
                ..ThemeContext::default()
            },
        );

        assert!(!matches!(
            resolved.resolved().palette().accent,
            Color::Rgb(_, _, _) | Color::Indexed(_)
        ));
        assert!(!matches!(
            resolved
                .resolved()
                .style(ThemeRole::Text, ThemeState::Normal)
                .fg,
            Some(Color::Rgb(_, _, _) | Color::Indexed(_))
        ));
    }

    #[test]
    fn transparent_policy_resets_named_surfaces_after_ansi_adaptation() {
        let definition = ThemeDefinition::new("transparent", ThemeKind::Night)
            .with_background(BackgroundPolicy::Transparent);
        let resolved = ThemeResolver::new(
            definition,
            ThemeContext {
                color_level: ColorLevel::Basic,
                ..ThemeContext::default()
            },
        );
        let palette = resolved.resolved().palette();

        assert_eq!(palette.bg_base, Color::Reset);
        assert_eq!(palette.bg_dark, Color::Reset);
        assert_eq!(palette.input_bg, Color::Reset);
        assert_eq!(palette.md_code_bg, Color::Reset);
        assert_eq!(palette.scrollbar_bg, Color::Reset);
        assert_eq!(palette.bg_elevated, Color::Reset);
        assert_eq!(palette.button_bg, Color::Reset);
        assert_ne!(palette.bg_hover, Color::Reset);
        assert_ne!(palette.bg_visual, Color::Reset);
    }

    #[test]
    fn contrast_requires_known_terminal_rgb_for_reset_colors() {
        let definition = ThemeDefinition::new("terminal", ThemeKind::Night)
            .with_background(BackgroundPolicy::TerminalDefault);
        let mut resolver = ThemeResolver::new(definition, ThemeContext::default());

        assert_eq!(
            resolver
                .resolved()
                .contrast(ThemeRole::Surface, ThemeState::Normal),
            None
        );
        assert!(resolver.set_context(ThemeContext {
            terminal_background: Some(Color::Black),
            ..ThemeContext::default()
        }));
        assert!(
            resolver
                .resolved()
                .contrast(ThemeRole::Surface, ThemeState::Normal)
                .is_some()
        );
    }

    #[test]
    fn unchanged_effective_resolution_does_not_advance_version() {
        let definition = ThemeDefinition::new("night", ThemeKind::Night);
        let mut resolver = ThemeResolver::new(definition.clone(), ThemeContext::default());
        let version = resolver.resolved().version();

        assert!(!resolver.set_definition(definition));
        assert_eq!(resolver.resolved().version(), version);
        assert!(
            resolver.set_definition(
                ThemeDefinition::new("night", ThemeKind::Night)
                    .with_background(BackgroundPolicy::TerminalDefault),
            )
        );
        assert_eq!(resolver.resolved().version(), version + 1);
    }
}
