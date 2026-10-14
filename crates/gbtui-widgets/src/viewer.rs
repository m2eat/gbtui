//! Generic source-aware content viewer.
//!
//! [`ContentDocument`] is deliberately data-only: the host owns document
//! identity, revisions, source, styled rows, and optional metadata. The viewer
//! owns only interaction state. Clipboard access and activation stay with the
//! host; [`ContentViewer::extract`] returns text for the host to handle.

use std::ops::Range;
use std::sync::Arc;

use crossterm::event::{KeyEvent, KeyModifiers, MouseButton};
use gbtui_core::renderable::RenderContext;
use gbtui_core::{Renderable, SafeBuf};
use gbtui_runtime::{FrameCtx, Pointer};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    ListItem, ListPaneState, SearchMode, SearchQuery, WrapMode, click_scrollbar, is_scroll_id,
    parse_row_id_ns, render_list_items_ns,
};

/// Hit namespace used by [`ContentViewer`].
pub const NS_CONTENT_VIEWER: u32 = 0x4356;

/// Stable host-provided document identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentDocumentId(pub u64);

/// A styled logical row with the source line that produced it.
///
/// `id` must remain stable while the row represents the same logical content.
/// `copy_text` is deliberately separate from `line`: hosts can render line
/// numbers, folds, or other decoration without leaking them into copied text.
#[derive(Debug, Clone)]
pub struct ContentRow {
    id: usize,
    source_line: usize,
    line: Line<'static>,
    copy_text: Arc<str>,
    revision: u64,
    /// Bytes in the display line before `copy_text` begins. This lets a host
    /// render a non-copyable prefix such as a code gutter without corrupting
    /// search highlighting for the source content.
    display_prefix_bytes: usize,
}

impl ContentRow {
    /// Create a source-mapped styled row. By default its visible plain text is
    /// also its copy/search projection.
    pub fn new(id: usize, source_line: usize, line: Line<'static>) -> Self {
        let copy_text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        Self {
            id,
            source_line,
            line,
            copy_text: Arc::from(copy_text),
            revision: 0,
            display_prefix_bytes: 0,
        }
    }

    /// Set the text used for search and extraction, excluding display-only
    /// decoration.
    pub fn with_copy_text(mut self, copy_text: impl Into<Arc<str>>) -> Self {
        self.copy_text = copy_text.into();
        self
    }

    /// Set the number of display bytes preceding the copyable source text.
    ///
    /// This is appropriate for a byte-for-byte non-copyable prefix such as a
    /// code gutter. Richer projections may leave it at zero; search remains
    /// available even when a host chooses not to map every decoration.
    pub fn with_display_prefix_bytes(mut self, bytes: usize) -> Self {
        self.display_prefix_bytes = bytes;
        self
    }

    /// Set a host-owned formatting revision. Change it whenever this row's
    /// styled content or copy projection changes in place.
    pub fn revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn source_line(&self) -> usize {
        self.source_line
    }

    pub fn line(&self) -> &Line<'static> {
        &self.line
    }

    pub fn copy_text(&self) -> &str {
        &self.copy_text
    }
}

/// Host-owned source plus optional source-mapped styled rows.
#[derive(Debug, Clone)]
pub struct ContentDocument {
    id: ContentDocumentId,
    revision: u64,
    source: Arc<str>,
    rendered_rows: Option<Vec<ContentRow>>,
    metadata: Option<Arc<str>>,
}

impl ContentDocument {
    /// Create a source document with no rendered projection.
    pub fn new(id: ContentDocumentId, revision: u64, source: impl Into<Arc<str>>) -> Self {
        Self {
            id,
            revision,
            source: source.into(),
            rendered_rows: None,
            metadata: None,
        }
    }

    /// Attach a host-rendered, source-mapped projection.
    pub fn with_rendered_rows(mut self, rows: Vec<ContentRow>) -> Self {
        self.rendered_rows = Some(rows);
        self
    }

    /// Attach optional host metadata available through explicit extraction.
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

    pub fn rendered_rows(&self) -> Option<&[ContentRow]> {
        self.rendered_rows.as_deref()
    }

    pub fn metadata(&self) -> Option<&str> {
        self.metadata.as_deref()
    }
}

/// Which projection is currently displayed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ContentView {
    /// Styled rows when the host supplied them; source otherwise.
    #[default]
    Rendered,
    /// Exact host source split into source lines.
    Source,
}

