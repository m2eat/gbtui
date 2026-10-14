//! Syntax highlighting support using syntect.

use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, LazyLock};

use anstyle::{Effects, Style as AnsiStyle};
use ratatui::style::{Color, Modifier, Style as RatatuiStyle};
use syntect::{
    easy::HighlightLines,
    highlighting::{Theme as SyntectTheme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};

/// Syntax color scheme selected independently of a terminal's background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SyntaxTheme {
    #[default]
    Dark,
    Light,
}
static SYNTAX_SET: LazyLock<Arc<SyntaxSet>> =
    LazyLock::new(|| Arc::new(two_face::syntax::extra_newlines()));

/// Syntax highlighting configuration.
///
/// Create one instance and pass it to the markdown renderer.
pub struct Syntect {
    /// The color theme for syntax highlighting.
    pub theme: SyntectTheme,
    /// The syntax definitions (supports 250+ languages via two-face).
    pub syntax_set: Arc<SyntaxSet>,
}

/// One source-preserving syntax-highlighted segment.
///
/// The text never includes the line terminator supplied to
/// [`Syntect::highlight_text_by_file_path`] or
/// [`Syntect::highlight_text_for_token`]. Consumers can therefore render the
/// returned rows directly without changing their own source mapping.
#[derive(Debug, Clone)]
pub struct SyntaxSpan {
    /// Exact source fragment for this segment, without a line terminator.
    pub text: String,
    /// Foreground and font modifiers resolved from the syntax theme.
    ///
    /// Background remains unset so callers can apply their own surface color.
    pub style: RatatuiStyle,
}

/// Source-preserving highlighted segments for one logical source line.
pub type SyntaxLine = Vec<SyntaxSpan>;

impl Syntect {
    /// Create a new Syntect instance from theme bytes.
    ///
    /// The theme bytes should be a TextMate `.tmTheme` file.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let syntect = Syntect::new(include_bytes!("assets/tokyo-night.tmTheme"));
    /// ```
    pub fn new(theme_bytes: &[u8]) -> Self {
        let mut cursor = Cursor::new(theme_bytes);
        let theme = ThemeSet::load_from_reader(&mut cursor).expect("Failed to load theme");
        Self::with_theme(theme)
    }

    /// Create a highlighter using a bundled syntax color scheme.
    ///
    /// The dark scheme retains the existing Tokyo Night rendering. The light
    /// scheme uses Syntect's bundled Inspired GitHub theme so token colors stay
    /// legible when the application resolved a light appearance.
    pub fn for_theme(theme: SyntaxTheme) -> Self {
        match theme {
            SyntaxTheme::Dark => Self::new(include_bytes!("../assets/tokyo-night.tmTheme")),
            SyntaxTheme::Light => {
                let mut themes = ThemeSet::load_defaults();
                let theme = themes
                    .themes
                    .remove("InspiredGitHub")
                    .expect("Syntect bundles the InspiredGitHub theme");
                Self::with_theme(theme)
            }
        }
    }

    fn with_theme(theme: SyntectTheme) -> Self {
        // Every color scheme shares the same extended parser definitions.
        Self {
            theme,
            syntax_set: Arc::clone(&SYNTAX_SET),
        }
    }

    /// Find a syntax definition by file path extension.
    pub fn find_syntax_by_file_path(&self, file_path: &Path) -> Option<&SyntaxReference> {
        let ext = file_path.extension()?.to_str()?;
        self.syntax_set.find_syntax_by_extension(ext)
    }

    /// Find a syntax definition by language token (e.g., "rust", "python").
    pub fn find_syntax_by_token(&self, token: &str) -> Option<&SyntaxReference> {
        self.syntax_set.find_syntax_by_token(token)
    }

