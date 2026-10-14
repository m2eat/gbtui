//! Cached adapter between the transcript and `gbtui-markdown`.
//!
//! The markdown crate owns parsing, streaming checkpoints, tables, and syntax
//! highlighting. This module only lays its already-rendered `Line`s out in the
//! transcript and projects renderer metadata back into selectable fragments
//! and terminal hyperlink spans.

use anstyle::{Ansi256Color, AnsiColor, Color as AnsiColorValue, Style as AnsiStyle};
use gbtui::Theme;
use gbtui::core::{
    ResolvedTheme, ThemeAppearance, ThemeRole, ThemeState, color_support::ColorLevel,
};
use gbtui_markdown::{
    HyperlinkTarget, MarkdownRenderView, MarkdownStyle, StreamingMarkdownRenderer, SyntaxTheme,
    Syntect, TableCopyMeta,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::{Arc, LazyLock};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

static DARK_SYNTECT: LazyLock<Syntect> = LazyLock::new(|| Syntect::for_theme(SyntaxTheme::Dark));
static LIGHT_SYNTECT: LazyLock<Syntect> = LazyLock::new(|| Syntect::for_theme(SyntaxTheme::Light));

pub(crate) fn syntect() -> &'static Syntect {
    &DARK_SYNTECT
}

pub(crate) fn syntect_for(appearance: Option<ThemeAppearance>) -> &'static Syntect {
    match appearance.unwrap_or(ThemeAppearance::Dark) {
        ThemeAppearance::Dark => &DARK_SYNTECT,
        ThemeAppearance::Light => &LIGHT_SYNTECT,
    }
}

pub(crate) fn syntax_theme_for(appearance: Option<ThemeAppearance>) -> SyntaxTheme {
    match appearance.unwrap_or(ThemeAppearance::Dark) {
        ThemeAppearance::Dark => SyntaxTheme::Dark,
        ThemeAppearance::Light => SyntaxTheme::Light,
    }
}

pub(crate) fn legacy_syntax_theme_for(theme: &Theme) -> SyntaxTheme {
    // The legacy palette-only API has no terminal context. RGB palettes still
    // select the obvious appearance; reset and ANSI palettes use the documented
    // dark default rather than guessing the terminal background.
    match theme.bg_base {
        Color::Rgb(red, green, blue)
            if 299 * u32::from(red) + 587 * u32::from(green) + 114 * u32::from(blue) >= 128_000 =>
        {
            SyntaxTheme::Light
        }
        _ => SyntaxTheme::Dark,
    }
}

pub(crate) fn legacy_syntect_for(theme: &Theme) -> &'static Syntect {
    match legacy_syntax_theme_for(theme) {
        SyntaxTheme::Dark => &DARK_SYNTECT,
        SyntaxTheme::Light => &LIGHT_SYNTECT,
    }
}

/// One coherent renderer policy; callers cannot mismatch syntax and color depth.
#[derive(Clone, Copy)]
pub(crate) struct MarkdownTheme {
    pub style: MarkdownStyle,
    pub theme_version: Option<u64>,
    pub syntax_theme: SyntaxTheme,
    pub color_level: ColorLevel,
}

impl MarkdownTheme {
    pub fn from_palette(theme: &Theme) -> Self {
        Self {
            style: style_from_theme(theme),
            theme_version: None,
            syntax_theme: legacy_syntax_theme_for(theme),
            color_level: gbtui::core::color_support::get(),
        }
    }

    pub fn from_resolved(theme: &ResolvedTheme) -> Self {
        Self {
            style: style_from_resolved_theme(theme),
            theme_version: Some(theme.version()),
            syntax_theme: syntax_theme_for(Some(theme.appearance())),
            color_level: theme.context().color_level,
        }
    }

    pub fn syntect(&self) -> &'static Syntect {
        match self.syntax_theme {
            SyntaxTheme::Dark => &DARK_SYNTECT,
            SyntaxTheme::Light => &LIGHT_SYNTECT,
        }
    }
}