/// An explicit host-owned extraction request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentCopyTarget {
    /// The currently selected row range in the current projection.
    Selection,
    /// Exact source text, including its original newline convention.
    Source,
    /// The complete rendered projection, excluding display-only decoration.
    Rendered,
    /// Optional metadata supplied by the host.
    Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DocumentVersion {
    id: ContentDocumentId,
    revision: u64,
}

#[derive(Debug, Clone)]
struct RawRow {
    id: usize,
    source_line: usize,
    range: Range<usize>,
}

#[derive(Debug, Clone, Copy)]
struct SourceAnchor {
    selected: Option<usize>,
    top: Option<usize>,
}

/// Stateful interaction and physical layout for a [`ContentDocument`].
///
/// The viewer delegates virtualization, scrolling, search highlighting,
/// range-selection visuals, hit registration, and scrollbars to
/// [`ListPaneState`]. It creates only borrowed row descriptors while drawing;
/// it never clones the document source or styled rows per frame.
#[derive(Debug, Clone)]
pub struct ContentViewer {
    list: ListPaneState,
    search: SearchQuery,
    view: ContentView,
    document: Option<DocumentVersion>,
    raw_rows: Vec<RawRow>,
    pending_anchor: Option<SourceAnchor>,
    dragging: bool,
    drag_extend: bool,
}

impl Default for ContentViewer {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentViewer {
    pub fn new() -> Self {
        let mut list = ListPaneState::with_wrap_mode(WrapMode::Wrap);
        list.set_visual_selection_enabled(true);
        let mut search = SearchQuery::new();
        // A content search highlights and navigates every source line; it must
        // not silently hide context the way a picker filter does.
        search.set_mode(SearchMode::Search);
        Self {
            list,
            search,
            view: ContentView::Rendered,
            document: None,
            raw_rows: Vec::new(),
            pending_anchor: None,
            dragging: false,
            drag_extend: false,
        }
    }

    pub fn view(&self, document: &ContentDocument) -> ContentView {
        if self.view == ContentView::Rendered && document.rendered_rows().is_none() {
            ContentView::Source
        } else {
            self.view
        }
    }

    pub fn wrapping(&self) -> bool {
        self.list.wrap_mode() == WrapMode::Wrap
    }

    /// Toggle physical soft wrapping without reparsing or copying content.
    pub fn toggle_wrapping(&mut self) {
        self.list.set_wrap_mode(if self.wrapping() {
            WrapMode::NoWrap
        } else {
            WrapMode::Wrap
        });
    }

    pub fn search_query(&self) -> &str {
        self.search.query()
    }

    pub fn search_error(&self) -> Option<&str> {
        self.search.error()
    }

    /// Set a query used to highlight every matching source row.
    pub fn set_search_query(&mut self, query: impl Into<String>) -> bool {
        let query = query.into();
        if self.search.query() == query {
            return false;
        }
        self.search.set_query(query);
        true
    }

    /// Change the current projection while retaining the selected and topmost
    /// source locations when the target projection maps them.
    pub fn set_view(&mut self, document: &ContentDocument, view: ContentView) -> bool {
        self.sync_document(document);
        let view = if view == ContentView::Rendered && document.rendered_rows().is_none() {
            ContentView::Source
        } else {
            view
        };
        if self.view == view {
            return false;
        }
        self.pending_anchor = Some(self.capture_anchor(document));
        self.view = view;
        self.list.invalidate_layout();
        // Ranges identify rows in the prior projection. Do not let matching
        // integer IDs select unrelated rendered rows after a mode switch.
        self.list.set_visual_selection_enabled(false);
        self.list.set_visual_selection_enabled(true);
        true
    }

    /// Toggle raw/rendered mode when a rendered projection exists.
    pub fn toggle_view(&mut self, document: &ContentDocument) -> bool {
        let target = match self.view(document) {
            ContentView::Rendered => ContentView::Source,
            ContentView::Source => ContentView::Rendered,
        };
        self.set_view(document, target)
    }

    /// Return the source line represented by the current cursor.
    pub fn selected_source_line(&self, document: &ContentDocument) -> Option<usize> {
        self.source_line_for_id(document, self.list.selected_id())
    }

    /// Return the source line at the physical top of the viewport.
    pub fn top_source_line(&self, document: &ContentDocument) -> Option<usize> {
        let visible = self.list.layout()?.item_at_y(self.list.offset)?;
        self.source_line_for_id(document, self.list.item_id(visible))
    }

