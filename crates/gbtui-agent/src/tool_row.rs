//! Host-data tool headers with source-preserving diff gutters and cached
//! extension/language syntax highlighting; no ACP ids or tool execution.
//!
//! ```text
//!  ◆ Read crates/gbtui-core/src/search.rs (1-40)     0.3s
//!  ◆ Edit crates/gbtui-core/src/search.rs +12/-3     0.6s
//!  ◆ $ cargo test -p gbtui-core search               1.4s
//! ```
//!
//! # Keys
//!
//! Enter / click on the row toggles `expanded` when `detail` is `Some`.
//! The host owns `expanded`; this widget only paints and registers a hit.
//!
//! # Mouse
//!
//! The header row registers [`tool_id`] through [`FrameCtx`].

use std::{ops::Range, path::Path, sync::Arc, time::Duration};

use gbtui::core::{HitId, ResolvedTheme, SafeBuf, ThemeRole, ThemeState};
use gbtui::runtime::FrameCtx;
use gbtui::terminal::glyphs::diamond_filled;
use gbtui::{Spinner, TickDemand};
use gbtui_markdown::{SyntaxSpan, SyntaxTheme, Syntect};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::diff::{
    DiffFileItem, DiffHunk, DiffLine, DiffLineKind, DiffMetadataKind, parse_unified_diff,
};
use crate::markdown::{quantize_style, quantize_style_for, syntax_theme_for, syntect, syntect_for};
use crate::paint::fill;

/// Hit namespace for tool header rows. Index is the host's item index.
pub const NS_TOOL: u32 = 0xA012;

pub fn tool_id(index: u32) -> HitId {
    HitId::namespaced(NS_TOOL, index)
}

pub fn parse_tool_id(id: HitId) -> Option<u32> {
    (id.namespace() == NS_TOOL).then_some(id.index())
}

/// Lifecycle of a generic tool row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    Running,
    Done,
    Error,
}

/// Generic presentation kind for a host-owned tool event.
///
/// This enum has no execution or registry behavior; it only determines the
/// visible header treatment for data supplied by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolKind {
    #[default]
    Generic,
    Read,
    Edit,
    Run,
    Search,
    List,
    WebSearch,
    WebFetch,
    MemorySearch,
    UseTool,
    Message,
    Hook,
}

/// Source-preserving physical detail rows shared by paint and transcript selection.
#[derive(Debug, Clone)]
pub(crate) struct ToolDetailLayout {
    /// The exact detail text supplied by the host. Gutters are deliberately absent.
    pub text: String,
    /// One row for each physically displayed line, including soft-wrap continuations.
    pub rows: Vec<ToolDetailLine>,
}

/// A physical detail row and its precise mapping back to [`ToolDetailLayout::text`].
#[derive(Debug, Clone)]
pub(crate) struct ToolDetailLine {
    /// Display line, including indentation or a diff gutter, relative to the whole tool area.
    pub line: Line<'static>,
    /// Display column where [`Self::visible`] begins, relative to the whole tool area.
    pub content_x: u16,
    /// Byte range in [`ToolDetailLayout::text`] covered by [`Self::visible`].
    pub content: Range<usize>,
    /// The visible source fragment without the non-copyable gutter.
    pub visible: String,
    style: ToolDetailStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolDetailStyle {
    Plain,
    FileMetadata,
    HunkHeader,
    Context,
    Addition,
    Deletion,
    NoNewlineMarker,
}

/// Prepared detail layout retained by the host between measurement, painting,
/// and selection.
///
/// A state belongs to one visible tool row. It owns the raw detail once while
/// expanded, so syntax highlighting and soft wrapping run only when relevant
/// host data or width changes.
#[derive(Debug, Clone, Default)]
pub struct ToolRowState {
    cached: Option<CachedToolDetail>,
}

#[derive(Debug, Clone)]
struct CachedToolDetail {
    width: u16,
    kind: ToolKind,
    summary: Arc<str>,
    language: Option<Arc<str>>,
    theme_version: Option<u64>,
    syntax_theme: SyntaxTheme,
    layout: Arc<ToolDetailLayout>,
}

/// Borrowed tool header. Host owns the backing strings and `expanded`.
#[derive(Debug, Clone, Copy)]
pub struct ToolRow<'a> {
    pub name: &'a str,
    pub summary: &'a str,
    pub detail: Option<&'a str>,
    pub elapsed: Option<Duration>,
    pub status: ToolStatus,
    pub kind: ToolKind,
    /// Optional explicit language token (for example, `rust` or `python`).
    ///
    /// When omitted, edit details derive syntax from each diff file extension,
    /// then from `summary` if a diff has no file metadata.
    pub language: Option<&'a str>,
    /// Read: `(start, end)` 1-based inclusive, shown as `(1-40)`.
    pub range: Option<(u32, u32)>,
    /// Edit collapsed suffix: `(insertions, deletions)` → ` +N/-M`.
    pub diffstat: Option<(u32, u32)>,
    /// List: `(N entries)` / `(1 entry)`.
    pub entries: Option<u32>,
    pub expanded: bool,
    pub index: u32,
}

impl<'a> ToolRow<'a> {
    pub fn new(name: &'a str, summary: &'a str) -> Self {
        Self {
            name,
            summary,
            detail: None,
            elapsed: None,
            status: ToolStatus::Pending,
            kind: ToolKind::Generic,
            language: None,
            range: None,
            diffstat: None,
            entries: None,
            expanded: false,
            index: 0,
        }
    }

    pub fn kind(mut self, kind: ToolKind) -> Self {
        self.kind = kind;
        self
    }

    /// Force syntax highlighting for edit details to a host-provided language
    /// token instead of deriving it from the diff file path.
    pub fn language(mut self, language: &'a str) -> Self {
        self.language = Some(language);
        self
    }

    pub fn range(mut self, start: u32, end: u32) -> Self {
        self.range = Some((start, end));
        self
    }

    pub fn diffstat(mut self, plus: u32, minus: u32) -> Self {
        self.diffstat = Some((plus, minus));
        self
    }

    pub fn entries(mut self, n: u32) -> Self {
        self.entries = Some(n);
        self
    }

    pub fn status(mut self, status: ToolStatus) -> Self {
        self.status = status;
        self
    }

