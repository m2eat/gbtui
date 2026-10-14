//! Virtualized selectable lists with physical-row scrolling.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gbtui_core::renderable::RenderContext;
use gbtui_core::{HitId, Renderable, ResolvedTheme, SafeBuf, Theme, ThemeRole, ThemeState, key};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::paint::fill;
use crate::search::{SearchKind, SearchMatch, SearchMode, SearchQuery};

/// Default list-row namespace. Picker / ChoiceList use their own.
pub const NS_ROW: u32 = 0x4C01;
/// Scrollbar track namespace. Index is the list's row namespace.
pub const NS_SCROLL: u32 = 0x4C02;

const PREFIX_WIDTH: u16 = 2;

/// How legacy string adapters map items to terminal rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WrapMode {
    /// Each string occupies one truncated row.
    #[default]
    NoWrap,
    /// Soft-wrap strings, preserving explicit newlines.
    Wrap,
}

/// A rich list row. `id` is stable across filtering, reordering, and mouse hits.
///
/// `revision` is an exact host-owned invalidation token. Change it whenever the
/// renderable's measured height, search text, prefix, or style-dependent layout
/// changes in place.
#[derive(Clone, Copy)]
pub struct ListItem<'a> {
    id: usize,
    content: &'a dyn Renderable,
    prefix: Option<&'a Line<'a>>,
    search_text: &'a str,
    copy_text: Option<&'a str>,
    selectable: bool,
    revision: u64,
}

impl<'a> ListItem<'a> {
    pub fn new(id: usize, content: &'a dyn Renderable, search_text: &'a str) -> Self {
        Self {
            id,
            content,
            prefix: None,
            search_text,
            copy_text: None,
            selectable: true,
            revision: 0,
        }
    }

    pub fn prefix(mut self, prefix: &'a Line<'a>) -> Self {
        self.prefix = Some(prefix);
        self
    }

    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    pub fn revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }

    /// Use this host-owned text when extracting a visual selection.
    pub fn with_copy_text(mut self, copy_text: &'a str) -> Self {
        self.copy_text = Some(copy_text);
        self
    }

    pub fn id(self) -> usize {
        self.id
    }
    pub fn search_text(self) -> &'a str {
        self.search_text
    }
    /// Host-owned text for extraction, defaulting to [`Self::search_text`].
    pub fn copy_text(self) -> &'a str {
        self.copy_text.unwrap_or(self.search_text)
    }
    pub fn is_selectable(self) -> bool {
        self.selectable
    }
    pub fn content_revision(self) -> u64 {
        self.revision
    }
}

/// Cached item heights and physical prefix sums.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListLayoutCache {
    FixedHeight {
        count: usize,
    },
    Variable {
        width: u16,
        heights: Vec<usize>,
        prefix_sums: Vec<usize>,
    },
}

impl ListLayoutCache {
    pub fn fixed(count: usize) -> Self {
        Self::FixedHeight { count }
    }

    pub fn from_heights(width: u16, heights: impl IntoIterator<Item = usize>) -> Self {
        let heights: Vec<_> = heights.into_iter().map(|height| height.max(1)).collect();
        let mut prefix_sums: Vec<usize> = Vec::with_capacity(heights.len().saturating_add(1));
        prefix_sums.push(0);
        for height in &heights {
            prefix_sums.push(
                prefix_sums
                    .last()
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(*height),
            );
        }
        Self::Variable {
            width,
            heights,
            prefix_sums,
        }
    }

    pub fn total_height(&self) -> usize {
        match self {
            Self::FixedHeight { count } => *count,
            Self::Variable { prefix_sums, .. } => prefix_sums.last().copied().unwrap_or(0),
        }
    }

    pub fn item_count(&self) -> usize {
        match self {
            Self::FixedHeight { count } => *count,
            Self::Variable { heights, .. } => heights.len(),
        }
    }

    pub fn virtual_y(&self, index: usize) -> usize {
        match self {
            Self::FixedHeight { .. } => index,
            Self::Variable { prefix_sums, .. } => prefix_sums.get(index).copied().unwrap_or(0),
        }
    }

    pub fn item_height(&self, index: usize) -> usize {
        match self {
            Self::FixedHeight { .. } => 1,
            Self::Variable { heights, .. } => heights.get(index).copied().unwrap_or(1),
        }
    }

    pub fn item_at_y(&self, y: usize) -> Option<usize> {
        match self {
            Self::FixedHeight { count } => count.checked_sub(1).map(|last| y.min(last)),
            Self::Variable { prefix_sums, .. } => {
                let last = prefix_sums.len().checked_sub(2)?;
                Some(
                    prefix_sums
                        .partition_point(|prefix| *prefix <= y)
                        .saturating_sub(1)
                        .min(last),
                )
            }
        }
    }

    pub fn cached_width(&self) -> Option<u16> {
        match self {
            Self::FixedHeight { .. } => None,
            Self::Variable { width, .. } => Some(*width),
        }
    }
}

#[derive(Debug, Clone)]
struct CachedLayout {
    mode: WrapMode,
    width: u16,
    revision: u64,
    query: String,
    query_kind: SearchKind,
    query_mode: SearchMode,
    source_ids: Vec<usize>,
    source_revisions: Vec<u64>,
    visible: Vec<usize>,
    visible_ids: Vec<usize>,
    selectable: Vec<bool>,
    visual_selection: Option<(usize, usize)>,
    matches: Vec<Option<SearchMatch>>,
    layout: ListLayoutCache,
    viewport_width: Option<u16>,
    has_scrollbar: bool,
}

/// Selection and physical viewport state shared by plain and rich lists.
#[derive(Debug, Clone)]
pub struct ListPaneState {
    /// Visible index, retained for existing fixed-row consumers.
    pub selected: usize,
    /// Visible index for fixed lists; physical terminal-row offset for rich/wrapped lists.
    pub offset: usize,
    pub follow: bool,
    /// Legacy substring filter used by [`render_list`]. Rich lists take [`SearchQuery`].
    pub filter: String,
    wrap_mode: WrapMode,
    layout_revision: u64,
    selected_id: Option<usize>,
    visual_selection_enabled: bool,
    visual_anchor_id: Option<usize>,
    visual_end_id: Option<usize>,
    filter_query: Option<SearchQuery>,
    cache: Option<CachedLayout>,
}

impl Default for ListPaneState {
    fn default() -> Self {
        Self::with_wrap_mode(WrapMode::NoWrap)
    }
}

impl ListPaneState {
    pub fn with_wrap_mode(wrap_mode: WrapMode) -> Self {
        Self {
            selected: 0,
            offset: 0,
            follow: false,
            filter: String::new(),
            wrap_mode,
            layout_revision: 0,
            selected_id: None,
            visual_selection_enabled: false,
            visual_anchor_id: None,
            visual_end_id: None,
            filter_query: Some(SearchQuery::new()),
            cache: None,
        }
    }

    pub fn wrap_mode(&self) -> WrapMode {
        self.wrap_mode
    }

    pub fn set_wrap_mode(&mut self, wrap_mode: WrapMode) {
        if self.wrap_mode != wrap_mode {
            self.wrap_mode = wrap_mode;
            self.invalidate_layout();
        }
    }

    /// Change this whenever legacy equal-length labels mutate or reorder.
    pub fn set_layout_revision(&mut self, revision: u64) {
        if self.layout_revision != revision {
            self.layout_revision = revision;
            self.invalidate_layout();
        }
    }

    pub(crate) fn set_selection_identity(&mut self, id: Option<usize>) {
        self.selected_id = id;
    }

    pub fn invalidate_layout(&mut self) {
        self.cache = None;
    }
    pub fn layout(&self) -> Option<&ListLayoutCache> {
        self.cache.as_ref().map(|cache| &cache.layout)
    }
    pub fn visible_count(&self) -> usize {
        self.layout().map_or(0, ListLayoutCache::item_count)
    }
    pub fn total_height(&self) -> usize {
        self.layout().map_or(0, ListLayoutCache::total_height)
    }
    pub fn selected_id(&self) -> Option<usize> {
        self.selected_id
    }