    /// Scroll through physical terminal rows.
    pub fn scroll(&mut self, delta_lines: i32, viewport_rows: u16) -> bool {
        if delta_lines == 0 {
            return false;
        }
        let before = (self.list.selected, self.list.offset, self.list.follow);
        self.list.handle_scroll(
            delta_lines,
            self.list.visible_count(),
            viewport_rows as usize,
        );
        before != (self.list.selected, self.list.offset, self.list.follow)
    }

    /// Forward conventional list keyboard navigation to the physical layout.
    pub fn handle_key(&mut self, key: &KeyEvent, viewport_rows: u16) -> bool {
        self.list
            .handle_key(key, self.list.visible_count(), viewport_rows as usize)
    }

    /// Select the next (or previous) matching row, wrapping around the
    /// document. Render or call [`Self::prepare`] first so the physical layout
    /// has the current width.
    pub fn next_search_match(
        &mut self,
        document: &ContentDocument,
        viewport_rows: u16,
        backwards: bool,
    ) -> bool {
        self.sync_document(document);
        if self.search.query().is_empty() || self.list.visible_count() == 0 {
            return false;
        }
        let rows = row_refs(&self.raw_rows, document, self.view(document));
        let current = self
            .list
            .selected_id()
            .and_then(|id| rows.iter().position(|row| row.id() == id));
        let count = rows.len();
        let start = current.map_or(0, |index| {
            if backwards {
                index.checked_sub(1).unwrap_or(count.saturating_sub(1))
            } else {
                (index + 1) % count
            }
        });
        for step in 0..count {
            let index = if backwards {
                (start + count - step) % count
            } else {
                (start + step) % count
            };
            let row = rows[index];
            if self.search.find(row.copy_text()).is_some() {
                return self.list.select_item(row.id(), viewport_rows as usize);
            }
        }
        false
    }

    /// Handle a pointer delivered through this viewer's registered row or
    /// scrollbar hit ID. `extend_range` lets a host force range extension;
    /// a synchronous Shift modifier on pointer-down is captured for dragging.
    pub fn handle_pointer(
        &mut self,
        id: gbtui_core::HitId,
        pointer: Pointer,
        area: Rect,
        extend_range: bool,
    ) -> bool {
        if matches!(pointer, Pointer::Cancel { .. }) {
            let was_dragging = self.dragging;
            self.dragging = false;
            self.drag_extend = false;
            return was_dragging;
        }
        if is_scroll_id(NS_CONTENT_VIEWER, id) {
            match pointer {
                Pointer::Down { row, .. } | Pointer::Drag { row, .. } => {
                    let visible_count = self.list.visible_count();
                    click_scrollbar(&mut self.list, row, area, visible_count);
                    self.dragging = matches!(pointer, Pointer::Drag { .. });
                    self.drag_extend = false;
                    return true;
                }
                Pointer::Up { .. } => {
                    let was_dragging = self.dragging;
                    self.dragging = false;
                    self.drag_extend = false;
                    return was_dragging;
                }
                _ => return false,
            }
        }
        let Some(row_id) = parse_row_id_ns(NS_CONTENT_VIEWER, id) else {
            return false;
        };
        match pointer {
            Pointer::Down {
                button: MouseButton::Left,
                modifiers,
                ..
            } => {
                self.dragging = true;
                self.drag_extend = extend_range || modifiers.contains(KeyModifiers::SHIFT);
                self.list
                    .select_item_with_modifier(row_id, area.height as usize, self.drag_extend)
            }
            Pointer::Drag {
                button: MouseButton::Left,
                row,
                ..
            } if self.dragging => {
                let Some(row_id) = self.row_id_at_pointer_row(row, area) else {
                    return false;
                };
                self.list
                    .select_item_with_modifier(row_id, area.height as usize, self.drag_extend)
            }
            Pointer::Up {
                button: MouseButton::Left,
                ..
            } => {
                let was_dragging = self.dragging;
                self.dragging = false;
                self.drag_extend = false;
                was_dragging
            }
            _ => false,
        }
    }

