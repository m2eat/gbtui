//! Optional Markdown and code adapters for the generic [`gbtui::ContentViewer`].
//!
//! Parsing and syntax highlighting remain in `gbtui-markdown`. These adapters
//! only retain the renderer output as source-mapped [`ContentRow`] values so a
//! generic host can use the same scroll/search/selection viewer for documents
//! that happen to be Markdown or source code.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gbtui::core::{ResolvedTheme, ThemeRole, ThemeState, color_support::ColorLevel};
use gbtui::{ContentDocument, ContentDocumentId, ContentRow, Theme};
use gbtui_markdown::{MarkdownStyle, StreamingMarkdownRenderer, SyntaxTheme, Syntect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::markdown::{
    MarkdownTheme, legacy_syntect_for, quantize_line_for, quantize_style_for, syntax_theme_for,
    syntect_for, viewer_copy_text,
};

/// Cached streaming Markdown projection for [`gbtui::ContentViewer`].
///
/// The host supplies a stable identity and revision. Append-only streaming
/// updates reuse [`StreamingMarkdownRenderer`] and its existing syntax caches;
/// replacement source resets only this document's renderer state.
#[derive(Debug, Clone)]
pub struct MarkdownDocument {
    id: ContentDocumentId,
    revision: u64,
    source: Arc<str>,
    metadata: Option<Arc<str>>,
    renderer: StreamingMarkdownRenderer,
    style: MarkdownStyle,
    theme_version: Option<u64>,
    syntax_theme: SyntaxTheme,
    color_level: ColorLevel,
    rendered_source: Arc<str>,
    rendered_streaming: bool,
    table_width: Option<usize>,
    content: ContentDocument,
    generation: u64,
    content_dirty: bool,
}

impl MarkdownDocument {
    /// Create an adapter with host-owned source. Call [`Self::document`] with
    /// the current theme and viewport width before rendering it.
    pub fn new(id: ContentDocumentId, revision: u64, source: impl Into<Arc<str>>) -> Self {
        let source = source.into();
        let style = MarkdownStyle::default();
        let mut renderer = StreamingMarkdownRenderer::new(style, true);
        renderer.set_collapse_soft_breaks(false);
        Self {
            id,
            revision,
            source: Arc::clone(&source),
            metadata: None,
            renderer,
            style,
            theme_version: None,
            syntax_theme: SyntaxTheme::Dark,
            color_level: gbtui::core::color_support::get(),
            rendered_source: Arc::from(""),
            rendered_streaming: false,
            table_width: None,
            content: ContentDocument::new(id, revision, source),
            generation: 0,
            content_dirty: true,
        }
    }

    /// Replace or append the host source. For an append-only streaming update,
    /// the existing renderer and highlighter state are retained.
    pub fn update(&mut self, id: ContentDocumentId, revision: u64, source: impl Into<Arc<str>>) {
        let source = source.into();
        if self.id == id && self.revision == revision && self.source == source {
            return;
        }
        self.id = id;
        self.revision = revision;
        self.source = source;
        self.content_dirty = true;
    }

    /// Attach metadata available to the host through
    /// [`gbtui::ContentCopyTarget::Metadata`].
    pub fn with_metadata(mut self, metadata: impl Into<Arc<str>>) -> Self {
        self.metadata = Some(metadata.into());
        self.content_dirty = true;
        self
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Return source plus styled rows for the supplied palette and table width.
    ///
    /// `streaming` means the source may grow further; setting it to `false`
    /// finalizes the renderer's pending tail without reparsing stable content.
    pub fn document(
        &mut self,
        theme: &Theme,
        content_width: u16,
        streaming: bool,
    ) -> &ContentDocument {
        self.document_with_theme(MarkdownTheme::from_palette(theme), content_width, streaming)
    }

    /// Return source plus styled rows from a resolved semantic theme.
    pub fn document_resolved(
        &mut self,
        theme: &ResolvedTheme,
        content_width: u16,
        streaming: bool,
    ) -> &ContentDocument {
        self.document_with_theme(
            MarkdownTheme::from_resolved(theme),
            content_width,
            streaming,
        )
    }

    fn document_with_theme(
        &mut self,
        theme: MarkdownTheme,
        content_width: u16,
        streaming: bool,
    ) -> &ContentDocument {
        let MarkdownTheme {
            style,
            theme_version,
            syntax_theme,
            color_level,
        } = theme;
        let syntect = theme.syntect();
        let width = Some(content_width.max(1) as usize);
        let theme_changed = self.style != style
            || self.theme_version != theme_version
            || self.syntax_theme != syntax_theme
            || self.color_level != color_level;
        let width_changed = self.table_width != width;
        if theme_changed {
            self.style = style;
            self.theme_version = theme_version;
            self.syntax_theme = syntax_theme;
            self.color_level = color_level;
            self.renderer.set_style(style);
            self.content_dirty = true;
        }
        if width_changed {
            self.table_width = width;
            self.renderer.set_max_table_width(width);
            self.content_dirty = true;
        }

        let source_changed = self.source != self.rendered_source;
        if source_changed {
            if streaming && let Some(chunk) = self.source.strip_prefix(&*self.rendered_source) {
                self.renderer.push_and_render(chunk, Some(syntect));
            } else {
                self.renderer = StreamingMarkdownRenderer::new(self.style, true);
                self.renderer.set_collapse_soft_breaks(false);
                self.renderer.set_max_table_width(self.table_width);
                self.renderer.push(&self.source);
                if streaming {
                    self.renderer.render(Some(syntect));
                } else {
                    self.renderer.finish(Some(syntect));
                }
            }
            self.rendered_source = Arc::clone(&self.source);
            self.content_dirty = true;
        } else if theme_changed || width_changed {
            self.renderer.render(Some(syntect));
            self.content_dirty = true;
        }

        if !streaming && self.rendered_streaming {
            self.renderer.finish(Some(syntect));
            self.content_dirty = true;
        }
        self.rendered_streaming = streaming;
        if self.content_dirty {
            self.rebuild_content();
        }
        &self.content
    }

    fn rebuild_content(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let view = self.renderer.view();
        let mut occurrences = std::collections::BTreeMap::<usize, usize>::new();
        let rows = view
            .lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                let source_line = view.line_source_map.get(index).copied().unwrap_or(index);
                let occurrence = occurrences.entry(source_line).or_default();
                let id = stable_row_id(source_line, *occurrence);
                *occurrence = occurrence.saturating_add(1);
                ContentRow::new(
                    id,
                    source_line,
                    quantize_line_for(line.clone(), self.color_level),
                )
                .with_copy_text(viewer_copy_text(view, index))
                .revision(self.generation)
            })
            .collect();
        let mut content = ContentDocument::new(self.id, self.revision, Arc::clone(&self.source))
            .with_rendered_rows(rows);
        if let Some(metadata) = &self.metadata {
            content = content.with_metadata(Arc::clone(metadata));
        }
        self.content = content;
        self.content_dirty = false;
    }
}