fn anstyle_color(color: Color) -> Option<AnsiColorValue> {
    Some(match color {
        Color::Reset => return None,
        Color::Black => AnsiColorValue::Ansi(AnsiColor::Black),
        Color::Red => AnsiColorValue::Ansi(AnsiColor::Red),
        Color::Green => AnsiColorValue::Ansi(AnsiColor::Green),
        Color::Yellow => AnsiColorValue::Ansi(AnsiColor::Yellow),
        Color::Blue => AnsiColorValue::Ansi(AnsiColor::Blue),
        Color::Magenta => AnsiColorValue::Ansi(AnsiColor::Magenta),
        Color::Cyan => AnsiColorValue::Ansi(AnsiColor::Cyan),
        Color::Gray => AnsiColorValue::Ansi(AnsiColor::White),
        Color::DarkGray => AnsiColorValue::Ansi(AnsiColor::BrightBlack),
        Color::LightRed => AnsiColorValue::Ansi(AnsiColor::BrightRed),
        Color::LightGreen => AnsiColorValue::Ansi(AnsiColor::BrightGreen),
        Color::LightYellow => AnsiColorValue::Ansi(AnsiColor::BrightYellow),
        Color::LightBlue => AnsiColorValue::Ansi(AnsiColor::BrightBlue),
        Color::LightMagenta => AnsiColorValue::Ansi(AnsiColor::BrightMagenta),
        Color::LightCyan => AnsiColorValue::Ansi(AnsiColor::BrightCyan),
        Color::White => AnsiColorValue::Ansi(AnsiColor::BrightWhite),
        Color::Indexed(index) => AnsiColorValue::Ansi256(Ansi256Color(index)),
        Color::Rgb(red, green, blue) => AnsiColorValue::Rgb(anstyle::RgbColor(red, green, blue)),
    })
}

fn anstyle_style(style: Style) -> AnsiStyle {
    let mut ansi = AnsiStyle::new()
        .fg_color(style.fg.and_then(anstyle_color))
        .bg_color(style.bg.and_then(anstyle_color));
    if style.add_modifier.contains(Modifier::BOLD) {
        ansi = ansi.bold();
    }
    if style.add_modifier.contains(Modifier::DIM) {
        ansi = ansi.dimmed();
    }
    if style.add_modifier.contains(Modifier::ITALIC) {
        ansi = ansi.italic();
    }
    if style.add_modifier.contains(Modifier::UNDERLINED) {
        ansi = ansi.underline();
    }
    if style.add_modifier.contains(Modifier::CROSSED_OUT) {
        ansi = ansi.strikethrough();
    }
    if style.add_modifier.contains(Modifier::HIDDEN) {
        ansi = ansi.hidden();
    }
    ansi
}

/// Maps the application's semantic styles onto the markdown renderer without
/// introducing a second palette policy.
pub(crate) fn style_from_theme(theme: &Theme) -> MarkdownStyle {
    style_from_roles(|role, state| theme.style(role, state))
}

/// Maps the fully resolved theme, including role overrides and terminal
/// background policy, onto the markdown renderer.
pub(crate) fn style_from_resolved_theme(theme: &ResolvedTheme) -> MarkdownStyle {
    style_from_roles(|role, state| theme.style(role, state))
}