    /// Enable range selection keys and visual range painting.
    ///
    /// It is disabled by default so lists embedded in editable controls retain
    /// their existing input behavior. Disabling it clears any active range.
    pub fn set_visual_selection_enabled(&mut self, enabled: bool) {
        if self.visual_selection_enabled == enabled {
            return;
        }
        self.visual_selection_enabled = enabled;
        self.clear_visual_selection();
    }

    /// Whether an anchored visual range is currently active.
    pub fn visual_mode(&self) -> bool {
        self.visual_anchor_id.is_some() && self.visual_end_id.is_some()
    }

    /// Stable `(anchor, end)` IDs for the active range.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.visual_anchor_id.zip(self.visual_end_id)
    }

    /// Clear the active visual range without changing the cursor selection.
    pub fn clear_visual_selection(&mut self) {
        self.visual_anchor_id = None;
        self.visual_end_id = None;
        if let Some(cache) = self.cache.as_mut() {
            cache.visual_selection = None;
        }
    }

    /// Iterate selected stable IDs in their current visible order.
    ///
    /// This does not allocate. Nonselectable rows between the two endpoints
    /// are omitted.
    pub fn selection_ids(&self) -> impl Iterator<Item = usize> + '_ {
        let (_, ids, selectable) =
            self.selection_slice()
                .unwrap_or((&[] as &[usize], &[] as &[usize], &[] as &[bool]));
        ids.iter()
            .zip(selectable)
            .filter_map(|(id, selectable)| selectable.then_some(*id))
    }

    /// Join selected rich rows' host-owned copy text with newlines.
    ///
    /// The caller retains clipboard ownership. Returns `None` if no visual
    /// range is active or an endpoint is absent from the supplied source.
    pub fn selected_text(&self, items: &[ListItem<'_>]) -> Option<String> {
        let (sources, ids, selectable) = self.selection_slice()?;
        let mut has_selection = false;
        let capacity = sources.iter().zip(ids).zip(selectable).try_fold(
            0usize,
            |total, ((source, id), selectable)| {
                if !*selectable {
                    return Some(total);
                }
                let item = items.get(*source).filter(|item| item.id == *id)?;
                let separator = usize::from(has_selection);
                has_selection = true;
                Some(
                    total
                        .saturating_add(separator)
                        .saturating_add(item.copy_text().len()),
                )
            },
        )?;
        if !has_selection {
            return None;
        }
        let mut text = String::with_capacity(capacity);
        let mut copied = false;
        for ((source, id), selectable) in sources.iter().zip(ids).zip(selectable) {
            if !*selectable {
                continue;
            }
            let item = items.get(*source).filter(|item| item.id == *id)?;
            if copied {
                text.push('\n');
            }
            text.push_str(item.copy_text());
            copied = true;
        }
        Some(text)
    }

    /// Legacy source position in the prepared layout.
    pub fn source_index(&self, visible_index: usize) -> Option<usize> {
        self.cache
            .as_ref()
            .and_then(|cache| cache.visible.get(visible_index).copied())
    }

    /// Stable rich-item identity in the prepared layout.
    pub fn item_id(&self, visible_index: usize) -> Option<usize> {
        self.cache
            .as_ref()
            .and_then(|cache| cache.visible_ids.get(visible_index).copied())
    }

    pub fn select_source(&mut self, source_index: usize, page: usize) -> bool {
        let Some(visible) = self.cache.as_ref().and_then(|cache| {
            cache
                .visible
                .iter()
                .position(|source| *source == source_index)
        }) else {
            return false;
        };
        self.select_visible(visible, self.visible_count(), page);
        true
    }

    pub fn select_item(&mut self, id: usize, page: usize) -> bool {
        let Some(visible) = self.cache.as_ref().and_then(|cache| {
            cache
                .visible_ids
                .iter()
                .zip(&cache.selectable)
                .position(|(candidate, selectable)| *candidate == id && *selectable)
        }) else {
            return false;
        };
        self.select_visible(visible, self.visible_count(), page);
        true
    }

    /// Select a stable item ID, optionally extending the active range.
    ///
    /// Pass the pointer modifier state as `extend_selection` for mouse input.
    /// A non-extended selection clears any active visual range.
    pub fn select_item_with_modifier(
        &mut self,
        id: usize,
        page: usize,
        extend_selection: bool,
    ) -> bool {
        let Some(visible) = self.cache.as_ref().and_then(|cache| {
            cache
                .visible_ids
                .iter()
                .zip(&cache.selectable)
                .position(|(candidate, selectable)| *candidate == id && *selectable)
        }) else {
            return false;
        };
        self.select_visible_with_modifier(visible, self.visible_count(), page, extend_selection);
        true
    }

    pub fn visible_indices<'a>(&self, labels: impl Iterator<Item = &'a str>) -> Vec<usize> {
        let filter = self.filter.as_bytes();
        labels
            .enumerate()
            .filter(|(_, label)| ascii_contains_case_insensitive(label, filter))
            .map(|(index, _)| index)
            .collect()
    }

    pub fn desired_height(&self, labels: &[&str]) -> u16 {
        self.visible_indices(labels.iter().copied())
            .len()
            .min(u16::MAX as usize) as u16
    }

    pub fn desired_height_at(&mut self, labels: &[&str], content_width: u16) -> u16 {
        self.prepare_layout(labels, content_width)
            .total_height()
            .min(u16::MAX as usize) as u16
    }

    pub fn desired_item_height_at(
        &mut self,
        items: &[ListItem<'_>],
        query: &SearchQuery,
        content_width: u16,
    ) -> u16 {
        let source = RichSource { items };
        self.prepare_source(&source, query, content_width.max(1));
        self.total_height().min(u16::MAX as usize) as u16
    }

    /// Prepare the legacy string adapter. Rich callers use [`render_list_items`].
    pub fn prepare_layout(&mut self, labels: &[&str], content_width: u16) -> &ListLayoutCache {
        {
            let filter_query = self
                .filter_query
                .as_mut()
                .expect("filter query is restored after preparation");
            if filter_query.query() != self.filter {
                filter_query.set_query(self.filter.clone());
            }
        }
        let query = self.filter_query.take().expect("filter query is present");
        let source = PlainSource {
            labels,
            wrap_mode: self.wrap_mode,
        };
        self.prepare_source(&source, &query, content_width.max(1));
        self.filter_query = Some(query);
        self.layout().expect("layout was prepared")
    }

    fn prepare_source<S: ListItemSource + ?Sized>(
        &mut self,
        source: &S,
        query: &SearchQuery,
        width: u16,
    ) {
        let stale = match self.cache.as_ref() {
            None => true,
            Some(cache) => {
                cache.mode != self.wrap_mode
                    || cache.width != width
                    || cache.revision != self.layout_revision
                    || cache.query != query.query()
                    || cache.query_kind != query.kind()
                    || cache.query_mode != query.mode()
                    || cache.source_ids.len() != source.len()
                    || (0..source.len()).any(|index| {
                        cache.source_ids[index] != source.id(index)
                            || cache.source_revisions[index] != source.revision(index)
                    })
            }
        };
        if !stale {
            return;
        }

        let old_selected = self.selected_id.or_else(|| {
            self.cache
                .as_ref()
                .and_then(|cache| cache.visible_ids.get(self.selected).copied())
        });
        let mut source_ids = Vec::with_capacity(source.len());
        let mut source_revisions = Vec::with_capacity(source.len());
        let mut rows = Vec::with_capacity(source.len());
        for index in 0..source.len() {
            source_ids.push(source.id(index));
            source_revisions.push(source.revision(index));
            let matched = source.find(index, query);
            if query.mode() == SearchMode::Filter && matched.is_none() {
                continue;
            }
            rows.push(PreparedRow {
                source: index,
                id: source.id(index),
                selectable: source.selectable(index),
                matched,
            });
        }
        if query.mode() == SearchMode::Filter
            && query.kind() == SearchKind::Fuzzy
            && !query.query().is_empty()
            && source.sort_fuzzy_matches()
        {
            rows.sort_by(|left, right| {
                right
                    .matched
                    .as_ref()
                    .map_or(i64::MIN, |m| m.score)
                    .cmp(&left.matched.as_ref().map_or(i64::MIN, |m| m.score))
            });
        }
        let visible: Vec<_> = rows.iter().map(|row| row.source).collect();
        let visible_ids: Vec<_> = rows.iter().map(|row| row.id).collect();
        let selectable: Vec<_> = rows.iter().map(|row| row.selectable).collect();
        let matches: Vec<_> = rows.into_iter().map(|row| row.matched).collect();
        let layout = if source.physical_rows() {
            ListLayoutCache::from_heights(
                width,
                visible.iter().map(|index| {
                    source.desired_height(
                        *index,
                        width
                            .saturating_sub(source.prefix(*index).map_or(0, line_width))
                            .max(1),
                    )
                }),
            )
        } else {
            ListLayoutCache::fixed(visible.len())
        };
        self.cache = Some(CachedLayout {
            mode: self.wrap_mode,
            width,
            revision: self.layout_revision,
            query: query.query().to_owned(),
            query_kind: query.kind(),
            query_mode: query.mode(),
            source_ids,
            source_revisions,
            visible,
            visible_ids,
            selectable,
            visual_selection: None,
            matches,
            layout,
            viewport_width: None,
            has_scrollbar: false,
        });
        self.reconcile_selection(old_selected);
        self.reconcile_visual_selection();
    }

    fn reconcile_selection(&mut self, previous: Option<usize>) {
        let Some(cache) = self.cache.as_ref() else {
            return;
        };
        let Some(first) = cache.selectable.iter().position(|selectable| *selectable) else {
            self.selected = 0;
            self.selected_id = None;
            self.offset = 0;
            return;
        };
        let selected = previous
            .and_then(|id| {
                cache
                    .visible_ids
                    .iter()
                    .zip(&cache.selectable)
                    .position(|(candidate, selectable)| *candidate == id && *selectable)
            })
            .unwrap_or_else(|| first.max(self.selected.min(cache.visible.len().saturating_sub(1))));
        self.selected = if cache.selectable.get(selected).copied().unwrap_or(false) {
            selected
        } else {
            first
        };
        self.selected_id = cache.visible_ids.get(self.selected).copied();
    }

    fn reconcile_visual_selection(&mut self) {
        let selection = self.selection_range().and_then(|(anchor, end)| {
            let cache = self.cache.as_ref()?;
            let anchor = cache
                .visible_ids
                .iter()
                .zip(&cache.selectable)
                .position(|(id, selectable)| *id == anchor && *selectable)?;
            let end = cache
                .visible_ids
                .iter()
                .zip(&cache.selectable)
                .position(|(id, selectable)| *id == end && *selectable)?;
            Some((anchor.min(end), anchor.max(end)))
        });
        if selection.is_none() {
            self.visual_anchor_id = None;
            self.visual_end_id = None;
        }
        if let Some(cache) = self.cache.as_mut() {
            cache.visual_selection = selection;
        }
    }

    fn selection_slice(&self) -> Option<(&[usize], &[usize], &[bool])> {
        let cache = self.cache.as_ref()?;
        let (start, end) = cache.visual_selection?;
        Some((
            &cache.visible[start..=end],
            &cache.visible_ids[start..=end],
            &cache.selectable[start..=end],
        ))
    }

    pub fn clamp(&mut self, vis_len: usize, page: usize) {
        if vis_len == 0 {
            self.selected = 0;
            self.offset = 0;
            self.selected_id = None;
            return;
        }
        if self.follow {
            self.selected = vis_len - 1;
        }
        self.selected = self.selected.min(vis_len - 1);
        let page = page.max(1);
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + page {
            self.offset = self.selected + 1 - page;
        }
        self.offset = self.offset.min(vis_len.saturating_sub(page));
    }

    pub fn handle_key(&mut self, key: &KeyEvent, vis_len: usize, page: usize) -> bool {
        if self.visual_selection_enabled
            && key.modifiers.is_empty()
            && matches!(key.code, KeyCode::Char('v'))
        {
            if self.visual_mode() {
                self.clear_visual_selection();
                return true;
            }
            return self.start_visual_selection();
        }
        if self.visual_selection_enabled && matches!(key.code, KeyCode::Esc) && self.visual_mode() {
            self.clear_visual_selection();
            return true;
        }

        let shifted_vertical = self.visual_selection_enabled
            && key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
            && key.modifiers.contains(KeyModifiers::SHIFT)
            && matches!(key.code, KeyCode::Up | KeyCode::Down);
        let extends_range = self.visual_mode() || shifted_vertical;
        let before = self.cursor_id();
        let mut navigation = *key;
        if extends_range && key.modifiers.contains(KeyModifiers::SHIFT) {
            navigation.modifiers = key.modifiers.difference(KeyModifiers::SHIFT);
        }
        let handled = if extends_range {
            self.handle_range_vertical(&navigation, page)
                .unwrap_or_else(|| self.handle_navigation(&navigation, vis_len, page))
        } else {
            self.handle_navigation(&navigation, vis_len, page)
        };
        if handled && extends_range {
            self.extend_visual_selection(before);
        }
        handled
    }

    fn handle_navigation(&mut self, key: &KeyEvent, vis_len: usize, page: usize) -> bool {
        if self.cache.is_none() {
            return self.handle_uncached_key(key, vis_len, page);
        }
        if self.is_physical() {
            self.handle_physical_key(key, page)
        } else {
            self.handle_cached_fixed_key(key, page)
        }
    }

    /// Move one selectable row without the ordinary list navigation wrap.
    ///
    /// Wrapping a range endpoint at a visible boundary would turn a one-step
    /// extension into a selection of unrelated rows.
    fn handle_range_vertical(&mut self, key: &KeyEvent, page: usize) -> Option<bool> {
        let down = match key.code {
            KeyCode::Down => true,
            KeyCode::Up => false,
            _ => return None,
        };
        let target = {
            let cache = self.cache.as_ref()?;
            if down {
                (self.selected.saturating_add(1)..cache.selectable.len())
                    .find(|index| cache.selectable[*index])
            } else {
                (0..self.selected.min(cache.selectable.len()))
                    .rev()
                    .find(|index| cache.selectable[*index])
            }
        };
        let Some(target) = target else {
            return Some(true);
        };
        self.follow = false;
        self.set_selected_visible(target);
        if self.is_physical() {
            self.clamp_physical(page);
        } else {
            self.clamp_cached_fixed(page);
        }
        Some(true)
    }

    fn cursor_id(&self) -> Option<usize> {
        self.selected_id.or_else(|| self.item_id(self.selected))
    }

    fn start_visual_selection(&mut self) -> bool {
        let Some(id) = self.cursor_id() else {
            return false;
        };
        let valid = self.cache.as_ref().is_some_and(|cache| {
            cache
                .visible_ids
                .iter()
                .zip(&cache.selectable)
                .any(|(candidate, selectable)| *candidate == id && *selectable)
        });
        if !valid {
            return false;
        }
        self.visual_anchor_id = Some(id);
        self.visual_end_id = Some(id);
        self.reconcile_visual_selection();
        true
    }

    fn extend_visual_selection(&mut self, initial_anchor: Option<usize>) {
        let Some(anchor) = self.visual_anchor_id.or(initial_anchor) else {
            self.clear_visual_selection();
            return;
        };
        let Some(end) = self.cursor_id() else {
            self.clear_visual_selection();
            return;
        };
        self.visual_anchor_id = Some(anchor);
        self.visual_end_id = Some(end);
        self.reconcile_visual_selection();
    }

    fn handle_uncached_key(&mut self, key: &KeyEvent, vis_len: usize, page: usize) -> bool {
        if vis_len == 0 {
            return false;
        }
        let target = if key!(Down).matches(key) || key!('j').matches(key) {
            Some((self.selected + 1).min(vis_len - 1))
        } else if key!(Up).matches(key) || key!('k').matches(key) {
            Some(self.selected.saturating_sub(1))
        } else if key!(Home).matches(key) || key!('g').matches(key) {
            Some(0)
        } else if key!(End).matches(key) || key!('G').matches(key) {
            self.follow = true;
            Some(vis_len - 1)
        } else if key!(PageDown).matches(key) {
            Some((self.selected + page.max(1)).min(vis_len - 1))
        } else if key!(PageUp).matches(key) {
            Some(self.selected.saturating_sub(page.max(1)))
        } else {
            None
        };
        let Some(target) = target else {
            return false;
        };
        if !key!(End).matches(key) && !key!('G').matches(key) {
            self.follow = false;
        }
        self.selected = target;
        self.clamp(vis_len, page);
        true
    }

    fn handle_cached_fixed_key(&mut self, key: &KeyEvent, page: usize) -> bool {
        let count = self.visible_count();
        if count == 0 {
            return false;
        }
        let down = key!(Down).matches(key) || key!('j').matches(key) || key!(PageDown).matches(key);
        let target = if key!(Home).matches(key) || key!('g').matches(key) {
            self.first_selectable(true)
        } else if key!(End).matches(key) || key!('G').matches(key) {
            self.follow = true;
            self.first_selectable(false)
        } else if down {
            self.next_selectable(
                self.selected
                    .saturating_add(if key!(PageDown).matches(key) {
                        page.max(1)
                    } else {
                        1
                    }),
                true,
            )
        } else if key!(Up).matches(key) || key!('k').matches(key) || key!(PageUp).matches(key) {
            self.next_selectable(
                self.selected.saturating_sub(if key!(PageUp).matches(key) {
                    page.max(1)
                } else {
                    1
                }),
                false,
            )
        } else {
            None
        };
        let Some(target) = target else {
            return false;
        };
        if !(key!(End).matches(key) || key!('G').matches(key)) {
            self.follow = false;
        }
        self.set_selected_visible(target);
        self.clamp_cached_fixed(page);
        true
    }

    fn handle_physical_key(&mut self, key: &KeyEvent, page: usize) -> bool {
        if self.visible_count() == 0 {
            return false;
        }
        if key!(PageDown).matches(key) {
            self.follow = false;
            self.offset = self.offset.saturating_add(page.max(1));
            self.select_at_offset();
            self.clamp_physical(page);
            return true;
        }
        if key!(PageUp).matches(key) {
            self.follow = false;
            self.offset = self.offset.saturating_sub(page.max(1));
            self.select_at_offset();
            self.clamp_physical(page);
            return true;
        }
        let target = if key!(Down).matches(key) || key!('j').matches(key) {
            self.next_selectable(self.selected.saturating_add(1), true)
        } else if key!(Up).matches(key) || key!('k').matches(key) {
            self.next_selectable(self.selected.saturating_sub(1), false)
        } else if key!(Home).matches(key) || key!('g').matches(key) {
            self.first_selectable(true)
        } else if key!(End).matches(key) || key!('G').matches(key) {
            self.follow = true;
            self.first_selectable(false)
        } else {
            None
        };
        let Some(target) = target else {
            return false;
        };
        if !(key!(End).matches(key) || key!('G').matches(key)) {
            self.follow = false;
        }
        self.set_selected_visible(target);
        self.clamp_physical(page);
        true
    }

    pub fn handle_scroll(&mut self, delta_lines: i32, vis_len: usize, page: usize) {
        if delta_lines == 0 {
            return;
        }
        if self.cache.is_some() && self.is_physical() {
            self.follow = false;
            if delta_lines.is_positive() {
                self.offset = self.offset.saturating_add(delta_lines as usize);
            } else {
                self.offset = self
                    .offset
                    .saturating_sub(delta_lines.unsigned_abs() as usize);
            }
            self.clamp_physical_offset(page);
            return;
        }
        if vis_len == 0 {
            return;
        }
        self.follow = false;
        self.selected = if delta_lines.is_positive() {
            (self.selected + delta_lines as usize).min(vis_len - 1)
        } else {
            self.selected
                .saturating_sub(delta_lines.unsigned_abs() as usize)
        };
        self.clamp(vis_len, page);
        if self.cache.is_some() {
            self.selected_id = self.item_id(self.selected);
        }
    }

    pub fn select_visible(&mut self, vis_index: usize, vis_len: usize, page: usize) {
        self.clear_visual_selection();
        self.select_visible_inner(vis_index, vis_len, page);
    }

    /// Select a visible row, extending a visual range when requested.
    ///
    /// This is the visible-index counterpart of
    /// [`Self::select_item_with_modifier`] for pointer adapters that already
    /// resolved their hit to a prepared row.
    pub fn select_visible_with_modifier(
        &mut self,
        vis_index: usize,
        vis_len: usize,
        page: usize,
        extend_selection: bool,
    ) {
        if extend_selection && self.visual_selection_enabled {
            let before = self.cursor_id();
            self.select_visible_inner(vis_index, vis_len, page);
            self.extend_visual_selection(before);
        } else {
            self.select_visible(vis_index, vis_len, page);
        }
    }

    fn select_visible_inner(&mut self, vis_index: usize, vis_len: usize, page: usize) {
        self.follow = false;
        self.selected = vis_index;
        if self.cache.is_some() {
            if self.is_physical() {
                self.clamp_physical(page);
            } else {
                self.clamp_cached_fixed(page);
            }
            self.selected_id = self.item_id(self.selected);
        } else {
            self.clamp(vis_len, page);
        }
    }

    fn is_physical(&self) -> bool {
        matches!(self.layout(), Some(ListLayoutCache::Variable { .. }))
    }

    fn first_selectable(&self, down: bool) -> Option<usize> {
        let cache = self.cache.as_ref()?;
        if down {
            cache.selectable.iter().position(|selectable| *selectable)
        } else {
            cache.selectable.iter().rposition(|selectable| *selectable)
        }
    }

    fn next_selectable(&self, from: usize, down: bool) -> Option<usize> {
        let cache = self.cache.as_ref()?;
        if down {
            (from..cache.selectable.len())
                .find(|index| cache.selectable[*index])
                .or_else(|| cache.selectable.iter().position(|selectable| *selectable))
        } else {
            (0..=from.min(cache.selectable.len().saturating_sub(1)))
                .rev()
                .find(|index| cache.selectable[*index])
                .or_else(|| cache.selectable.iter().rposition(|selectable| *selectable))
        }
    }

    fn set_selected_visible(&mut self, selected: usize) {
        self.selected = selected;
        self.selected_id = self.item_id(selected);
    }

    fn clamp_cached_fixed(&mut self, page: usize) {
        let count = self.visible_count();
        if count == 0 {
            self.selected = 0;
            self.selected_id = None;
            self.offset = 0;
            return;
        }
        if self.follow {
            self.selected = self.first_selectable(false).unwrap_or(0);
        }
        self.selected = self.selected.min(count - 1);
        if !self.cache.as_ref().is_some_and(|cache| {
            cache
                .selectable
                .get(self.selected)
                .copied()
                .unwrap_or(false)
        }) {
            self.selected = self.next_selectable(self.selected, true).unwrap_or(0);
        }
        let page = page.max(1);
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + page {
            self.offset = self.selected + 1 - page;
        }
        self.offset = self.offset.min(count.saturating_sub(page));
        self.selected_id = self.item_id(self.selected);
    }

    fn clamp_physical(&mut self, page: usize) {
        let Some(layout) = self.layout() else {
            return;
        };
        let count = layout.item_count();
        let total = layout.total_height();
        if count == 0 {
            self.selected = 0;
            self.selected_id = None;
            self.offset = 0;
            return;
        }
        let page = page.max(1);
        if self.follow {
            self.selected = self.first_selectable(false).unwrap_or(0);
            self.selected_id = self.item_id(self.selected);
            self.offset = total.saturating_sub(page);
            return;
        }
        self.selected = self.selected.min(count - 1);
        if !self.cache.as_ref().is_some_and(|cache| {
            cache
                .selectable
                .get(self.selected)
                .copied()
                .unwrap_or(false)
        }) {
            self.selected = self.next_selectable(self.selected, true).unwrap_or(0);
        }
        let top = self
            .layout()
            .map_or(0, |layout| layout.virtual_y(self.selected));
        let bottom = top.saturating_add(
            self.layout()
                .map_or(1, |layout| layout.item_height(self.selected)),
        );
        if bottom.saturating_sub(top) > page {
            self.offset = self.offset.clamp(top, bottom.saturating_sub(page));
        } else if top < self.offset {
            self.offset = top;
        } else if bottom > self.offset.saturating_add(page) {
            self.offset = bottom.saturating_sub(page);
        }
        self.offset = self.offset.min(total.saturating_sub(page));
        self.selected_id = self.item_id(self.selected);
    }

    /// Bound a wheel-driven physical viewport without pulling it back to the
    /// keyboard selection. Selection changes use [`Self::clamp_physical`].
    fn clamp_physical_offset(&mut self, page: usize) {
        let Some(layout) = self.layout() else {
            return;
        };
        if layout.item_count() == 0 {
            self.offset = 0;
            return;
        }
        self.offset = self
            .offset
            .min(layout.total_height().saturating_sub(page.max(1)));
    }

    fn select_at_offset(&mut self) {
        let candidate = self
            .layout()
            .and_then(|layout| layout.item_at_y(self.offset));
        if let Some(candidate) = candidate {
            self.selected = self
                .next_selectable(candidate, true)
                .or_else(|| self.next_selectable(candidate, false))
                .unwrap_or(candidate);
            self.selected_id = self.item_id(self.selected);
        }
    }

    fn cached_scrollbar_for(&self, viewport_width: u16) -> Option<bool> {
        self.cache
            .as_ref()
            .filter(|cache| cache.viewport_width == Some(viewport_width))
            .map(|cache| cache.has_scrollbar)
    }

    fn remember_scrollbar(&mut self, viewport_width: u16, has_scrollbar: bool) {
        if let Some(cache) = self.cache.as_mut() {
            cache.viewport_width = Some(viewport_width);
            cache.has_scrollbar = has_scrollbar;
        }
    }
}