    pub fn detail(mut self, detail: &'a str) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn maybe_detail(mut self, detail: Option<&'a str>) -> Self {
        self.detail = detail;
        self
    }

    pub fn elapsed(mut self, elapsed: Duration) -> Self {
        self.elapsed = Some(elapsed);
        self
    }

    pub fn expanded(mut self, on: bool) -> Self {
        self.expanded = on;
        self
    }

    pub fn index(mut self, index: u32) -> Self {
        self.index = index;
        self
    }

    /// Prepare the one detail layout shared by measurement, painting, and
    /// transcript selection with the legacy dark syntax scheme.
    pub fn prepare(&self, state: &mut ToolRowState, width: u16) {
        self.prepare_with_syntax(state, width, None, SyntaxTheme::Dark, syntect());
    }

    /// Prepare detail syntax from a resolved theme appearance.
    pub fn prepare_resolved(&self, state: &mut ToolRowState, width: u16, theme: &ResolvedTheme) {
        self.prepare_with_syntax(
            state,
            width,
            Some(theme.version()),
            syntax_theme_for(Some(theme.appearance())),
            syntect_for(Some(theme.appearance())),
        );
    }

    fn prepare_with_syntax(
        &self,
        state: &mut ToolRowState,
        width: u16,
        theme_version: Option<u64>,
        syntax_theme: SyntaxTheme,
        syntect: &Syntect,
    ) {
        let Some(detail) = self.detail.filter(|detail| !detail.is_empty()) else {
            state.cached = None;
            return;
        };
        if state.cached.as_ref().is_some_and(|cached| {
            cached.width == width
                && cached.kind == self.kind
                && cached.layout.text == detail
                && cached.summary.as_ref() == self.summary
                && cached.language.as_deref() == self.language
                && cached.theme_version == theme_version
                && cached.syntax_theme == syntax_theme
        }) {
            return;
        }
        let layout = match self.kind {
            ToolKind::Edit => parse_unified_diff(detail)
                .map(|diff| {
                    diff_detail_layout(detail, width, &diff, self.summary, self.language, syntect)
                })
                .unwrap_or_else(|| plain_detail_layout(detail, width)),
            _ => plain_detail_layout(detail, width),
        };
        state.cached = Some(CachedToolDetail {
            width,
            kind: self.kind,
            summary: Arc::from(self.summary),
            language: self.language.map(Arc::from),
            theme_version,
            syntax_theme,
            layout: Arc::new(layout),
        });
    }

    /// Obtain the prepared detail representation after refreshing it if needed.
    ///
    /// The returned `Arc` keeps transcript selection and painting independent
    /// without copying the full raw detail or physical row layout.
    pub(crate) fn detail_layout(
        &self,
        state: &mut ToolRowState,
        width: u16,
    ) -> Option<Arc<ToolDetailLayout>> {
        self.detail_layout_with_syntax(state, width, None, SyntaxTheme::Dark, syntect())
    }

    pub(crate) fn detail_layout_with_syntax(
        &self,
        state: &mut ToolRowState,
        width: u16,
        theme_version: Option<u64>,
        syntax_theme: SyntaxTheme,
        syntect: &Syntect,
    ) -> Option<Arc<ToolDetailLayout>> {
        self.prepare_with_syntax(state, width, theme_version, syntax_theme, syntect);
        state
            .cached
            .as_ref()
            .map(|cached| Arc::clone(&cached.layout))
    }

    /// Physical rows needed at `width` (header + optional prepared detail).
    pub fn measure(&self, state: &mut ToolRowState, width: u16) -> usize {
        let detail_rows = self
            .expanded
            .then(|| self.detail_layout(state, width))
            .flatten()
            .map_or(0, |detail| detail.rows.len());
        1usize.saturating_add(detail_rows)
    }

    /// Paint into `area` using the state prepared for this row. Returns visible
    /// rows used.
    pub fn render(
        &self,
        state: &mut ToolRowState,
        area: Rect,
        buf: &mut Buffer,
        cx: &mut FrameCtx<'_>,
    ) -> u16 {
        if area.height == 0 || area.width < 6 {
            return 0;
        }
        let (theme_version, syntax_theme, syntect) = syntax_for_frame(cx);
        let layout = self
            .expanded
            .then(|| {
                self.detail_layout_with_syntax(
                    state,
                    area.width,
                    theme_version,
                    syntax_theme,
                    syntect,
                )
            })
            .flatten();
        let height = 1usize
            .saturating_add(layout.as_ref().map_or(0, |detail| detail.rows.len()))
            .min(area.height as usize) as u16;
        self.render_clipped_with_layout(Rect { height, ..area }, 0, layout.as_deref(), buf, cx);
        height
    }

    /// Paint a physical row slice from the same prepared layout as selection.
    pub(crate) fn render_clipped(
        &self,
        state: &mut ToolRowState,
        area: Rect,
        line_offset: usize,
        buf: &mut Buffer,
        cx: &mut FrameCtx<'_>,
    ) {
        if area.height == 0 || area.width < 6 {
            return;
        }
        let (theme_version, syntax_theme, syntect) = syntax_for_frame(cx);
        let layout = self
            .expanded
            .then(|| {
                self.detail_layout_with_syntax(
                    state,
                    area.width,
                    theme_version,
                    syntax_theme,
                    syntect,
                )
            })
            .flatten();
        self.render_clipped_with_layout(area, line_offset, layout.as_deref(), buf, cx);
    }