/// Host-owned code source plus optional language/path metadata.
#[derive(Debug, Clone)]
pub struct CodeDocument {
    id: ContentDocumentId,
    revision: u64,
    source: Arc<str>,
    language: Option<Arc<str>>,
    path: Option<PathBuf>,
    metadata: Option<Arc<str>>,
}

impl CodeDocument {
    pub fn new(id: ContentDocumentId, revision: u64, source: impl Into<Arc<str>>) -> Self {
        Self {
            id,
            revision,
            source: source.into(),
            language: None,
            path: None,
            metadata: None,
        }
    }

    /// Use a language token supported by the shared [`Syntect`](gbtui_markdown::Syntect)
    /// highlighter when no path-based syntax wins.
    pub fn with_language(mut self, language: impl Into<Arc<str>>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Prefer syntax selected from this path's extension.
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Attach host metadata for explicit extraction.
    pub fn with_metadata(mut self, metadata: impl Into<Arc<str>>) -> Self {
        self.metadata = Some(metadata.into());
        self
    }

    pub fn id(&self) -> ContentDocumentId {
        self.id
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeViewStamp {
    Palette {
        id: ContentDocumentId,
        revision: u64,
        theme: Theme,
    },
    Resolved {
        id: ContentDocumentId,
        revision: u64,
        version: u64,
        palette: Theme,
        code_style: Style,
        gutter_style: Style,
        syntax_theme: SyntaxTheme,
        color_level: ColorLevel,
    },
}

/// Cached syntax-highlighted [`ContentDocument`] for a [`CodeDocument`].
///
/// Gutters are display-only spans; each row's copy/search projection is the
/// original code line, never its line number or separator.
#[derive(Debug, Default)]
pub struct CodeView {
    content: Option<ContentDocument>,
    stamp: Option<CodeViewStamp>,
    generation: u64,
}

impl CodeView {
    pub fn new() -> Self {
        Self::default()
    }

    /// Re-highlight only when the host revision or palette changes.
    pub fn document<'a>(
        &'a mut self,
        document: &CodeDocument,
        theme: &Theme,
    ) -> &'a ContentDocument {
        let stamp = CodeViewStamp::Palette {
            id: document.id,
            revision: document.revision,
            theme: *theme,
        };
        if self.stamp != Some(stamp) {
            self.rebuild(
                document,
                theme.style(ThemeRole::CodeBlock, ThemeState::Normal),
                theme
                    .style(ThemeRole::Muted, ThemeState::Normal)
                    .add_modifier(Modifier::DIM),
                legacy_syntect_for(theme),
                gbtui::core::color_support::get(),
            );
            self.stamp = Some(stamp);
        }
        self.content
            .as_ref()
            .expect("CodeView always builds content before returning")
    }

    /// Re-highlight from a resolved semantic theme.
    ///
    /// The stamp includes both resolver version and rendered style inputs:
    /// versions are resolver-local, while the style snapshot prevents a stale
    /// cache when a different resolver is supplied.
    pub fn document_resolved<'a>(
        &'a mut self,
        document: &CodeDocument,
        theme: &ResolvedTheme,
    ) -> &'a ContentDocument {
        let context = theme.context();
        let code_style = theme.style(ThemeRole::CodeBlock, ThemeState::Normal);
        let gutter_style = theme
            .style(ThemeRole::Muted, ThemeState::Normal)
            .add_modifier(Modifier::DIM);
        let syntax_theme = syntax_theme_for(Some(theme.appearance()));
        let stamp = CodeViewStamp::Resolved {
            id: document.id,
            revision: document.revision,
            version: theme.version(),
            palette: *theme.palette(),
            code_style,
            gutter_style,
            syntax_theme,
            color_level: context.color_level,
        };
        if self.stamp != Some(stamp) {
            self.rebuild(
                document,
                code_style,
                gutter_style,
                syntect_for(Some(theme.appearance())),
                context.color_level,
            );
            self.stamp = Some(stamp);
        }
        self.content
            .as_ref()
            .expect("CodeView always builds content before returning")
    }

    fn rebuild(
        &mut self,
        document: &CodeDocument,
        code_style: Style,
        gutter_style: Style,
        syntect: &Syntect,
        color_level: ColorLevel,
    ) {
        self.generation = self.generation.wrapping_add(1);
        let highlighted = document
            .path()
            .and_then(|path| syntect.highlight_text_by_file_path(path, document.source()))
            .or_else(|| {
                document.language().and_then(|language| {
                    syntect.highlight_text_for_token(language, document.source())
                })
            });
        let lines: Vec<_> = document.source().split('\n').collect();
        let gutter_width = lines.len().max(1).to_string().len();
        let gutter_style = quantize_style_for(gutter_style, color_level);
        let code_style = quantize_style_for(code_style, color_level);
        let rows = lines
            .iter()
            .enumerate()
            .map(|(index, original)| {
                let source_line = original.strip_suffix('\r').unwrap_or(original);
                let gutter = format!("{:>width$} │ ", index + 1, width = gutter_width);
                let mut spans = Vec::new();
                spans.push(Span::styled(gutter.clone(), gutter_style));
                if let Some(highlighted) = highlighted.as_ref().and_then(|rows| rows.get(index)) {
                    spans.extend(highlighted.iter().cloned().map(|span| {
                        Span::styled(span.text, quantize_style_for(span.style, color_level))
                    }));
                } else {
                    spans.push(Span::raw(source_line.to_owned()));
                }
                ContentRow::new(index, index, Line::from(spans).style(code_style))
                    .with_copy_text(source_line)
                    .with_display_prefix_bytes(gutter.len())
                    .revision(self.generation)
            })
            .collect();
        let mut content =
            ContentDocument::new(document.id, document.revision, Arc::clone(&document.source))
                .with_rendered_rows(rows);
        if let Some(metadata) = &document.metadata {
            content = content.with_metadata(Arc::clone(metadata));
        }
        self.content = Some(content);
    }
}