    /// Extract text for a host-owned clipboard or other host action.
    ///
    /// This method performs no I/O. `Selection` follows the current raw or
    /// rendered projection; `Rendered` always means the supplied styled rows,
    /// regardless of the active view.
    pub fn extract(
        &mut self,
        document: &ContentDocument,
        target: ContentCopyTarget,
    ) -> Option<String> {
        self.sync_document(document);
        match target {
            ContentCopyTarget::Source => Some(document.source().to_owned()),
            ContentCopyTarget::Metadata => document.metadata().map(str::to_owned),
            ContentCopyTarget::Rendered => document.rendered_rows().map(|rows| {
                rows.iter()
                    .map(ContentRow::copy_text)
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
            ContentCopyTarget::Selection => {
                let rows = row_refs(&self.raw_rows, document, self.view(document));
                let renderables: Vec<_> = rows
                    .iter()
                    .copied()
                    .map(|row| ViewerRow::new(row, self.wrapping()))
                    .collect();
                let items: Vec<_> = rows
                    .iter()
                    .zip(&renderables)
                    .map(|(row, renderable)| {
                        ListItem::new(row.id(), renderable, row.copy_text())
                            .revision(row.revision())
                            .with_copy_text(row.copy_text())
                    })
                    .collect();
                self.list.selected_text(&items)
            }
        }
    }

    /// Prepare the current projection for keyboard/search interaction before
    /// a render pass. Rendering calls this automatically.
    pub fn prepare(&mut self, document: &ContentDocument, content_width: u16) {
        self.sync_document(document);
        let rows = row_refs(&self.raw_rows, document, self.view(document));
        let renderables: Vec<_> = rows
            .iter()
            .copied()
            .map(|row| ViewerRow::new(row, self.wrapping()))
            .collect();
        let items: Vec<_> = rows
            .iter()
            .zip(&renderables)
            .map(|(row, renderable)| {
                ListItem::new(row.id(), renderable, row.copy_text())
                    .revision(row.revision())
                    .with_copy_text(row.copy_text())
            })
            .collect();
        self.list
            .desired_item_height_at(&items, &self.search, content_width.max(1));
        let pending_anchor = self.pending_anchor.take();
        apply_pending_anchor(&mut self.list, pending_anchor, &rows, 1);
    }

    /// Render the current projection using the shared list engine.
    pub fn render(
        &mut self,
        document: &ContentDocument,
        area: Rect,
        buf: &mut Buffer,
        cx: &mut FrameCtx<'_>,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        self.sync_document(document);
        let rows = row_refs(&self.raw_rows, document, self.view(document));
        let renderables: Vec<_> = rows
            .iter()
            .copied()
            .map(|row| ViewerRow::new(row, self.wrapping()))
            .collect();
        let items: Vec<_> = rows
            .iter()
            .zip(&renderables)
            .map(|(row, renderable)| {
                ListItem::new(row.id(), renderable, row.copy_text())
                    .revision(row.revision())
                    .with_copy_text(row.copy_text())
            })
            .collect();
        // The list engine reserves its own prefix and optional scrollbar. This
        // initial preparation is only to resolve an anchor before the same
        // engine performs its final, exact-width preparation below.
        self.list
            .desired_item_height_at(&items, &self.search, area.width.saturating_sub(2).max(1));
        let pending_anchor = self.pending_anchor.take();
        apply_pending_anchor(&mut self.list, pending_anchor, &rows, area.height as usize);
        render_list_items_ns(
            &mut self.list,
            &items,
            &self.search,
            NS_CONTENT_VIEWER,
            area,
            buf,
            cx,
        );
    }

    fn sync_document(&mut self, document: &ContentDocument) {
        let version = DocumentVersion {
            id: document.id(),
            revision: document.revision(),
        };
        if self.document == Some(version) {
            return;
        }
        self.document = Some(version);
        self.raw_rows = source_rows(document.source());
        let wrap_mode = self.list.wrap_mode();
        self.list = ListPaneState::with_wrap_mode(wrap_mode);
        self.list.set_visual_selection_enabled(true);
        self.pending_anchor = None;
        self.dragging = false;
        self.drag_extend = false;
    }

    fn capture_anchor(&self, document: &ContentDocument) -> SourceAnchor {
        let selected = self.source_line_for_id(document, self.list.selected_id());
        let top = self
            .list
            .layout()
            .and_then(|layout| layout.item_at_y(self.list.offset))
            .and_then(|visible| self.source_line_for_id(document, self.list.item_id(visible)));
        SourceAnchor { selected, top }
    }

    fn row_id_at_pointer_row(&self, row: u16, area: Rect) -> Option<usize> {
        let physical_row = usize::from(row.saturating_sub(area.y));
        let virtual_row = self.list.offset.saturating_add(physical_row);
        self.list
            .layout()?
            .item_at_y(virtual_row)
            .and_then(|visible| self.list.item_id(visible))
    }

    fn source_line_for_id(&self, document: &ContentDocument, id: Option<usize>) -> Option<usize> {
        let id = id?;
        row_refs(&self.raw_rows, document, self.view(document))
            .into_iter()
            .find(|row| row.id() == id)
            .map(RowRef::source_line)
    }
}

fn apply_pending_anchor(
    list: &mut ListPaneState,
    pending_anchor: Option<SourceAnchor>,
    rows: &[RowRef<'_>],
    page: usize,
) {
    let Some(anchor) = pending_anchor else {
        return;
    };
    if let Some(source_line) = anchor.selected
        && let Some(row) = row_for_source(rows, source_line)
    {
        list.select_item(row.id(), page);
    }
    if let Some(source_line) = anchor.top
        && let Some(row) = row_for_source(rows, source_line)
        && let Some(index) =
            (0..list.visible_count()).find(|index| list.item_id(*index) == Some(row.id()))
        && let Some(layout) = list.layout()
    {
        let max_offset = layout.total_height().saturating_sub(page);
        list.offset = layout.virtual_y(index).min(max_offset);
    }
}

fn row_refs<'a>(
    raw_rows: &'a [RawRow],
    document: &'a ContentDocument,
    view: ContentView,
) -> Vec<RowRef<'a>> {
    match view {
        ContentView::Source => raw_rows
            .iter()
            .map(|row| RowRef::Raw {
                id: row.id,
                source_line: row.source_line,
                text: document.source().get(row.range.clone()).unwrap_or_default(),
                revision: document.revision(),
            })
            .collect(),
        ContentView::Rendered => document
            .rendered_rows()
            .map(|rows| rows.iter().map(RowRef::Styled).collect())
            .unwrap_or_else(|| {
                raw_rows
                    .iter()
                    .map(|row| RowRef::Raw {
                        id: row.id,
                        source_line: row.source_line,
                        text: document.source().get(row.range.clone()).unwrap_or_default(),
                        revision: document.revision(),
                    })
                    .collect()
            }),
    }
}

fn source_rows(source: &str) -> Vec<RawRow> {
    let mut rows = Vec::new();
    let mut start: usize = 0;
    for (source_line, line) in source.split('\n').enumerate() {
        let end = start.saturating_add(line.len());
        rows.push(RawRow {
            id: source_line,
            source_line,
            range: start..end,
        });
        start = end.saturating_add(1);
    }
    rows
}

fn row_for_source<'a>(rows: &'a [RowRef<'a>], source_line: usize) -> Option<RowRef<'a>> {
    rows.iter()
        .copied()
        .find(|row| row.source_line() == source_line)
        .or_else(|| {
            rows.iter()
                .copied()
                .find(|row| row.source_line() > source_line)
        })
        .or_else(|| rows.last().copied())
}

#[derive(Clone, Copy)]
enum RowRef<'a> {
    Raw {
        id: usize,
        source_line: usize,
        text: &'a str,
        revision: u64,
    },
    Styled(&'a ContentRow),
}

impl<'a> RowRef<'a> {
    fn id(self) -> usize {
        match self {
            Self::Raw { id, .. } => id,
            Self::Styled(row) => row.id(),
        }
    }

    fn source_line(self) -> usize {
        match self {
            Self::Raw { source_line, .. } => source_line,
            Self::Styled(row) => row.source_line(),
        }
    }

    fn copy_text(self) -> &'a str {
        match self {
            Self::Raw { text, .. } => text,
            Self::Styled(row) => row.copy_text(),
        }
    }

    fn revision(self) -> u64 {
        match self {
            Self::Raw { revision, .. } => revision,
            Self::Styled(row) => row.revision,
        }
    }
}

struct ViewerRow<'a> {
    row: RowRef<'a>,
    wrap: bool,
}

impl<'a> ViewerRow<'a> {
    fn new(row: RowRef<'a>, wrap: bool) -> Self {
        Self { row, wrap }
    }