    fn render_clipped_with_layout(
        &self,
        area: Rect,
        line_offset: usize,
        detail: Option<&ToolDetailLayout>,
        buf: &mut Buffer,
        cx: &mut FrameCtx<'_>,
    ) {
        if area.height == 0 || area.width < 6 {
            return;
        }
        fill(buf, area, cx.style(ThemeRole::Surface, ThemeState::Normal));

        if line_offset == 0 {
            let header = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            };
            cx.register(tool_id(self.index), header);
            let header_style = cx.style(
                ThemeRole::Surface,
                if cx.hovered(header) {
                    ThemeState::Hovered
                } else {
                    ThemeState::Normal
                },
            );
            fill(buf, header, header_style);

            let bullet_x = header.x.saturating_add(1);
            match self.status {
                ToolStatus::Running => {
                    cx.request_tick(TickDemand::Fast);
                    Spinner::new()
                        .running(true)
                        .render(bullet_x, header.y, buf, cx);
                }
                ToolStatus::Pending | ToolStatus::Done => {
                    let role = if self.expanded {
                        ThemeRole::Text
                    } else {
                        ThemeRole::Muted
                    };
                    buf.set_string_safe(
                        bullet_x,
                        header.y,
                        diamond_filled(),
                        header_style.patch(foreground_style(cx.style(role, ThemeState::Normal))),
                    );
                }
                ToolStatus::Error => {
                    buf.set_string_safe(
                        bullet_x,
                        header.y,
                        diamond_filled(),
                        header_style.patch(foreground_style(
                            cx.style(ThemeRole::Error, ThemeState::Normal),
                        )),
                    );
                }
            }

            let x = header.x.saturating_add(3);
            let mut right = header.right();
            if let Some(elapsed) = self.elapsed {
                let s = fmt_elapsed(elapsed);
                let w = s.width() as u16;
                if right.saturating_sub(w) > x {
                    right = right.saturating_sub(w);
                    buf.set_string_safe(
                        right,
                        header.y,
                        &s,
                        header_style.patch(foreground_style(
                            cx.style(ThemeRole::Muted, ThemeState::Normal),
                        )),
                    );
                    right = right.saturating_sub(1);
                }
            }
            paint_header(
                self,
                x,
                header.y,
                right.saturating_sub(x),
                header_style,
                buf,
                cx,
            );
        }

        if let Some(detail) = detail {
            let detail_y = if line_offset == 0 {
                area.y.saturating_add(1)
            } else {
                area.y
            };
            if detail_y < area.bottom() {
                paint_detail(
                    detail,
                    Rect {
                        x: area.x,
                        y: detail_y,
                        width: area.width,
                        height: area.bottom().saturating_sub(detail_y),
                    },
                    line_offset.saturating_sub(1),
                    buf,
                    cx,
                );
            }
        }
    }
}
fn syntax_for_frame(cx: &FrameCtx<'_>) -> (Option<u64>, SyntaxTheme, &'static Syntect) {
    if let Some(theme) = cx.resolved_theme() {
        (
            Some(theme.version()),
            syntax_theme_for(Some(theme.appearance())),
            syntect_for(Some(theme.appearance())),
        )
    } else {
        (None, SyntaxTheme::Dark, syntect())
    }
}

fn plain_detail_layout(detail: &str, width: u16) -> ToolDetailLayout {
    let mut builder = DetailLayoutBuilder::new(detail, width);
    let mut start = 0usize;
    for raw in detail.split_inclusive('\n') {
        let full_end = start.saturating_add(raw.len());
        let line_end = raw.strip_suffix('\n').map_or(full_end, |_| full_end - 1);
        let content_end = if detail.as_bytes().get(line_end.saturating_sub(1)) == Some(&b'\r') {
            line_end.saturating_sub(1)
        } else {
            line_end
        };
        builder.push_wrapped(
            start..content_end,
            "    ".to_owned(),
            ToolDetailStyle::Plain,
            None,
        );
        start = full_end;
    }
    builder.finish()
}

fn diff_detail_layout(
    detail: &str,
    width: u16,
    diff: &crate::diff::UnifiedDiff<'_>,
    summary: &str,
    language: Option<&str>,
    syntect: &Syntect,
) -> ToolDetailLayout {
    let mut builder = DetailLayoutBuilder::new(detail, width);
    for file in &diff.files {
        let mut syntax = DiffFileSyntax::new(detail, file, summary, language, syntect);
        for item in &file.items {
            match item {
                DiffFileItem::Metadata(metadata) => {
                    let style = match metadata.kind {
                        DiffMetadataKind::FileHeader
                        | DiffMetadataKind::OldFile
                        | DiffMetadataKind::NewFile => ToolDetailStyle::FileMetadata,
                        DiffMetadataKind::Extended => ToolDetailStyle::Plain,
                    };
                    builder.push_wrapped(metadata.range.clone(), "  ".to_owned(), style, None);
                }
                DiffFileItem::Hunk(hunk) => {
                    builder.push_wrapped(
                        hunk.header.range.clone(),
                        "  ".to_owned(),
                        ToolDetailStyle::HunkHeader,
                        None,
                    );
                    let gutter = DiffGutter::for_hunk(hunk);
                    for line in &hunk.lines {
                        let (prefix, style) = match line.kind {
                            DiffLineKind::Context => {
                                (gutter.prefix(line), ToolDetailStyle::Context)
                            }
                            DiffLineKind::Addition => {
                                (gutter.prefix(line), ToolDetailStyle::Addition)
                            }
                            DiffLineKind::Deletion => {
                                (gutter.prefix(line), ToolDetailStyle::Deletion)
                            }
                            DiffLineKind::NoNewlineMarker => {
                                (gutter.blank_prefix(), ToolDetailStyle::NoNewlineMarker)
                            }
                        };
                        builder.push_wrapped(line.range.clone(), prefix, style, syntax.take(line));
                    }
                }
            }
        }
    }
    builder.finish()
}

/// Syntax state for one diff file. The old and new sides each have a distinct
/// parser stream, while context advances both streams.
#[derive(Debug)]
struct DiffFileSyntax {
    lines: Vec<Option<DiffSyntaxLine>>,
    cursor: usize,
}

#[derive(Debug)]
struct DiffSyntaxLine {
    range: Range<usize>,
    body: Range<usize>,
    spans: Vec<SyntaxSpan>,
}

#[derive(Debug)]
struct SyntaxInput {
    range: Range<usize>,
    body: Range<usize>,
    old_index: Option<usize>,
    new_index: Option<usize>,
    kind: DiffLineKind,
}

