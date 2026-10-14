//! Conversation transcript using the streaming `gbtui-markdown` renderer for
//! assistant and expanded-thinking bodies.
//!
//! ```text
//!  ❯ write a failing test                              3:13 PM
//!
//!  The matcher is green.
//!  `cargo test` is cyan like Grok markdown.
//!
//!  Worked for 14m33s
//!  ◆ session_end                                [hooks: 2]
//! ```
//!
//! # Keys
//!
//! `↑`/`↓`/`j`/`k` scroll by physical row · `g`/`Home` top · `G`/`End`
//! follow-tail · `PgUp`/`PgDn` page · wheel via [`TranscriptState::handle_scroll`].

use std::{sync::Arc, time::Duration};

use crossterm::event::{KeyEvent, MouseButton};
use gbtui::core::{HitId, ResolvedTheme, SafeBuf, ThemeRole, ThemeState, wrapping::wrap_text};
use gbtui::key;
use gbtui::runtime::{FrameCtx, Pointer};
use gbtui::terminal::glyphs::{PROMPT_ARROW_WIDTH, diamond_filled, prompt_arrow};
use gbtui::{SearchQuery, Spinner, TickDemand};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::markdown::{
    MarkdownCache, MarkdownKind, MarkdownLayout, MarkdownSegment, MarkdownTheme,
};
use crate::paint::fill;
use crate::tool_row::{ToolDetailLayout, ToolKind, ToolRow, ToolRowState};
/// Hit namespace for transcript items. Index is the item index in the slice.
pub const NS_TRANSCRIPT: u32 = 0xA011;
/// Whole-pane body so wheel hits land even on padding.
pub const BODY: HitId = HitId::named("gbtui.agent.transcript.body");

/// Blank physical row between blocks (grok vpad).
const GAP: usize = 1;

pub fn item_id(index: u32) -> HitId {
    HitId::namespaced(NS_TRANSCRIPT, index)
}

pub fn parse_item_id(id: HitId) -> Option<u32> {
    (id.namespace() == NS_TRANSCRIPT).then_some(id.index())
}

/// Borrowed row. Host owns the backing data.
#[derive(Debug, Clone, Copy)]
pub enum TranscriptItem<'a> {
    User {
        author: Option<&'a str>,
        text: &'a str,
        time: Option<&'a str>,
    },
    Assistant {
        author: Option<&'a str>,
        text: &'a str,
        time: Option<&'a str>,
        streaming: bool,
    },
    Thinking {
        text: &'a str,
        expanded: bool,
        elapsed: Option<std::time::Duration>,
        streaming: bool,
    },
    System {
        text: &'a str,
    },
    /// Turn-complete marker (`Worked for 14m33s`).
    Marker {
        text: &'a str,
    },
    /// Lifecycle / hook row (`◆ session_end  [hooks: 2]`).
    Hook {
        name: &'a str,
        count: u32,
    },
    Tool(ToolRow<'a>),
}

/// How a detail-bearing transcript block occupies the scrollback.
///
/// [`Collapsed`](Self::Collapsed) exposes only its header. [`Truncated`](Self::Truncated)
/// retains the first physical rows, including the header. [`Expanded`](Self::Expanded)
/// exposes the full body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptDisplay {
    Collapsed,
    Truncated(usize),
    Expanded,
}

/// Which detail-bearing transcript blocks a display operation affects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptDisplayTarget {
    Item(usize),
    All,
    Thinking,
}

/// Transcript block kinds addressable by [`TranscriptState::navigate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptNavigation {
    UserTurn,
    AssistantResponse,
}

/// Direction for transcript search and block navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptDirection {
    Forward,
    Backward,
}