    fn display_prefix_bytes(&self) -> usize {
        match self.row {
            RowRef::Raw { .. } => 0,
            RowRef::Styled(row) => row.display_prefix_bytes,
        }
    }
}

impl Renderable for ViewerRow<'_> {
    fn render_slice(&self, area: Rect, buf: &mut Buffer, line_offset: usize) {
        self.render_slice_with_context(
            area,
            buf,
            line_offset,
            RenderContext {
                source_id: None,
                search_text: "",
                match_ranges: &[],
                selected: false,
                hovered: false,
                style: Style::default(),
                match_style: Style::default(),
            },
        );
    }

    fn desired_height(&self, width: u16) -> usize {
        if !self.wrap {
            return 1;
        }
        match self.row {
            RowRef::Raw { text, .. } => wrapped_text_height(text, width.max(1)),
            RowRef::Styled(row) => wrapped_height(&row.line, width.max(1)),
        }
    }

    fn render_slice_with_context(
        &self,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        context: RenderContext<'_>,
    ) {
        match self.row {
            RowRef::Raw { text, .. } => {
                let line = Line::raw(text);
                render_viewer_line(
                    &line,
                    self.wrap,
                    self.display_prefix_bytes(),
                    area,
                    buf,
                    line_offset,
                    context,
                );
            }
            RowRef::Styled(row) => render_viewer_line(
                &row.line,
                self.wrap,
                self.display_prefix_bytes(),
                area,
                buf,
                line_offset,
                context,
            ),
        }
    }
}