impl DiffFileSyntax {
    fn new(
        detail: &str,
        file: &crate::diff::DiffFile<'_>,
        summary: &str,
        language: Option<&str>,
        syntect: &Syntect,
    ) -> Self {
        let mut old_source = String::new();
        let mut new_source = String::new();
        let mut old_count = 0usize;
        let mut new_count = 0usize;
        let mut inputs = Vec::new();
        for item in &file.items {
            let DiffFileItem::Hunk(hunk) = item else {
                continue;
            };
            for line in &hunk.lines {
                let body = match line.kind {
                    DiffLineKind::Context | DiffLineKind::Addition | DiffLineKind::Deletion => {
                        line.range.start.saturating_add(1)..line.range.end
                    }
                    DiffLineKind::NoNewlineMarker => continue,
                };
                let text = line
                    .text
                    .get(1..)
                    .expect("parsed unified diff body has a prefix");
                let old_index = match line.kind {
                    DiffLineKind::Context | DiffLineKind::Deletion => {
                        let index = old_count;
                        old_source.push_str(text);
                        old_source.push('\n');
                        old_count = old_count.saturating_add(1);
                        Some(index)
                    }
                    DiffLineKind::Addition | DiffLineKind::NoNewlineMarker => None,
                };
                let new_index = match line.kind {
                    DiffLineKind::Context | DiffLineKind::Addition => {
                        let index = new_count;
                        new_source.push_str(text);
                        new_source.push('\n');
                        new_count = new_count.saturating_add(1);
                        Some(index)
                    }
                    DiffLineKind::Deletion | DiffLineKind::NoNewlineMarker => None,
                };
                inputs.push(SyntaxInput {
                    range: line.range.clone(),
                    body,
                    old_index,
                    new_index,
                    kind: line.kind,
                });
            }
        }

        let path = diff_file_path(file).unwrap_or(summary);
        let highlight = |text: &str| {
            if text.is_empty() {
                return None;
            }
            if let Some(language) = language {
                syntect.highlight_text_for_token(language, text)
            } else {
                syntect.highlight_text_by_file_path(Path::new(path), text)
            }
        };
        let mut old = highlight(&old_source).map(|lines| {
            lines
                .into_iter()
                .map(Some)
                .collect::<Vec<Option<Vec<SyntaxSpan>>>>()
        });
        let mut new = highlight(&new_source).map(|lines| {
            lines
                .into_iter()
                .map(Some)
                .collect::<Vec<Option<Vec<SyntaxSpan>>>>()
        });
        let lines = inputs
            .into_iter()
            .map(|input| {
                let mut take_old = || {
                    input
                        .old_index
                        .and_then(|index| old.as_mut()?.get_mut(index)?.take())
                };
                let mut take_new = || {
                    input
                        .new_index
                        .and_then(|index| new.as_mut()?.get_mut(index)?.take())
                };
                let spans = match input.kind {
                    DiffLineKind::Context => take_new().or_else(take_old),
                    DiffLineKind::Addition => take_new(),
                    DiffLineKind::Deletion => take_old(),
                    DiffLineKind::NoNewlineMarker => None,
                }?;
                let source = detail.get(input.body.clone())?;
                syntax_matches_source(&spans, source).then_some(DiffSyntaxLine {
                    range: input.range,
                    body: input.body,
                    spans,
                })
            })
            .collect();
        Self { lines, cursor: 0 }
    }

    fn take(&mut self, line: &DiffLine<'_>) -> Option<&DiffSyntaxLine> {
        if matches!(line.kind, DiffLineKind::NoNewlineMarker) {
            return None;
        }
        let index = self.cursor;
        self.cursor = self.cursor.saturating_add(1);
        self.lines
            .get(index)?
            .as_ref()
            .filter(|syntax| syntax.range == line.range)
    }
}

fn syntax_matches_source(spans: &[SyntaxSpan], source: &str) -> bool {
    let mut offset = 0usize;
    for span in spans {
        let end = offset.saturating_add(span.text.len());
        if source.get(offset..end) != Some(span.text.as_str()) {
            return false;
        }
        offset = end;
    }
    offset == source.len()
}

fn diff_file_path<'a>(file: &'a crate::diff::DiffFile<'a>) -> Option<&'a str> {
    for kind in [DiffMetadataKind::NewFile, DiffMetadataKind::OldFile] {
        if let Some(path) = file.items.iter().find_map(|item| {
            let DiffFileItem::Metadata(metadata) = item else {
                return None;
            };
            (metadata.kind == kind)
                .then(|| metadata.text.get(4..).and_then(diff_path))
                .flatten()
        }) {
            return Some(path);
        }
    }
    file.items.iter().find_map(|item| {
        let DiffFileItem::Metadata(metadata) = item else {
            return None;
        };
        (metadata.kind == DiffMetadataKind::FileHeader)
            .then(|| metadata.text.split_whitespace().nth(3).and_then(diff_path))
            .flatten()
    })
}

fn diff_path(path: &str) -> Option<&str> {
    let path = path.split('\t').next()?.trim().trim_matches('"');
    let path = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);
    (path != "/dev/null" && !path.is_empty()).then_some(path)
}

struct DetailLayoutBuilder<'a> {
    source: &'a str,
    width: u16,
    rows: Vec<ToolDetailLine>,
}

impl<'a> DetailLayoutBuilder<'a> {
    fn new(source: &'a str, width: u16) -> Self {
        Self {
            source,
            width,
            rows: Vec::new(),
        }
    }

    fn push_wrapped(
        &mut self,
        content: Range<usize>,
        first_prefix: String,
        style: ToolDetailStyle,
        syntax: Option<&DiffSyntaxLine>,
    ) {
        let source = self
            .source
            .get(content.clone())
            .expect("detail layout only contains source ranges");
        let content_x = u16::try_from(first_prefix.width())
            .expect("tool detail gutter fits terminal coordinate");
        let body_width = usize::from(self.width)
            .saturating_sub(usize::from(content_x))
            .max(1);
        for (index, relative) in wrap_source_ranges(source, body_width)
            .into_iter()
            .enumerate()
        {
            let content = content.start.saturating_add(relative.start)
                ..content.start.saturating_add(relative.end);
            let visible = self
                .source
                .get(content.clone())
                .expect("wrapped detail range stays within source")
                .to_owned();
            let gutter = if index == 0 {
                first_prefix.clone()
            } else {
                " ".repeat(usize::from(content_x))
            };
            let mut spans = vec![Span::raw(gutter)];
            spans.extend(styled_source_spans(self.source, content.clone(), syntax));
            self.rows.push(ToolDetailLine {
                line: Line::from(spans),
                content_x,
                content,
                visible,
                style,
            });
        }
    }

    fn finish(self) -> ToolDetailLayout {
        ToolDetailLayout {
            text: self.source.to_owned(),
            rows: self.rows,
        }
    }
}