fn style_from_roles(mut style_for: impl FnMut(ThemeRole, ThemeState) -> Style) -> MarkdownStyle {
    let normal = ThemeState::Normal;
    let heading = [
        ThemeRole::Accent,
        ThemeRole::Warning,
        ThemeRole::Path,
        ThemeRole::Error,
        ThemeRole::Success,
        ThemeRole::Accent,
    ]
    .map(|role| style_for(role, normal));
    let text = style_for(ThemeRole::Text, normal);
    let muted = style_for(ThemeRole::Muted, normal);
    let inline_code = style_for(ThemeRole::InlineCode, normal);
    let code_block = style_for(ThemeRole::CodeBlock, normal);
    MarkdownStyle {
        heading_inner: heading.map(|style| anstyle_style(style.add_modifier(Modifier::BOLD))),
        heading_outer: heading
            .map(|style| anstyle_style(style.add_modifier(Modifier::DIM | Modifier::HIDDEN))),
        strong_inner: anstyle_style(text.add_modifier(Modifier::BOLD)),
        strong_outer: anstyle_style(
            Style::default().add_modifier(Modifier::DIM | Modifier::HIDDEN),
        ),
        emphasis_inner: anstyle_style(text.add_modifier(Modifier::ITALIC)),
        emphasis_outer: anstyle_style(
            Style::default().add_modifier(Modifier::DIM | Modifier::HIDDEN),
        ),
        strikethrough_inner: anstyle_style(text.add_modifier(Modifier::CROSSED_OUT)),
        strikethrough_outer: anstyle_style(
            Style::default().add_modifier(Modifier::DIM | Modifier::HIDDEN),
        ),
        inline_code_inner: anstyle_style(inline_code.add_modifier(Modifier::BOLD)),
        inline_code_outer: anstyle_style(
            inline_code.add_modifier(Modifier::DIM | Modifier::HIDDEN),
        ),
        blockquote_outer: anstyle_style(muted.add_modifier(Modifier::DIM)),
        task_checked: anstyle_style(style_for(ThemeRole::Success, normal)),
        task_unchecked: anstyle_style(muted.add_modifier(Modifier::DIM)),
        list_item: anstyle_style(muted),
        rule: anstyle_style(muted),
        link_outer: anstyle_style(muted),
        link_text: anstyle_style(
            style_for(ThemeRole::Link, normal).add_modifier(Modifier::UNDERLINED),
        ),
        link_url: anstyle_style(muted),
        link_title: anstyle_style(style_for(ThemeRole::Warning, normal)),
        code_outer: anstyle_style(inline_code.add_modifier(Modifier::DIM | Modifier::HIDDEN)),
        code_language: anstyle_style(
            style_for(ThemeRole::Warning, normal).add_modifier(Modifier::HIDDEN),
        ),
        code_untagged: anstyle_style(code_block),
        code_background: anstyle_style(code_block),
        code_background_reset: code_block.bg == Some(Color::Reset),
        table_outer: anstyle_style(
            style_for(ThemeRole::Border, normal).add_modifier(Modifier::HIDDEN),
        ),
        text: anstyle_style(text),
        math: anstyle_style(text.add_modifier(Modifier::ITALIC)),
    }
}

/// Quantize a renderer-produced style to the supplied terminal capability.
pub(crate) fn quantize_style_for(mut style: Style, level: ColorLevel) -> Style {
    if let Some(color) = style.fg {
        style = style.fg(gbtui::core::quantize_color(color, level));
    }
    if let Some(color) = style.bg {
        style = style.bg(gbtui::core::quantize_color(color, level));
    }
    style
}

/// Quantize syntax and renderer-produced styles through gbtui's terminal
/// capability policy for the legacy palette-only API.
pub(crate) fn quantize_style(style: Style) -> Style {
    quantize_style_for(style, gbtui::core::color_support::get())
}