struct PreparedRow {
    source: usize,
    id: usize,
    selectable: bool,
    matched: Option<SearchMatch>,
}

pub(crate) trait ListItemSource {
    fn len(&self) -> usize;
    fn id(&self, index: usize) -> usize;
    fn revision(&self, index: usize) -> u64;
    fn selectable(&self, index: usize) -> bool;
    fn search_text(&self, index: usize) -> &str;
    fn prefix(&self, index: usize) -> Option<&Line<'_>>;
    fn physical_rows(&self) -> bool;
    fn desired_height(&self, index: usize, width: u16) -> usize;
    fn render_slice(
        &self,
        index: usize,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        context: RenderContext<'_>,
    );
    fn find(&self, index: usize, query: &SearchQuery) -> Option<SearchMatch> {
        query.find(self.search_text(index))
    }
    fn sort_fuzzy_matches(&self) -> bool {
        true
    }
}

struct RichSource<'items, 'content> {
    items: &'items [ListItem<'content>],
}

impl ListItemSource for RichSource<'_, '_> {
    fn len(&self) -> usize {
        self.items.len()
    }
    fn id(&self, index: usize) -> usize {
        self.items[index].id
    }
    fn revision(&self, index: usize) -> u64 {
        self.items[index].revision
    }
    fn selectable(&self, index: usize) -> bool {
        self.items[index].selectable
    }
    fn search_text(&self, index: usize) -> &str {
        self.items[index].search_text
    }
    fn prefix(&self, index: usize) -> Option<&Line<'_>> {
        self.items[index].prefix
    }
    fn physical_rows(&self) -> bool {
        true
    }
    fn desired_height(&self, index: usize, width: u16) -> usize {
        self.items[index].content.desired_height(width).max(1)
    }
    fn render_slice(
        &self,
        index: usize,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        context: RenderContext<'_>,
    ) {
        self.items[index]
            .content
            .render_slice_with_context(area, buf, line_offset, context);
    }
}