fn styled_source_spans(
    source: &str,
    content: Range<usize>,
    syntax: Option<&DiffSyntaxLine>,
) -> Vec<Span<'static>> {
    let Some(syntax) = syntax else {
        return vec![Span::raw(
            source
                .get(content)
                .expect("detail layout only contains source ranges")
                .to_owned(),
        )];
    };
    let mut spans = Vec::with_capacity(syntax.spans.len().saturating_add(2));
    let mut cursor = content.start;
    let mut syntax_cursor = syntax.body.start;
    for syntax_span in &syntax.spans {
        let syntax_end = syntax_cursor.saturating_add(syntax_span.text.len());
        let overlap_start = syntax_cursor.max(content.start);
        let overlap_end = syntax_end.min(content.end);
        if overlap_start < overlap_end {
            if cursor < overlap_start {
                spans.push(Span::raw(
                    source
                        .get(cursor..overlap_start)
                        .expect("syntax span starts within source")
                        .to_owned(),
                ));
            }
            spans.push(Span::styled(
                source
                    .get(overlap_start..overlap_end)
                    .expect("syntax span stays within source")
                    .to_owned(),
                syntax_span.style,
            ));
            cursor = overlap_end;
        }
        syntax_cursor = syntax_end;
    }
    if cursor < content.end {
        spans.push(Span::raw(
            source
                .get(cursor..content.end)
                .expect("syntax span ends within source")
                .to_owned(),
        ));
    }
    spans
}

/// Soft-wrap source text without trimming, normalizing, or splitting grapheme clusters.
fn wrap_source_ranges(text: &str, width: usize) -> Vec<Range<usize>> {
    if text.is_empty() {
        return std::iter::once(0..0).collect();
    }
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut start = 0usize;
    let mut used = 0usize;
    for (byte, grapheme) in text.grapheme_indices(true) {
        let grapheme_width = grapheme.width();
        if byte > start && grapheme_width > 0 && used.saturating_add(grapheme_width) > width {
            rows.push(start..byte);
            start = byte;
            used = 0;
        }
        used = used.saturating_add(grapheme_width);
    }
    rows.push(start..text.len());
    rows
}

struct DiffGutter {
    old_width: usize,
    new_width: usize,
}

impl DiffGutter {
    fn for_hunk(hunk: &DiffHunk<'_>) -> Self {
        let old_width = hunk
            .lines
            .iter()
            .filter_map(|line| line.old_line)
            .map(decimal_width)
            .max()
            .unwrap_or(1);
        let new_width = hunk
            .lines
            .iter()
            .filter_map(|line| line.new_line)
            .map(decimal_width)
            .max()
            .unwrap_or(1);
        Self {
            old_width,
            new_width,
        }
    }

    fn prefix(&self, line: &DiffLine<'_>) -> String {
        let old = line
            .old_line
            .map_or_else(String::new, |number| number.to_string());
        let new = line
            .new_line
            .map_or_else(String::new, |number| number.to_string());
        format!(
            "  {old:>old_width$} {new:>new_width$} │ ",
            old_width = self.old_width,
            new_width = self.new_width,
        )
    }

    fn blank_prefix(&self) -> String {
        " ".repeat(2 + self.old_width + 1 + self.new_width + 3)
    }
}

fn decimal_width(number: u32) -> usize {
    number.to_string().len()
}

fn paint_header(
    row: &ToolRow<'_>,
    x: u16,
    y: u16,
    width: u16,
    header_style: Style,
    buf: &mut Buffer,
    cx: &FrameCtx<'_>,
) {
    let muted = matches!(row.status, ToolStatus::Pending);
    let role_style =
        |role| header_style.patch(foreground_style(cx.style(role, ThemeState::Normal)));
    let bold = role_style(verb_role(row.status)).add_modifier(Modifier::BOLD);
    let path_st = role_style(if muted {
        ThemeRole::Muted
    } else {
        ThemeRole::Path
    });
    let dim = role_style(ThemeRole::Muted);
    if row.kind == ToolKind::Generic {
        buf.set_string_safe(x, y, row.name, bold);
        let col = x.saturating_add(row.name.width() as u16 + 1);
        let budget = width.saturating_sub(col.saturating_sub(x)) as usize;
        let shown = gbtui::core::line_utils::truncate_str(row.summary, budget);
        buf.set_string_safe(col, y, &shown, dim);
        return;
    }
    let (prefix, prefix_st, target_st) = match row.kind {
        ToolKind::Read => ("Read ", bold, path_st),
        ToolKind::Edit => ("Edit ", bold, path_st),
        ToolKind::Run => ("$ ", dim, role_style(ThemeRole::Text)),
        ToolKind::Search => ("Search ", bold, path_st),
        ToolKind::List => ("List ", bold, path_st),
        ToolKind::WebSearch => ("Web search ", bold, path_st),
        ToolKind::WebFetch => ("Fetch ", bold, path_st),
        ToolKind::MemorySearch => ("Memory ", bold, role_style(ThemeRole::Accent)),
        ToolKind::UseTool => ("Tool ", bold, role_style(ThemeRole::Text)),
        ToolKind::Message => ("Message ", bold, role_style(ThemeRole::Muted)),
        ToolKind::Hook => ("Hook ", bold, role_style(ThemeRole::Warning)),
        ToolKind::Generic => unreachable!(),
    };
    let mut col = x;
    buf.set_string_safe(col, y, prefix, prefix_st);
    col = col.saturating_add(prefix.width() as u16);
    let suffix = row
        .range
        .map(|(a, b)| format!(" ({a}-{b})"))
        .unwrap_or_default();
    let budget = width
        .saturating_sub(col.saturating_sub(x))
        .saturating_sub(suffix.width() as u16);
    let shown = gbtui::core::line_utils::truncate_str(row.summary, budget as usize);
    buf.set_string_safe(col, y, &shown, target_st);
    col = col.saturating_add(shown.width() as u16);
    if !suffix.is_empty() {
        buf.set_string_safe(col, y, &suffix, dim);
        col = col.saturating_add(suffix.width() as u16);
    }
    if row.kind == ToolKind::Edit
        && !row.expanded
        && let Some((plus, minus)) = row.diffstat
    {
        let plus_s = format!(" +{plus}");
        buf.set_string_safe(col, y, &plus_s, role_style(ThemeRole::Success));
        col = col.saturating_add(plus_s.width() as u16);
        buf.set_string_safe(col, y, "/", dim);
        col = col.saturating_add(1);
        let minus_s = format!("-{minus}");
        buf.set_string_safe(col, y, &minus_s, role_style(ThemeRole::Error));
    }
    if row.kind == ToolKind::List
        && let Some(n) = row.entries
        && n > 0
    {
        let noun = if n == 1 { "entry" } else { "entries" };
        let suffix = format!(" ({n} {noun})");
        buf.set_string_safe(col, y, &suffix, dim);
    }
}