fn render_viewer_line(
    line: &Line<'_>,
    wrap: bool,
    prefix: usize,
    area: Rect,
    buf: &mut Buffer,
    line_offset: usize,
    context: RenderContext<'_>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = area.width.max(1) as usize;
    let mut physical = 0usize;
    let mut column = 0usize;
    let mut display_bytes = 0usize;
    for span in &line.spans {
        let mut base_style = context.style.patch(span.style);
        if (context.selected || context.hovered)
            && let Some(background) = context.style.bg
        {
            base_style = base_style.bg(background);
        }
        for grapheme in span.content.graphemes(true) {
            if grapheme == "\n" {
                physical = physical.saturating_add(1);
                column = 0;
                display_bytes = display_bytes.saturating_add(grapheme.len());
                continue;
            }
            let grapheme_width = grapheme.width();
            if wrap && grapheme_width > 0 && column > 0 && column + grapheme_width > width {
                physical = physical.saturating_add(1);
                column = 0;
            }
            let matched_range = display_bytes
                .checked_sub(prefix)
                .map(|start| start..start.saturating_add(grapheme.len()));
            let matched = matched_range.is_some_and(|range| {
                context
                    .match_ranges
                    .iter()
                    .any(|hit| hit.start < range.end && range.start < hit.end)
            });
            if physical >= line_offset
                && physical < line_offset.saturating_add(area.height as usize)
                && grapheme_width <= width
                && (grapheme_width == 0 || column.saturating_add(grapheme_width) <= width)
            {
                let y = area.y.saturating_add((physical - line_offset) as u16);
                let x = area.x.saturating_add(column as u16);
                let style = if matched {
                    base_style.patch(context.match_style)
                } else {
                    base_style
                };
                buf.set_string_safe(x, y, grapheme, style);
            }
            if grapheme_width > 0 {
                column = column.saturating_add(grapheme_width);
            }
            display_bytes = display_bytes.saturating_add(grapheme.len());
        }
    }
}

fn wrapped_text_height(text: &str, width: u16) -> usize {
    let width = width.max(1) as usize;
    let mut rows = 1usize;
    let mut column = 0usize;
    for grapheme in text.graphemes(true) {
        let grapheme_width = grapheme.width();
        if grapheme_width > 0 && column > 0 && column + grapheme_width > width {
            rows = rows.saturating_add(1);
            column = 0;
        }
        if grapheme_width > 0 {
            column = column.saturating_add(grapheme_width);
        }
    }
    rows
}