struct PlainSource<'slice, 'text> {
    labels: &'slice [&'text str],
    wrap_mode: WrapMode,
}

impl ListItemSource for PlainSource<'_, '_> {
    fn len(&self) -> usize {
        self.labels.len()
    }
    fn id(&self, index: usize) -> usize {
        index
    }
    fn revision(&self, _index: usize) -> u64 {
        0
    }
    fn selectable(&self, _index: usize) -> bool {
        true
    }
    fn search_text(&self, index: usize) -> &str {
        self.labels[index]
    }
    fn prefix(&self, _index: usize) -> Option<&Line<'_>> {
        None
    }
    fn physical_rows(&self) -> bool {
        self.wrap_mode == WrapMode::Wrap
    }
    fn desired_height(&self, index: usize, width: u16) -> usize {
        if self.wrap_mode == WrapMode::Wrap {
            wrapped_line_count(self.labels[index], width as usize)
        } else {
            1
        }
    }
    fn render_slice(
        &self,
        index: usize,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        context: RenderContext<'_>,
    ) {
        paint_plain_slice(
            buf,
            area,
            self.labels[index],
            self.wrap_mode,
            line_offset,
            context,
        );
    }
}

pub fn row_id(i: usize) -> HitId {
    row_id_ns(NS_ROW, i)
}
pub fn row_id_ns(ns: u32, i: usize) -> HitId {
    HitId::namespaced(ns, i as u32)
}
pub fn parse_row_id(id: HitId) -> Option<usize> {
    parse_row_id_ns(NS_ROW, id)
}
pub fn parse_row_id_ns(ns: u32, id: HitId) -> Option<usize> {
    (id.namespace() == ns).then_some(id.index() as usize)
}
pub fn scroll_id(ns: u32) -> HitId {
    HitId::namespaced(NS_SCROLL, ns)
}
pub fn is_scroll_id(ns: u32, id: HitId) -> bool {
    id.namespace() == NS_SCROLL && id.index() == ns
}