/// Apply [`quantize_style_for`] to every stored line and span without reparsing.
pub(crate) fn quantize_line_for(mut line: Line<'static>, level: ColorLevel) -> Line<'static> {
    line.style = quantize_style_for(line.style, level);
    for span in &mut line.spans {
        span.style = quantize_style_for(span.style, level);
    }
    line
}

#[derive(Debug, Clone)]
pub(crate) struct MarkdownSegment {
    pub x: u16,
    pub content: Range<usize>,
    pub visible: String,
}

/// One physical row fragment of a renderer-provided markdown hyperlink.
#[derive(Debug, Clone)]
pub(crate) struct MarkdownLink {
    pub columns: Range<u16>,
    pub url: Arc<str>,
    pub id: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct MarkdownRow {
    pub line: Line<'static>,
    pub segments: Vec<MarkdownSegment>,
    pub links: Vec<MarkdownLink>,
    pub code: bool,
}

/// One markdown output plus the only wrapping pass used by measuring,
/// painting, hit testing, and selection.
#[derive(Debug, Clone, Default)]
pub(crate) struct MarkdownLayout {
    pub text: String,
    pub rows: Vec<MarkdownRow>,
}

impl MarkdownLayout {
    pub fn height(&self) -> usize {
        self.rows.len().max(1)
    }

    fn from_view(view: MarkdownRenderView<'_>, width: u16, color_level: ColorLevel) -> Self {
        let code_lines = view.code_lines;
        let mut text = String::new();
        let mut logical = Vec::with_capacity(view.lines.len());
        let mut previous: Option<(usize, bool)> = None;
        let mut link_urls = BTreeMap::<u32, Arc<str>>::new();
        let mut active_table = None;

        for (index, line) in view.lines.iter().enumerate() {
            let plain = line_text(line);
            let links = hyperlink_fragments(
                view.hyperlinks
                    .iter()
                    .filter(|link| link.line_index == index),
                &mut link_urls,
            );
            let source_line = view.line_source_map.get(index).copied().unwrap_or(index);
            let table = view.tables.iter().find(|table| {
                index >= table.line_index && index < table.line_index + table.line_count
            });
            if let Some(table) = table {
                if index == table.line_index {
                    if previous.is_some() {
                        text.push('\n');
                    }
                    active_table = Some(TableProjection::append(&mut text, table, source_line));
                    let last = table
                        .line_index
                        .saturating_add(table.line_count.saturating_sub(1));
                    previous = Some((
                        view.line_source_map
                            .get(last)
                            .copied()
                            .unwrap_or(source_line),
                        true,
                    ));
                }
                let fragments = active_table
                    .as_mut()
                    .map(|projection| projection.fragments(&plain, source_line))
                    .unwrap_or_default();
                logical.push(LogicalLine {
                    copy: CopyLine {
                        line,
                        plain,
                        code: code_lines.binary_search(&index).is_ok(),
                    },
                    fragments,
                    links,
                });
                if index + 1 == table.line_index.saturating_add(table.line_count) {
                    active_table = None;
                }
                continue;
            }

            let ranges = content_ranges(
                &plain,
                view.hyperlinks
                    .iter()
                    .filter(|link| link.line_index == index),
                code_lines.binary_search(&index).is_ok(),
            );
            let mut fragments = Vec::with_capacity(ranges.len());
            if !ranges.is_empty() {
                if let Some((previous_line, was_table)) = previous {
                    if was_table {
                        text.push('\n');
                    } else {
                        let hard_breaks = source_line.saturating_sub(previous_line).max(1);
                        for _ in 0..hard_breaks {
                            text.push('\n');
                        }
                    }
                }
                for range in ranges {
                    let start = text.len();
                    text.push_str(&plain[range.clone()]);
                    fragments.push(CopyFragment {
                        display: range,
                        content: start..text.len(),
                    });
                }
                previous = Some((source_line, false));
            }
            logical.push(LogicalLine {
                copy: CopyLine {
                    line,
                    plain,
                    code: code_lines.binary_search(&index).is_ok(),
                },
                fragments,
                links,
            });
        }

        let mut rows = Vec::new();
        for logical_line in logical {
            rows.extend(wrap_line(
                logical_line.copy.line,
                &logical_line.copy.plain,
                &logical_line.fragments,
                &logical_line.links,
                width.max(1),
                logical_line.copy.code,
            ));
        }
        if rows.is_empty() {
            rows.push(MarkdownRow {
                line: Line::default(),
                segments: Vec::new(),
                links: Vec::new(),
                code: false,
            });
        }
        for row in &mut rows {
            row.line = quantize_line_for(std::mem::take(&mut row.line), color_level);
        }
        Self { text, rows }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkdownKind {
    Assistant,
    Thinking,
}

#[derive(Debug, Clone)]
pub(crate) struct MarkdownSlot {
    kind: MarkdownKind,
    raw: String,
    streaming: bool,
    width: u16,
    style: MarkdownStyle,
    theme_version: Option<u64>,
    syntax_theme: SyntaxTheme,
    color_level: ColorLevel,
    renderer: StreamingMarkdownRenderer,
    pub layout: MarkdownLayout,
}

impl MarkdownSlot {
    fn new(
        kind: MarkdownKind,
        raw: &str,
        streaming: bool,
        width: u16,
        theme: MarkdownTheme,
    ) -> Self {
        let MarkdownTheme {
            style,
            theme_version,
            syntax_theme,
            color_level,
        } = theme;
        let syntect = theme.syntect();
        let mut renderer = StreamingMarkdownRenderer::new(style, true);
        // Transcript entries historically treat source newlines as copyable hard
        // breaks; physical terminal wrapping is represented separately below.
        renderer.set_collapse_soft_breaks(false);
        renderer.set_max_table_width(Some(width.max(1) as usize));
        if streaming {
            renderer.push_and_render(raw, Some(syntect));
        } else {
            renderer.push(raw);
            renderer.finish(Some(syntect));
        }
        let layout = MarkdownLayout::from_view(renderer.view(), width, color_level);
        Self {
            kind,
            raw: raw.to_owned(),
            streaming,
            width,
            style,
            theme_version,
            syntax_theme,
            color_level,
            renderer,
            layout,
        }
    }

    fn sync(&mut self, raw: &str, streaming: bool, width: u16, theme: MarkdownTheme) {
        let MarkdownTheme {
            style,
            theme_version,
            syntax_theme,
            color_level,
        } = theme;
        let syntect = theme.syntect();
        let theme_changed = self.style != style
            || self.theme_version != theme_version
            || self.syntax_theme != syntax_theme
            || self.color_level != color_level;
        let width_changed = self.width != width;
        let Some(chunk) = raw.strip_prefix(&self.raw) else {
            *self = Self::new(self.kind, raw, streaming, width, theme);
            return;
        };
        if chunk.is_empty() && self.streaming == streaming && !theme_changed && !width_changed {
            return;
        }

        if theme_changed {
            self.style = style;
            self.theme_version = theme_version;
            self.syntax_theme = syntax_theme;
            self.color_level = color_level;
            self.renderer.set_style(style);
        }
        if width_changed {
            self.width = width;
            self.renderer
                .set_max_table_width(Some(width.max(1) as usize));
        }
        if !chunk.is_empty() {
            self.renderer.push_and_render(chunk, Some(syntect));
        } else if theme_changed || width_changed {
            self.renderer.render(Some(syntect));
        }
        if !streaming && (self.streaming || theme_changed || width_changed || !chunk.is_empty()) {
            self.renderer.finish(Some(syntect));
        }
        self.raw.push_str(chunk);
        self.streaming = streaming;
        self.layout = MarkdownLayout::from_view(self.renderer.view(), width, color_level);
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MarkdownCache {
    slots: Vec<Option<MarkdownSlot>>,
}

impl MarkdownCache {
    pub fn sync<'a>(
        &mut self,
        items: impl IntoIterator<Item = (MarkdownKind, usize, &'a str, bool, u16)>,
        theme: MarkdownTheme,
    ) {
        let mut seen = BTreeSet::new();
        for (kind, index, text, streaming, width) in items {
            seen.insert(index);
            if let Some(slot) = self.slots.get_mut(index).and_then(Option::as_mut)
                && slot.kind == kind
            {
                slot.sync(text, streaming, width, theme);
                continue;
            }
            if self.slots.len() <= index {
                self.slots.resize_with(index + 1, || None);
            }
            self.slots[index] = Some(MarkdownSlot::new(kind, text, streaming, width, theme));
        }
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if !seen.contains(&index) {
                *slot = None;
            }
        }
    }

    pub fn layout(&self, index: usize, kind: MarkdownKind) -> Option<&MarkdownLayout> {
        self.slots
            .get(index)
            .and_then(Option::as_ref)
            .filter(|slot| slot.kind == kind)
            .map(|slot| &slot.layout)
    }
}

struct CopyLine<'a> {
    line: &'a Line<'static>,
    plain: String,
    code: bool,
}

struct LogicalLine<'a> {
    copy: CopyLine<'a>,
    fragments: Vec<CopyFragment>,
    links: Vec<LinkFragment>,
}

#[derive(Debug, Clone)]
struct CopyFragment {
    display: Range<usize>,
    content: Range<usize>,
}

#[derive(Debug, Clone)]
struct LinkFragment {
    display: Range<usize>,
    url: Arc<str>,
    id: u32,
}

#[derive(Debug)]
struct TableProjection {
    first_source_line: usize,
    n_cols: usize,
    cells: Vec<TableCellProjection>,
}

#[derive(Debug)]
struct TableCellProjection {
    text: String,
    content: Range<usize>,
    cursor: usize,
}

impl TableProjection {
    fn append(text: &mut String, table: &TableCopyMeta, first_source_line: usize) -> Self {
        let mut cells = Vec::with_capacity(table.cells.len());
        if table.n_cols == 0 {
            return Self {
                first_source_line,
                n_cols: 0,
                cells,
            };
        }
        for (index, cell) in table.cells.iter().enumerate() {
            if index > 0 {
                text.push(if index % table.n_cols == 0 {
                    '\n'
                } else {
                    '\t'
                });
            }
            let start = text.len();
            text.push_str(&cell.text);
            cells.push(TableCellProjection {
                text: cell.text.clone(),
                content: start..text.len(),
                cursor: 0,
            });
        }
        Self {
            first_source_line,
            n_cols: table.n_cols,
            cells,
        }
    }

    fn fragments(&mut self, plain: &str, source_line: usize) -> Vec<CopyFragment> {
        let Some(source_offset) = source_line.checked_sub(self.first_source_line) else {
            return Vec::new();
        };
        let source_row = match source_offset {
            0 => 0,
            1 => return Vec::new(),
            offset => offset.saturating_sub(1),
        };
        let Some(row_start) = source_row.checked_mul(self.n_cols) else {
            return Vec::new();
        };
        table_cell_ranges(plain, self.n_cols)
            .into_iter()
            .filter_map(|(column, display)| {
                let cell = self.cells.get_mut(row_start.saturating_add(column))?;
                let visible = plain.get(display.clone())?;
                let relative = cell.text.get(cell.cursor..)?.find(visible)?;
                let start = cell.cursor.saturating_add(relative);
                let end = start.saturating_add(visible.len());
                cell.cursor = end;
                Some(CopyFragment {
                    display,
                    content: cell.content.start.saturating_add(start)
                        ..cell.content.start.saturating_add(end),
                })
            })
            .collect()
    }
}

fn hyperlink_fragments<'a>(
    links: impl Iterator<Item = &'a HyperlinkTarget>,
    urls: &mut BTreeMap<u32, Arc<str>>,
) -> Vec<LinkFragment> {
    links
        .filter(|link| !link.column_range.is_empty())
        .map(|link| LinkFragment {
            display: link.column_range.clone(),
            url: urls
                .entry(link.id)
                .or_insert_with(|| Arc::from(link.url.as_str()))
                .clone(),
            id: link.id,
        })
        .collect()
}

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// Copy projection for one rendered markdown line, shared by the generic
/// document viewer adapter. It deliberately omits renderer-only bullets,
/// rules, link destination suffixes, table borders, and empty table rules.
pub(crate) fn viewer_copy_text(view: MarkdownRenderView<'_>, index: usize) -> String {
    let Some(line) = view.lines.get(index) else {
        return String::new();
    };
    let plain = line_text(line);
    let code = view.code_lines.binary_search(&index).is_ok();
    if let Some(table) = view
        .tables
        .iter()
        .find(|table| index >= table.line_index && index < table.line_index + table.line_count)
    {
        return table_cell_ranges(&plain, table.n_cols)
            .into_iter()
            .filter_map(|(_, range)| plain.get(range))
            .collect::<Vec<_>>()
            .join("\t");
    }
    content_ranges(
        &plain,
        view.hyperlinks
            .iter()
            .filter(|link| link.line_index == index),
        code,
    )
    .into_iter()
    .filter_map(|range| plain.get(range))
    .collect()
}

fn content_ranges<'a>(
    plain: &str,
    links: impl Iterator<Item = &'a HyperlinkTarget>,
    code: bool,
) -> Vec<Range<usize>> {
    if plain.is_empty() || (!code && is_rule(plain)) {
        return Vec::new();
    }
    if code {
        return std::iter::once(0..plain.len()).collect();
    }
    let prefix = display_content_start(plain);
    if prefix >= plain.len() {
        return Vec::new();
    }
    let mut removed = Vec::new();
    for link in links {
        let end = byte_at_display_column(plain, link.column_range.end);
        let suffix = format!(" ({})", link.url);
        if plain
            .get(end..)
            .is_some_and(|rest| rest.starts_with(&suffix))
        {
            removed.push(end..end + suffix.len());
        }
    }
    subtract_ranges(prefix..plain.len(), removed)
}

fn table_cell_ranges(plain: &str, n_cols: usize) -> Vec<(usize, Range<usize>)> {
    if n_cols == 0 {
        return Vec::new();
    }
    let borders: Vec<_> = plain.match_indices('│').map(|(index, _)| index).collect();
    if borders.len() < n_cols + 1 {
        return Vec::new();
    }
    borders
        .windows(2)
        .take(n_cols)
        .enumerate()
        .filter_map(|(column, pair)| {
            let start = pair[0] + '│'.len_utf8();
            let cell = plain.get(start..pair[1])?;
            let trimmed = cell.trim();
            (!trimmed.is_empty()).then(|| {
                let before = cell.len() - cell.trim_start().len();
                let end = before + trimmed.len();
                (column, (start + before)..(start + end))
            })
        })
        .collect()
}

fn is_rule(plain: &str) -> bool {
    let trimmed = plain.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|ch| matches!(ch, '─' | '━' | '-' | '_'))
}