fn paint_detail(
    layout: &ToolDetailLayout,
    area: Rect,
    line_offset: usize,
    buf: &mut Buffer,
    cx: &FrameCtx<'_>,
) {
    for (draw_index, row) in layout
        .rows
        .iter()
        .skip(line_offset)
        .take(area.height as usize)
        .enumerate()
    {
        let y = area.y.saturating_add(draw_index as u16);
        let body_style = detail_style(row.style, cx);
        fill(
            buf,
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            },
            body_style,
        );
        paint_detail_line(row, area.x, y, area.width, body_style, cx, buf);
    }
}

/// Paint borrowed prepared spans directly so scrolling details does not clone
/// the source text or syntax span vector on every frame.
fn paint_detail_line(
    row: &ToolDetailLine,
    x: u16,
    y: u16,
    width: u16,
    body_style: Style,
    cx: &FrameCtx<'_>,
    buf: &mut Buffer,
) {
    let right = x.saturating_add(width);
    let gutter_style = body_style.patch(detail_gutter_style(row.style, cx));
    let mut column = x;
    for (index, source) in row.line.spans.iter().enumerate() {
        if column >= right {
            break;
        }
        let style = if index == 0 {
            gutter_style
        } else {
            let syntax_style = if let Some(theme) = cx.resolved_theme() {
                quantize_style_for(source.style, theme.context().color_level)
            } else {
                quantize_style(source.style)
            };
            body_style.patch(row.line.style).patch(syntax_style)
        };
        let span = Span::styled(source.content.as_ref(), style);
        buf.set_span_safe(column, y, &span, right.saturating_sub(column));
        column = column.saturating_add(source.content.width() as u16);
    }
}

fn detail_style(style: ToolDetailStyle, cx: &FrameCtx<'_>) -> Style {
    cx.style(detail_role(style), ThemeState::Normal)
}

fn detail_role(style: ToolDetailStyle) -> ThemeRole {
    match style {
        ToolDetailStyle::Plain | ToolDetailStyle::Context => ThemeRole::CodeBlock,
        ToolDetailStyle::FileMetadata => ThemeRole::Path,
        ToolDetailStyle::HunkHeader => ThemeRole::DiffHeader,
        ToolDetailStyle::Addition => ThemeRole::DiffAdded,
        ToolDetailStyle::Deletion => ThemeRole::DiffRemoved,
        ToolDetailStyle::NoNewlineMarker => ThemeRole::Muted,
    }
}

fn detail_gutter_style(style: ToolDetailStyle, cx: &FrameCtx<'_>) -> Style {
    let semantic = match style {
        ToolDetailStyle::Addition => ThemeRole::Success,
        ToolDetailStyle::Deletion => ThemeRole::Error,
        ToolDetailStyle::Context => ThemeRole::Muted,
        ToolDetailStyle::Plain
        | ToolDetailStyle::FileMetadata
        | ToolDetailStyle::HunkHeader
        | ToolDetailStyle::NoNewlineMarker => ThemeRole::Muted,
    };
    foreground_style(cx.style(semantic, ThemeState::Normal))
}

/// Keeps role foreground and modifiers while letting the component's surface
/// remain responsible for the background.
fn foreground_style(style: Style) -> Style {
    let mut foreground = Style::default().add_modifier(style.add_modifier);
    if let Some(color) = style.fg {
        foreground = foreground.fg(color);
    }
    foreground
}

fn verb_role(status: ToolStatus) -> ThemeRole {
    match status {
        ToolStatus::Running => ThemeRole::Accent,
        ToolStatus::Error => ThemeRole::Error,
        ToolStatus::Done => ThemeRole::Text,
        ToolStatus::Pending => ThemeRole::Muted,
    }
}