#[derive(Debug, Clone, Copy)]
pub struct ListPaneStyle {
    pub selection_bg: ratatui::style::Color,
    pub hover_bg: ratatui::style::Color,
    pub scrollbar_bg: ratatui::style::Color,
    pub scrollbar_fg: ratatui::style::Color,
}

impl ListPaneStyle {
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            selection_bg: theme
                .style(ThemeRole::Selection, ThemeState::Selected)
                .bg
                .unwrap_or(Color::Reset),
            hover_bg: theme
                .style(ThemeRole::Hover, ThemeState::Hovered)
                .bg
                .unwrap_or(Color::Reset),
            scrollbar_bg: theme
                .style(ThemeRole::Scrollbar, ThemeState::Normal)
                .bg
                .unwrap_or(Color::Reset),
            scrollbar_fg: theme
                .style(ThemeRole::Scrollbar, ThemeState::Normal)
                .fg
                .unwrap_or(Color::Reset),
        }
    }

    /// Extract legacy color slots from a resolved theme, including global
    /// semantic overrides.
    pub fn from_resolved_theme(theme: &ResolvedTheme) -> Self {
        Self {
            selection_bg: theme
                .style(ThemeRole::Selection, ThemeState::Selected)
                .bg
                .unwrap_or(Color::Reset),
            hover_bg: theme
                .style(ThemeRole::Hover, ThemeState::Hovered)
                .bg
                .unwrap_or(Color::Reset),
            scrollbar_bg: theme
                .style(ThemeRole::Scrollbar, ThemeState::Normal)
                .bg
                .unwrap_or(Color::Reset),
            scrollbar_fg: theme
                .style(ThemeRole::Scrollbar, ThemeState::Normal)
                .fg
                .unwrap_or(Color::Reset),
        }
    }
}