fn display_content_start(plain: &str) -> usize {
    let mut start = plain.len() - plain.trim_start().len();
    let mut rest = &plain[start..];
    while let Some(after) = rest.strip_prefix("│ ") {
        start += "│ ".len();
        rest = after;
        let spaces = rest.len() - rest.trim_start().len();
        start += spaces;
        rest = &rest[spaces..];
    }
    for marker in ["• ", "◦ ", "☐ ", "☑ "] {
        if rest.starts_with(marker) {
            return start + marker.len();
        }
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0
        && rest
            .get(digits..)
            .is_some_and(|suffix| suffix.starts_with(". ") || suffix.starts_with(") "))
    {
        return start + digits + 2;
    }
    start
}

fn subtract_ranges(base: Range<usize>, mut removed: Vec<Range<usize>>) -> Vec<Range<usize>> {
    removed.sort_unstable_by_key(|range| range.start);
    let mut cursor = base.start;
    let mut kept = Vec::new();
    for range in removed {
        if range.end <= cursor || range.start >= base.end {
            continue;
        }
        let start = range.start.max(base.start);
        if cursor < start {
            kept.push(cursor..start);
        }
        cursor = cursor.max(range.end.min(base.end));
    }
    if cursor < base.end {
        kept.push(cursor..base.end);
    }
    kept
}

fn byte_at_display_column(text: &str, column: usize) -> usize {
    let mut used = 0usize;
    for (start, grapheme) in text.grapheme_indices(true) {
        if used >= column {
            return start;
        }
        used += grapheme.width();
        if used >= column {
            return start + grapheme.len();
        }
    }
    text.len()
}

fn wrap_line(
    line: &Line<'_>,
    plain: &str,
    fragments: &[CopyFragment],
    links: &[LinkFragment],
    width: u16,
    code: bool,
) -> Vec<MarkdownRow> {
    let width = width.max(1) as usize;
    let mut rows = Vec::new();
    let mut spans = Vec::new();
    let mut row_start = 0usize;
    let mut row_end = 0usize;
    let mut row_column_start = 0usize;
    let mut column_offset = 0usize;
    let mut row_width = 0usize;
    let mut byte_offset = 0usize;

    for span in &line.spans {
        for grapheme in span.content.graphemes(true) {
            let grapheme_width = grapheme.width();
            if row_width > 0 && row_width + grapheme_width > width {
                rows.push(markdown_row(
                    Line {
                        style: line.style,
                        alignment: line.alignment,
                        spans: std::mem::take(&mut spans),
                    },
                    plain,
                    fragments,
                    links,
                    row_start..row_end,
                    row_column_start..column_offset,
                    code,
                ));
                row_start = byte_offset;
                row_column_start = column_offset;
                row_width = 0;
            }
            push_span(&mut spans, grapheme, span.style);
            row_width += grapheme_width;
            byte_offset += grapheme.len();
            column_offset += grapheme_width;
            row_end = byte_offset;
        }
    }
    if rows.is_empty() || row_end > row_start || plain.is_empty() {
        rows.push(markdown_row(
            Line {
                style: line.style,
                alignment: line.alignment,
                spans,
            },
            plain,
            fragments,
            links,
            row_start..row_end,
            row_column_start..column_offset,
            code,
        ));
    }
    rows
}

fn markdown_row(
    line: Line<'static>,
    plain: &str,
    fragments: &[CopyFragment],
    links: &[LinkFragment],
    row_bytes: Range<usize>,
    row_columns: Range<usize>,
    code: bool,
) -> MarkdownRow {
    let row_start = row_bytes.start;
    let row_end = row_bytes.end;
    let row_column_start = row_columns.start;
    let row_column_end = row_columns.end;
    let segments = fragments
        .iter()
        .filter_map(|fragment| {
            let start = fragment.display.start.max(row_start);
            let end = fragment.display.end.min(row_end);
            (start < end).then(|| MarkdownSegment {
                x: plain[row_start..start].width().min(u16::MAX as usize) as u16,
                content: (fragment.content.start + start - fragment.display.start)
                    ..(fragment.content.start + end - fragment.display.start),
                visible: plain[start..end].to_owned(),
            })
        })
        .collect();
    let links = links
        .iter()
        .filter_map(|link| {
            let start = link.display.start.max(row_column_start);
            let end = link.display.end.min(row_column_end);
            let start = u16::try_from(start.saturating_sub(row_column_start)).ok()?;
            let end = u16::try_from(end.saturating_sub(row_column_start)).ok()?;
            (start < end).then(|| MarkdownLink {
                columns: start..end,
                url: Arc::clone(&link.url),
                id: link.id,
            })
        })
        .collect();
    MarkdownRow {
        line,
        segments,
        links,
        code,
    }
}

fn push_span(spans: &mut Vec<Span<'static>>, text: &str, style: Style) {
    if let Some(last) = spans.last_mut()
        && last.style == style
    {
        last.content.to_mut().push_str(text);
    } else {
        spans.push(Span::styled(text.to_owned(), style));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui::core::ThemeResolver;

    #[test]
    fn palette_and_resolved_markdown_styles_share_semantics() {
        let palette = Theme::day();
        let resolver = ThemeResolver::explicit(palette);

        assert_eq!(
            style_from_theme(&palette),
            style_from_resolved_theme(resolver.resolved())
        );
    }
}