fn fmt_elapsed(d: Duration) -> String {
    let s = d.as_secs_f32();
    if s < 10.0 {
        format!("{s:.1}s")
    } else if d.as_secs() >= 60 {
        format!("{}m{:02}s", d.as_secs() / 60, d.as_secs() % 60)
    } else {
        format!("{}s", d.as_secs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui::core::{
        AnimationClock, BackgroundPolicy, Theme, ThemeAppearance, ThemeContext, ThemeDefinition,
        ThemeKind, ThemeResolver, ThemeRole, ThemeState,
    };
    use gbtui::runtime::{HitMap, HitResult};
    use std::time::Instant;

    fn ctx<'a>(
        theme: &'a Theme,
        anim: &'a mut AnimationClock,
        hits: &'a mut HitMap,
        mouse: Option<(u16, u16)>,
    ) -> FrameCtx<'a> {
        FrameCtx::new(theme, mouse, anim, hits)
    }

    #[test]
    fn pending_is_idle_and_gray_glyph() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = ctx(&theme, &mut anim, &mut hits, None);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 2));
        let row = ToolRow::new("bash", "cargo test").index(3);
        let mut state = ToolRowState::default();
        assert_eq!(row.measure(&mut state, 40), 1);
        row.render(&mut state, Rect::new(0, 0, 40, 2), &mut buf, &mut cx);
        assert_eq!(anim.demand(), TickDemand::None);
        assert_eq!(buf[(1, 0)].fg, theme.gray);
        assert_eq!(hits.hit(1, 0), HitResult::Hit(tool_id(3)));
    }

    #[test]
    fn running_requests_fast() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = ctx(&theme, &mut anim, &mut hits, None);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        let mut state = ToolRowState::default();
        ToolRow::new("bash", "ls")
            .status(ToolStatus::Running)
            .render(&mut state, Rect::new(0, 0, 40, 1), &mut buf, &mut cx);
        assert_eq!(anim.demand(), TickDemand::Fast);
    }

    #[test]
    fn done_uses_gray_diamond_when_collapsed() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = ctx(&theme, &mut anim, &mut hits, None);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        let mut state = ToolRowState::default();
        ToolRow::new("Read", "ok").status(ToolStatus::Done).render(
            &mut state,
            Rect::new(0, 0, 40, 1),
            &mut buf,
            &mut cx,
        );
        assert_eq!(buf[(1, 0)].symbol(), diamond_filled());
        assert_eq!(buf[(1, 0)].fg, theme.gray);
        assert_eq!(anim.demand(), TickDemand::None);
    }

    #[test]
    fn error_uses_error_diamond() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = ctx(&theme, &mut anim, &mut hits, None);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        let mut state = ToolRowState::default();
        ToolRow::new("Ran", "fail")
            .status(ToolStatus::Error)
            .render(&mut state, Rect::new(0, 0, 40, 1), &mut buf, &mut cx);
        assert_eq!(buf[(1, 0)].symbol(), diamond_filled());
        assert_eq!(buf[(1, 0)].fg, theme.error);
    }

    fn join_row(buf: &Buffer, y: u16, w: u16) -> String {
        (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect()
    }

    #[test]
    fn read_header_is_grok_prefix() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = ctx(&theme, &mut anim, &mut hits, None);
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 1));
        let mut state = ToolRowState::default();
        ToolRow::new("read", "crates/gbtui-core/src/search.rs")
            .kind(ToolKind::Read)
            .range(1, 40)
            .status(ToolStatus::Done)
            .render(&mut state, Rect::new(0, 0, 60, 1), &mut buf, &mut cx);
        let row = join_row(&buf, 0, 60);
        assert!(row.contains("Read "), "{row}");
        assert!(row.contains("(1-40)"), "{row}");
        assert!(row.contains("search.rs"), "{row}");
    }

    #[test]
    fn edit_header_uses_diffstat_slash() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = ctx(&theme, &mut anim, &mut hits, None);
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 1));
        let mut state = ToolRowState::default();
        ToolRow::new("edit", "crates/gbtui-core/src/search.rs")
            .kind(ToolKind::Edit)
            .diffstat(12, 3)
            .status(ToolStatus::Done)
            .render(&mut state, Rect::new(0, 0, 60, 1), &mut buf, &mut cx);
        let row = join_row(&buf, 0, 60);
        assert!(row.contains("Edit "), "{row}");
        assert!(row.contains("+12"), "{row}");
        assert!(row.contains("-3"), "{row}");
        assert!(row.contains("/"), "{row}");
    }

    #[test]
    fn run_header_uses_dollar() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = ctx(&theme, &mut anim, &mut hits, None);
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 1));
        let mut state = ToolRowState::default();
        ToolRow::new("bash", "cargo test -p gbtui-core search")
            .kind(ToolKind::Run)
            .status(ToolStatus::Done)
            .render(&mut state, Rect::new(0, 0, 50, 1), &mut buf, &mut cx);
        let row = join_row(&buf, 0, 50);
        assert!(row.contains("$ "), "{row}");
        assert!(row.contains("cargo test"), "{row}");
    }

    #[test]
    fn additional_host_tool_kinds_have_distinct_headers() {
        let cases = [
            (ToolKind::WebSearch, "Web search "),
            (ToolKind::WebFetch, "Fetch "),
            (ToolKind::MemorySearch, "Memory "),
            (ToolKind::UseTool, "Tool "),
            (ToolKind::Message, "Message "),
            (ToolKind::Hook, "Hook "),
        ];
        let theme = Theme::night();
        for (kind, prefix) in cases {
            let mut hits = HitMap::new();
            let mut anim = AnimationClock::new(Instant::now());
            let mut cx = ctx(&theme, &mut anim, &mut hits, None);
            let mut buf = Buffer::empty(Rect::new(0, 0, 48, 1));
            let mut state = ToolRowState::default();
            ToolRow::new("host", "summary").kind(kind).render(
                &mut state,
                Rect::new(0, 0, 48, 1),
                &mut buf,
                &mut cx,
            );
            let header = join_row(&buf, 0, 48);
            assert!(header.contains(prefix), "{kind:?}: {header}");
        }
    }

    #[test]
    fn expanded_detail_increases_height() {
        let row = ToolRow::new("bash", "cargo test")
            .detail("exit 0\nall passed")
            .expanded(true);
        let mut state = ToolRowState::default();
        assert!(row.measure(&mut state, 40) >= 2);
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = ctx(&theme, &mut anim, &mut hits, None);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
        let h = row.render(&mut state, Rect::new(0, 0, 40, 4), &mut buf, &mut cx);
        assert!(h >= 2);
        let joined: String = (0..40).map(|x| buf[(x, 1)].symbol().to_string()).collect();
        assert!(joined.contains("exit"), "{joined}");
    }

    fn line_text(line: &ToolDetailLine) -> String {
        line.line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn edit_detail_uses_dual_gutter_without_leaking_it_into_copy_text() {
        let detail = "--- a/f.rs\n+++ b/f.rs\n@@ -9,2 +10,2 @@\n keep\n-old\n+new";
        let row = ToolRow::new("edit", "f.rs")
            .kind(ToolKind::Edit)
            .detail(detail);
        let mut state = ToolRowState::default();
        let layout = row
            .detail_layout(&mut state, 80)
            .expect("nonempty detail layout");
        let deleted = layout
            .rows
            .iter()
            .find(|row| row.visible == "-old")
            .expect("deleted diff line");
        let display = line_text(deleted);
        assert!(display.contains("10"), "{display}");
        assert!(display.ends_with("-old"), "{display}");
        assert_eq!(
            &layout.text[deleted.content.clone()],
            deleted.visible.as_str()
        );
        assert!(!deleted.visible.contains('│'));
        assert!(deleted.content_x > 4);
    }

    #[test]
    fn expanded_edit_renders_hunk_header_and_numbered_change_rows() {
        let detail = "--- a/f.rs\n+++ b/f.rs\n@@ -9,2 +10,2 @@\n keep\n-old\n+new";
        let row = ToolRow::new("edit", "f.rs")
            .kind(ToolKind::Edit)
            .detail(detail)
            .expanded(true);
        let mut state = ToolRowState::default();
        let layout = row
            .detail_layout(&mut state, 80)
            .expect("nonempty detail layout");
        let deleted_row = layout
            .rows
            .iter()
            .position(|line| line.visible == "-old")
            .expect("deleted diff row");
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = ctx(&theme, &mut anim, &mut hits, None);
        let height = row.measure(&mut state, 80) as u16;
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, height));
        row.render(&mut state, Rect::new(0, 0, 80, height), &mut buf, &mut cx);
        let hunk = join_row(&buf, 3, 80);
        let deleted = join_row(&buf, deleted_row as u16 + 1, 80);
        assert!(hunk.contains("@@ -9,2 +10,2 @@"), "{hunk}");
        assert!(deleted.contains("│ -old"), "{deleted}");
        assert!(deleted.contains("10"), "{deleted}");
    }

    #[test]
    fn wrapped_diff_continuations_keep_source_ranges_and_blank_gutter() {
        let detail = "@@ -0,0 +1 @@\n+😀😀abcdef";
        let row = ToolRow::new("edit", "f.rs")
            .kind(ToolKind::Edit)
            .detail(detail);
        let mut state = ToolRowState::default();
        let layout = row
            .detail_layout(&mut state, 12)
            .expect("nonempty detail layout");
        let body_start = detail.find('\n').unwrap() + 1;
        let wrapped: Vec<_> = layout
            .rows
            .iter()
            .filter(|row| row.content.start >= body_start)
            .collect();
        assert!(wrapped.len() > 1);
        assert_eq!(
            wrapped
                .iter()
                .map(|row| row.visible.as_str())
                .collect::<String>(),
            "+😀😀abcdef"
        );
        for row in &wrapped {
            assert_eq!(&layout.text[row.content.clone()], row.visible.as_str());
        }
        assert!(wrapped[1].line.spans[0].content.as_ref().trim().is_empty());
    }

    #[test]
    fn clipped_diff_render_uses_the_same_physical_rows_as_layout() {
        let detail = "@@ -0,0 +1 @@\n+😀😀abcdef";
        let row = ToolRow::new("edit", "f.rs")
            .kind(ToolKind::Edit)
            .detail(detail)
            .expanded(true);
        let mut state = ToolRowState::default();
        let layout = row
            .detail_layout(&mut state, 12)
            .expect("nonempty detail layout");
        let expected = layout.rows[1].visible.clone();
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = ctx(&theme, &mut anim, &mut hits, None);
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 1));
        row.render_clipped(&mut state, Rect::new(0, 0, 12, 1), 2, &mut buf, &mut cx);
        let rendered = join_row(&buf, 0, 12);
        assert!(rendered.contains(&expected), "{rendered}");
    }

    #[test]
    fn malformed_edit_detail_stays_plain_and_source_complete() {
        let detail = "tool output\n@@ not a hunk @@\n+not a diff";
        let row = ToolRow::new("edit", "f.rs")
            .kind(ToolKind::Edit)
            .detail(detail);
        let mut state = ToolRowState::default();
        let layout = row
            .detail_layout(&mut state, 80)
            .expect("nonempty detail layout");
        assert_eq!(layout.text, detail);
        assert_eq!(
            layout
                .rows
                .iter()
                .map(|row| row.visible.as_str())
                .collect::<String>(),
            "tool output@@ not a hunk @@+not a diff"
        );
        assert!(layout.rows.iter().all(|row| row.content_x == 4));
    }

    #[test]
    fn multi_file_diff_preserves_copy_ranges_and_semantic_change_colors() {
        let detail = concat!(
            "diff --git a/a.rs b/a.rs\n",
            "--- a/a.rs\n",
            "+++ b/a.rs\n",
            "@@ -1 +1 @@\n",
            "-fn old() {}\n",
            "+fn new() {}\n",
            "diff --git a/b.py b/b.py\n",
            "--- a/b.py\n",
            "+++ b/b.py\n",
            "@@ -1 +1 @@\n",
            "-def old():\n",
            "+def new():"
        );
        let row = ToolRow::new("edit", "fallback.txt")
            .kind(ToolKind::Edit)
            .detail(detail)
            .expanded(true);
        let mut state = ToolRowState::default();
        let layout = row
            .detail_layout(&mut state, 100)
            .expect("nonempty detail layout");
        let python_added = layout
            .rows
            .iter()
            .position(|line| line.visible == "+def new():")
            .expect("python addition");
        let python_deleted = layout
            .rows
            .iter()
            .position(|line| line.visible == "-def old():")
            .expect("python deletion");
        for detail_line in layout
            .rows
            .iter()
            .filter(|line| line.visible.starts_with('+') || line.visible.starts_with('-'))
        {
            assert_eq!(
                &layout.text[detail_line.content.clone()],
                detail_line.visible.as_str()
            );
        }
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = ctx(&theme, &mut anim, &mut hits, None);
        let height = row.measure(&mut state, 100) as u16;
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, height));
        row.render(&mut state, Rect::new(0, 0, 100, height), &mut buf, &mut cx);
        assert_eq!(buf[(0, python_added as u16 + 1)].fg, theme.success);
        assert_eq!(buf[(0, python_deleted as u16 + 1)].fg, theme.error);
    }
    #[test]
    fn resolved_diff_uses_role_override_without_reintroducing_transparent_code_bg() {
        use ratatui::style::Color;

        let definition = ThemeDefinition::from(ThemeKind::Auto)
            .with_background(BackgroundPolicy::Transparent)
            .with_style(
                ThemeRole::DiffAdded,
                ThemeState::Normal,
                Style::default().fg(Color::LightGreen).bg(Color::Blue),
            );
        let resolver = ThemeResolver::new(
            definition,
            ThemeContext {
                appearance: Some(ThemeAppearance::Light),
                color_level: gbtui::core::ColorLevel::Basic,
                ..ThemeContext::default()
            },
        );
        let resolved = resolver.resolved();
        let row = ToolRow::new("edit", "f.rs")
            .kind(ToolKind::Edit)
            .detail("@@ -0,0 +1 @@\n+new")
            .expanded(true);
        let mut state = ToolRowState::default();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(resolved.palette(), None, &mut anim, &mut hits)
            .with_resolved_theme(resolved);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 3));
        row.render(&mut state, Rect::new(0, 0, 40, 3), &mut buf, &mut cx);

        assert_eq!(buf[(0, 1)].bg, Color::Reset);
        assert_eq!(buf[(0, 2)].bg, Color::Blue);
    }
}