pub fn render_list(
    state: &mut ListPaneState,
    labels: &[&str],
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> usize {
    if area.width == 0 || area.height == 0 {
        return 0;
    }
    {
        let filter_query = state
            .filter_query
            .as_mut()
            .expect("filter query is restored after rendering");
        if filter_query.query() != state.filter {
            filter_query.set_query(state.filter.clone());
        }
    }
    let query = state.filter_query.take().expect("filter query is present");
    let source = PlainSource {
        labels,
        wrap_mode: state.wrap_mode,
    };
    let page = render_list_source_ns(state, &source, &query, NS_ROW, area, buf, cx);
    state.filter_query = Some(query);
    page
}

pub fn render_list_ns(
    state: &mut ListPaneState,
    labels: &[&str],
    ns: u32,
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> usize {
    if area.width == 0 || area.height == 0 {
        return 0;
    }
    {
        let filter_query = state
            .filter_query
            .as_mut()
            .expect("filter query is restored after rendering");
        if filter_query.query() != state.filter {
            filter_query.set_query(state.filter.clone());
        }
    }
    let query = state.filter_query.take().expect("filter query is present");
    let source = PlainSource {
        labels,
        wrap_mode: state.wrap_mode,
    };
    let page = render_list_source_ns(state, &source, &query, ns, area, buf, cx);
    state.filter_query = Some(query);
    page
}

pub fn render_list_items(
    state: &mut ListPaneState,
    items: &[ListItem<'_>],
    query: &SearchQuery,
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> usize {
    render_list_items_ns(state, items, query, NS_ROW, area, buf, cx)
}

pub fn render_list_items_ns(
    state: &mut ListPaneState,
    items: &[ListItem<'_>],
    query: &SearchQuery,
    ns: u32,
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> usize {
    let source = RichSource { items };
    render_list_source_ns(state, &source, query, ns, area, buf, cx)
}

pub(crate) fn render_list_source_ns<S: ListItemSource + ?Sized>(
    state: &mut ListPaneState,
    source: &S,
    query: &SearchQuery,
    ns: u32,
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> usize {
    if area.width == 0 || area.height == 0 {
        return 0;
    }
    let page = area.height as usize;
    let cached_bar = area.width > 2 && state.cached_scrollbar_for(area.width).unwrap_or(false);
    let mut list_width = if cached_bar {
        area.width.saturating_sub(1)
    } else {
        area.width
    };
    state.prepare_source(
        source,
        query,
        list_width.saturating_sub(PREFIX_WIDTH).max(1),
    );
    let mut bar = area.width > 2 && state.total_height() > page;
    if bar != cached_bar {
        list_width = if bar {
            area.width.saturating_sub(1)
        } else {
            area.width
        };
        state.prepare_source(
            source,
            query,
            list_width.saturating_sub(PREFIX_WIDTH).max(1),
        );
        bar = area.width > 2 && state.total_height() > page;
    }
    state.remember_scrollbar(area.width, bar);
    if state.is_physical() {
        state.clamp_physical_offset(page);
    } else {
        state.clamp_cached_fixed(page);
    }
    fill(buf, area, cx.style(ThemeRole::Surface, ThemeState::Normal));

    let Some(cache) = state.cache.as_ref() else {
        cx.register_scroll(scroll_id(ns), area);
        return page;
    };
    let Some(first) = cache.layout.item_at_y(state.offset) else {
        cx.register_scroll(scroll_id(ns), area);
        return page;
    };
    let end_offset = state.offset.saturating_add(page);
    let total = cache.layout.total_height();
    for visible_index in first..cache.layout.item_count() {
        let top = cache.layout.virtual_y(visible_index);
        if top >= end_offset {
            break;
        }
        let height = cache.layout.item_height(visible_index);
        let bottom = top.saturating_add(height);
        if bottom <= state.offset {
            continue;
        }
        let first_line = state.offset.saturating_sub(top);
        let last_line = bottom.min(end_offset).saturating_sub(top);
        let row = Rect {
            x: area.x,
            y: area
                .y
                .saturating_add(top.saturating_add(first_line).saturating_sub(state.offset) as u16),
            width: list_width,
            height: last_line.saturating_sub(first_line).min(u16::MAX as usize) as u16,
        };
        let source_index = cache.visible[visible_index];
        let id = cache.visible_ids[visible_index];
        cx.register(row_id_ns(ns, id), row);
        let cursor_selected = state.selected == visible_index && cache.selectable[visible_index];
        let range_selected = cache.visual_selection.is_some_and(|(start, end)| {
            visible_index >= start && visible_index <= end && cache.selectable[visible_index]
        });
        let selected = cursor_selected || range_selected;
        let hovered = cx.hovered(row);
        let row_state = ThemeState::from_flags(false, false, selected, false, hovered);
        let row_role = match row_state {
            ThemeState::Selected => ThemeRole::Selection,
            ThemeState::Hovered => ThemeRole::Hover,
            _ => ThemeRole::Text,
        };
        let mut base = cx
            .style(ThemeRole::Surface, ThemeState::Normal)
            .patch(cx.style(row_role, row_state));
        if cursor_selected {
            base = base.add_modifier(Modifier::BOLD);
        }
        let row_background = Style::default().bg(base.bg.unwrap_or(Color::Reset));
        fill(buf, row, base);
        let prefix = if cursor_selected { "▸ " } else { "  " };
        buf.set_string_safe(row.x, row.y, prefix, base);
        let prefix_width = source
            .prefix(source_index)
            .map_or(0, line_width)
            .min(row.width.saturating_sub(PREFIX_WIDTH));
        if first_line == 0
            && let Some(item_prefix) = source.prefix(source_index)
        {
            buf.set_line_safe(
                row.x.saturating_add(PREFIX_WIDTH),
                row.y,
                item_prefix,
                prefix_width,
            );
            buf.set_style(
                Rect {
                    x: row.x.saturating_add(PREFIX_WIDTH),
                    y: row.y,
                    width: prefix_width,
                    height: 1,
                },
                row_background,
            );
        }
        let content_x = row
            .x
            .saturating_add(PREFIX_WIDTH)
            .saturating_add(prefix_width);
        let content = Rect {
            x: content_x,
            y: row.y,
            width: row.right().saturating_sub(content_x),
            height: row.height,
        };
        let ranges = cache.matches[visible_index]
            .as_ref()
            .map_or(&[][..], |matched| matched.ranges.as_slice());
        let match_style = base
            .patch(cx.style(ThemeRole::Fuzzy, ThemeState::Normal))
            .add_modifier(Modifier::BOLD);
        source.render_slice(
            source_index,
            content,
            buf,
            first_line,
            RenderContext {
                source_id: Some(id),
                search_text: source.search_text(source_index),
                match_ranges: ranges,
                selected,
                hovered,
                style: base,
                match_style,
            },
        );
        buf.set_style(row, row_background);
    }
    cx.register_scroll(scroll_id(ns), area);
    if bar {
        paint_scrollbar(area, total, state.offset, page, ns, buf, cx);
    }
    page
}

fn line_width(line: &Line<'_>) -> u16 {
    line.spans
        .iter()
        .map(|span| span.content.width())
        .sum::<usize>()
        .min(u16::MAX as usize) as u16
}

fn paint_plain_slice(
    buf: &mut Buffer,
    area: Rect,
    text: &str,
    mode: WrapMode,
    line_offset: usize,
    context: RenderContext<'_>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match mode {
        WrapMode::NoWrap if line_offset == 0 => {
            let line = text.split('\n').next().unwrap_or_default();
            paint_highlighted(buf, area.x, area.y, area.width, line, 0, context);
        }
        WrapMode::Wrap => {
            visit_wrapped_segments(text, area.width as usize, |line, offset, segment| {
                if line == line_offset {
                    paint_highlighted(buf, area.x, area.y, area.width, segment, offset, context);
                }
            })
        }
        _ => {}
    }
}

fn paint_highlighted(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    text: &str,
    text_offset: usize,
    context: RenderContext<'_>,
) {
    let mut x = x;
    let right = x.saturating_add(width);
    for (offset, grapheme) in text.grapheme_indices(true) {
        let grapheme_width = grapheme.width() as u16;
        if grapheme_width > 0 && x.saturating_add(grapheme_width) > right {
            break;
        }
        let range =
            text_offset.saturating_add(offset)..text_offset.saturating_add(offset + grapheme.len());
        let matched = context
            .match_ranges
            .iter()
            .any(|hit| hit.start < range.end && range.start < hit.end);
        buf.set_string_safe(
            x,
            y,
            grapheme,
            if matched {
                context.match_style
            } else {
                context.style
            },
        );
        x = x.saturating_add(grapheme_width);
    }
}

fn wrapped_line_count(text: &str, width: usize) -> usize {
    let mut count = 0;
    visit_wrapped_segments(text, width, |_, _, _| count += 1);
    count.max(1)
}

fn visit_wrapped_segments<'a>(
    text: &'a str,
    width: usize,
    mut visit: impl FnMut(usize, usize, &'a str),
) {
    let width = width.max(1);
    let mut physical = 0;
    let mut logical_offset = 0;
    for logical in text.split('\n') {
        if logical.is_empty() {
            visit(physical, logical_offset, "");
            physical += 1;
            logical_offset += 1;
            continue;
        }
        let mut start = 0;
        let mut used = 0usize;
        for (index, grapheme) in logical.grapheme_indices(true) {
            let grapheme_width = grapheme.width();
            if used > 0 && used.saturating_add(grapheme_width) > width {
                visit(physical, logical_offset + start, &logical[start..index]);
                physical += 1;
                start = index;
                used = 0;
            }
            used = used.saturating_add(grapheme_width);
        }
        visit(physical, logical_offset + start, &logical[start..]);
        physical += 1;
        logical_offset += logical.len() + 1;
    }
}

fn ascii_contains_case_insensitive(label: &str, query: &[u8]) -> bool {
    query.is_empty()
        || label
            .as_bytes()
            .windows(query.len())
            .any(|window| window.eq_ignore_ascii_case(query))
}

fn paint_scrollbar(
    area: Rect,
    total: usize,
    offset: usize,
    page: usize,
    ns: u32,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    let style = cx.style(ThemeRole::Scrollbar, ThemeState::Normal);
    let track = Rect {
        x: area.right().saturating_sub(1),
        y: area.y,
        width: 1,
        height: area.height,
    };
    cx.register(scroll_id(ns), track);
    fill(buf, track, style);
    let h = area.height.max(1) as usize;
    let thumb_h = ((page * h) / total.max(1)).max(1).min(h);
    let max_offset = total.saturating_sub(page).max(1);
    let thumb_y = (offset * (h - thumb_h)) / max_offset;
    for row in 0..thumb_h {
        buf.set_string_safe(
            track.x,
            area.y + (thumb_y + row).min(h.saturating_sub(1)) as u16,
            "█",
            style,
        );
    }
}

/// Map a scrollbar click or drag to the current list offset.
pub fn click_scrollbar(state: &mut ListPaneState, y: u16, area: Rect, vis_len: usize) {
    let page = area.height as usize;
    if area.height == 0 {
        return;
    }
    if state.is_physical() && state.cache.is_some() {
        let total = state.total_height();
        if total <= page {
            return;
        }
        let rel = y.saturating_sub(area.y) as usize;
        state.follow = false;
        state.offset =
            rel * total.saturating_sub(page) / (area.height as usize).saturating_sub(1).max(1);
        state.select_at_offset();
        state.clamp_physical(page);
        return;
    }
    if vis_len <= page {
        return;
    }
    let rel = y.saturating_sub(area.y) as usize;
    state.follow = false;
    state.offset =
        rel * vis_len.saturating_sub(page) / (area.height as usize).saturating_sub(1).max(1);
    state.selected = state.offset.min(vis_len.saturating_sub(1));
    state.clamp(vis_len, page);
    if state.cache.is_some() {
        state.selected_id = state.item_id(state.selected);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::AnimationClock;
    use gbtui_runtime::HitMap;
    use std::time::Instant;

    struct Tall(&'static str);
    impl Renderable for Tall {
        fn render_slice(&self, area: Rect, buf: &mut Buffer, line_offset: usize) {
            if line_offset < 3 {
                buf.set_string_safe(area.x, area.y, self.0, Style::default());
            }
        }
        fn desired_height(&self, _width: u16) -> usize {
            3
        }
    }

    #[test]
    fn rich_rows_keep_following_rows_reachable_and_hit_stable_ids() {
        let first = Tall("long");
        let second = Tall("tail");
        let items = [
            ListItem::new(41, &first, "long").revision(1),
            ListItem::new(99, &second, "tail").revision(1),
        ];
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 2));
        let mut state = ListPaneState::default();
        let query = SearchQuery::new();
        render_list_items(
            &mut state,
            &items,
            &query,
            Rect::new(0, 0, 12, 2),
            &mut buf,
            &mut cx,
        );
        state.handle_scroll(99, items.len(), 2);
        render_list_items(
            &mut state,
            &items,
            &query,
            Rect::new(0, 0, 12, 2),
            &mut buf,
            &mut cx,
        );
        assert_eq!(hits.hit(1, 1), gbtui_runtime::HitResult::Hit(row_id(99)));
    }

    #[test]
    fn wrapped_long_plain_row_keeps_its_successor_mouse_reachable() {
        let labels = ["a single line long enough to wrap", "tail"];
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 2));
        let mut state = ListPaneState::with_wrap_mode(WrapMode::Wrap);
        render_list(
            &mut state,
            &labels,
            Rect::new(0, 0, 6, 2),
            &mut buf,
            &mut cx,
        );
        state.handle_scroll(99, labels.len(), 2);
        render_list(
            &mut state,
            &labels,
            Rect::new(0, 0, 6, 2),
            &mut buf,
            &mut cx,
        );
        assert_eq!(hits.hit(1, 1), gbtui_runtime::HitResult::Hit(row_id(1)));
    }

    #[test]
    fn rich_filter_stably_ranks_fuzzy_matches_and_keeps_unicode_graphemes() {
        let first = Tall("first");
        let second = Tall("second");
        let items = [
            ListItem::new(7, &first, "甲a😀b"),
            ListItem::new(8, &second, "ab"),
        ];
        let mut query = SearchQuery::new();
        query.set_kind(SearchKind::Fuzzy);
        query.set_query("ab");
        let mut state = ListPaneState::default();
        state.desired_item_height_at(&items, &query, 10);
        assert_eq!(state.item_id(0), Some(8));

        let mut unicode = SearchQuery::new();
        unicode.set_query("e");
        assert_eq!(unicode.find("e\u{301}cho").unwrap().ranges, vec![0..3]);
    }

    #[test]
    fn styled_line_highlights_a_full_combining_grapheme() {
        let content = Line::from("e\u{301}cho");
        let items = [ListItem::new(41, &content, "e\u{301}cho")];
        let mut query = SearchQuery::new();
        query.set_query("e");
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 1));
        render_list_items(
            &mut ListPaneState::default(),
            &items,
            &query,
            Rect::new(0, 0, 12, 1),
            &mut buf,
            &mut cx,
        );
        assert_eq!(buf[(2, 0)].style().fg, Some(theme.fuzzy_accent));
    }

    #[test]
    fn fuzzy_search_mode_keeps_original_row_order() {
        let first = Tall("first");
        let second = Tall("second");
        let items = [
            ListItem::new(7, &first, "甲a😀b"),
            ListItem::new(8, &second, "ab"),
        ];
        let mut query = SearchQuery::new();
        query.set_kind(SearchKind::Fuzzy);
        query.set_mode(SearchMode::Search);
        query.set_query("ab");
        let mut state = ListPaneState::default();
        state.desired_item_height_at(&items, &query, 10);
        assert_eq!(state.item_id(0), Some(7));
        assert_eq!(state.item_id(1), Some(8));
    }

    struct Sized(usize);
    impl Renderable for Sized {
        fn render_slice(&self, _area: Rect, _buf: &mut Buffer, _line_offset: usize) {}
        fn desired_height(&self, _width: u16) -> usize {
            self.0
        }
    }

    #[test]
    fn item_revision_invalidates_measured_height_without_losing_identity() {
        let short = Sized(1);
        let tall = Sized(3);
        let query = SearchQuery::new();
        let mut state = ListPaneState::default();
        assert_eq!(
            state.desired_item_height_at(
                &[ListItem::new(77, &short, "same").revision(1)],
                &query,
                10
            ),
            1
        );
        assert_eq!(
            state.desired_item_height_at(
                &[ListItem::new(77, &tall, "same").revision(2)],
                &query,
                10
            ),
            3
        );
        assert_eq!(state.item_id(0), Some(77));
    }

    #[test]
    fn wrapped_cache_rebuilds_for_width_and_explicit_content_revision() {
        let mut state = ListPaneState::with_wrap_mode(WrapMode::Wrap);
        state.set_layout_revision(1);
        assert_eq!(state.desired_height_at(&["abcdefgh"], 2), 4);
        assert_eq!(state.desired_height_at(&["abcdefgh"], 4), 2);
        state.set_layout_revision(2);
        assert_eq!(state.desired_height_at(&["abcd"], 2), 2);
    }

    #[test]
    fn visual_range_keeps_stable_endpoints_across_reorder() {
        let first = Line::from("first");
        let header = Line::from("header");
        let second = Line::from("second");
        let third = Line::from("third");
        let initial = [
            ListItem::new(10, &first, "first"),
            ListItem::new(20, &header, "header").selectable(false),
            ListItem::new(30, &second, "second"),
            ListItem::new(40, &third, "third"),
        ];
        let query = SearchQuery::new();
        let mut state = ListPaneState::default();
        state.set_visual_selection_enabled(true);
        state.desired_item_height_at(&initial, &query, 20);
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
            initial.len(),
            4,
        ));
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
            initial.len(),
            4,
        ));
        assert_eq!(state.selection_ids().collect::<Vec<_>>(), [10, 30]);

        let reordered = [
            ListItem::new(40, &third, "third"),
            ListItem::new(20, &header, "header").selectable(false),
            ListItem::new(30, &second, "second"),
            ListItem::new(10, &first, "first"),
        ];
        state.desired_item_height_at(&reordered, &query, 20);
        assert_eq!(state.selection_range(), Some((10, 30)));
        assert_eq!(state.selection_ids().collect::<Vec<_>>(), [30, 10]);
    }

    #[test]
    fn visual_range_cancels_when_its_anchor_is_filtered_or_removed() {
        let first = Line::from("first");
        let second = Line::from("second");
        let third = Line::from("third");
        let items = [
            ListItem::new(10, &first, "first"),
            ListItem::new(30, &second, "second"),
            ListItem::new(40, &third, "third"),
        ];
        let unfiltered = SearchQuery::new();
        let mut state = ListPaneState::default();
        state.set_visual_selection_enabled(true);
        state.desired_item_height_at(&items, &unfiltered, 20);
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
            items.len(),
            4,
        ));
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
            items.len(),
            4,
        ));

        let mut filtered = SearchQuery::new();
        filtered.set_mode(SearchMode::Filter);
        filtered.set_query("second");
        state.desired_item_height_at(&items, &filtered, 20);
        assert!(!state.visual_mode());
        assert_eq!(state.selection_ids().next(), None);

        state.desired_item_height_at(&items, &unfiltered, 20);
        assert!(state.select_item(10, 4));
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
            items.len(),
            4,
        ));
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
            items.len(),
            4,
        ));
        let without_anchor = [
            ListItem::new(30, &second, "second"),
            ListItem::new(40, &third, "third"),
        ];
        state.desired_item_height_at(&without_anchor, &unfiltered, 20);
        assert!(!state.visual_mode());
        assert_eq!(state.selection_range(), None);
    }

    #[test]
    fn visual_range_extracts_host_copy_text_without_losing_unicode() {
        let first = Line::from("first");
        let second = Line::from("second");
        let items = [
            ListItem::new(7, &first, "fallback α").with_copy_text("甲e\u{301} 😀"),
            ListItem::new(8, &second, "fallback β"),
        ];
        let query = SearchQuery::new();
        let mut state = ListPaneState::default();
        state.set_visual_selection_enabled(true);
        state.desired_item_height_at(&items, &query, 20);
        assert!(state.select_item(7, 2));
        assert!(state.select_item_with_modifier(8, 2, true));
        assert_eq!(state.selection_ids().collect::<Vec<_>>(), [7, 8]);
        assert_eq!(
            state.selected_text(&items).as_deref(),
            Some("甲e\u{301} 😀\nfallback β")
        );
    }

    #[test]
    fn visual_range_highlights_every_visible_physical_row() {
        let first = Tall("first");
        let second = Tall("second");
        let items = [
            ListItem::new(7, &first, "first"),
            ListItem::new(8, &second, "second"),
        ];
        let query = SearchQuery::new();
        let mut state = ListPaneState::default();
        state.set_visual_selection_enabled(true);
        state.desired_item_height_at(&items, &query, 12);
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
            items.len(),
            4,
        ));
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
            items.len(),
            4,
        ));

        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 4));
        render_list_items(
            &mut state,
            &items,
            &query,
            Rect::new(0, 0, 12, 4),
            &mut buf,
            &mut cx,
        );
        for y in 0..4 {
            assert_eq!(buf[(0, y)].style().bg, Some(theme.bg_visual));
        }
    }

    #[test]
    fn visual_range_keys_are_inactive_until_explicitly_enabled() {
        let line = Line::from("row");
        let items = [ListItem::new(7, &line, "row")];
        let query = SearchQuery::new();
        let mut state = ListPaneState::default();
        state.desired_item_height_at(&items, &query, 20);
        assert!(!state.handle_key(
            &KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
            items.len(),
            1,
        ));
        assert_eq!(state.selected_id(), Some(7));
        assert!(!state.visual_mode());
        assert_eq!(state.selection_ids().next(), None);

        state.set_visual_selection_enabled(true);
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
            items.len(),
            1,
        ));
        assert!(state.visual_mode());
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            items.len(),
            1,
        ));
        assert!(!state.visual_mode());
    }

    #[test]
    fn shift_range_extension_stops_at_visible_boundaries() {
        let first = Line::from("first");
        let second = Line::from("second");
        let items = [
            ListItem::new(7, &first, "first"),
            ListItem::new(8, &second, "second"),
        ];
        let query = SearchQuery::new();
        let mut state = ListPaneState::default();
        state.set_visual_selection_enabled(true);
        state.desired_item_height_at(&items, &query, 20);
        assert!(state.select_item(8, 2));
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
            items.len(),
            2,
        ));
        assert_eq!(state.selection_range(), Some((8, 8)));
        assert_eq!(state.selection_ids().collect::<Vec<_>>(), [8]);
    }
}