    /// Create a highlighter for the given file path.
    pub fn highlight_lines_by_file_path(&self, file_path: &Path) -> Option<HighlightLines<'_>> {
        Some(HighlightLines::new(
            self.find_syntax_by_file_path(file_path)?,
            &self.theme,
        ))
    }

    /// Create a highlighter for the given language token.
    pub fn highlight_lines_for_token(&self, token: &str) -> Option<HighlightLines<'_>> {
        Some(HighlightLines::new(
            self.find_syntax_by_token(token)?,
            &self.theme,
        ))
    }

    /// Highlight `text` using the syntax resolved from `file_path`.
    ///
    /// Each returned row corresponds to one input line. The highlighter keeps
    /// its parser state across those rows, so a multiline construct in a
    /// context line correctly affects later lines.
    pub fn highlight_text_by_file_path(
        &self,
        file_path: &Path,
        text: &str,
    ) -> Option<Vec<SyntaxLine>> {
        self.highlight_text(self.find_syntax_by_file_path(file_path)?, text)
    }

    /// Highlight `text` using a language token such as `rust` or `python`.
    ///
    /// Each returned row corresponds to one input line and retains syntax state
    /// from its preceding rows.
    pub fn highlight_text_for_token(&self, token: &str, text: &str) -> Option<Vec<SyntaxLine>> {
        self.highlight_text(self.find_syntax_by_token(token)?, text)
    }

    fn highlight_text(&self, syntax: &SyntaxReference, text: &str) -> Option<Vec<SyntaxLine>> {
        let mut highlighter = HighlightLines::new(syntax, &self.theme);
        LinesWithEndings::from(text)
            .map(|line| {
                let mut spans = highlighter
                    .highlight_line(line, &self.syntax_set)
                    .ok()?
                    .into_iter()
                    .map(|(style, text)| SyntaxSpan {
                        text: text.to_owned(),
                        style: syntax_style(style),
                    })
                    .collect::<Vec<_>>();
                strip_line_ending(&mut spans);
                Some(spans)
            })
            .collect()
    }

    /// Highlighter for a fenced code block *info* string: a language token (`rust`, `python`) or a `lineStart:lineEnd:path` line-range citation.
    /// A citation's path resolves via [`Syntect::find_syntax_by_file_path`].
    /// When the path has no known syntax, the whole string falls back to [`Syntect::find_syntax_by_token`], so plain ` ```lang` blocks keep working.
    pub fn highlight_lines_for_fence_info(&self, fence_info: &str) -> Option<HighlightLines<'_>> {
        Some(HighlightLines::new(
            self.find_syntax_for_fence_info(fence_info)?,
            &self.theme,
        ))
    }

    /// Resolve the [`SyntaxReference`] for a fenced code block *info* string, with the same rules as [`Syntect::highlight_lines_for_fence_info`].
    /// This is exposed so the incremental open-code highlighter can build its resumable `ParseState`/`HighlightState` from the same syntax.
    /// That keeps its output byte-identical to the batch `HighlightLines` path.
    pub(crate) fn find_syntax_for_fence_info(&self, fence_info: &str) -> Option<&SyntaxReference> {
        if let Some((_, _, path)) = parse_line_citation_fence_info(fence_info)
            && let Some(s) = self.find_syntax_by_file_path(Path::new(path))
        {
            return Some(s);
        }
        self.find_syntax_by_token(fence_info)
    }
}

impl Default for Syntect {
    fn default() -> Self {
        Self::for_theme(SyntaxTheme::Dark)
    }
}

/// ```text
/// lineStart:lineEnd:path/to/file.ext
/// ```
///
/// The path is the segment after the **second** colon; it is then parsed with [`Path::new`].
/// Paths with extra colons in the first two segments (e.g. some Windows `C:...` forms) are not supported; use a repo-relative or forward-slash form.
fn parse_line_citation_fence_info(info: &str) -> Option<(&str, &str, &str)> {
    let mut it = info.splitn(3, ':');
    let start = it.next()?;
    let end = it.next()?;
    let path = it.next()?;
    if start.is_empty() || !start.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if end.is_empty() || !end.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if path.is_empty() {
        return None;
    }
    Some((start, end, path))
}

/// Syntax highlight code, returning raw styled segments per line.
///
/// `fence_info` is the fenced code block *info* string (language tag or citation); see [`Syntect::highlight_lines_for_fence_info`].
/// This function lives here (not in `parse`) so both the parser and the streaming highlighter caches depend one-way on `syntax`.
pub(crate) fn syntax_highlight_raw(
    syntect: Option<&Syntect>,
    fence_info: &str,
    text: &str,
) -> Option<Vec<Vec<(syntect::highlighting::Style, String)>>> {
    use syntect::util::LinesWithEndings;

    let syn = syntect?;
    let mut hl = syn.highlight_lines_for_fence_info(fence_info)?;
    let mut lines = Vec::new();
    for line in LinesWithEndings::from(text) {
        let highlighted = hl.highlight_line(line, &syn.syntax_set).ok()?;
        lines.push(
            highlighted
                .into_iter()
                .map(|(s, t)| (s, t.to_string()))
                .collect(),
        );
    }
    Some(lines)
}

fn strip_line_ending(spans: &mut Vec<SyntaxSpan>) {
    let Some(last) = spans.last_mut() else {
        return;
    };
    if last.text.ends_with('\n') {
        last.text.pop();
        if last.text.ends_with('\r') {
            last.text.pop();
        }
    }
    if last.text.is_empty() {
        spans.pop();
    }
}

fn syntax_style(style: syntect::highlighting::Style) -> RatatuiStyle {
    anstyle_to_ratatui_style(anstyle_syntect::to_anstyle(style).bg_color(None))
}