fn stable_row_id(source_line: usize, occurrence: usize) -> usize {
    // Source lines are the stable anchor. An occurrence distinguishes renderer
    // rows (for example, table lines) that map to the same source line.
    source_line.wrapping_mul(1_000_003).wrapping_add(occurrence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui::core::{
        BackgroundPolicy, ColorLevel, ThemeAppearance, ThemeContext, ThemeDefinition, ThemeKind,
        ThemeResolver, ThemeRole, ThemeState,
    };
    use ratatui::style::Color;

    #[test]
    fn code_copy_projection_excludes_the_gutter() {
        let document =
            CodeDocument::new(ContentDocumentId(7), 1, "let answer = 42;\n").with_language("rust");
        let mut view = CodeView::new();
        let rendered = view.document(&document, &Theme::night());
        assert_eq!(
            rendered
                .rendered_rows()
                .unwrap()
                .first()
                .map(ContentRow::copy_text),
            Some("let answer = 42;")
        );
        assert_eq!(
            rendered.source(),
            "let answer = 42;\n",
            "whole-source extraction remains host source"
        );
    }
    #[test]
    fn resolved_code_view_resets_transparent_surface_and_quantizes_syntax() {
        let resolver = ThemeResolver::new(
            ThemeDefinition::from(ThemeKind::Auto).with_background(BackgroundPolicy::Transparent),
            ThemeContext {
                appearance: Some(ThemeAppearance::Light),
                color_level: ColorLevel::Basic,
                ..ThemeContext::default()
            },
        );
        let document =
            CodeDocument::new(ContentDocumentId(10), 1, "let answer = 42;\n").with_language("rust");
        let mut view = CodeView::new();
        let rendered = view.document_resolved(&document, resolver.resolved());
        let row = rendered
            .rendered_rows()
            .and_then(|rows| rows.first())
            .expect("resolved code row");
        assert_eq!(row.line().style.bg, Some(Color::Reset));
        for style in row
            .line()
            .spans
            .iter()
            .map(|span| span.style)
            .chain(std::iter::once(row.line().style))
        {
            assert!(
                !matches!(style.fg, Some(Color::Rgb(_, _, _) | Color::Indexed(_)))
                    && !matches!(style.bg, Some(Color::Rgb(_, _, _) | Color::Indexed(_))),
                "resolved Basic-color code output must not retain RGB or indexed styles: {style:?}"
            );
        }
    }

    #[test]
    fn explicit_theme_kind_controls_syntax_despite_opposite_terminal_appearance() {
        let document =
            CodeDocument::new(ContentDocumentId(13), 1, "let answer = 42;\n").with_language("rust");
        for (kind, opposite, matching) in [
            (
                ThemeKind::Day,
                ThemeAppearance::Dark,
                ThemeAppearance::Light,
            ),
            (
                ThemeKind::Night,
                ThemeAppearance::Light,
                ThemeAppearance::Dark,
            ),
        ] {
            let definition =
                ThemeDefinition::from(kind).with_background(BackgroundPolicy::Transparent);
            let actual = ThemeResolver::new(
                definition.clone(),
                ThemeContext {
                    appearance: Some(opposite),
                    ..ThemeContext::default()
                },
            );
            let expected = ThemeResolver::new(
                definition,
                ThemeContext {
                    appearance: Some(matching),
                    ..ThemeContext::default()
                },
            );
            let mut actual_view = CodeView::new();
            let mut expected_view = CodeView::new();
            let actual = actual_view.document_resolved(&document, actual.resolved());
            let expected = expected_view.document_resolved(&document, expected.resolved());
            assert_eq!(
                actual.rendered_rows().unwrap()[0].line(),
                expected.rendered_rows().unwrap()[0].line(),
                "explicit {kind:?} must select syntax colors independently of terminal appearance",
            );
        }
    }

    #[test]
    fn markdown_rows_follow_the_renderer_source_map() {
        let mut document = MarkdownDocument::new(ContentDocumentId(8), 1, "# title\n\nbody\n");
        let rendered = document.document(&Theme::night(), 40, false);
        let rows = rendered.rendered_rows().unwrap();
        assert!(rows.iter().any(|row| row.source_line() == 0));
        assert!(rows.iter().any(|row| row.source_line() == 2));
    }
    #[test]
    fn resolved_markdown_keeps_link_override_and_transparent_code_surface() {
        let definition = ThemeDefinition::from(ThemeKind::Auto)
            .with_background(BackgroundPolicy::Transparent)
            .with_style(
                ThemeRole::Link,
                ThemeState::Normal,
                Style::default().fg(Color::Magenta),
            );
        let resolver = ThemeResolver::new(
            definition,
            ThemeContext {
                appearance: Some(ThemeAppearance::Light),
                color_level: ColorLevel::Basic,
                ..ThemeContext::default()
            },
        );
        let mut document = MarkdownDocument::new(
            ContentDocumentId(9),
            1,
            "See [docs](https://example.test).\n\n```rust\nlet answer = 42;\n```\n",
        );
        let rendered = document.document_resolved(resolver.resolved(), 80, false);
        let rows = rendered.rendered_rows().expect("resolved markdown rows");
        let link = rows
            .iter()
            .flat_map(|row| row.line().spans.iter())
            .find(|span| span.content == "docs")
            .expect("rendered link text");
        assert_eq!(link.style.fg, Some(Color::Magenta));
        assert!(link.style.add_modifier.contains(Modifier::UNDERLINED));

        let code = rows
            .iter()
            .find(|row| row.copy_text().contains("let answer = 42;"))
            .expect("rendered code row");
        assert_eq!(code.line().style.bg, Some(Color::Reset));
        assert!(
            code.line().spans.iter().all(|span| span.style.bg.is_none()),
            "syntax must not overwrite the terminal-default code surface"
        );
        for row in rows {
            for style in std::iter::once(row.line().style)
                .chain(row.line().spans.iter().map(|span| span.style))
            {
                assert!(
                    !matches!(style.fg, Some(Color::Rgb(_, _, _) | Color::Indexed(_)))
                        && !matches!(style.bg, Some(Color::Rgb(_, _, _) | Color::Indexed(_))),
                    "resolved Basic-color output must not retain RGB or indexed styles: {style:?}"
                );
            }
        }
    }

    #[test]
    fn resolved_code_cache_observes_versioned_role_overrides() {
        let context = ThemeContext {
            appearance: Some(ThemeAppearance::Light),
            color_level: ColorLevel::Basic,
            ..ThemeContext::default()
        };
        let mut resolver = ThemeResolver::new(
            ThemeDefinition::from(ThemeKind::Auto).with_background(BackgroundPolicy::Transparent),
            context,
        );
        let document = CodeDocument::new(ContentDocumentId(11), 1, "plain text");
        let mut view = CodeView::new();
        view.document_resolved(&document, resolver.resolved());

        assert!(
            resolver.set_definition(
                ThemeDefinition::from(ThemeKind::Auto)
                    .with_background(BackgroundPolicy::Transparent)
                    .with_style(
                        ThemeRole::CodeBlock,
                        ThemeState::Normal,
                        Style::default().fg(Color::Magenta),
                    ),
            )
        );

        let row = view
            .document_resolved(&document, resolver.resolved())
            .rendered_rows()
            .and_then(|rows| rows.first())
            .expect("re-resolved code row");
        assert_eq!(row.line().style.fg, Some(Color::Magenta));
        assert_eq!(row.line().style.bg, Some(Color::Reset));
    }
}