/// A grapheme-safe full-text match in a transcript item's rendered search projection.
///
/// `range` is a byte range in the source field containing the hit. For body
/// text it maps to the item's selectable source; header hits map to their
/// corresponding rendered header field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptSearchMatch {
    pub item: usize,
    pub range: std::ops::Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchField {
    Body,
    ToolLabel,
    ToolName,
    ToolSummary,
    Marker,
    HookName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SearchLocation {
    field: SearchField,
    physical_row: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SearchSource {
    ptr: usize,
    len: usize,
    header_ptr: usize,
    header_len: usize,
    name_ptr: usize,
    name_len: usize,
    kind: u8,
    tool_kind: Option<ToolKind>,
}

#[derive(Debug, Clone, Default)]
struct TranscriptSearch {
    query: SearchQuery,
    matches: Vec<TranscriptSearchMatch>,
    locations: Vec<SearchLocation>,
    selected: Option<usize>,
    sources: Vec<SearchSource>,
    width: u16,
    dirty: bool,
}

/// Scroll offset, follow-tail state, display modes, and cached transcript
/// layouts for the current borrowed item slice.
#[derive(Debug, Clone)]
pub struct TranscriptState {
    pub offset: usize,
    pub follow: bool,
    markdown: MarkdownCache,
    displays: Vec<Option<TranscriptDisplay>>,
    tool_rows: Vec<Option<ToolRowState>>,
    tool_layouts: Vec<Option<Arc<ToolDetailLayout>>>,
    search: TranscriptSearch,
    navigation: Option<(TranscriptNavigation, usize)>,
    layout_width: u16,
}

impl Default for TranscriptState {
    fn default() -> Self {
        Self {
            offset: 0,
            follow: true,
            markdown: MarkdownCache::default(),
            displays: Vec::new(),
            tool_rows: Vec::new(),
            tool_layouts: Vec::new(),
            search: TranscriptSearch::default(),
            navigation: None,
            layout_width: 0,
        }
    }
}

impl TranscriptState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prepare cached rich-text layout before measuring or handling pointer input.
    /// Rendering calls this automatically; hosts may also use it for pre-draw sizing.
    pub fn prepare(&mut self, items: &[TranscriptItem<'_>], width: u16, theme: &gbtui::Theme) {
        self.prepare_with_markdown(items, width, MarkdownTheme::from_palette(theme));
    }

    /// Prepare rich-text layout from a resolved semantic theme.
    ///
    /// The resolver version is part of each slot's cache stamp, so semantic
    /// overrides re-render frozen markdown even when its source is unchanged.
    pub fn prepare_resolved(
        &mut self,
        items: &[TranscriptItem<'_>],
        width: u16,
        theme: &ResolvedTheme,
    ) {
        self.prepare_with_markdown(items, width, MarkdownTheme::from_resolved(theme));
    }

    fn prepare_with_markdown(
        &mut self,
        items: &[TranscriptItem<'_>],
        width: u16,
        theme: MarkdownTheme,
    ) {
        let theme_version = theme.theme_version;
        let syntax_theme = theme.syntax_theme;
        let syntect = theme.syntect();
        self.displays.truncate(items.len());
        self.tool_rows.truncate(items.len());
        self.tool_layouts.truncate(items.len());
        self.layout_width = width;
        if self.tool_rows.len() < items.len() {
            self.tool_rows.resize(items.len(), None);
            self.tool_layouts.resize(items.len(), None);
        }
        for (index, item) in items.iter().enumerate() {
            if let TranscriptItem::Tool(row) = item {
                let prepare_detail = self.display(index, item) != TranscriptDisplay::Collapsed;
                let state = self.tool_rows[index].get_or_insert_with(ToolRowState::default);
                self.tool_layouts[index] = prepare_detail
                    .then(|| row.detail.filter(|detail| !detail.is_empty()))
                    .flatten()
                    .and_then(|_| {
                        row.detail_layout_with_syntax(
                            state,
                            width,
                            theme_version,
                            syntax_theme,
                            syntect,
                        )
                    });
            } else {
                self.tool_rows[index] = None;
                self.tool_layouts[index] = None;
            }
        }
        let entries = items.iter().enumerate().filter_map(|(index, item)| {
            markdown_spec(item, width).map(|(kind, text, streaming, content_width)| {
                (kind, index, text, streaming, content_width)
            })
        });
        self.markdown.sync(entries, theme);
    }

    fn markdown_layout(
        &self,
        index: usize,
        item: &TranscriptItem<'_>,
        width: u16,
    ) -> Option<&MarkdownLayout> {
        let (kind, ..) = markdown_spec(item, width)?;
        self.markdown.layout(index, kind)
    }

    fn markdown_text(&self, index: usize, item: &TranscriptItem<'_>) -> Option<&str> {
        let kind = match item {
            TranscriptItem::Assistant { .. } => MarkdownKind::Assistant,
            TranscriptItem::Thinking { .. } => MarkdownKind::Thinking,
            _ => return None,
        };
        self.markdown
            .layout(index, kind)
            .map(|layout| layout.text.as_str())
    }

    fn display(&self, index: usize, item: &TranscriptItem<'_>) -> TranscriptDisplay {
        self.displays
            .get(index)
            .copied()
            .flatten()
            .unwrap_or(match item {
                TranscriptItem::Thinking { expanded, .. } => {
                    if *expanded {
                        TranscriptDisplay::Expanded
                    } else {
                        TranscriptDisplay::Collapsed
                    }
                }
                TranscriptItem::Tool(row) => {
                    if row.expanded {
                        TranscriptDisplay::Expanded
                    } else {
                        TranscriptDisplay::Collapsed
                    }
                }
                _ => TranscriptDisplay::Expanded,
            })
    }

    fn supports_display(item: &TranscriptItem<'_>) -> bool {
        matches!(item, TranscriptItem::Thinking { .. })
            || matches!(item, TranscriptItem::Tool(row) if row.detail.is_some_and(|detail| !detail.is_empty()))
    }

    fn normalize_display(display: TranscriptDisplay) -> TranscriptDisplay {
        match display {
            TranscriptDisplay::Truncated(0) => TranscriptDisplay::Collapsed,
            display => display,
        }
    }

    /// Set a display mode for one block, every detail-bearing block, or only
    /// thinking blocks. This does not alter scroll follow state.
    pub fn set_display(
        &mut self,
        items: &[TranscriptItem<'_>],
        target: TranscriptDisplayTarget,
        display: TranscriptDisplay,
    ) -> bool {
        let display = Self::normalize_display(display);
        if self.displays.len() < items.len() {
            self.displays.resize(items.len(), None);
        }
        let mut changed = false;
        for (index, item) in items.iter().enumerate() {
            let selected = match target {
                TranscriptDisplayTarget::Item(item) => index == item,
                TranscriptDisplayTarget::All => Self::supports_display(item),
                TranscriptDisplayTarget::Thinking => {
                    matches!(item, TranscriptItem::Thinking { .. })
                }
            };
            if selected && Self::supports_display(item) && self.display(index, item) != display {
                self.displays[index] = Some(display);
                changed = true;
            }
        }
        changed
    }

    /// Toggle one detail-bearing block between collapsed and expanded.
    pub fn toggle_display(&mut self, items: &[TranscriptItem<'_>], index: usize) -> bool {
        let Some(item) = items.get(index) else {
            return false;
        };
        if !Self::supports_display(item) {
            return false;
        }
        let display = match self.display(index, item) {
            TranscriptDisplay::Expanded => TranscriptDisplay::Collapsed,
            TranscriptDisplay::Collapsed | TranscriptDisplay::Truncated(_) => {
                TranscriptDisplay::Expanded
            }
        };
        self.set_display(items, TranscriptDisplayTarget::Item(index), display)
    }

    /// The effective display mode for an item, including its host-provided
    /// initial `expanded` value when no transcript override exists.
    pub fn display_mode(&self, index: usize, item: &TranscriptItem<'_>) -> TranscriptDisplay {
        self.display(index, item)
    }

    fn item_height(&self, index: usize, item: &TranscriptItem<'_>, width: u16) -> usize {
        let full = measure_content(self, index, item, width).max(1);
        match self.display(index, item) {
            TranscriptDisplay::Collapsed if Self::supports_display(item) => 1,
            TranscriptDisplay::Truncated(rows) if Self::supports_display(item) => {
                full.min(rows.max(1))
            }
            TranscriptDisplay::Collapsed
            | TranscriptDisplay::Expanded
            | TranscriptDisplay::Truncated(_) => full,
        }
    }

    /// Physical content rows occupied by all items, including inter-item gaps.
    ///
    /// Call [`Self::prepare`] or either render function first, with the same
    /// slice and width. This reads the prepared layout without parsing again.
    pub fn transcript_height(&self, items: &[TranscriptItem<'_>], width: u16) -> usize {
        total_height(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| self.item_height(index, item, width)),
        )
    }

    /// Replace the active full-text query. Matches are refreshed lazily from
    /// prepared source projections without reparsing Markdown.
    pub fn set_search_query(&mut self, query: impl Into<String>) -> bool {
        let query = query.into();
        if self.search.query.query() == query {
            return false;
        }
        self.search.query.set_query(query);
        self.search.matches.clear();
        self.search.locations.clear();
        self.search.selected = None;
        self.search.dirty = true;
        true
    }

    /// The active full-text query.
    pub fn search_query(&self) -> &str {
        self.search.query.query()
    }

    /// Mark a same-length in-place host data update for search refresh.
    ///
    /// Streaming growth is detected automatically. Hosts that replace item
    /// text without changing its allocation or byte length must call this.
    pub fn invalidate_search(&mut self) {
        self.search.dirty = true;
    }

    /// Return matches in the full prepared transcript projection.
    ///
    /// Call [`Self::prepare`] or render first with the same width; this method
    /// reads cached Markdown and tool projections without reparsing them.
    pub fn search_matches(
        &mut self,
        items: &[TranscriptItem<'_>],
        width: u16,
    ) -> &[TranscriptSearchMatch] {
        self.sync_search(items, width);
        &self.search.matches
    }

    /// Return the selected full-text match, if any.
    pub fn selected_search_match(
        &mut self,
        items: &[TranscriptItem<'_>],
        width: u16,
    ) -> Option<TranscriptSearchMatch> {
        self.sync_search(items, width);
        self.search
            .selected
            .and_then(|index| self.search.matches.get(index))
            .cloned()
    }

    /// Select the next match, wrapping from the final hit to the first.
    pub fn next_search_match(
        &mut self,
        items: &[TranscriptItem<'_>],
        width: u16,
        viewport_rows: u16,
    ) -> Option<TranscriptSearchMatch> {
        self.select_search_match(items, width, viewport_rows, TranscriptDirection::Forward)
    }

    /// Select the previous match, wrapping from the first hit to the final.
    pub fn previous_search_match(
        &mut self,
        items: &[TranscriptItem<'_>],
        width: u16,
        viewport_rows: u16,
    ) -> Option<TranscriptSearchMatch> {
        self.select_search_match(items, width, viewport_rows, TranscriptDirection::Backward)
    }

    fn select_search_match(
        &mut self,
        items: &[TranscriptItem<'_>],
        width: u16,
        viewport_rows: u16,
        direction: TranscriptDirection,
    ) -> Option<TranscriptSearchMatch> {
        self.sync_search(items, width);
        let len = self.search.matches.len();
        if len == 0 {
            self.search.selected = None;
            return None;
        }
        let selected = match (self.search.selected, direction) {
            (Some(index), TranscriptDirection::Forward) => (index + 1) % len,
            (Some(0), TranscriptDirection::Backward) => len - 1,
            (Some(index), TranscriptDirection::Backward) => index - 1,
            (None, TranscriptDirection::Forward) => 0,
            (None, TranscriptDirection::Backward) => len - 1,
        };
        self.search.selected = Some(selected);
        let hit = self.search.matches[selected].clone();
        self.reveal_search_match(items, width, viewport_rows, selected);
        Some(hit)
    }

    /// Jump among user turns or assistant responses, wrapping at either end.
    pub fn navigate(
        &mut self,
        items: &[TranscriptItem<'_>],
        width: u16,
        viewport_rows: u16,
        target: TranscriptNavigation,
        direction: TranscriptDirection,
    ) -> Option<usize> {
        let previous = self.navigation.and_then(|(kind, index)| {
            (kind == target
                && items
                    .get(index)
                    .is_some_and(|item| Self::navigation_matches(item, target)))
            .then_some(index)
        });
        let matching = |(index, item): (usize, &TranscriptItem<'_>)| {
            Self::navigation_matches(item, target).then_some(index)
        };
        let item = match (previous, direction) {
            (Some(index), TranscriptDirection::Forward) => items
                .iter()
                .enumerate()
                .skip(index.saturating_add(1))
                .find_map(matching)
                .or_else(|| items.iter().enumerate().find_map(matching)),
            (Some(index), TranscriptDirection::Backward) => items
                .iter()
                .enumerate()
                .take(index)
                .rev()
                .find_map(matching)
                .or_else(|| items.iter().enumerate().rev().find_map(matching)),
            (None, TranscriptDirection::Forward) => items.iter().enumerate().find_map(matching),
            (None, TranscriptDirection::Backward) => {
                items.iter().enumerate().rev().find_map(matching)
            }
        };
        let Some(item) = item else {
            self.navigation = None;
            return None;
        };
        self.navigation = Some((target, item));
        self.reveal_item(items, width, viewport_rows, item);
        Some(item)
    }

    fn navigation_matches(item: &TranscriptItem<'_>, target: TranscriptNavigation) -> bool {
        matches!(
            (target, item),
            (TranscriptNavigation::UserTurn, TranscriptItem::User { .. })
                | (
                    TranscriptNavigation::AssistantResponse,
                    TranscriptItem::Assistant { .. }
                )
        )
    }

    fn reveal_item(
        &mut self,
        items: &[TranscriptItem<'_>],
        width: u16,
        viewport_rows: u16,
        item: usize,
    ) {
        let start: usize = items
            .iter()
            .enumerate()
            .take(item)
            .map(|(index, item)| self.item_height(index, item, width).saturating_add(GAP))
            .sum();
        let content_rows = self.transcript_height(items, width);
        let max = content_rows.saturating_sub(usize::from(viewport_rows.max(1)));
        self.follow = false;
        self.offset = start.min(max);
    }

    fn reveal_search_match(
        &mut self,
        items: &[TranscriptItem<'_>],
        width: u16,
        viewport_rows: u16,
        selected: usize,
    ) {
        let Some(item_index) = self.search.matches.get(selected).map(|hit| hit.item) else {
            return;
        };
        let Some(location) = self.search.locations.get(selected).copied() else {
            return;
        };
        let Some(item) = items.get(item_index) else {
            return;
        };
        if location.field == SearchField::Body
            && Self::supports_display(item)
            && location.physical_row >= self.item_height(item_index, item, width)
        {
            if self.displays.len() < items.len() {
                self.displays.resize(items.len(), None);
            }
            self.displays[item_index] = Some(TranscriptDisplay::Expanded);
        }
        self.reveal_item_row(
            items,
            width,
            viewport_rows,
            item_index,
            location.physical_row,
        );
    }

    fn reveal_item_row(
        &mut self,
        items: &[TranscriptItem<'_>],
        width: u16,
        viewport_rows: u16,
        item: usize,
        physical_row: usize,
    ) {
        let start = items
            .iter()
            .enumerate()
            .take(item)
            .map(|(index, item)| self.item_height(index, item, width).saturating_add(GAP))
            .sum::<usize>();
        let target = start.saturating_add(physical_row);
        let viewport = usize::from(viewport_rows.max(1));
        let content_rows = self.transcript_height(items, width);
        let max = content_rows.saturating_sub(viewport);
        let offset = if self.follow {
            max
        } else {
            self.offset.min(max)
        };
        self.follow = false;
        self.offset = if target < offset {
            target
        } else if target >= offset.saturating_add(viewport) {
            target.saturating_add(1).saturating_sub(viewport).min(max)
        } else {
            offset
        };
    }

    fn search_sources_current(&self, items: &[TranscriptItem<'_>], width: u16) -> bool {
        self.search.width == width
            && self.search.sources.len() == items.len()
            && items
                .iter()
                .enumerate()
                .all(|(index, item)| self.search.sources[index] == self.search_source(index, item))
    }

    fn search_source(&self, index: usize, item: &TranscriptItem<'_>) -> SearchSource {
        let (text, header, name, kind, tool_kind) = match *item {
            TranscriptItem::User { text, .. } => (text, "", "", 1, None),
            TranscriptItem::Assistant { .. } => (
                self.markdown_text(index, item).unwrap_or_default(),
                "",
                "",
                2,
                None,
            ),
            TranscriptItem::Thinking { .. } => (
                self.markdown_text(index, item).unwrap_or_default(),
                "",
                "",
                3,
                None,
            ),
            TranscriptItem::System { text } => (text, "", "", 4, None),
            TranscriptItem::Marker { text } => (text, "", "", 5, None),
            TranscriptItem::Hook { name, .. } => (name, "", "", 6, None),
            TranscriptItem::Tool(row) => {
                let text = row.detail.unwrap_or_default();
                let name = if row.kind == ToolKind::Generic {
                    row.name
                } else {
                    ""
                };
                (text, row.summary, name, 7, Some(row.kind))
            }
        };
        SearchSource {
            ptr: text.as_ptr() as usize,
            len: text.len(),
            header_ptr: header.as_ptr() as usize,
            header_len: header.len(),
            name_ptr: name.as_ptr() as usize,
            name_len: name.len(),
            kind,
            tool_kind,
        }
    }

    fn prepare_search_details(&mut self, items: &[TranscriptItem<'_>], width: u16) {
        if self.tool_rows.len() < items.len() {
            self.tool_rows.resize(items.len(), None);
            self.tool_layouts.resize(items.len(), None);
        }
        for (index, item) in items.iter().enumerate() {
            let TranscriptItem::Tool(row) = item else {
                continue;
            };
            let state = self.tool_rows[index].get_or_insert_with(ToolRowState::default);
            self.tool_layouts[index] = row
                .detail
                .filter(|detail| !detail.is_empty())
                .and_then(|_| row.detail_layout(state, width));
        }
    }

    fn sync_search(&mut self, items: &[TranscriptItem<'_>], width: u16) {
        if self.search.query.query().is_empty() {
            self.search.matches.clear();
            self.search.locations.clear();
            self.search.selected = None;
            self.search.sources.clear();
            self.search.width = width;
            self.search.dirty = false;
            return;
        }
        if !self.search.dirty && self.search_sources_current(items, width) {
            return;
        }
        self.prepare_search_details(items, width);
        let selected = self.search.selected.and_then(|index| {
            self.search
                .matches
                .get(index)
                .cloned()
                .zip(self.search.locations.get(index).copied())
        });
        let mut matches = Vec::new();
        let mut locations = Vec::new();
        let mut sources = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            sources.push(self.search_source(index, item));
            collect_search_matches(
                self,
                &self.search.query,
                index,
                item,
                width,
                &mut matches,
                &mut locations,
            );
        }
        self.search.matches = matches;
        self.search.locations = locations;
        self.search.sources = sources;
        self.search.width = width;
        self.search.selected = selected.and_then(|(selected, location)| {
            self.search
                .matches
                .iter()
                .zip(&self.search.locations)
                .position(|(candidate, candidate_location)| {
                    candidate == &selected && candidate_location.field == location.field
                })
        });
        self.search.dirty = false;
    }

    fn clamp(&mut self, content_rows: usize, viewport_rows: u16) {
        let max = content_rows.saturating_sub(usize::from(viewport_rows.max(1)));
        if self.follow {
            self.offset = max;
        } else {
            self.offset = self.offset.min(max);
        }
    }

    /// Apply a key using physical content and viewport row counts.
    pub fn handle_key(&mut self, key: &KeyEvent, content_rows: usize, viewport_rows: u16) -> bool {
        if content_rows == 0 {
            return false;
        }
        let page = usize::from(viewport_rows.max(1));
        let max = content_rows.saturating_sub(page);
        let offset = if self.follow {
            max
        } else {
            self.offset.min(max)
        };
        if key!(Down).matches(key) || key!('j').matches(key) {
            self.follow = false;
            self.offset = offset.saturating_add(1).min(max);
            return true;
        }
        if key!(Up).matches(key) || key!('k').matches(key) {
            self.follow = false;
            self.offset = offset.saturating_sub(1);
            return true;
        }
        if key!(Home).matches(key) || key!('g').matches(key) {
            self.follow = false;
            self.offset = 0;
            return true;
        }
        if key!(End).matches(key) || key!('G').matches(key) {
            self.follow = true;
            self.offset = max;
            return true;
        }
        if key!(PageDown).matches(key) {
            self.follow = false;
            self.offset = offset.saturating_add(page).min(max);
            return true;
        }
        if key!(PageUp).matches(key) {
            self.follow = false;
            self.offset = offset.saturating_sub(page);
            return true;
        }
        false
    }

    /// Apply a wheel delta using physical content and viewport row counts.
    pub fn handle_scroll(&mut self, delta_lines: i32, content_rows: usize, viewport_rows: u16) {
        if content_rows == 0 || delta_lines == 0 {
            return;
        }
        let max = content_rows.saturating_sub(usize::from(viewport_rows.max(1)));
        let offset = if self.follow {
            max
        } else {
            self.offset.min(max)
        };
        self.follow = false;
        self.offset = if delta_lines > 0 {
            offset.saturating_add(delta_lines as usize).min(max)
        } else {
            offset.saturating_sub(delta_lines.unsigned_abs() as usize)
        };
    }
}

/// A text-selection caret in a transcript item.
///
/// `item` is the host slice index and `byte` is a UTF-8 grapheme boundary in
/// that item's selectable body. It never refers to chrome such as timestamps,
/// prompt arrows, tool headers, or Markdown display decorations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranscriptTextPosition {
    pub item: usize,
    pub byte: usize,
}

#[derive(Debug, Clone, Copy)]
struct SelectionPoint {
    position: TranscriptTextPosition,
    fingerprint: u64,
}

/// Protocol-free mouse selection for a borrowed [`TranscriptItem`] slice.
///
/// The host retains transcript data and forwards pointer events through
/// [`Self::handle_pointer`]. Copying is deliberately left to the host: call
/// [`Self::selected_text`] and pass the result to its `ClipboardProvider`.
#[derive(Debug, Clone, Default)]
pub struct TranscriptSelection {
    anchor: Option<SelectionPoint>,
    head: Option<SelectionPoint>,
    dragging: bool,
}

impl TranscriptSelection {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remove the current selection.
    pub fn clear(&mut self) {
        self.anchor = None;
        self.head = None;
        self.dragging = false;
    }

    /// Whether two distinct transcript positions are currently selected.
    pub fn has_selection(&self) -> bool {
        matches!(
            (self.anchor, self.head),
            (Some(anchor), Some(head)) if anchor.position != head.position
        )
    }

    /// Update selection from a runtime pointer event.
    ///
    /// Supply the same `state`, `items`, and scrollback `area` used by
    /// [`render_transcript_with_selection`]. Left-button down, drag, up, and
    /// cancellation participate. Cancellation keeps the last selected range
    /// but stops extending it; callers remain responsible for other controls.
    pub fn handle_pointer(
        &mut self,
        pointer: Pointer,
        state: &TranscriptState,
        items: &[TranscriptItem<'_>],
        area: Rect,
    ) -> bool {
        match pointer {
            Pointer::Down {
                button: MouseButton::Left,
                col,
                row,
                ..
            } => {
                let point = selection_point_at(state, items, area, col, row);
                self.anchor = point;
                self.head = point;
                self.dragging = point.is_some();
                true
            }
            Pointer::Drag {
                button: MouseButton::Left,
                col,
                row,
            } if self.dragging => {
                if let Some(point) = selection_point_at(state, items, area, col, row) {
                    self.head = Some(point);
                    return true;
                }
                false
            }
            Pointer::Up {
                button: MouseButton::Left,
                col,
                row,
            } if self.dragging => {
                self.dragging = false;
                if let Some(point) = selection_point_at(state, items, area, col, row) {
                    self.head = Some(point);
                }
                true
            }
            Pointer::Cancel {
                button: MouseButton::Left,
                ..
            } if self.dragging => {
                self.dragging = false;
                true
            }
            _ => false,
        }
    }

    /// Return the selected bodies, preserving hard newlines but not soft wraps.
    ///
    /// The result is `None` for a collapsed, stale, or invalid selection.
    /// Removing/replacing a selected item therefore clears the selection
    /// instead of copying text from a different current item.
    pub fn selected_text(
        &mut self,
        state: &TranscriptState,
        items: &[TranscriptItem<'_>],
    ) -> Option<String> {
        let (start, end) = self.selection_bounds(state, items)?;
        let mut copied = String::new();
        for index in start.item..=end.item {
            let item = items.get(index)?;
            let Some(text) = selectable_text(state, index, item) else {
                continue;
            };
            let selected = (if index == start.item { start.byte } else { 0 })
                ..(if index == end.item {
                    end.byte
                } else {
                    text.len()
                });
            let part = match state.display(index, item) {
                TranscriptDisplay::Truncated(_) => {
                    copy_truncated_selection(state, index, item, selected)?
                }
                TranscriptDisplay::Collapsed | TranscriptDisplay::Expanded => {
                    text.get(selected)?.to_owned()
                }
            };
            if !part.is_empty() {
                if !copied.is_empty() && !copied.ends_with('\n') {
                    copied.push('\n');
                }
                copied.push_str(&part);
            }
        }
        (!copied.is_empty()).then_some(copied)
    }

    fn selection_bounds(
        &mut self,
        state: &TranscriptState,
        items: &[TranscriptItem<'_>],
    ) -> Option<(TranscriptTextPosition, TranscriptTextPosition)> {
        let (anchor, head) = (self.anchor?, self.head?);
        if !selection_point_valid(state, anchor, items)
            || !selection_point_valid(state, head, items)
        {
            self.clear();
            return None;
        }
        let first = anchor.position;
        let second = head.position;
        (first != second).then_some(if position_before(first, second) {
            (first, second)
        } else {
            (second, first)
        })
    }
}

fn copy_truncated_selection(
    state: &TranscriptState,
    index: usize,
    item: &TranscriptItem<'_>,
    selected: std::ops::Range<usize>,
) -> Option<String> {
    let layout = selectable_layout(state, index, item, state.layout_width)?;
    let visible_rows = state.item_height(index, item, state.layout_width);
    let mut copied = String::new();
    let mut previous_end = None;
    for row in layout
        .rows
        .iter()
        .filter(|row| row.physical_row < visible_rows)
    {
        for segment in &row.segments {
            let start = segment.content.start.max(selected.start);
            let end = segment.content.end.min(selected.end);
            if start >= end {
                continue;
            }
            if let Some(previous_end) = previous_end
                && previous_end < start
                && layout.text.get(previous_end..start)?.contains('\n')
                && !copied.ends_with('\n')
            {
                copied.push('\n');
            }
            copied.push_str(layout.text.get(start..end)?);
            previous_end = Some(end);
        }
    }
    Some(copied)
}

fn position_before(a: TranscriptTextPosition, b: TranscriptTextPosition) -> bool {
    a.item < b.item || (a.item == b.item && a.byte < b.byte)
}

fn selection_point_valid(
    state: &TranscriptState,
    point: SelectionPoint,
    items: &[TranscriptItem<'_>],
) -> bool {
    let Some(item) = items.get(point.position.item) else {
        return false;
    };
    if selection_fingerprint(item) != Some(point.fingerprint) {
        return false;
    }
    selectable_text(state, point.position.item, item).is_some_and(|text| {
        point.position.byte <= text.len()
            && text.is_char_boundary(point.position.byte)
            && is_grapheme_boundary(text, point.position.byte)
            && selection_position_visible(state, point.position.item, item, point.position.byte)
    })
}

fn is_grapheme_boundary(text: &str, byte: usize) -> bool {
    byte == text.len() || text.grapheme_indices(true).any(|(start, _)| start == byte)
}

fn selection_fingerprint(item: &TranscriptItem<'_>) -> Option<u64> {
    let (tag, text) = match *item {
        TranscriptItem::User { text, .. } => (b'u', text),
        TranscriptItem::Assistant { text, .. } => (b'a', text),
        TranscriptItem::Thinking { text, .. } => (b't', text),
        TranscriptItem::System { text } => (b's', text),
        TranscriptItem::Tool(row) if row.detail.is_some_and(|detail| !detail.is_empty()) => {
            (b'o', row.detail.unwrap_or_default())
        }
        TranscriptItem::Marker { .. } | TranscriptItem::Hook { .. } | TranscriptItem::Tool(_) => {
            return None;
        }
    };
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in std::iter::once(tag).chain(text.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^= text.as_ptr() as usize as u64;
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    Some(hash)
}

fn selectable_text<'a>(
    state: &'a TranscriptState,
    index: usize,
    item: &TranscriptItem<'a>,
) -> Option<&'a str> {
    match *item {
        TranscriptItem::User { text, .. } | TranscriptItem::System { text } => Some(text),
        TranscriptItem::Assistant { .. } => state.markdown_text(index, item),
        TranscriptItem::Thinking { .. }
            if state.display(index, item) != TranscriptDisplay::Collapsed =>
        {
            state.markdown_text(index, item)
        }
        TranscriptItem::Tool(row)
            if state.display(index, item) != TranscriptDisplay::Collapsed
                && row.detail.is_some_and(|detail| !detail.is_empty()) =>
        {
            Some(row.detail.unwrap_or_default())
        }
        TranscriptItem::Thinking { .. }
        | TranscriptItem::Marker { .. }
        | TranscriptItem::Hook { .. }
        | TranscriptItem::Tool(_) => None,
    }
}

#[derive(Debug)]
struct SelectableSegment {
    x: u16,
    content: std::ops::Range<usize>,
    visible: String,
}

#[derive(Debug)]
struct SelectableRow {
    physical_row: usize,
    segments: Vec<SelectableSegment>,
}

#[derive(Debug, Clone)]
enum SelectableText {
    Owned(String),
    Tool(Arc<ToolDetailLayout>),
}

impl Default for SelectableText {
    fn default() -> Self {
        Self::Owned(String::new())
    }
}

impl std::ops::Deref for SelectableText {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(text) => text,
            Self::Tool(detail) => &detail.text,
        }
    }
}

#[derive(Debug, Default)]
struct SelectableLayout {
    text: SelectableText,
    rows: Vec<SelectableRow>,
}

impl SelectableLayout {
    fn from_wrapped(text: &str, width: usize, x: u16, physical_start: usize) -> Self {
        let mut layout = Self {
            text: SelectableText::Owned(text.to_owned()),
            rows: Vec::new(),
        };
        let mut search_from = 0usize;
        for (physical_row, visible) in wrap_text(text, width.max(1)).into_iter().enumerate() {
            let offset = text
                .get(search_from..)
                .and_then(|rest| rest.find(&visible))
                .unwrap_or(0);
            let start = search_from.saturating_add(offset);
            let end = start.saturating_add(visible.len());
            layout.rows.push(SelectableRow {
                physical_row: physical_start.saturating_add(physical_row),
                segments: (!visible.is_empty())
                    .then_some(SelectableSegment {
                        x,
                        content: start..end,
                        visible,
                    })
                    .into_iter()
                    .collect(),
            });
            search_from = end;
        }
        layout
    }

    fn from_markdown(layout: &MarkdownLayout, x: u16, physical_start: usize) -> Self {
        Self {
            text: SelectableText::Owned(layout.text.clone()),
            rows: layout
                .rows
                .iter()
                .enumerate()
                .map(|(physical_row, row)| SelectableRow {
                    physical_row: physical_start.saturating_add(physical_row),
                    segments: row
                        .segments
                        .iter()
                        .map(
                            |MarkdownSegment {
                                 x: segment_x,
                                 content,
                                 visible,
                             }| SelectableSegment {
                                x: x.saturating_add(*segment_x),
                                content: content.clone(),
                                visible: visible.clone(),
                            },
                        )
                        .collect(),
                })
                .collect(),
        }
    }
}

fn selectable_layout(
    state: &TranscriptState,
    index: usize,
    item: &TranscriptItem<'_>,
    width: u16,
) -> Option<SelectableLayout> {
    let inner = width.saturating_sub(2);
    match *item {
        TranscriptItem::User { text, time, .. } => {
            let timestamp = time.map_or(0, |value| {
                u16::try_from(value.width())
                    .unwrap_or(u16::MAX)
                    .saturating_add(1)
            });
            Some(SelectableLayout::from_wrapped(
                text,
                inner
                    .saturating_sub(timestamp)
                    .saturating_sub(PROMPT_ARROW_WIDTH)
                    .max(1) as usize,
                1 + PROMPT_ARROW_WIDTH,
                0,
            ))
        }
        TranscriptItem::Assistant { .. } => state
            .markdown_layout(index, item, width)
            .map(|layout| SelectableLayout::from_markdown(layout, 1, 0)),
        TranscriptItem::Thinking { .. }
            if state.display(index, item) != TranscriptDisplay::Collapsed =>
        {
            state
                .markdown_layout(index, item, width)
                .map(|layout| SelectableLayout::from_markdown(layout, 3, 1))
        }
        TranscriptItem::System { text } => Some(SelectableLayout::from_wrapped(
            text,
            inner.max(1) as usize,
            1,
            0,
        )),
        TranscriptItem::Tool(row)
            if state.display(index, item) != TranscriptDisplay::Collapsed
                && row.detail.is_some_and(|detail| !detail.is_empty()) =>
        {
            let detail = state.tool_layouts.get(index)?.as_ref()?;
            Some(SelectableLayout {
                text: SelectableText::Tool(Arc::clone(detail)),
                rows: detail
                    .rows
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(physical_row, row)| SelectableRow {
                        physical_row: physical_row.saturating_add(1),
                        segments: (!row.visible.is_empty())
                            .then_some(SelectableSegment {
                                x: row.content_x,
                                content: row.content,
                                visible: row.visible,
                            })
                            .into_iter()
                            .collect(),
                    })
                    .collect(),
            })
        }
        TranscriptItem::Thinking { .. }
        | TranscriptItem::Marker { .. }
        | TranscriptItem::Hook { .. }
        | TranscriptItem::Tool(_) => None,
    }
}

fn search_layout(
    state: &TranscriptState,
    index: usize,
    item: &TranscriptItem<'_>,
    width: u16,
) -> Option<SelectableLayout> {
    let inner = width.saturating_sub(2);
    match *item {
        TranscriptItem::User { text, time, .. } => {
            let timestamp = time.map_or(0, |value| {
                u16::try_from(value.width())
                    .unwrap_or(u16::MAX)
                    .saturating_add(1)
            });
            Some(SelectableLayout::from_wrapped(
                text,
                inner
                    .saturating_sub(timestamp)
                    .saturating_sub(PROMPT_ARROW_WIDTH)
                    .max(1) as usize,
                1 + PROMPT_ARROW_WIDTH,
                0,
            ))
        }
        TranscriptItem::Assistant { .. } => state
            .markdown_layout(index, item, width)
            .map(|layout| SelectableLayout::from_markdown(layout, 1, 0)),
        TranscriptItem::Thinking { .. } => state
            .markdown_layout(index, item, width)
            .map(|layout| SelectableLayout::from_markdown(layout, 3, 1)),
        TranscriptItem::System { text } => Some(SelectableLayout::from_wrapped(
            text,
            inner.max(1) as usize,
            1,
            0,
        )),
        TranscriptItem::Tool(row) if row.detail.is_some_and(|detail| !detail.is_empty()) => {
            let detail = state.tool_layouts.get(index)?.as_ref()?;
            Some(SelectableLayout {
                text: SelectableText::Tool(Arc::clone(detail)),
                rows: detail
                    .rows
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(physical_row, row)| SelectableRow {
                        physical_row: physical_row.saturating_add(1),
                        segments: (!row.visible.is_empty())
                            .then_some(SelectableSegment {
                                x: row.content_x,
                                content: row.content,
                                visible: row.visible,
                            })
                            .into_iter()
                            .collect(),
                    })
                    .collect(),
            })
        }
        TranscriptItem::Marker { .. } | TranscriptItem::Hook { .. } | TranscriptItem::Tool(_) => {
            None
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct HeaderSearchLayout<'a> {
    text: &'a str,
    x: u16,
    visible_end: usize,
}

fn collect_search_matches(
    state: &TranscriptState,
    query: &SearchQuery,
    index: usize,
    item: &TranscriptItem<'_>,
    width: u16,
    matches: &mut Vec<TranscriptSearchMatch>,
    locations: &mut Vec<SearchLocation>,
) {
    for field in [
        SearchField::ToolLabel,
        SearchField::ToolName,
        SearchField::ToolSummary,
        SearchField::Marker,
        SearchField::HookName,
    ] {
        let Some(header) = header_search_layout(item, width, field) else {
            continue;
        };
        let Some(found) = query.find(header.text) else {
            continue;
        };
        for range in found.ranges {
            matches.push(TranscriptSearchMatch { item: index, range });
            locations.push(SearchLocation {
                field,
                physical_row: 0,
            });
        }
    }
    let Some(layout) = search_layout(state, index, item, width) else {
        return;
    };
    let Some(found) = query.find(&layout.text) else {
        return;
    };
    for range in found.ranges {
        let Some(physical_row) = search_match_row(&layout, &range) else {
            continue;
        };
        matches.push(TranscriptSearchMatch { item: index, range });
        locations.push(SearchLocation {
            field: SearchField::Body,
            physical_row,
        });
    }
}

fn search_match_row(layout: &SelectableLayout, range: &std::ops::Range<usize>) -> Option<usize> {
    if !layout_range_visible(layout, usize::MAX, range) {
        return None;
    }
    layout
        .rows
        .iter()
        .find(|row| {
            row.segments.iter().any(|segment| {
                segment.content.start <= range.start && range.start < segment.content.end
            })
        })
        .map(|row| row.physical_row)
}

fn header_search_layout<'a>(
    item: &TranscriptItem<'a>,
    width: u16,
    field: SearchField,
) -> Option<HeaderSearchLayout<'a>> {
    match (*item, field) {
        (TranscriptItem::Marker { text }, SearchField::Marker) => Some(HeaderSearchLayout {
            text,
            x: 1,
            visible_end: visible_source_end(text, width.saturating_sub(1) as usize),
        }),
        (TranscriptItem::Hook { name, .. }, SearchField::HookName) => Some(HeaderSearchLayout {
            text: name,
            x: 3,
            visible_end: visible_source_end(name, width.saturating_sub(3) as usize),
        }),
        (TranscriptItem::Tool(row), SearchField::ToolLabel) => {
            tool_header_prefix(row.kind).map(|text| HeaderSearchLayout {
                text,
                x: 3,
                visible_end: visible_source_end(text, width.saturating_sub(3) as usize),
            })
        }
        (TranscriptItem::Tool(row), SearchField::ToolName | SearchField::ToolSummary) => {
            tool_header_search_layout(row, width, field)
        }
        _ => None,
    }
}

fn tool_header_search_layout<'a>(
    row: ToolRow<'a>,
    width: u16,
    field: SearchField,
) -> Option<HeaderSearchLayout<'a>> {
    if width < 6 {
        return None;
    }
    let x = 3u16;
    let right = tool_header_right(row, width, x);
    if row.kind == ToolKind::Generic {
        return match field {
            SearchField::ToolName => Some(HeaderSearchLayout {
                text: row.name,
                x,
                visible_end: visible_source_end(row.name, width.saturating_sub(x) as usize),
            }),
            SearchField::ToolSummary => {
                let summary_x = x.saturating_add(row.name.width() as u16).saturating_add(1);
                let budget = right.saturating_sub(summary_x) as usize;
                Some(HeaderSearchLayout {
                    text: row.summary,
                    x: summary_x,
                    visible_end: truncated_source_end(row.summary, budget),
                })
            }
            _ => None,
        };
    }
    if field != SearchField::ToolSummary {
        return None;
    }
    let prefix = tool_header_prefix(row.kind)?;
    let summary_x = x.saturating_add(prefix.width() as u16);
    let suffix_width = row
        .range
        .map(|(start, end)| {
            decimal_digits(start)
                .saturating_add(decimal_digits(end))
                .saturating_add(4)
        })
        .unwrap_or(0);
    let budget = right.saturating_sub(summary_x).saturating_sub(suffix_width) as usize;
    Some(HeaderSearchLayout {
        text: row.summary,
        x: summary_x,
        visible_end: truncated_source_end(row.summary, budget),
    })
}

fn tool_header_right(row: ToolRow<'_>, width: u16, x: u16) -> u16 {
    let mut right = width;
    if let Some(elapsed) = row.elapsed {
        let elapsed = if elapsed.as_secs_f32() < 10.0 {
            format!("{:.1}s", elapsed.as_secs_f32())
        } else if elapsed.as_secs() >= 60 {
            format!("{}m{:02}s", elapsed.as_secs() / 60, elapsed.as_secs() % 60)
        } else {
            format!("{}s", elapsed.as_secs())
        };
        let elapsed_width = elapsed.width() as u16;
        if right.saturating_sub(elapsed_width) > x {
            right = right.saturating_sub(elapsed_width).saturating_sub(1);
        }
    }
    right
}

fn tool_header_prefix(kind: ToolKind) -> Option<&'static str> {
    match kind {
        ToolKind::Generic => None,
        ToolKind::Read => Some("Read "),
        ToolKind::Edit => Some("Edit "),
        ToolKind::Run => Some("$ "),
        ToolKind::Search => Some("Search "),
        ToolKind::List => Some("List "),
        ToolKind::WebSearch => Some("Web search "),
        ToolKind::WebFetch => Some("Fetch "),
        ToolKind::MemorySearch => Some("Memory "),
        ToolKind::UseTool => Some("Tool "),
        ToolKind::Message => Some("Message "),
        ToolKind::Hook => Some("Hook "),
    }
}

fn decimal_digits(number: u32) -> u16 {
    number
        .checked_ilog10()
        .map_or(1, |digits| digits.saturating_add(1) as u16)
}

fn visible_source_end(text: &str, width: usize) -> usize {
    gbtui::core::line_utils::byte_offset_at_width(text, width)
}

fn truncated_source_end(text: &str, width: usize) -> usize {
    if text.width() <= width {
        text.len()
    } else {
        visible_source_end(text, width.saturating_sub(1))
    }
}

fn layout_range_visible(
    layout: &SelectableLayout,
    visible_rows: usize,
    range: &std::ops::Range<usize>,
) -> bool {
    let mut covered = range.start;
    for segment in layout
        .rows
        .iter()
        .filter(|row| row.physical_row < visible_rows)
        .flat_map(|row| row.segments.iter())
    {
        if segment.content.end <= covered {
            continue;
        }
        if segment.content.start > covered {
            return false;
        }
        covered = segment.content.end.min(range.end);
        if covered == range.end {
            return true;
        }
    }
    false
}

fn selection_position_visible(
    state: &TranscriptState,
    index: usize,
    item: &TranscriptItem<'_>,
    byte: usize,
) -> bool {
    if state.layout_width == 0 {
        return true;
    }
    let Some(layout) = selectable_layout(state, index, item, state.layout_width) else {
        return false;
    };
    let visible_rows = state.item_height(index, item, state.layout_width);
    layout
        .rows
        .iter()
        .filter(|row| row.physical_row < visible_rows)
        .flat_map(|row| row.segments.iter())
        .any(|segment| segment.content.start <= byte && byte <= segment.content.end)
}

fn selection_point_at(
    state: &TranscriptState,
    items: &[TranscriptItem<'_>],
    area: Rect,
    col: u16,
    row: u16,
) -> Option<SelectionPoint> {
    if col < area.x || col >= area.right() || row < area.y || row >= area.bottom() {
        return None;
    }
    let content_row = state
        .offset
        .saturating_add(row.saturating_sub(area.y) as usize);
    let mut item_start = 0usize;
    for (index, item) in items.iter().enumerate() {
        let height = state.item_height(index, item, area.width);
        let item_end = item_start.saturating_add(height);
        if content_row >= item_start && content_row < item_end {
            let layout = selectable_layout(state, index, item, area.width)?;
            let local_row = content_row.saturating_sub(item_start);
            let line = layout
                .rows
                .iter()
                .find(|line| line.physical_row == local_row)?;
            let local_col = col.saturating_sub(area.x);
            let mut segment = line.segments.first()?;
            for candidate in &line.segments {
                if local_col < candidate.x {
                    break;
                }
                segment = candidate;
                if local_col <= candidate.x.saturating_add(candidate.visible.width() as u16) {
                    break;
                }
            }
            let byte = segment.content.start.saturating_add(byte_at_column(
                &segment.visible,
                local_col.saturating_sub(segment.x),
            ));
            return Some(SelectionPoint {
                position: TranscriptTextPosition { item: index, byte },
                fingerprint: selection_fingerprint(item)?,
            });
        }
        item_start = item_end.saturating_add((index + 1 < items.len()) as usize * GAP);
    }
    None
}

fn byte_at_column(text: &str, column: u16) -> usize {
    let mut used = 0u16;
    for (start, grapheme) in text.grapheme_indices(true) {
        let width = grapheme.width() as u16;
        if column <= used {
            return start;
        }
        if column < used.saturating_add(width) {
            return start.saturating_add(grapheme.len());
        }
        used = used.saturating_add(width);
    }
    text.len()
}

#[derive(Debug, Clone, Copy)]
struct VisibleItem {
    start: usize,
    height: usize,
    line_offset: usize,
    visible_start: usize,
    viewport: Rect,
    area: Rect,
}

impl VisibleItem {
    fn new(start: usize, height: usize, visible_start: usize, viewport: Rect) -> Option<Self> {
        let end = start.saturating_add(height);
        let visible_end = visible_start.saturating_add(viewport.height as usize);
        let top = start.max(visible_start);
        let bottom = end.min(visible_end);
        if top >= bottom {
            return None;
        }
        Some(Self {
            start,
            height,
            line_offset: top.saturating_sub(start),
            visible_start,
            viewport,
            area: Rect {
                x: viewport.x,
                y: viewport
                    .y
                    .saturating_add(top.saturating_sub(visible_start) as u16),
                width: viewport.width,
                height: (bottom - top) as u16,
            },
        })
    }

    fn y(self, physical_row: usize) -> Option<u16> {
        if physical_row >= self.height {
            return None;
        }
        let content_row = self.start.saturating_add(physical_row);
        (content_row >= self.visible_start
            && content_row
                < self
                    .visible_start
                    .saturating_add(self.viewport.height as usize))
        .then_some(
            self.viewport
                .y
                .saturating_add(content_row.saturating_sub(self.visible_start) as u16),
        )
    }
}

#[derive(Clone, Copy)]
struct ItemSearchMatches<'a> {
    first: usize,
    hits: &'a [TranscriptSearchMatch],
    locations: &'a [SearchLocation],
    selected: Option<usize>,
}

impl<'a> ItemSearchMatches<'a> {
    fn for_item(state: &'a TranscriptState, item: usize) -> Self {
        let range = search_match_range(&state.search.matches, item);
        Self {
            first: range.start,
            hits: &state.search.matches[range.clone()],
            locations: &state.search.locations[range],
            selected: state.search.selected,
        }
    }

    fn has_body_match(self) -> bool {
        self.locations
            .iter()
            .any(|location| location.field == SearchField::Body)
    }

    fn style(self, offset: usize, cx: &FrameCtx<'_>) -> Style {
        if self.selected == Some(self.first + offset) {
            Style::default()
                .fg(cx.theme.text_primary)
                .bg(cx.theme.bg_visual)
        } else {
            Style::default()
                .fg(cx.theme.text_primary)
                .bg(cx.theme.bg_hover)
        }
    }
}

fn paint_selection(
    buf: &mut Buffer,
    cx: &FrameCtx<'_>,
    layout: &SelectableLayout,
    selected: std::ops::Range<usize>,
    visible: VisibleItem,
) {
    paint_text_range(
        buf,
        layout,
        selected,
        visible,
        Style::default()
            .fg(cx.theme.text_primary)
            .bg(cx.theme.bg_visual),
    );
}

fn search_match_range(matches: &[TranscriptSearchMatch], item: usize) -> std::ops::Range<usize> {
    let start = matches.partition_point(|hit| hit.item < item);
    let end = matches.partition_point(|hit| hit.item <= item);
    start..end
}

fn paint_search_matches(
    matches: ItemSearchMatches<'_>,
    buf: &mut Buffer,
    cx: &FrameCtx<'_>,
    layout: &SelectableLayout,
    visible: VisibleItem,
) {
    for (offset, (hit, location)) in matches.hits.iter().zip(matches.locations).enumerate() {
        if location.field != SearchField::Body {
            continue;
        }
        paint_text_range(
            buf,
            layout,
            hit.range.clone(),
            visible,
            matches.style(offset, cx),
        );
    }
}

fn paint_search_headers(
    item: &TranscriptItem<'_>,
    matches: ItemSearchMatches<'_>,
    visible: VisibleItem,
    buf: &mut Buffer,
    cx: &FrameCtx<'_>,
) {
    let Some(y) = visible.y(0) else {
        return;
    };
    for (offset, (hit, location)) in matches.hits.iter().zip(matches.locations).enumerate() {
        if location.field == SearchField::Body {
            continue;
        }
        let Some(header) = header_search_layout(item, visible.viewport.width, location.field)
        else {
            continue;
        };
        if hit.range.end > header.visible_end {
            continue;
        }
        paint_header_range(
            buf,
            header,
            hit.range.clone(),
            y,
            visible.viewport,
            matches.style(offset, cx),
        );
    }
}

fn paint_header_range(
    buf: &mut Buffer,
    header: HeaderSearchLayout<'_>,
    range: std::ops::Range<usize>,
    y: u16,
    area: Rect,
    style: Style,
) {
    let Some(prefix) = header.text.get(..range.start) else {
        return;
    };
    let Some(matched) = header.text.get(range) else {
        return;
    };
    let mut x = area
        .x
        .saturating_add(header.x)
        .saturating_add(prefix.width() as u16);
    for grapheme in matched.graphemes(true) {
        for cell_x in x..x.saturating_add(grapheme.width() as u16).min(area.right()) {
            if let Some(cell) = buf.cell_mut((cell_x, y)) {
                cell.set_style(style);
            }
        }
        x = x.saturating_add(grapheme.width() as u16);
    }
}

fn paint_text_range(
    buf: &mut Buffer,
    layout: &SelectableLayout,
    selected: std::ops::Range<usize>,
    visible: VisibleItem,
    style: Style,
) {
    for line in layout
        .rows
        .iter()
        .filter(|line| line.physical_row < visible.height)
    {
        let Some(y) = visible.y(line.physical_row) else {
            continue;
        };
        for segment in &line.segments {
            if segment.content.end <= selected.start || segment.content.start >= selected.end {
                continue;
            }
            let mut x = visible.viewport.x.saturating_add(segment.x);
            for (start, grapheme) in segment.visible.grapheme_indices(true) {
                let end = start.saturating_add(grapheme.len());
                if segment.content.start.saturating_add(start) < selected.end
                    && segment.content.start.saturating_add(end) > selected.start
                {
                    for cell_x in x..x
                        .saturating_add(grapheme.width() as u16)
                        .min(visible.viewport.right())
                    {
                        if let Some(cell) = buf.cell_mut((cell_x, y)) {
                            cell.set_style(style);
                        }
                    }
                }
                x = x.saturating_add(grapheme.width() as u16);
            }
        }
    }
}

fn item_heights(state: &TranscriptState, items: &[TranscriptItem<'_>], width: u16) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| state.item_height(index, item, width))
        .collect()
}

fn total_height(rows: impl IntoIterator<Item = usize>) -> usize {
    let mut total = 0usize;
    let mut has_item = false;
    for height in rows {
        if has_item {
            total = total.saturating_add(GAP);
        }
        total = total.saturating_add(height);
        has_item = true;
    }
    total
}

fn markdown_spec<'a>(
    item: &TranscriptItem<'a>,
    width: u16,
) -> Option<(MarkdownKind, &'a str, bool, u16)> {
    let inner = width.saturating_sub(2);
    match *item {
        TranscriptItem::Assistant {
            text,
            time,
            streaming,
            ..
        } => {
            let timestamp = time.map_or(0, |value| {
                u16::try_from(value.width())
                    .unwrap_or(u16::MAX)
                    .saturating_add(1)
            });
            Some((
                MarkdownKind::Assistant,
                text,
                streaming,
                inner.saturating_sub(timestamp).max(1),
            ))
        }
        TranscriptItem::Thinking {
            text, streaming, ..
        } => Some((
            MarkdownKind::Thinking,
            text,
            streaming,
            inner.saturating_sub(2).max(1),
        )),
        _ => None,
    }
}

fn measure_content(
    state: &TranscriptState,
    index: usize,
    item: &TranscriptItem<'_>,
    width: u16,
) -> usize {
    let inner = width.saturating_sub(2);
    match *item {
        TranscriptItem::User { text, time, .. } => {
            let ts = time.map_or(0, |t| {
                u16::try_from(t.width())
                    .unwrap_or(u16::MAX)
                    .saturating_add(1)
            });
            user_lines(text, inner.saturating_sub(ts)).len().max(1)
        }
        TranscriptItem::Assistant { .. } => state
            .markdown_layout(index, item, width)
            .map_or(1, MarkdownLayout::height),
        TranscriptItem::Thinking { .. } => 1usize.saturating_add(
            state
                .markdown_layout(index, item, width)
                .map_or(0, MarkdownLayout::height),
        ),
        TranscriptItem::Marker { .. } | TranscriptItem::Hook { .. } => 1,
        TranscriptItem::System { text } => wrap_text(text, inner.max(1) as usize).len().max(1),
        TranscriptItem::Tool(row) => {
            let detail_rows = if state.display(index, item) != TranscriptDisplay::Collapsed
                && row.detail.is_some_and(|detail| !detail.is_empty())
            {
                state
                    .tool_layouts
                    .get(index)
                    .and_then(Option::as_ref)
                    .map_or(0, |detail| detail.rows.len())
            } else {
                0
            };
            1usize.saturating_add(detail_rows)
        }
    }
}

/// Paint physical item rows that intersect `area`. Follow-tail pins the final row.
///
/// For selectable rendering, use [`render_transcript_with_selection`].
pub fn render_transcript(
    state: &mut TranscriptState,
    items: &[TranscriptItem<'_>],
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    render_transcript_impl(state, None, items, area, buf, cx);
}

/// Paint a transcript and its current [`TranscriptSelection`] highlight.
///
/// Callers must forward left-button events through
/// [`TranscriptSelection::handle_pointer`] using the same items and area.
pub fn render_transcript_with_selection(
    state: &mut TranscriptState,
    selection: &mut TranscriptSelection,
    items: &[TranscriptItem<'_>],
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    render_transcript_impl(state, Some(selection), items, area, buf, cx);
}

fn render_transcript_impl(
    state: &mut TranscriptState,
    mut selection: Option<&mut TranscriptSelection>,
    items: &[TranscriptItem<'_>],
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    let empty = items.is_empty();
    if empty {
        state.offset = 0;
        state.markdown = MarkdownCache::default();
        state.displays.clear();
        state.tool_rows.clear();
        state.tool_layouts.clear();
        state.search.matches.clear();
        state.search.locations.clear();
        state.search.selected = None;
        state.search.sources.clear();
        state.search.dirty = true;
        state.navigation = None;
        if let Some(selection) = selection.as_deref_mut() {
            selection.clear();
        }
    }
    if area.height == 0 || area.width == 0 {
        return;
    }
    fill(buf, area, cx.style(ThemeRole::Surface, ThemeState::Normal));
    if empty {
        cx.register_scroll(BODY, area);
        return;
    }

    if let Some(theme) = cx.resolved_theme() {
        state.prepare_resolved(items, area.width, theme);
    } else {
        state.prepare(items, area.width, cx.theme);
    }
    state.sync_search(items, area.width);
    let selection_bounds = selection.and_then(|selection| selection.selection_bounds(state, items));
    let heights = item_heights(state, items, area.width);
    let content_rows = total_height(heights.iter().copied());
    state.clamp(content_rows, area.height);
    let visible_start = state.offset;
    let visible_end = visible_start.saturating_add(area.height as usize);
    let mut item_start = 0usize;

    for (index, (item, item_height)) in items.iter().zip(heights).enumerate() {
        if item_start >= visible_end {
            break;
        }
        let item_end = item_start.saturating_add(item_height);
        if let Some(visible) = VisibleItem::new(item_start, item_height, visible_start, area) {
            cx.register(item_id(index as u32), visible.area);
            paint_item(state, index, item, visible, buf, cx);
            let matches = ItemSearchMatches::for_item(state, index);
            paint_search_headers(item, matches, visible, buf, cx);
            let needs_text_layout = matches.has_body_match()
                || selection_bounds
                    .is_some_and(|(start, end)| index >= start.item && index <= end.item);
            if needs_text_layout
                && let Some(layout) = selectable_layout(state, index, item, area.width)
            {
                paint_search_matches(matches, buf, cx, &layout, visible);
                if let Some((start, end)) = selection_bounds
                    && index >= start.item
                    && index <= end.item
                {
                    let selected_start = if index == start.item { start.byte } else { 0 };
                    let selected_end = if index == end.item {
                        end.byte
                    } else {
                        layout.text.len()
                    };
                    if selected_start < selected_end {
                        paint_selection(buf, cx, &layout, selected_start..selected_end, visible);
                    }
                }
            }
        }
        item_start = item_end;
        if index + 1 < items.len() {
            item_start = item_start.saturating_add(GAP);
        }
    }
    cx.register_scroll(BODY, area);
}

fn paint_item(
    state: &mut TranscriptState,
    index: usize,
    item: &TranscriptItem<'_>,
    visible: VisibleItem,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    let area = visible.area;
    let line_offset = visible.line_offset;
    match *item {
        TranscriptItem::User { text, time, .. } => {
            paint_user(text, time, area, line_offset, buf, cx);
        }
        TranscriptItem::Assistant {
            time, streaming, ..
        } => {
            if let Some(layout) = state.markdown_layout(index, item, area.width) {
                paint_assistant(layout, time, streaming, area, line_offset, buf, cx);
            }
        }
        TranscriptItem::Thinking {
            elapsed, streaming, ..
        } => paint_thinking(
            (state.display(index, item) != TranscriptDisplay::Collapsed)
                .then(|| state.markdown_layout(index, item, area.width))
                .flatten(),
            elapsed,
            streaming,
            area,
            line_offset,
            buf,
            cx,
        ),
        TranscriptItem::System { text } => paint_system(text, area, line_offset, buf, cx),
        TranscriptItem::Marker { text } => paint_marker(text, area, line_offset, buf, cx),
        TranscriptItem::Hook { name, count } => paint_hook(name, count, area, line_offset, buf, cx),
        TranscriptItem::Tool(mut row) => {
            row.expanded = state.display(index, item) != TranscriptDisplay::Collapsed
                && row.detail.is_some_and(|detail| !detail.is_empty());
            if let Some(tool_state) = state.tool_rows.get_mut(index).and_then(Option::as_mut) {
                row.render_clipped(tool_state, area, line_offset, buf, cx);
            }
        }
    }
}

fn content_rect(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y,
        width: area.width.saturating_sub(2),
        height: area.height,
    }
}

fn paint_user(
    text: &str,
    time: Option<&str>,
    area: Rect,
    line_offset: usize,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    let hot = cx.hovered(area);
    let bg = if hot {
        cx.theme.bg_hover
    } else {
        cx.theme.bg_highlight
    };
    fill(buf, area, Style::default().bg(bg).fg(cx.theme.text_primary));
    let inner = content_rect(area);
    let ts_w = time.map(|t| t.width() as u16 + 1).unwrap_or(0);
    let lines = user_lines(text, inner.width.saturating_sub(ts_w));
    let st = Style::default().fg(cx.theme.text_primary).bg(bg);
    let arrow_st = Style::default().fg(cx.theme.text_secondary).bg(bg);
    for (draw_i, (line_i, line)) in lines
        .iter()
        .enumerate()
        .skip(line_offset)
        .take(area.height as usize)
        .enumerate()
    {
        let y = inner.y.saturating_add(draw_i as u16);
        if line_i == 0 {
            buf.set_string_safe(inner.x, y, prompt_arrow(), arrow_st);
        }
        buf.set_string_safe(inner.x.saturating_add(PROMPT_ARROW_WIDTH), y, line, st);
    }
    if line_offset == 0
        && let Some(ts) = time
    {
        let w = ts.width() as u16;
        let x = inner.right().saturating_sub(w);
        buf.set_string_safe(x, inner.y, ts, Style::default().fg(cx.theme.gray).bg(bg));
    }
}

fn user_lines(text: &str, inner_w: u16) -> Vec<String> {
    let budget = inner_w.saturating_sub(PROMPT_ARROW_WIDTH).max(1) as usize;
    wrap_text(text, budget)
}

fn paint_markdown(
    layout: &MarkdownLayout,
    area: Rect,
    line_offset: usize,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> Option<(u16, u16)> {
    let mut tail = None;
    for (draw_index, (row_index, row)) in layout
        .rows
        .iter()
        .enumerate()
        .skip(line_offset)
        .take(area.height as usize)
        .enumerate()
    {
        let y = area.y.saturating_add(draw_index as u16);
        if row.code {
            fill(
                buf,
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                },
                cx.style(ThemeRole::CodeBlock, ThemeState::Normal),
            );
        }
        buf.set_line_safe(area.x, y, &row.line, area.width);
        for link in &row.links {
            let x = area.x.saturating_add(link.columns.start);
            let width = link
                .columns
                .end
                .saturating_sub(link.columns.start)
                .min(area.right().saturating_sub(x));
            if width > 0 {
                cx.register_link(
                    Rect {
                        x,
                        y,
                        width,
                        height: 1,
                    },
                    std::sync::Arc::clone(&link.url),
                    Some(link.id),
                );
            }
        }
        if row_index + 1 == layout.rows.len() {
            tail = Some((y, row.line.width().min(u16::MAX as usize) as u16));
        }
    }
    tail
}

fn paint_assistant(
    layout: &MarkdownLayout,
    time: Option<&str>,
    streaming: bool,
    area: Rect,
    line_offset: usize,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    let hot = cx.hovered(area);
    let bg = if hot {
        cx.theme.bg_hover
    } else {
        cx.theme.bg_base
    };
    fill(buf, area, Style::default().bg(bg).fg(cx.theme.text_primary));
    let inner = content_rect(area);
    let tail = paint_markdown(layout, inner, line_offset, buf, cx);
    if line_offset == 0
        && let Some(ts) = time
    {
        let w = ts.width() as u16;
        let x = inner.right().saturating_sub(w);
        buf.set_string_safe(x, inner.y, ts, Style::default().fg(cx.theme.gray).bg(bg));
    }
    if streaming {
        cx.request_tick(TickDemand::Fast);
        if let Some((y, last)) = tail {
            let col = inner
                .x
                .saturating_add(last)
                .saturating_add(1)
                .min(inner.right().saturating_sub(1));
            Spinner::new().running(true).render(col, y, buf, cx);
        }
    }
}

fn paint_thinking(
    body: Option<&MarkdownLayout>,
    elapsed: Option<Duration>,
    streaming: bool,
    area: Rect,
    line_offset: usize,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    let bg = cx.theme.bg_base;
    fill(buf, area, Style::default().bg(bg).fg(cx.theme.gray));
    let inner = content_rect(area);
    let label_st = Style::default()
        .fg(cx.theme.gray)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let detail_st = Style::default().fg(cx.theme.gray).bg(bg);
    if line_offset == 0 {
        buf.set_string_safe(
            inner.x,
            inner.y,
            diamond_filled(),
            Style::default().fg(cx.theme.gray).bg(bg),
        );
        let col = inner.x.saturating_add(2);
        if streaming {
            buf.set_string_safe(col, inner.y, "Thinking…", label_st);
        } else {
            buf.set_string_safe(col, inner.y, "Thought", label_st);
            let mut x = col.saturating_add("Thought".width() as u16);
            if let Some(d) = elapsed {
                let rest = format!(" for {}", fmt_thought(d));
                buf.set_string_safe(x, inner.y, &rest, detail_st);
                x = x.saturating_add(rest.width() as u16);
            }
            if body.is_none() {
                let hint = "  (ctrl+e to expand)";
                if x + hint.width() as u16 <= inner.right() {
                    buf.set_string_safe(x, inner.y, hint, detail_st);
                }
            }
        }
    }
    if let Some(layout) = body {
        let body_area = Rect {
            x: inner.x.saturating_add(2),
            y: inner.y.saturating_add((line_offset == 0) as u16),
            width: inner.width.saturating_sub(2),
            height: area.height.saturating_sub((line_offset == 0) as u16),
        };
        let _ = paint_markdown(layout, body_area, line_offset.saturating_sub(1), buf, cx);
    }
    if streaming {
        cx.request_tick(TickDemand::Fast);
    }
}

fn fmt_thought(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let mins = (secs / 60.0).floor() as u32;
        let remaining = secs - f64::from(mins) * 60.0;
        format!("{mins}m{remaining:.0}s")
    }
}

fn paint_system(
    text: &str,
    area: Rect,
    line_offset: usize,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    let bg = cx.theme.bg_base;
    fill(buf, area, Style::default().bg(bg).fg(cx.theme.gray));
    let inner = content_rect(area);
    let lines = wrap_text(text, inner.width.max(1) as usize);
    let st = Style::default()
        .fg(cx.theme.gray)
        .bg(bg)
        .add_modifier(Modifier::ITALIC);
    for (draw_i, line) in lines
        .iter()
        .skip(line_offset)
        .take(area.height as usize)
        .enumerate()
    {
        buf.set_string_safe(inner.x, inner.y.saturating_add(draw_i as u16), line, st);
    }
}

fn paint_marker(
    text: &str,
    area: Rect,
    line_offset: usize,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    let bg = cx.theme.bg_base;
    fill(buf, area, Style::default().bg(bg));
    if line_offset == 0 {
        let inner = content_rect(area);
        buf.set_string_safe(
            inner.x,
            inner.y,
            text,
            Style::default().fg(cx.theme.gray).bg(bg),
        );
    }
}

fn paint_hook(
    name: &str,
    count: u32,
    area: Rect,
    line_offset: usize,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    let bg = cx.theme.bg_base;
    fill(buf, area, Style::default().bg(bg));
    if line_offset != 0 {
        return;
    }
    let inner = content_rect(area);
    buf.set_string_safe(
        inner.x,
        inner.y,
        diamond_filled(),
        Style::default().fg(cx.theme.gray).bg(bg),
    );
    let name_st = Style::default()
        .fg(cx.theme.gray)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let mut x = inner.x.saturating_add(2);
    buf.set_string_safe(x, inner.y, name, name_st);
    x = x.saturating_add(name.width() as u16);
    let bracket = Style::default().fg(cx.theme.gray).bg(bg);
    buf.set_string_safe(x, inner.y, "  [hooks: ", bracket);
    x = x.saturating_add("  [hooks: ".width() as u16);
    let count_s = count.to_string();
    buf.set_string_safe(
        x,
        inner.y,
        &count_s,
        Style::default().fg(cx.theme.success).bg(bg),
    );
    x = x.saturating_add(count_s.width() as u16);
    buf.set_string_safe(x, inner.y, "]", bracket);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_row::{ToolRow, ToolStatus};
    use crossterm::event::{KeyCode, KeyModifiers, MouseButton};
    use gbtui::core::{AnimationClock, Theme};
    use gbtui::runtime::{HitMap, HitResult};
    use gbtui::terminal::{HyperlinkCapabilities, Osc8Support};
    use std::time::Instant;

    fn join_row(buf: &Buffer, y: u16, w: u16) -> String {
        (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect()
    }

    fn line_text(line: &ratatui::text::Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn state_at(offset: usize) -> TranscriptState {
        let mut state = TranscriptState::new();
        state.offset = offset;
        state.follow = false;
        state
    }

    fn transcript_rows(
        state: &mut TranscriptState,
        items: &[TranscriptItem<'_>],
        width: u16,
    ) -> usize {
        state.prepare(items, width, &Theme::night());
        state.transcript_height(items, width)
    }

    fn screen(
        state: &mut TranscriptState,
        items: &[TranscriptItem<'_>],

        width: u16,
        height: u16,
    ) -> String {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        render_transcript(
            state,
            items,
            Rect::new(0, 0, width, height),
            &mut buf,
            &mut cx,
        );
        (0..height)
            .map(|y| join_row(&buf, y, width))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn link_snapshot(cx: &FrameCtx<'_>) -> Vec<(u16, u16, u16, String, Option<u32>)> {
        cx.frame_links()
            .iter()
            .map(|link| {
                (
                    link.row,
                    link.col_start,
                    link.col_end,
                    link.url.to_string(),
                    link.id,
                )
            })
            .collect()
    }

    fn native_hyperlink_capabilities() -> HyperlinkCapabilities {
        let mut capabilities = HyperlinkCapabilities::default();
        capabilities.osc8 = Osc8Support::Native;
        capabilities.id_param = true;
        capabilities
    }

    fn selection_edge(
        state: &TranscriptState,
        items: &[TranscriptItem<'_>],
        width: u16,
        item_index: usize,
        line_index: usize,
        at_end: bool,
    ) -> (u16, u16) {
        let item_start = items
            .iter()
            .enumerate()
            .take(item_index)
            .map(|(index, item)| state.item_height(index, item, width).saturating_add(GAP))
            .sum::<usize>();
        let layout = selectable_layout(state, item_index, &items[item_index], width)
            .expect("selectable item");
        let line = &layout.rows[line_index];
        let segment = line.segments.first().expect("selectable segment");
        let x = segment.x.saturating_add(if at_end {
            segment.visible.width() as u16
        } else {
            0
        });
        let y = item_start
            .saturating_add(line.physical_row)
            .saturating_sub(state.offset) as u16;
        (x, y)
    }

    fn drag_select(
        state: &TranscriptState,
        selection: &mut TranscriptSelection,
        items: &[TranscriptItem<'_>],
        width: u16,
        height: u16,
        from: (u16, u16),
        to: (u16, u16),
    ) {
        let area = Rect::new(0, 0, width, height);
        assert!(selection.handle_pointer(
            Pointer::Down {
                button: MouseButton::Left,
                col: from.0,
                row: from.1,
                clicks: 1,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            state,
            items,
            area,
        ));
        assert!(selection.handle_pointer(
            Pointer::Drag {
                button: MouseButton::Left,
                col: to.0,
                row: to.1,
            },
            state,
            items,
            area,
        ));
        assert!(selection.handle_pointer(
            Pointer::Up {
                button: MouseButton::Left,
                col: to.0,
                row: to.1,
            },
            state,
            items,
            area,
        ));
    }

    fn selected_buffer(
        state: &mut TranscriptState,
        selection: &mut TranscriptSelection,
        items: &[TranscriptItem<'_>],
        width: u16,
        height: u16,
    ) -> Buffer {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        render_transcript_with_selection(
            state,
            selection,
            items,
            Rect::new(0, 0, width, height),
            &mut buf,
            &mut cx,
        );
        buf
    }

    #[test]
    fn follow_pins_last_item() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 24, 3));
        let items = [
            TranscriptItem::User {
                author: Some("you"),
                text: "FIRST_UNIQUE",
                time: None,
            },
            TranscriptItem::System { text: "mid" },
            TranscriptItem::Assistant {
                author: Some("bot"),
                text: "LAST_UNIQUE",
                time: None,
                streaming: false,
            },
        ];
        let mut state = TranscriptState::new();
        render_transcript(
            &mut state,
            &items,
            Rect::new(0, 0, 24, 3),
            &mut buf,
            &mut cx,
        );
        let screen: String = (0..3).map(|y| join_row(&buf, y, 24)).collect();
        assert!(screen.contains("LAST"), "{screen}");
        assert!(!screen.contains("FIRST"), "{screen}");
        assert_eq!(anim.demand(), TickDemand::None);
    }

    #[test]
    fn user_uses_prompt_arrow() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 3));
        let items = [TranscriptItem::User {
            author: None,
            text: "hello",
            time: None,
        }];
        let mut state = TranscriptState::new();
        render_transcript(
            &mut state,
            &items,
            Rect::new(0, 0, 40, 3),
            &mut buf,
            &mut cx,
        );
        let screen: String = (0..3).map(|y| join_row(&buf, y, 40)).collect();
        assert!(screen.contains("hello"), "{screen}");
        assert!(
            screen.contains('\u{276F}') || screen.contains('>'),
            "expected prompt arrow in {screen}"
        );
    }

    #[test]
    fn wrap_increases_measure() {
        let item = TranscriptItem::User {
            author: Some("you"),
            text: "abcdefghijklmnopqrstuvwxyz",
            time: None,
        };
        let state = TranscriptState::new();
        assert!(state.item_height(0, &item, 10) > 1);
    }

    #[test]
    fn streaming_requests_fast() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 4));
        let items = [TranscriptItem::Assistant {
            author: Some("bot"),
            text: "hi",
            time: None,
            streaming: true,
        }];
        let mut state = TranscriptState::new();
        render_transcript(
            &mut state,
            &items,
            Rect::new(0, 0, 20, 4),
            &mut buf,
            &mut cx,
        );
        assert_eq!(anim.demand(), TickDemand::Fast);
    }

    #[test]
    fn hover_hit_uses_item_namespace() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, Some((2, 0)), &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 4));
        let items = [TranscriptItem::System { text: "hello" }];
        let mut state = state_at(0);
        render_transcript(
            &mut state,
            &items,
            Rect::new(0, 0, 20, 4),
            &mut buf,
            &mut cx,
        );
        assert_eq!(hits.hit(2, 0), HitResult::Hit(item_id(0)));
    }

    #[test]
    fn tool_row_embeds_and_keeps_idle_when_pending() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 3));
        let tool = ToolRow::new("Read", "search.rs")
            .status(ToolStatus::Pending)
            .index(7);
        let items = [TranscriptItem::Tool(tool)];
        let mut state = TranscriptState::new();
        render_transcript(
            &mut state,
            &items,
            Rect::new(0, 0, 40, 3),
            &mut buf,
            &mut cx,
        );
        assert_eq!(anim.demand(), TickDemand::None);
        match hits.hit(2, 0) {
            HitResult::Hit(id) => {
                assert!(crate::tool_row::parse_tool_id(id).is_some());
            }
            other => panic!("expected tool hit, got {other:?}"),
        }
    }

    #[test]
    fn long_item_end_and_home_render_physical_rows() {
        let items = [TranscriptItem::Assistant {
            author: None,
            text: "row01\nrow02\nrow03\nrow04\nrow05\nrow06\nrow07\nrow08\nrow09\nrow10\nrow11\nrow12\nrow13\nrow14\nrow15\nrow16\nrow17\nrow18\nrow19\nrow20",
            time: None,
            streaming: false,
        }];
        let mut state = TranscriptState::new();
        let rows = transcript_rows(&mut state, &items, 20);
        assert_eq!(rows, 20);
        assert!(state.handle_key(&KeyEvent::new(KeyCode::End, KeyModifiers::NONE), rows, 5,));
        assert_eq!(state.offset, 15);
        let tail = screen(&mut state, &items, 20, 5);
        for row in ["row16", "row17", "row18", "row19", "row20"] {
            assert!(tail.contains(row), "{tail}");
        }
        assert!(!tail.contains("row15"), "{tail}");

        assert!(state.handle_key(&KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), rows, 5,));
        assert!(!state.follow);
        assert_eq!(state.offset, 0);
        let head = screen(&mut state, &items, 20, 5);
        for row in ["row01", "row02", "row03", "row04", "row05"] {
            assert!(head.contains(row), "{head}");
        }
        assert!(!head.contains("row06"), "{head}");
    }

    #[test]
    fn physical_controls_reach_rows_between_head_and_tail() {
        let items = [TranscriptItem::Assistant {
            author: None,
            text: "row01\nrow02\nrow03\nrow04\nrow05\nrow06\nrow07\nrow08\nrow09\nrow10\nrow11\nrow12\nrow13\nrow14\nrow15\nrow16\nrow17\nrow18\nrow19\nrow20",
            time: None,
            streaming: false,
        }];
        let mut state = state_at(0);
        let rows = transcript_rows(&mut state, &items, 20);

        assert!(state.handle_key(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), rows, 5,));
        assert_eq!(state.offset, 1);
        let down = screen(&mut state, &items, 20, 5);
        assert!(down.contains("row02") && down.contains("row06"), "{down}");

        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
            rows,
            5,
        ));
        assert_eq!(state.offset, 6);
        let page_down = screen(&mut state, &items, 20, 5);
        assert!(
            page_down.contains("row07") && page_down.contains("row11"),
            "{page_down}"
        );

        assert!(state.handle_key(&KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), rows, 5,));
        assert_eq!(state.offset, 1);
        state.handle_scroll(7, rows, 5);
        assert_eq!(state.offset, 8);
        let wheel = screen(&mut state, &items, 20, 5);
        assert!(
            wheel.contains("row09") && wheel.contains("row13"),
            "{wheel}"
        );
    }

    #[test]
    fn gaps_and_partially_visible_items_keep_correct_hits() {
        let items = [
            TranscriptItem::System { text: "first" },
            TranscriptItem::System { text: "second" },
        ];
        let mut state = state_at(1);
        let gap = screen(&mut state, &items, 20, 1);
        assert!(gap.trim().is_empty(), "{gap:?}");
        state.offset = 2;
        let second = screen(&mut state, &items, 20, 1);
        assert!(second.contains("second"), "{second}");

        let long = [TranscriptItem::Assistant {
            author: None,
            text: "row01\nrow02\nrow03\nrow04\nrow05",
            time: None,
            streaming: false,
        }];
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 2));
        let mut state = state_at(2);
        render_transcript(&mut state, &long, Rect::new(0, 0, 20, 2), &mut buf, &mut cx);
        assert_eq!(hits.hit(2, 0), HitResult::Hit(item_id(0)));
        assert!(join_row(&buf, 0, 20).contains("row03"));
    }

    #[test]
    fn follow_tail_tracks_stream_growth_and_resize() {
        let short = [TranscriptItem::Assistant {
            author: None,
            text: "stream01\nstream02",
            time: None,
            streaming: true,
        }];
        let mut state = TranscriptState::new();
        let initial = screen(&mut state, &short, 20, 1);
        assert!(initial.contains("stream02"), "{initial}");
        assert_eq!(state.offset, 1);

        let grown = [TranscriptItem::Assistant {
            author: None,
            text: "stream01\nstream02\nstream03\nstream04",
            time: None,
            streaming: true,
        }];
        let tail = screen(&mut state, &grown, 20, 1);
        assert!(tail.contains("stream04"), "{tail}");
        assert!(state.follow);
        assert_eq!(state.offset, 3);

        let resize = [TranscriptItem::System {
            text: "one two three four five six",
        }];
        let mut resized = TranscriptState::new();
        let _ = screen(&mut resized, &resize, 24, 1);
        let narrow = screen(&mut resized, &resize, 8, 1);
        let narrow_rows = resized.transcript_height(&resize, 8);
        assert_eq!(resized.offset, narrow_rows - 1);
        assert!(narrow.contains("six"), "{narrow}");
    }

    #[test]
    fn multiline_user_thinking_and_system_rows_are_accessible() {
        let user = [TranscriptItem::User {
            author: None,
            text: "user01\nuser02",
            time: None,
        }];
        let mut state = state_at(1);
        assert!(screen(&mut state, &user, 20, 1).contains("user02"));

        let thinking = [TranscriptItem::Thinking {
            text: "think01\nthink02",
            expanded: true,
            elapsed: None,
            streaming: false,
        }];
        state.offset = 2;
        assert!(screen(&mut state, &thinking, 20, 1).contains("think02"));

        let system = [TranscriptItem::System {
            text: "system01\nsystem02",
        }];
        state.offset = 1;
        assert!(screen(&mut state, &system, 20, 1).contains("system02"));
    }

    #[test]
    fn expanded_tool_tail_is_accessible() {
        let tool = ToolRow::new("Read", "output")
            .detail(
                "tool01\ntool02\ntool03\ntool04\ntool05\ntool06\ntool07\ntool08\ntool09\ntool10",
            )
            .expanded(true)
            .index(7);
        let items = [TranscriptItem::Tool(tool)];
        let mut state = TranscriptState::new();
        let tail = screen(&mut state, &items, 20, 3);
        for row in ["tool08", "tool09", "tool10"] {
            assert!(tail.contains(row), "{tail}");
        }
        assert!(!tail.contains("tool07"), "{tail}");
    }

    #[test]
    fn thinking_collapsed_is_thought_for() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 2));
        let items = [TranscriptItem::Thinking {
            text: "inspect search.rs",
            expanded: false,
            elapsed: Some(std::time::Duration::from_millis(800)),
            streaming: false,
        }];
        let mut state = TranscriptState::new();
        render_transcript(
            &mut state,
            &items,
            Rect::new(0, 0, 50, 2),
            &mut buf,
            &mut cx,
        );
        let screen: String = (0..2).map(|y| join_row(&buf, y, 50)).collect();
        assert!(screen.contains("Thought"), "{screen}");
        assert!(screen.contains("for 0.8s"), "{screen}");
        assert!(screen.contains("ctrl+e to expand"), "{screen}");
        assert!(
            screen.contains('\u{25C6}') || screen.contains('\u{2666}'),
            "expected diamond bullet in {screen}"
        );
    }

    #[test]
    fn thinking_running_is_ellipsis() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 2));
        let items = [TranscriptItem::Thinking {
            text: "",
            expanded: false,
            elapsed: None,
            streaming: true,
        }];
        let mut state = TranscriptState::new();
        render_transcript(
            &mut state,
            &items,
            Rect::new(0, 0, 40, 2),
            &mut buf,
            &mut cx,
        );
        let screen: String = (0..2).map(|y| join_row(&buf, y, 40)).collect();
        assert!(screen.contains("Thinking"), "{screen}");
    }

    #[test]
    fn cancelling_drag_preserves_range_and_ignores_late_motion() {
        let items = [TranscriptItem::Assistant {
            author: None,
            text: "alpha beta",
            time: None,
            streaming: false,
        }];
        let area = Rect::new(0, 0, 40, 4);
        let mut state = state_at(0);
        let _ = transcript_rows(&mut state, &items, area.width);
        let start = selection_edge(&state, &items, area.width, 0, 0, false);
        let end = selection_edge(&state, &items, area.width, 0, 0, true);
        let mut selection = TranscriptSelection::new();
        for pointer in [
            Pointer::Down {
                button: MouseButton::Left,
                col: start.0,
                row: start.1,
                clicks: 1,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            Pointer::Drag {
                button: MouseButton::Left,
                col: start.0 + 5,
                row: start.1,
            },
            Pointer::Cancel {
                button: MouseButton::Left,
                col: end.0,
                row: end.1,
            },
        ] {
            assert!(selection.handle_pointer(pointer, &state, &items, area));
        }
        assert_eq!(
            selection.selected_text(&state, &items).as_deref(),
            Some("alpha")
        );
        for pointer in [
            Pointer::Drag {
                button: MouseButton::Left,
                col: end.0,
                row: end.1,
            },
            Pointer::Up {
                button: MouseButton::Left,
                col: end.0,
                row: end.1,
            },
        ] {
            assert!(!selection.handle_pointer(pointer, &state, &items, area));
        }
        assert_eq!(
            selection.selected_text(&state, &items).as_deref(),
            Some("alpha")
        );
    }

    #[test]
    fn reverse_drag_selects_text_and_paints_visible_range() {
        let items = [TranscriptItem::Assistant {
            author: None,
            text: "first\nsecond",
            time: None,
            streaming: false,
        }];
        let mut state = state_at(0);
        let _ = transcript_rows(&mut state, &items, 20);
        let mut selection = TranscriptSelection::new();
        drag_select(
            &state,
            &mut selection,
            &items,
            20,
            3,
            selection_edge(&state, &items, 20, 0, 1, true),
            selection_edge(&state, &items, 20, 0, 0, false),
        );
        assert_eq!(
            selection.selected_text(&state, &items).as_deref(),
            Some("first\nsecond")
        );

        let buf = selected_buffer(&mut state, &mut selection, &items, 20, 3);
        assert_eq!(
            buf[(1, 0)].style().bg,
            Some(Theme::night().bg_visual),
            "selected body text must use the visible selection style"
        );
    }

    #[test]
    fn selection_restores_soft_wraps_and_keeps_hard_breaks_across_blocks() {
        let items = [
            TranscriptItem::User {
                author: None,
                text: "甲e\u{301} soft wrapped words\nnext",
                time: None,
            },
            TranscriptItem::Assistant {
                author: None,
                text: "> quoted\n- listed\n1. numbered",
                time: None,
                streaming: false,
            },
        ];
        let mut state = state_at(0);
        let total_rows = transcript_rows(&mut state, &items, 9);
        let mut selection = TranscriptSelection::new();
        let last = selectable_layout(&state, 1, &items[1], 9)
            .unwrap()
            .rows
            .len()
            - 1;
        drag_select(
            &state,
            &mut selection,
            &items,
            9,
            u16::try_from(total_rows).unwrap(),
            selection_edge(&state, &items, 9, 0, 0, false),
            selection_edge(&state, &items, 9, 1, last, true),
        );
        assert_eq!(
            selection.selected_text(&state, &items).as_deref(),
            Some("甲e\u{301} soft wrapped words\nnext\nquoted\nlisted\nnumbered")
        );
    }

    #[test]
    fn selection_survives_resize_and_highlights_partial_long_block() {
        let items = [TranscriptItem::Assistant {
            author: None,
            text: "row01\nrow02\nrow03\nrow04\nrow05\nrow06",
            time: None,
            streaming: false,
        }];
        let mut state = state_at(2);
        let _ = transcript_rows(&mut state, &items, 20);
        let mut selection = TranscriptSelection::new();
        drag_select(
            &state,
            &mut selection,
            &items,
            20,
            2,
            selection_edge(&state, &items, 20, 0, 2, false),
            selection_edge(&state, &items, 20, 0, 3, true),
        );
        let partial = selected_buffer(&mut state, &mut selection, &items, 20, 2);
        assert_eq!(partial[(1, 0)].style().bg, Some(Theme::night().bg_visual));
        assert_eq!(
            selection.selected_text(&state, &items).as_deref(),
            Some("row03\nrow04")
        );

        let _ = selected_buffer(&mut state, &mut selection, &items, 40, 6);
        assert_eq!(
            selection.selected_text(&state, &items).as_deref(),
            Some("row03\nrow04")
        );
    }

    #[test]
    fn tool_detail_selection_keeps_diff_prefixes_and_clears_on_data_reset() {
        let items = [TranscriptItem::Tool(
            ToolRow::new("Edit", "ignored title")
                .detail("+ added\n- removed")
                .expanded(true)
                .index(0),
        )];
        let mut state = state_at(0);
        state.prepare(&items, 30, &Theme::night());
        let mut selection = TranscriptSelection::new();
        drag_select(
            &state,
            &mut selection,
            &items,
            30,
            3,
            selection_edge(&state, &items, 30, 0, 0, false),
            selection_edge(&state, &items, 30, 0, 1, true),
        );
        assert_eq!(
            selection.selected_text(&state, &items).as_deref(),
            Some("+ added\n- removed")
        );

        let _ = selected_buffer(&mut state, &mut selection, &[], 30, 3);
        assert!(!selection.has_selection());
        assert_eq!(selection.selected_text(&state, &[]), None);
    }

    #[test]
    fn streaming_finish_matches_one_shot_markdown() {
        let source = "## Result\n\n**cached** [label](https://example.com)\n\n```rust\nlet answer = 42;\n```\n";
        let split = source.find(" [label]").expect("label boundary");
        let partial = TranscriptItem::Assistant {
            author: None,
            text: &source[..split],
            time: None,
            streaming: true,
        };
        let final_item = TranscriptItem::Assistant {
            author: None,
            text: source,
            time: None,
            streaming: false,
        };

        let mut streamed = TranscriptState::new();
        let _ = transcript_rows(&mut streamed, &[partial], 48);
        let _ = transcript_rows(&mut streamed, &[final_item], 48);
        let streamed_layout = streamed
            .markdown_layout(0, &final_item, 48)
            .expect("streamed layout")
            .clone();

        let mut batch = TranscriptState::new();
        let _ = transcript_rows(&mut batch, &[final_item], 48);
        let batch_layout = batch
            .markdown_layout(0, &final_item, 48)
            .expect("batch layout");
        assert_eq!(streamed_layout.text, batch_layout.text);
        assert_eq!(
            streamed_layout
                .rows
                .iter()
                .map(|row| line_text(&row.line))
                .collect::<Vec<_>>(),
            batch_layout
                .rows
                .iter()
                .map(|row| line_text(&row.line))
                .collect::<Vec<_>>(),
        );
    }
    #[test]
    fn selected_markdown_links_project_wrapped_unicode_metadata_into_visible_frame() {
        let items = [TranscriptItem::Assistant {
            author: None,
            text: "[甲alpha](https://example.com/path)",
            time: None,
            streaming: false,
        }];
        let mut state = state_at(0);
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let area = Rect::new(4, 3, 16, 2);
        state.prepare(&items, area.width, &theme);
        let fingerprint = selection_fingerprint(&items[0]).expect("selectable markdown");
        let selected_end = {
            let layout = state
                .markdown_layout(0, &items[0], area.width)
                .expect("markdown layout");
            assert!(!layout.text.contains('\u{1b}'));
            layout.text.len()
        };
        let mut selection = TranscriptSelection {
            anchor: Some(SelectionPoint {
                position: TranscriptTextPosition { item: 0, byte: 0 },
                fingerprint,
            }),
            head: Some(SelectionPoint {
                position: TranscriptTextPosition {
                    item: 0,
                    byte: selected_end,
                },
                fingerprint,
            }),
            dragging: false,
        };
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits)
            .with_viewport(area)
            .with_hyperlink_capabilities(native_hyperlink_capabilities());
        let mut buf = Buffer::empty(area);

        render_transcript_with_selection(
            &mut state,
            &mut selection,
            &items,
            area,
            &mut buf,
            &mut cx,
        );

        assert_eq!(
            link_snapshot(&cx),
            vec![
                (3, 5, 12, "https://example.com/path".to_owned(), Some(0),),
                (3, 14, 19, "https://example.com/path".to_owned(), Some(1),),
                (4, 5, 19, "https://example.com/path".to_owned(), Some(1),),
            ],
            "only visible physical rows should register renderer metadata"
        );

        cx.occlude(Rect::new(5, 3, 7, 1));
        assert_eq!(
            link_snapshot(&cx),
            vec![
                (3, 14, 19, "https://example.com/path".to_owned(), Some(1),),
                (4, 5, 19, "https://example.com/path".to_owned(), Some(1),),
            ],
            "a later overlay must remove only its covered markdown link"
        );
    }

    #[test]
    fn rich_copy_uses_rendered_content_not_markdown_syntax() {
        let source = "**strong** [link label](https://example.com)\n\n```rust\n- list-looking code\nlet value = 1;\n```\n";
        let item = TranscriptItem::Assistant {
            author: None,
            text: source,
            time: None,
            streaming: false,
        };
        let mut state = TranscriptState::new();
        let _ = transcript_rows(&mut state, &[item], 60);
        let copy = state
            .markdown_layout(0, &item, 60)
            .expect("rich markdown layout")
            .text
            .as_str();
        assert!(copy.contains("strong"));
        assert!(copy.contains("link label"));
        assert!(copy.contains("- list-looking code"));
        assert!(!copy.contains("**"));
        assert!(!copy.contains("https://example.com"));
        assert!(!copy.contains("```"));
    }

    #[test]
    fn markdown_width_change_rewraps_cached_output_and_narrows_tables() {
        let source =
            "| Name | Detail |\n| --- | --- |\n| cache | incremental output remains selectable |\n";
        let item = TranscriptItem::Assistant {
            author: None,
            text: source,
            time: None,
            streaming: false,
        };
        let mut state = TranscriptState::new();
        let _ = transcript_rows(&mut state, &[item], 60);
        let wide_rows = state
            .markdown_layout(0, &item, 60)
            .expect("wide layout")
            .height();
        let wide_copy = state.markdown_layout(0, &item, 60).unwrap().text.clone();

        let _ = transcript_rows(&mut state, &[item], 20);
        let narrow = state.markdown_layout(0, &item, 20).expect("narrow layout");
        assert!(narrow.height() >= wide_rows);
        assert_eq!(
            narrow.text, wide_copy,
            "resize must preserve copied table contents"
        );
        assert!(
            narrow
                .rows
                .iter()
                .all(|row| row.line.width() <= 18 || row.line.width() == 0),
            "table must honor the content width",
        );
    }

    #[test]
    fn table_selection_keeps_canonical_offsets_after_rewrap() {
        let source =
            "| Name | Detail |\n| --- | --- |\n| cache | incremental output remains selectable |\n";
        let item = TranscriptItem::Assistant {
            author: None,
            text: source,
            time: None,
            streaming: false,
        };
        let mut state = TranscriptState::new();
        let _ = transcript_rows(&mut state, &[item], 60);
        let layout = selectable_layout(&state, 0, &item, 60).expect("wide table layout");
        let first = layout
            .rows
            .iter()
            .flat_map(|row| row.segments.iter())
            .next()
            .expect("first table cell");
        let last = layout
            .rows
            .iter()
            .flat_map(|row| row.segments.iter())
            .last()
            .expect("last table cell");
        let fingerprint = selection_fingerprint(&item).expect("selectable item");
        let mut selection = TranscriptSelection {
            anchor: Some(SelectionPoint {
                position: TranscriptTextPosition {
                    item: 0,
                    byte: first.content.start,
                },
                fingerprint,
            }),
            head: Some(SelectionPoint {
                position: TranscriptTextPosition {
                    item: 0,
                    byte: last.content.end,
                },
                fingerprint,
            }),
            dragging: false,
        };
        let copied = selection
            .selected_text(&state, &[item])
            .expect("wide table selection");

        let _ = selected_buffer(&mut state, &mut selection, &[item], 20, 20);
        assert_eq!(
            selection.selected_text(&state, &[item]).as_deref(),
            Some(copied.as_str())
        );
    }

    #[test]
    fn diff_selection_uses_tool_detail_ranges_without_gutters() {
        let detail = "@@ -10,1 +10,1 @@\n-old value\n+new value\n";
        let item = TranscriptItem::Tool(
            ToolRow::new("Edit", "file.rs")
                .kind(crate::tool_row::ToolKind::Edit)
                .detail(detail)
                .expanded(true)
                .index(0),
        );
        let mut state = TranscriptState::new();
        state.prepare(&[item], 40, &Theme::night());
        let layout = selectable_layout(&state, 0, &item, 40).expect("tool detail layout");
        let selectable = layout
            .rows
            .iter()
            .flat_map(|row| row.segments.iter())
            .map(|segment| &layout.text[segment.content.clone()])
            .collect::<String>();
        assert!(selectable.contains("-old value"));
        assert!(selectable.contains("+new value"));
        assert!(!selectable.contains("10 │"));
    }

    #[test]
    fn streaming_code_copy_preserves_indentation_and_markers() {
        let item = TranscriptItem::Assistant {
            author: None,
            text: "```rust\n    - literal code\n    next()",
            time: None,
            streaming: true,
        };
        let mut state = TranscriptState::new();
        state.prepare(&[item], 60, &Theme::night());
        let copy = state.markdown_text(0, &item).unwrap();
        assert!(copy.contains("    - literal code"), "{copy:?}");
        assert!(copy.contains("    next()"), "{copy:?}");
    }
    #[test]
    fn search_tracks_unicode_matches_across_folded_blocks_and_wraps() {
        let items = [
            TranscriptItem::User {
                author: None,
                text: "unrelated",
                time: None,
            },
            TranscriptItem::Thinking {
                text: "甲 needle",
                expanded: true,
                elapsed: None,
                streaming: false,
            },
            TranscriptItem::Assistant {
                author: None,
                text: "response: 甲 needle",
                time: None,
                streaming: false,
            },
        ];
        let mut state = TranscriptState::new();
        state.prepare(&items, 40, &Theme::night());
        assert!(state.set_search_query("甲 needle"));
        assert_eq!(
            state
                .search_matches(&items, 40)
                .iter()
                .map(|hit| hit.item)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            state.next_search_match(&items, 40, 4).map(|hit| hit.item),
            Some(1)
        );
        assert_eq!(
            state.next_search_match(&items, 40, 4).map(|hit| hit.item),
            Some(2)
        );
        assert!(state.set_display(
            &items,
            TranscriptDisplayTarget::Thinking,
            TranscriptDisplay::Collapsed
        ));
        assert_eq!(
            state
                .search_matches(&items, 40)
                .iter()
                .map(|hit| hit.item)
                .collect::<Vec<_>>(),
            vec![1, 2],
            "folding must preserve full-search hit counts"
        );
        assert_eq!(
            state.selected_search_match(&items, 40).map(|hit| hit.item),
            Some(2),
            "folding must retain the selected match"
        );
        assert_eq!(
            state.next_search_match(&items, 40, 4).map(|hit| hit.item),
            Some(1),
            "next search wraps at the tail"
        );
        assert_eq!(
            state.display_mode(1, &items[1]),
            TranscriptDisplay::Expanded,
            "navigating a collapsed thought reveals its matching body"
        );
        assert_eq!(
            state
                .previous_search_match(&items, 40, 4)
                .map(|hit| hit.item),
            Some(2),
            "previous search wraps at the head"
        );
    }

    #[test]
    fn search_reveals_a_truncated_tool_detail_after_header_hits() {
        let items = [TranscriptItem::Tool(
            ToolRow::new("Read", "header needle")
                .detail("first line\nlater needle")
                .expanded(true)
                .index(0),
        )];
        let mut state = TranscriptState::new();
        assert!(state.set_display(
            &items,
            TranscriptDisplayTarget::Item(0),
            TranscriptDisplay::Truncated(2)
        ));
        state.prepare(&items, 40, &Theme::night());
        assert!(state.set_search_query("needle"));
        assert_eq!(state.search_matches(&items, 40).len(), 2);
        assert_eq!(
            state.next_search_match(&items, 40, 2).map(|hit| hit.item),
            Some(0)
        );
        assert_eq!(
            state.display_mode(0, &items[0]),
            TranscriptDisplay::Truncated(2),
            "the visible summary hit does not disturb truncation"
        );
        assert_eq!(
            state.next_search_match(&items, 40, 2).map(|hit| hit.item),
            Some(0)
        );
        assert_eq!(
            state.display_mode(0, &items[0]),
            TranscriptDisplay::Expanded,
            "the later detail hit expands the truncated tool"
        );
    }

    #[test]
    fn search_navigation_scrolls_to_distant_assistant_match_rows() {
        let text = [
            "first needle",
            "row 01",
            "row 02",
            "row 03",
            "row 04",
            "row 05",
            "row 06",
            "row 07",
            "row 08",
            "last needle",
        ]
        .join("\n");
        let items = [TranscriptItem::Assistant {
            author: None,
            text: &text,
            time: None,
            streaming: false,
        }];
        let mut state = TranscriptState::new();
        state.prepare(&items, 40, &Theme::night());
        assert!(state.set_search_query("needle"));
        assert_eq!(
            state.next_search_match(&items, 40, 3).map(|hit| hit.range),
            Some(6..12)
        );
        let first_offset = state.offset;
        assert_eq!(
            state.next_search_match(&items, 40, 3).map(|hit| hit.range),
            Some(74..80)
        );
        assert!(
            state.offset > first_offset,
            "the second distant match must scroll beyond the first"
        );
        let frame = screen(&mut state, &items, 40, 3);
        assert!(frame.contains("last needle"));
        assert!(!frame.contains("first needle"));
    }

    #[test]
    fn transcript_navigation_wraps_users_and_assistant_responses() {
        let items = [
            TranscriptItem::User {
                author: None,
                text: "one",
                time: None,
            },
            TranscriptItem::System { text: "middle" },
            TranscriptItem::User {
                author: None,
                text: "two",
                time: None,
            },
            TranscriptItem::Assistant {
                author: None,
                text: "answer one",
                time: None,
                streaming: false,
            },
            TranscriptItem::Assistant {
                author: None,
                text: "answer two",
                time: None,
                streaming: false,
            },
        ];
        let mut state = TranscriptState::new();
        state.prepare(&items, 40, &Theme::night());
        assert_eq!(
            state.navigate(
                &items,
                40,
                3,
                TranscriptNavigation::UserTurn,
                TranscriptDirection::Forward
            ),
            Some(0)
        );
        assert_eq!(
            state.navigate(
                &items,
                40,
                3,
                TranscriptNavigation::UserTurn,
                TranscriptDirection::Forward
            ),
            Some(2)
        );
        assert_eq!(
            state.navigate(
                &items,
                40,
                3,
                TranscriptNavigation::UserTurn,
                TranscriptDirection::Forward
            ),
            Some(0)
        );
        assert_eq!(
            state.navigate(
                &items,
                40,
                3,
                TranscriptNavigation::AssistantResponse,
                TranscriptDirection::Backward
            ),
            Some(4)
        );
    }

    #[test]
    fn truncating_hides_detail_and_clears_a_selection_that_reaches_hidden_rows() {
        let items = [TranscriptItem::Thinking {
            text: "visible\nhidden",
            expanded: true,
            elapsed: None,
            streaming: false,
        }];
        let mut state = state_at(0);
        let _ = transcript_rows(&mut state, &items, 30);
        let mut selection = TranscriptSelection::new();
        drag_select(
            &state,
            &mut selection,
            &items,
            30,
            3,
            selection_edge(&state, &items, 30, 0, 0, false),
            selection_edge(&state, &items, 30, 0, 1, true),
        );
        assert!(selection.has_selection());
        assert!(state.set_display(
            &items,
            TranscriptDisplayTarget::Item(0),
            TranscriptDisplay::Truncated(2)
        ));
        assert_eq!(state.transcript_height(&items, 30), 2);
        assert!(!state.follow, "manual truncation must not re-enable follow");
        let frame = screen(&mut state, &items, 30, 3);
        assert!(frame.contains("visible"));
        assert!(!frame.contains("hidden"));
        assert_eq!(selection.selected_text(&state, &items), None);
        assert!(!selection.has_selection());
    }
}