fn anstyle_to_ratatui_style(style: AnsiStyle) -> RatatuiStyle {
    let mut ratatui = RatatuiStyle::default();
    if let Some(fg) = style.get_fg_color() {
        ratatui = ratatui.fg(anstyle_to_ratatui_color(fg));
    }
    let effects = style.get_effects();
    let mut modifiers = Modifier::empty();
    if effects.contains(Effects::BOLD) {
        modifiers |= Modifier::BOLD;
    }
    if effects.contains(Effects::DIMMED) {
        modifiers |= Modifier::DIM;
    }
    if effects.contains(Effects::ITALIC) {
        modifiers |= Modifier::ITALIC;
    }
    if effects.contains(Effects::UNDERLINE) {
        modifiers |= Modifier::UNDERLINED;
    }
    if effects.contains(Effects::STRIKETHROUGH) {
        modifiers |= Modifier::CROSSED_OUT;
    }
    if effects.contains(Effects::HIDDEN) {
        modifiers |= Modifier::HIDDEN;
    }
    ratatui.add_modifier(modifiers)
}

fn anstyle_to_ratatui_color(color: anstyle::Color) -> Color {
    match color {
        anstyle::Color::Ansi(ansi) => match ansi {
            anstyle::AnsiColor::Black => Color::Black,
            anstyle::AnsiColor::Red => Color::Red,
            anstyle::AnsiColor::Green => Color::Green,
            anstyle::AnsiColor::Yellow => Color::Yellow,
            anstyle::AnsiColor::Blue => Color::Blue,
            anstyle::AnsiColor::Magenta => Color::Magenta,
            anstyle::AnsiColor::Cyan => Color::Cyan,
            anstyle::AnsiColor::White => Color::Gray,
            anstyle::AnsiColor::BrightBlack => Color::DarkGray,
            anstyle::AnsiColor::BrightRed => Color::LightRed,
            anstyle::AnsiColor::BrightGreen => Color::LightGreen,
            anstyle::AnsiColor::BrightYellow => Color::LightYellow,
            anstyle::AnsiColor::BrightBlue => Color::LightBlue,
            anstyle::AnsiColor::BrightMagenta => Color::LightMagenta,
            anstyle::AnsiColor::BrightCyan => Color::LightCyan,
            anstyle::AnsiColor::BrightWhite => Color::White,
        },
        anstyle::Color::Ansi256(index) => Color::Indexed(index.index()),
        anstyle::Color::Rgb(rgb) => Color::Rgb(rgb.0, rgb.1, rgb.2),
    }
}

/// Get a shared default Syntect instance for tests.
#[cfg(test)]
#[allow(dead_code)]
pub fn test_syntect() -> &'static Syntect {
    use std::sync::LazyLock;

    static TEST_SYNTECT: LazyLock<Syntect> = LazyLock::new(Syntect::default);
    &TEST_SYNTECT
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{SyntaxTheme, Syntect, parse_line_citation_fence_info};
    #[test]
    fn line_citation_fence_parses_start_end_path() {
        assert_eq!(
            parse_line_citation_fence_info("37:65:crates/example/src/tools/read.rs"),
            Some(("37", "65", "crates/example/src/tools/read.rs"))
        );
    }

    #[test]
    fn line_citation_rejects_non_numeric_line() {
        assert_eq!(parse_line_citation_fence_info("37:ab:file.rs"), None);
    }

    #[test]
    fn line_citation_rejects_plain_lang_token() {
        assert_eq!(parse_line_citation_fence_info("rust"), None);
        assert_eq!(parse_line_citation_fence_info(""), None);
    }

    #[test]
    fn highlight_lines_for_fence_info_resolves_citation_path_to_rust() {
        let s = Syntect::default();
        assert!(
            s.highlight_lines_for_fence_info("37:65:crates/gbtui-markdown/src/parse.rs")
                .is_some()
        );
    }

    #[test]
    fn highlight_lines_for_fence_info_still_accepts_rust_token() {
        let s = super::test_syntect();
        assert!(s.highlight_lines_for_fence_info("rust").is_some());
    }
    #[test]
    fn bundled_light_theme_is_distinct_from_dark() {
        let dark = Syntect::for_theme(SyntaxTheme::Dark);
        let light = Syntect::for_theme(SyntaxTheme::Light);
        assert_ne!(
            dark.theme.settings.foreground, light.theme.settings.foreground,
            "resolved light appearances must not reuse dark syntax colors"
        );
    }

    #[test]
    fn public_file_highlight_preserves_logical_source_lines() {
        let highlighted = Syntect::default()
            .highlight_text_by_file_path(Path::new("src/example.rs"), "fn main() {}\nlet x = 1;\n")
            .expect("rust extension is available");
        let text: Vec<_> = highlighted
            .iter()
            .map(|line| {
                line.iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>()
            })
            .collect();
        assert_eq!(text, ["fn main() {}", "let x = 1;"]);
    }
}