fn wrapped_height(line: &Line<'_>, width: u16) -> usize {
    let width = width.max(1) as usize;
    let mut rows = 1usize;
    let mut column = 0usize;
    for span in &line.spans {
        for grapheme in span.content.graphemes(true) {
            if grapheme == "\n" {
                rows = rows.saturating_add(1);
                column = 0;
                continue;
            }
            let grapheme_width = grapheme.width();
            if grapheme_width > 0 && column > 0 && column + grapheme_width > width {
                rows = rows.saturating_add(1);
                column = 0;
            }
            if grapheme_width > 0 {
                column = column.saturating_add(grapheme_width);
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::HitMap;
    use ratatui::buffer::Buffer;
    use ratatui::text::{Line, Span};
    use std::time::Instant;

    fn render(viewer: &mut ContentViewer, document: &ContentDocument, area: Rect) -> Buffer {
        let theme = Theme::night();
        let mut clock = AnimationClock::new(Instant::now());
        let mut hits = HitMap::new();
        let mut buffer = Buffer::empty(area);
        let mut cx = FrameCtx::new(&theme, None, &mut clock, &mut hits);
        viewer.render(document, area, &mut buffer, &mut cx);
        buffer
    }

    #[test]
    fn raw_rendering_wraps_utf8_at_narrow_widths() {
        let document = ContentDocument::new(ContentDocumentId(1), 1, "甲😀a");
        let mut viewer = ContentViewer::new();
        let buffer = render(&mut viewer, &document, Rect::new(0, 0, 4, 3));
        assert_eq!(buffer[(2, 0)].symbol(), "甲");
        assert_eq!(buffer[(2, 1)].symbol(), "😀");
        assert_eq!(buffer[(2, 2)].symbol(), "a");
    }

    #[test]
    fn toggle_preserves_source_cursor_and_top_location() {
        let document = ContentDocument::new(ContentDocumentId(1), 1, "abcdefghij\none\ntwo")
            .with_rendered_rows(vec![
                ContentRow::new(100, 0, Line::raw("FIRST")),
                ContentRow::new(101, 1, Line::raw("ONE")),
                ContentRow::new(102, 2, Line::raw("TWO")),
            ]);
        let mut viewer = ContentViewer::new();
        let area = Rect::new(0, 0, 8, 2);
        viewer.set_view(&document, ContentView::Source);
        let _ = render(&mut viewer, &document, area);
        assert!(viewer.list.select_item(1, 2));
        // The source row before `one` occupies two physical rows here.
        viewer.list.offset = 2;

        assert!(viewer.toggle_view(&document));
        let _ = render(&mut viewer, &document, area);

        assert_eq!(viewer.selected_source_line(&document), Some(1));
        assert_eq!(viewer.top_source_line(&document), Some(1));
    }

    #[test]
    fn mode_toggle_remeasures_same_id_rows_at_narrow_widths() {
        let document =
            ContentDocument::new(ContentDocumentId(5), 1, "abcd").with_rendered_rows(vec![
                ContentRow::new(0, 0, Line::raw(" 1 │ abcd"))
                    .with_copy_text("abcd")
                    .with_display_prefix_bytes(" 1 │ ".len()),
            ]);
        let mut viewer = ContentViewer::new();
        let area = Rect::new(0, 0, 6, 3);

        let _ = render(&mut viewer, &document, area);
        assert_eq!(viewer.list.total_height(), 3);

        assert!(viewer.toggle_view(&document));
        let _ = render(&mut viewer, &document, area);
        assert_eq!(viewer.list.total_height(), 1);
    }

    #[test]
    fn extraction_excludes_display_decorations_and_keeps_metadata_explicit() {
        let document = ContentDocument::new(ContentDocumentId(1), 1, "let value = 1;\n")
            .with_rendered_rows(vec![
                ContentRow::new(
                    9,
                    0,
                    Line::from(vec![Span::raw(" 1 │ "), Span::raw("let value = 1;")]),
                )
                .with_copy_text("let value = 1;")
                .with_display_prefix_bytes(" 1 │ ".len()),
            ])
            .with_metadata("src/example.rs");
        let mut viewer = ContentViewer::new();

        assert_eq!(
            viewer.extract(&document, ContentCopyTarget::Rendered),
            Some("let value = 1;".into())
        );
        assert_eq!(
            viewer.extract(&document, ContentCopyTarget::Metadata),
            Some("src/example.rs".into())
        );
        assert_eq!(
            viewer.extract(&document, ContentCopyTarget::Source),
            Some("let value = 1;\n".into())
        );
    }

    #[test]
    fn visual_range_keys_copy_source_rows_without_gutters() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let document = ContentDocument::new(ContentDocumentId(3), 1, "first\nsecond")
            .with_rendered_rows(vec![
                ContentRow::new(31, 0, Line::raw(" 1 │ first"))
                    .with_copy_text("first")
                    .with_display_prefix_bytes(" 1 │ ".len()),
                ContentRow::new(32, 1, Line::raw(" 2 │ second"))
                    .with_copy_text("second")
                    .with_display_prefix_bytes(" 2 │ ".len()),
            ]);
        let mut viewer = ContentViewer::new();
        let _ = render(&mut viewer, &document, Rect::new(0, 0, 16, 2));

        assert!(viewer.handle_key(&KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE), 2,));
        assert!(viewer.handle_key(&KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT), 2,));
        assert_eq!(
            viewer.extract(&document, ContentCopyTarget::Selection),
            Some("first\nsecond".into())
        );
    }

    #[test]
    fn shift_click_captures_range_intent_for_drag_selection() {
        use crossterm::event::KeyModifiers;

        let document = ContentDocument::new(ContentDocumentId(4), 1, "first\nsecond");
        let mut viewer = ContentViewer::new();
        let area = Rect::new(0, 0, 16, 2);
        let _ = render(&mut viewer, &document, area);

        assert!(viewer.handle_pointer(
            crate::row_id_ns(NS_CONTENT_VIEWER, 0),
            Pointer::Down {
                button: MouseButton::Left,
                col: 2,
                row: 0,
                clicks: 1,
                modifiers: KeyModifiers::NONE,
            },
            area,
            false,
        ));
        assert!(viewer.handle_pointer(
            crate::row_id_ns(NS_CONTENT_VIEWER, 1),
            Pointer::Down {
                button: MouseButton::Left,
                col: 2,
                row: 1,
                clicks: 1,
                modifiers: KeyModifiers::SHIFT,
            },
            area,
            false,
        ));
        assert_eq!(
            viewer.extract(&document, ContentCopyTarget::Selection),
            Some("first\nsecond".into())
        );
    }

    #[test]
    fn drag_uses_pointer_row_after_original_row_is_captured() {
        use crossterm::event::KeyModifiers;

        let document = ContentDocument::new(ContentDocumentId(6), 1, "first\nsecond");
        let mut viewer = ContentViewer::new();
        let area = Rect::new(0, 0, 16, 2);
        let _ = render(&mut viewer, &document, area);
        let captured_id = crate::row_id_ns(NS_CONTENT_VIEWER, 0);

        assert!(viewer.handle_pointer(
            captured_id,
            Pointer::Down {
                button: MouseButton::Left,
                col: 2,
                row: 0,
                clicks: 1,
                modifiers: KeyModifiers::SHIFT,
            },
            area,
            false,
        ));
        assert!(viewer.handle_pointer(
            captured_id,
            Pointer::Drag {
                button: MouseButton::Left,
                col: 2,
                row: 1,
            },
            area,
            false,
        ));
        assert_eq!(
            viewer.extract(&document, ContentCopyTarget::Selection),
            Some("first\nsecond".into())
        );
    }

    #[test]
    fn cancellation_stops_a_captured_drag_without_selecting_its_later_motion() {
        use crossterm::event::KeyModifiers;

        let document = ContentDocument::new(ContentDocumentId(7), 1, "first\nsecond");
        let mut viewer = ContentViewer::new();
        let area = Rect::new(0, 0, 16, 2);
        let _ = render(&mut viewer, &document, area);
        let captured_id = crate::row_id_ns(NS_CONTENT_VIEWER, 0);

        assert!(viewer.handle_pointer(
            captured_id,
            Pointer::Down {
                button: MouseButton::Left,
                col: 2,
                row: 0,
                clicks: 1,
                modifiers: KeyModifiers::SHIFT,
            },
            area,
            false,
        ));
        assert!(viewer.handle_pointer(
            captured_id,
            Pointer::Cancel {
                button: MouseButton::Left,
                col: 2,
                row: 0,
            },
            area,
            false,
        ));
        assert!(!viewer.handle_pointer(
            captured_id,
            Pointer::Drag {
                button: MouseButton::Left,
                col: 2,
                row: 1,
            },
            area,
            false,
        ));
        assert_eq!(
            viewer.extract(&document, ContentCopyTarget::Selection),
            Some("first".into())
        );
    }

    #[test]
    fn replacing_a_document_clears_the_prior_range() {
        let first = ContentDocument::new(ContentDocumentId(1), 1, "first\nsecond");
        let second = ContentDocument::new(ContentDocumentId(1), 2, "other");
        let mut viewer = ContentViewer::new();
        let _ = render(&mut viewer, &first, Rect::new(0, 0, 12, 2));
        assert!(viewer.list.select_item_with_modifier(0, 2, false));
        assert!(viewer.list.select_item_with_modifier(1, 2, true));
        assert_eq!(
            viewer.extract(&first, ContentCopyTarget::Selection),
            Some("first\nsecond".into())
        );

        let _ = render(&mut viewer, &second, Rect::new(0, 0, 12, 2));
        assert_eq!(viewer.extract(&second, ContentCopyTarget::Selection), None);
    }
}
