//! Searchable rich picker built on the virtualized list engine.

use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gbtui_core::renderable::RenderContext;
use gbtui_core::{HitId, SafeBuf, ThemeRole, ThemeState, key};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Text};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::line_editor::{LineEditOutcome, LineEditor};
use crate::list::{
    self, ListItemSource, ListPaneState, click_scrollbar, parse_row_id_ns, render_list_source_ns,
};
use crate::paint::fill;
use crate::search::{SearchKind, SearchMatch, SearchMode, SearchQuery};

pub const NS_PICKER: u32 = 0x504B;
pub const NS_PICKER_GROUP: u32 = 0x504C;
pub const NS_PICKER_EXPAND: u32 = 0x504D;

/// One additional detail line in a [`PickerRow`].
#[derive(Debug, Clone)]
pub struct PickerField<'a> {
    line: Line<'a>,
}

impl<'a> PickerField<'a> {
    pub fn new(line: impl Into<Line<'a>>) -> Self {
        Self { line: line.into() }
    }
}

/// A selectable rich picker row. Constructors keep optional presentation data
/// out of callers' field literals while retaining borrowed, allocation-free text.
#[derive(Debug, Clone)]
pub struct PickerRow<'a> {
    id: usize,
    label: Line<'a>,
    right: Option<Line<'a>>,
    summary: Option<Line<'a>>,
    description: Option<Text<'a>>,
    fields: &'a [PickerField<'a>],
    badge: Option<Line<'a>>,
    indent: u16,
    dimmed: bool,
    always_expanded: bool,
    search_text: &'a str,
    search_fields: &'a [&'a str],
    inline_search_fields: [Option<&'a str>; 4],
    revision: u64,
}

impl<'a> PickerRow<'a> {
    pub fn new(id: usize, label: &'a str) -> Self {
        Self {
            id,
            label: Line::from(label),
            right: None,
            summary: None,
            description: None,
            fields: &[],
            badge: None,
            indent: 0,
            dimmed: false,
            always_expanded: false,
            search_text: label,
            search_fields: &[],
            inline_search_fields: [None; 4],
            revision: 0,
        }
    }

    pub fn label(mut self, label: impl Into<Line<'a>>) -> Self {
        self.label = label.into();
        self
    }
    pub fn right(mut self, right: impl Into<Line<'a>>) -> Self {
        self.right = Some(right.into());
        self
    }
    pub fn summary(mut self, summary: impl Into<Line<'a>>) -> Self {
        self.summary = Some(summary.into());
        self
    }
    pub fn description(mut self, description: impl Into<Text<'a>>) -> Self {
        self.description = Some(description.into());
        self
    }
    pub fn fields(mut self, fields: &'a [PickerField<'a>]) -> Self {
        self.fields = fields;
        self
    }
    pub fn badge(mut self, badge: impl Into<Line<'a>>) -> Self {
        self.badge = Some(badge.into());
        self
    }
    pub fn indent(mut self, indent: u16) -> Self {
        self.indent = indent;
        self
    }
    pub fn dimmed(mut self) -> Self {
        self.dimmed = true;
        self
    }
    /// Keep descriptions and fields visible; state expansion and expander hits do not apply.
    pub fn always_expanded(mut self) -> Self {
        self.always_expanded = true;
        self
    }
    pub fn search_text(mut self, search_text: &'a str) -> Self {
        self.search_text = search_text;
        self
    }
    /// Add borrowed searchable text without building a concatenated string.
    ///
    /// The configured [`Self::search_text`] (the `new` label by default)
    /// remains the primary field used for highlighting. Auxiliary-field
    /// matches participate in filtering, navigation, and fuzzy ranking.
    pub fn search_fields(mut self, search_fields: &'a [&'a str]) -> Self {
        self.search_fields = search_fields;
        self
    }
    /// Add one of up to four auxiliary borrowed search fragments without an
    /// allocation. For a longer stable collection, use [`Self::search_fields`].
    pub fn search_field(mut self, search_field: &'a str) -> Self {
        let slot = self
            .inline_search_fields
            .iter_mut()
            .find(|slot| slot.is_none())
            .expect("PickerRow supports at most four inline search fields; use search_fields");
        *slot = Some(search_field);
        self
    }
    /// Exact host-owned revision for borrowed text or layout-affecting style.
    ///
    /// Change this when a row with the same stable id changes in place.
    pub fn revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }

    pub fn id(&self) -> usize {
        self.id
    }
    fn is_expandable(&self) -> bool {
        self.description.is_some() || !self.fields.is_empty()
    }
    fn is_persistently_expanded(&self) -> bool {
        self.always_expanded && self.is_expandable()
    }

    fn find(&self, query: &SearchQuery) -> Option<SearchMatch> {
        let mut best = query.find(self.search_text);
        for field in self.search_fields {
            let candidate = query.find(field);
            if candidate.as_ref().is_some_and(|candidate| {
                best.as_ref()
                    .is_none_or(|current| candidate.score > current.score)
            }) {
                best = candidate;
            }
        }
        for field in self.inline_search_fields.iter().flatten() {
            let candidate = query.find(field);
            if candidate.as_ref().is_some_and(|candidate| {
                best.as_ref()
                    .is_none_or(|current| candidate.score > current.score)
            }) {
                best = candidate;
            }
        }
        best
    }

    fn height(&self, width: u16, inherited_indent: u16, expanded: bool) -> usize {
        let mut width = inner_width(width, inherited_indent.saturating_add(self.indent));
        if self.is_expandable() {
            width = width.saturating_sub(2).max(1);
        }
        let (right, badge, spacing) = metadata_widths(
            width,
            self.right.as_ref().map_or(0, line_width),
            self.badge.as_ref().map_or(0, line_width),
        );
        let label_width = width
            .saturating_sub(right.saturating_add(badge).saturating_add(spacing))
            .max(1);
        let mut height = line_height(&self.label, label_width);
        if expanded {
            if let Some(description) = &self.description {
                for line in &description.lines {
                    height = height.saturating_add(line_height(line, width));
                }
            }
            for field in self.fields {
                height = height.saturating_add(line_height(&field.line, width));
            }
        } else if let Some(summary) = &self.summary {
            height = height.saturating_add(line_height(summary, width));
        }
        height.max(1)
    }

    fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        inherited_indent: u16,
        expanded: bool,
        context: RenderContext<'_>,
    ) {
        let indent = inherited_indent.saturating_add(self.indent);
        let marker_x = area.x.saturating_add(indent.saturating_mul(2));
        let expandable = self.is_expandable();
        let left = marker_x.saturating_add(if expandable { 2 } else { 0 });
        let width = area.right().saturating_sub(left).max(1);
        let (right_width, badge_width, spacing) = metadata_widths(
            width,
            self.right.as_ref().map_or(0, line_width),
            self.badge.as_ref().map_or(0, line_width),
        );
        let label_width = width
            .saturating_sub(
                right_width
                    .saturating_add(badge_width)
                    .saturating_add(spacing),
            )
            .max(1);
        let mut offset = 0usize;
        let label_height = line_height(&self.label, label_width);
        let body = Rect {
            x: left,
            y: area.y,
            width,
            height: area.height,
        };
        if expandable && !self.is_persistently_expanded() && line_offset == 0 {
            buf.set_string_safe(
                marker_x,
                area.y,
                if expanded { "▾ " } else { "▸ " },
                context.style,
            );
        }
        paint_component(
            &self.label,
            buf,
            Rect {
                width: label_width,
                ..body
            },
            offset,
            line_offset,
            context,
            self.dimmed,
        );
        let context = RenderContext {
            match_ranges: &[],
            ..context
        };
        if line_offset == 0 {
            let right_x = body.right().saturating_sub(right_width);
            if let Some(badge) = &self.badge
                && badge_width > 0
            {
                let x = right_x.saturating_sub(spacing).saturating_sub(badge_width);
                draw_line_slice(
                    badge,
                    buf,
                    Rect {
                        x,
                        y: body.y,
                        width: badge_width,
                        height: 1,
                    },
                    0,
                    context,
                    false,
                    self.dimmed,
                );
            }
            if let Some(right) = &self.right
                && right_width > 0
            {
                draw_line_slice(
                    right,
                    buf,
                    Rect {
                        x: right_x,
                        y: body.y,
                        width: right_width,
                        height: 1,
                    },
                    0,
                    context,
                    false,
                    self.dimmed,
                );
            }
        }
        offset = offset.saturating_add(label_height);
        if expanded {
            if let Some(description) = &self.description {
                let context = RenderContext {
                    style: context.style.patch(description.style),
                    ..context
                };
                for line in &description.lines {
                    paint_component(line, buf, body, offset, line_offset, context, self.dimmed);
                    offset = offset.saturating_add(line_height(line, width));
                }
            }
            for field in self.fields {
                paint_component(
                    &field.line,
                    buf,
                    body,
                    offset,
                    line_offset,
                    context,
                    self.dimmed,
                );
                offset = offset.saturating_add(line_height(&field.line, width));
            }
        } else if let Some(summary) = &self.summary {
            paint_component(
                summary,
                buf,
                body,
                offset,
                line_offset,
                context,
                self.dimmed,
            );
        }
    }
}

/// A non-selectable group header. Its children are subsequent entries until a
/// group at the same or smaller indentation level.
#[derive(Debug, Clone)]
pub struct PickerGroup<'a> {
    id: usize,
    label: Line<'a>,
    search_text: &'a str,
    indent: u16,
    collapsible: bool,
    revision: u64,
}

impl<'a> PickerGroup<'a> {
    pub fn new(id: usize, label: &'a str) -> Self {
        Self {
            id,
            label: Line::from(label),
            search_text: label,
            indent: 0,
            collapsible: true,
            revision: 0,
        }
    }
    pub fn label(mut self, label: impl Into<Line<'a>>) -> Self {
        self.label = label.into();
        self
    }
    pub fn search_text(mut self, search_text: &'a str) -> Self {
        self.search_text = search_text;
        self
    }
    pub fn indent(mut self, indent: u16) -> Self {
        self.indent = indent;
        self
    }
    pub fn collapsible(mut self, collapsible: bool) -> Self {
        self.collapsible = collapsible;
        self
    }
    /// Exact host-owned revision for a group with a stable id.
    pub fn revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }
    pub fn id(&self) -> usize {
        self.id
    }
}

/// Picker tree entry. Use the constructors rather than enum literals.
#[derive(Debug, Clone)]
#[expect(
    clippy::large_enum_variant,
    reason = "Rows dominate the contiguous entry array; boxing adds per-row allocation and indirection"
)]
pub enum PickerEntry<'a> {
    Group(PickerGroup<'a>),
    Row(PickerRow<'a>),
}

impl<'a> PickerEntry<'a> {
    pub fn group(group: PickerGroup<'a>) -> Self {
        Self::Group(group)
    }
    pub fn row(row: PickerRow<'a>) -> Self {
        Self::Row(row)
    }

    fn id(&self) -> usize {
        match self {
            Self::Group(group) => group.id,
            Self::Row(row) => row.id,
        }
    }
    fn revision(&self) -> u64 {
        match self {
            Self::Group(group) => group.revision,
            Self::Row(row) => row.revision,
        }
    }
    fn group_indent(&self) -> Option<u16> {
        match self {
            Self::Group(group) => Some(group.indent),
            Self::Row(_) => None,
        }
    }
    fn is_collapsible_group(&self) -> bool {
        matches!(
            self,
            Self::Group(PickerGroup {
                collapsible: true,
                ..
            })
        )
    }
    fn is_row_expandable(&self) -> bool {
        matches!(self, Self::Row(row) if row.is_expandable())
    }
    fn is_persistently_expanded(&self) -> bool {
        matches!(self, Self::Row(row) if row.is_persistently_expanded())
    }
    fn row_indent(&self) -> u16 {
        match self {
            Self::Group(_) => 0,
            Self::Row(row) => row.indent,
        }
    }
    fn search_text(&self) -> &str {
        match self {
            Self::Group(group) => group.search_text,
            Self::Row(row) => row.search_text,
        }
    }

    fn find(&self, query: &SearchQuery) -> Option<SearchMatch> {
        match self {
            Self::Group(_) => query.find(self.search_text()),
            Self::Row(row) => row.find(query),
        }
    }
    fn height(&self, width: u16, indent: u16, expanded: bool) -> usize {
        match self {
            Self::Group(_) => 1,
            Self::Row(row) => row.height(width, indent, expanded),
        }
    }
    fn render_at(
        &self,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        indent: u16,
        expanded: bool,
        context: RenderContext<'_>,
    ) {
        match self {
            Self::Group(group) => {
                if line_offset != 0 {
                    return;
                }
                let x = area.x.saturating_add(indent.saturating_mul(2));
                let width = area.right().saturating_sub(x);
                let style = context.style.add_modifier(Modifier::DIM);
                let marker = if group.collapsible {
                    if expanded { "▾ " } else { "▸ " }
                } else {
                    "── "
                };
                let marker_width = if group.collapsible { 2 } else { 3 };
                if width >= marker_width {
                    buf.set_string_safe(x, area.y, marker, style);
                }
                let prefix = marker_width.min(width);
                draw_line_slice(
                    &group.label,
                    buf,
                    Rect {
                        x: x.saturating_add(prefix),
                        y: area.y,
                        width: width.saturating_sub(prefix),
                        height: 1,
                    },
                    0,
                    context,
                    false,
                    true,
                );
                let used = prefix.saturating_add(line_width(&group.label));
                if width.saturating_sub(used) >= 3 {
                    buf.set_string_safe(x.saturating_add(used), area.y, " ──", style);
                }
            }
            Self::Row(row) => row.render(area, buf, line_offset, indent, expanded, context),
        }
    }
}

/// Search editor, group collapse state, and list navigation for a picker.
#[derive(Debug, Clone)]
pub struct PickerState {
    pub list: ListPaneState,
    editor: LineEditor,
    search: SearchQuery,
    collapsed: HashSet<usize>,
    expanded_rows: HashSet<usize>,
    flat: Vec<FlatRow>,
    groups: Vec<ActiveGroup>,
    source_ids: Vec<(usize, u64)>,
    prepared_query_revision: Option<u64>,
    tree_revision: u64,
    list_area: Rect,
}

impl Default for PickerState {
    fn default() -> Self {
        let mut search = SearchQuery::new();
        search.set_kind(SearchKind::Fuzzy);
        Self {
            list: ListPaneState::default(),
            editor: LineEditor::default(),
            search,
            collapsed: HashSet::new(),
            expanded_rows: HashSet::new(),
            flat: Vec::new(),
            groups: Vec::new(),
            source_ids: Vec::new(),
            prepared_query_revision: None,
            tree_revision: 0,
            list_area: Rect::default(),
        }
    }
}

impl PickerState {
    pub fn query(&self) -> &str {
        self.editor.text()
    }
    pub fn search_kind(&self) -> SearchKind {
        self.search.kind()
    }
    pub fn search_mode(&self) -> SearchMode {
        self.search.mode()
    }
    pub fn search_error(&self) -> Option<&str> {
        self.search.error()
    }

    pub fn set_query(&mut self, query: impl AsRef<str>) {
        let query = query.as_ref();
        if self.editor.text() == query {
            return;
        }
        self.editor.set_text(query);
        self.search.set_query(self.editor.text());
        self.invalidate_tree();
    }

    pub fn set_search_kind(&mut self, kind: SearchKind) {
        if self.search.kind() == kind {
            return;
        }
        self.search.set_kind(kind);
        self.invalidate_tree();
    }

    pub fn set_search_mode(&mut self, mode: SearchMode) {
        if self.search.mode() == mode {
            return;
        }
        self.search.set_mode(mode);
        self.invalidate_tree();
    }

    pub fn cycle_search_kind(&mut self) {
        self.set_search_kind(match self.search.kind() {
            SearchKind::Substring => SearchKind::Fuzzy,
            SearchKind::Fuzzy => SearchKind::Regex,
            SearchKind::Regex => SearchKind::Substring,
        });
    }

    pub fn toggle_search_mode(&mut self) {
        self.set_search_mode(match self.search.mode() {
            SearchMode::Filter => SearchMode::Search,
            SearchMode::Search => SearchMode::Filter,
        });
    }

    pub fn toggle_group(&mut self, group_id: usize) {
        if !self.collapsed.insert(group_id) {
            self.collapsed.remove(&group_id);
        }
        self.invalidate_tree();
    }

    pub fn expand_row(&mut self, row_id: usize) {
        if self.expanded_rows.insert(row_id) {
            self.invalidate_tree();
        }
    }

    pub fn collapse_row(&mut self, row_id: usize) {
        if self.expanded_rows.remove(&row_id) {
            self.invalidate_tree();
        }
    }

    pub fn toggle_row(&mut self, row_id: usize) {
        if !self.expanded_rows.insert(row_id) {
            self.expanded_rows.remove(&row_id);
        }
        self.invalidate_tree();
    }

    pub fn is_row_expanded(&self, row_id: usize) -> bool {
        self.flat
            .iter()
            .find(|row| !row.group && row.id == row_id)
            .map_or_else(|| self.expanded_rows.contains(&row_id), |row| row.expanded)
    }

    /// Rebuild source-independent tree projection after query, collapse, or item revision changes.
    pub fn prepare(&mut self, items: &[PickerEntry<'_>]) {
        if self.search.query() != self.editor.text() {
            self.search.set_query(self.editor.text());
        }
        let stale = self.source_ids.len() != items.len()
            || self.prepared_query_revision != Some(self.search.revision())
            || self
                .source_ids
                .iter()
                .zip(items)
                .any(|((id, revision), entry)| *id != entry.id() || *revision != entry.revision());
        if !stale {
            return;
        }
        self.source_ids.clear();
        self.source_ids
            .extend(items.iter().map(|entry| (entry.id(), entry.revision())));
        self.flat.clear();
        self.groups.clear();
        for (source, entry) in items.iter().enumerate() {
            if let Some(indent) = entry.group_indent() {
                while self
                    .groups
                    .last()
                    .is_some_and(|active| active.indent >= indent)
                {
                    self.groups.pop();
                }
                let ancestor_collapsed = self.groups.iter().any(|active| active.collapsed);
                let collapsed =
                    entry.is_collapsible_group() && self.collapsed.contains(&entry.id());
                let matched = group_matches(items, source, &self.search);
                let visible = if self.query().is_empty() {
                    !ancestor_collapsed
                } else if ancestor_collapsed {
                    matched
                } else {
                    self.search.mode() == SearchMode::Search || matched
                };
                if visible {
                    self.flat.push(FlatRow {
                        source,
                        id: entry.id(),
                        group: true,
                        indent,
                        expanded: !collapsed,
                        matched,
                        score: 0,
                    });
                }
                self.groups.push(ActiveGroup {
                    indent,
                    collapsed: ancestor_collapsed || collapsed,
                });
                continue;
            }
            let matched = entry.find(&self.search);
            let collapsed = self.groups.iter().any(|active| active.collapsed);
            let visible = if self.query().is_empty() {
                !collapsed
            } else if self.search.mode() == SearchMode::Filter {
                matched.is_some()
            } else {
                !collapsed || matched.is_some()
            };
            if visible {
                self.flat.push(FlatRow {
                    source,
                    id: entry.id(),
                    group: false,
                    indent: self
                        .groups
                        .last()
                        .map_or(0, |active| active.indent.saturating_add(1)),
                    expanded: entry.is_persistently_expanded()
                        || self.expanded_rows.contains(&entry.id()),
                    matched: matched.is_some(),
                    score: matched.as_ref().map_or(0, |m| m.score),
                });
            }
        }
        if self.search.mode() == SearchMode::Filter
            && self.search.kind() == SearchKind::Fuzzy
            && !self.query().is_empty()
        {
            let mut run = 0;
            while run < self.flat.len() {
                if self.flat[run].group {
                    run += 1;
                    continue;
                }
                let end = (run..self.flat.len())
                    .find(|index| self.flat[*index].group)
                    .unwrap_or(self.flat.len());
                self.flat[run..end].sort_by(|left, right| right.score.cmp(&left.score));
                run = end;
            }
        }
        self.prepared_query_revision = Some(self.search.revision());
        self.tree_revision = self.tree_revision.wrapping_add(1);
        self.list.set_layout_revision(self.tree_revision);
        self.normalize_selection();
    }

    pub fn visual_len(&mut self, items: &[PickerEntry<'_>]) -> usize {
        self.prepare(items);
        self.flat.len()
    }

    /// Current selectable stable row id. This is safe before first paint and after no-result changes.
    pub fn selected_id(&mut self, items: &[PickerEntry<'_>]) -> Option<usize> {
        self.prepare(items);
        self.normalize_selection();
        self.list.selected_id()
    }

    pub fn select_id(&mut self, id: usize, items: &[PickerEntry<'_>], page: usize) -> bool {
        self.prepare(items);
        let Some(visible) = self.flat.iter().position(|row| !row.group && row.id == id) else {
            return false;
        };
        self.list.select_visible(visible, self.flat.len(), page);
        self.list.set_selection_identity(Some(id));
        true
    }

    pub fn handle_key(&mut self, key: &KeyEvent, items: &[PickerEntry<'_>], page: usize) -> bool {
        self.prepare(items);
        self.normalize_selection();
        if self.query().is_empty()
            && key.modifiers.is_empty()
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
            && let Some(row_id) = self.list.selected_id()
            && items.iter().any(|entry| {
                entry.id() == row_id
                    && entry.is_row_expandable()
                    && !entry.is_persistently_expanded()
            })
        {
            if key.code == KeyCode::Right {
                self.expand_row(row_id);
            } else {
                self.collapse_row(row_id);
            }
            self.prepare(items);
            return true;
        }
        if self.search.mode() == SearchMode::Search && !self.query().is_empty() {
            let match_navigation = (matches!(key.code, KeyCode::Char('n'))
                && key.modifiers.is_empty())
                || (matches!(key.code, KeyCode::Char('N'))
                    && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT));
            if match_navigation {
                return self.navigate_match(matches!(key.code, KeyCode::Char('n')), page);
            }
        }
        if key!('n', CONTROL).matches(key) || key!('p', CONTROL).matches(key) {
            let mut nav = *key;
            nav.code = if key!('n', CONTROL).matches(key) {
                KeyCode::Down
            } else {
                KeyCode::Up
            };
            nav.modifiers = KeyModifiers::NONE;
            let changed = self.list.handle_key(&nav, self.flat.len(), page);
            self.normalize_input_selection();
            return changed;
        }
        let empty = self.query().is_empty();
        if empty
            && let KeyCode::Char(c) = key.code
            && key.modifiers.is_empty()
            && matches!(c, 'j' | 'k' | 'g' | 'G')
        {
            let changed = self.list.handle_key(key, self.flat.len(), page);
            self.normalize_input_selection();
            return changed;
        }
        match self.editor.handle_key(key) {
            LineEditOutcome::Unhandled => {
                let changed = self.list.handle_key(key, self.flat.len(), page);
                self.normalize_input_selection();
                changed
            }
            LineEditOutcome::HandledNoChange | LineEditOutcome::CursorChanged => true,
            LineEditOutcome::TextChanged => {
                self.search.set_query(self.editor.text());
                self.invalidate_tree();
                self.prepare(items);
                true
            }
        }
    }

    /// Handle a hit from either `render_picker` or `render_picker_content`.
    pub fn handle_hit(
        &mut self,
        id: HitId,
        pointer_row: u16,
        full_area: Rect,
        items: &[PickerEntry<'_>],
    ) -> bool {
        self.prepare(items);
        let list_area = self.list_area_for(full_area);
        if let Some(row) = parse_picker_expander(id) {
            if items.iter().any(|entry| {
                entry.id() == row && entry.is_row_expandable() && !entry.is_persistently_expanded()
            }) {
                self.toggle_row(row);
                return true;
            }
            return false;
        }
        if let Some(group) = parse_picker_group(id) {
            if items
                .iter()
                .any(|entry| entry.id() == group && entry.is_collapsible_group())
            {
                self.toggle_group(group);
                return true;
            }
            return false;
        }
        if let Some(row) = parse_picker_row(id) {
            return self.select_id(row, items, list_area.height.max(1) as usize);
        }
        if is_picker_scroll(id) {
            click_scrollbar(&mut self.list, pointer_row, list_area, self.flat.len());
            self.normalize_input_selection();
            return true;
        }
        false
    }

    pub fn handle_scroll(
        &mut self,
        delta_lines: i32,
        full_area: Rect,
        items: &[PickerEntry<'_>],
    ) -> bool {
        self.prepare(items);
        let list_area = self.list_area_for(full_area);
        if list_area.height == 0 {
            return false;
        }
        self.list
            .handle_scroll(delta_lines, self.flat.len(), list_area.height as usize);
        self.normalize_input_selection();
        true
    }

    fn navigate_match(&mut self, down: bool, page: usize) -> bool {
        let current = self.list.selected_id();
        let start = current
            .and_then(|id| self.flat.iter().position(|row| row.id == id))
            .unwrap_or(0);
        let found = if down {
            ((start + 1)..self.flat.len())
                .chain(0..=start)
                .find(|index| !self.flat[*index].group && self.flat[*index].matched)
        } else {
            (0..start)
                .rev()
                .chain((start..self.flat.len()).rev())
                .find(|index| !self.flat[*index].group && self.flat[*index].matched)
        };
        let Some(found) = found else {
            return false;
        };
        self.list.select_visible(found, self.flat.len(), page);
        self.list.set_selection_identity(Some(self.flat[found].id));
        true
    }

    fn normalize_selection(&mut self) {
        let selected = self
            .list
            .selected_id()
            .and_then(|id| self.flat.iter().position(|row| !row.group && row.id == id))
            .or_else(|| {
                self.flat
                    .get(self.list.selected)
                    .filter(|row| !row.group)
                    .map(|_| self.list.selected)
            })
            .or_else(|| self.flat.iter().position(|row| !row.group));
        match selected {
            Some(index) => {
                self.list.selected = index;
                self.list.set_selection_identity(Some(self.flat[index].id));
            }
            None => {
                self.list.selected = 0;
                self.list.offset = 0;
                self.list.set_selection_identity(None);
            }
        }
    }

    /// `ListPaneState` cannot know a picker's non-selectable headers before
    /// its render cache exists. Preserve the logical input target immediately
    /// after a query or collapse change instead of retaining a stale row id.
    fn normalize_input_selection(&mut self) {
        if self.list.layout().is_some() {
            self.normalize_selection();
            return;
        }
        let current = self.list.selected;
        let selected = self
            .flat
            .get(current)
            .filter(|row| !row.group)
            .map(|_| current)
            .or_else(|| {
                self.flat
                    .iter()
                    .enumerate()
                    .skip(current.saturating_add(1))
                    .find_map(|(index, row)| (!row.group).then_some(index))
            })
            .or_else(|| {
                self.flat
                    .iter()
                    .enumerate()
                    .take(current)
                    .rev()
                    .find_map(|(index, row)| (!row.group).then_some(index))
            });
        match selected {
            Some(index) => {
                self.list.selected = index;
                self.list.set_selection_identity(Some(self.flat[index].id));
            }
            None => {
                self.list.selected = 0;
                self.list.offset = 0;
                self.list.set_selection_identity(None);
            }
        }
    }

    fn invalidate_tree(&mut self) {
        self.flat.clear();
        self.prepared_query_revision = None;
        self.list.invalidate_layout();
    }
    fn list_area_for(&self, full_area: Rect) -> Rect {
        if self.list_area.width > 0 && self.list_area.height > 0 {
            self.list_area
        } else {
            Rect {
                x: full_area.x,
                y: full_area.y.saturating_add(1),
                width: full_area.width,
                height: full_area.height.saturating_sub(1),
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FlatRow {
    source: usize,
    id: usize,
    group: bool,
    indent: u16,
    expanded: bool,
    matched: bool,
    score: i64,
}

#[derive(Debug, Clone, Copy)]
struct ActiveGroup {
    indent: u16,
    collapsed: bool,
}

fn group_matches(items: &[PickerEntry<'_>], group: usize, query: &SearchQuery) -> bool {
    if items[group].find(query).is_some() {
        return true;
    }
    let indent = items[group].group_indent().unwrap_or(0);
    for entry in &items[group + 1..] {
        if entry.group_indent().is_some_and(|next| next <= indent) {
            break;
        }
        if entry.find(query).is_some() {
            return true;
        }
    }
    false
}

struct PickerSource<'items, 'entry, 'flat> {
    items: &'items [PickerEntry<'entry>],
    flat: &'flat [FlatRow],
}

impl ListItemSource for PickerSource<'_, '_, '_> {
    fn len(&self) -> usize {
        self.flat.len()
    }
    fn id(&self, index: usize) -> usize {
        self.flat[index].id
    }
    fn revision(&self, index: usize) -> u64 {
        self.items[self.flat[index].source].revision()
            ^ u64::from(self.flat[index].indent)
            ^ if self.flat[index].expanded {
                1u64 << 32
            } else {
                0
            }
    }
    fn selectable(&self, index: usize) -> bool {
        !self.flat[index].group
    }
    fn search_text(&self, index: usize) -> &str {
        self.items[self.flat[index].source].search_text()
    }
    fn prefix(&self, _index: usize) -> Option<&Line<'_>> {
        None
    }
    fn physical_rows(&self) -> bool {
        true
    }
    fn desired_height(&self, index: usize, width: u16) -> usize {
        self.items[self.flat[index].source].height(
            width,
            self.flat[index].indent,
            self.flat[index].expanded,
        )
    }
    fn render_slice(
        &self,
        index: usize,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        context: RenderContext<'_>,
    ) {
        self.items[self.flat[index].source].render_at(
            area,
            buf,
            line_offset,
            self.flat[index].indent,
            self.flat[index].expanded,
            context,
        );
    }
    fn find(&self, index: usize, query: &SearchQuery) -> Option<SearchMatch> {
        let flat = self.flat[index];
        if !flat.matched {
            return None;
        }
        if flat.group {
            return Some(SearchMatch {
                score: 0,
                ranges: Vec::new(),
            });
        }
        let entry = &self.items[flat.source];
        query.find(entry.search_text()).or_else(|| {
            entry.find(query).map(|matched| SearchMatch {
                score: matched.score,
                ranges: Vec::new(),
            })
        })
    }
    fn sort_fuzzy_matches(&self) -> bool {
        false
    }
}

pub fn render_picker(
    state: &mut PickerState,
    items: &[PickerEntry<'_>],
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> usize {
    if area.height < 2 || area.width == 0 {
        return 0;
    }
    paint_search(state, area, buf, cx);
    let list_area = Rect {
        x: area.x,
        y: area.y.saturating_add(1),
        width: area.width,
        height: area.height.saturating_sub(1),
    };
    render_picker_body(state, items, list_area, buf, cx)
}

/// Render the same rich picker list without an editor row (for host-owned query UIs).
pub fn render_picker_content(
    state: &mut PickerState,
    items: &[PickerEntry<'_>],
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> usize {
    render_picker_body(state, items, area, buf, cx)
}

fn render_picker_body(
    state: &mut PickerState,
    items: &[PickerEntry<'_>],
    list_area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> usize {
    state.prepare(items);
    state.list_area = list_area;
    let source = PickerSource {
        items,
        flat: &state.flat,
    };
    let page = render_list_source_ns(
        &mut state.list,
        &source,
        &state.search,
        NS_PICKER,
        list_area,
        buf,
        cx,
    );
    register_group_hits(&state.list, &state.flat, list_area, cx);
    register_row_expanders(&state.list, &state.flat, items, list_area, cx);
    state.normalize_selection();
    page
}

fn paint_search(state: &PickerState, area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) {
    let search = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    let input_style = cx.style(ThemeRole::Input, ThemeState::Focused);
    fill(buf, search, input_style);
    let base = input_style
        .patch(cx.style(ThemeRole::Fuzzy, ThemeState::Normal))
        .add_modifier(Modifier::BOLD);
    buf.set_string_safe(search.x, search.y, "/", base);
    let viewport = state
        .editor
        .viewport(search.width.saturating_sub(1) as usize);
    let shown = &state.query()[viewport.visible_byte_range];
    buf.set_string_safe(search.x.saturating_add(1), search.y, shown, base);
    let mut status_x = search
        .x
        .saturating_add(1)
        .saturating_add(shown.width().min(u16::MAX as usize) as u16);
    let mode = match state.search.mode() {
        SearchMode::Filter => " filter",
        SearchMode::Search => " search",
    };
    let kind = match state.search.kind() {
        SearchKind::Substring => " substring",
        SearchKind::Fuzzy => " fuzzy",
        SearchKind::Regex => " regex",
    };
    for text in [kind, mode] {
        let width = text.width().min(u16::MAX as usize) as u16;
        if status_x.saturating_add(width) >= search.right() {
            break;
        }
        buf.set_string_safe(
            status_x,
            search.y,
            text,
            input_style.patch(cx.style(ThemeRole::Muted, ThemeState::Normal)),
        );
        status_x = status_x.saturating_add(width);
    }
    if let Some(error) = state.search.error() {
        let prefix = "  regex: ";
        let available = search.right().saturating_sub(status_x) as usize;
        if available > prefix.width() {
            let error_style = input_style.patch(cx.style(ThemeRole::Error, ThemeState::Invalid));
            buf.set_string_safe(status_x, search.y, prefix, error_style);
            let error_x = status_x.saturating_add(prefix.width() as u16);
            buf.set_string_safe(
                error_x,
                search.y,
                gbtui_core::line_utils::truncate_str(
                    error,
                    search.right().saturating_sub(error_x) as usize,
                ),
                error_style,
            );
        }
    }
    if search.width > 1 {
        let col = 1u16.saturating_add(viewport.cursor_display_column.min(u16::MAX as usize) as u16);
        cx.set_cursor(
            search
                .x
                .saturating_add(col.min(search.width.saturating_sub(1))),
            search.y,
        );
    }
}

fn register_group_hits(list: &ListPaneState, flat: &[FlatRow], area: Rect, cx: &mut FrameCtx<'_>) {
    let Some(layout) = list.layout() else {
        return;
    };
    let end = list.offset.saturating_add(area.height as usize);
    for visible in 0..layout.item_count() {
        let Some(source) = list.source_index(visible) else {
            continue;
        };
        let Some(row) = flat.get(source) else {
            continue;
        };
        if !row.group {
            continue;
        }
        let top = layout.virtual_y(visible);
        let bottom = top.saturating_add(layout.item_height(visible));
        if bottom <= list.offset || top >= end {
            continue;
        }
        let y = area
            .y
            .saturating_add(top.saturating_sub(list.offset) as u16);
        let height = bottom.min(end).saturating_sub(top.max(list.offset)) as u16;
        cx.register(
            HitId::namespaced(NS_PICKER_GROUP, row.id as u32),
            Rect {
                x: area.x,
                y,
                width: area.width,
                height,
            },
        );
    }
}

fn register_row_expanders(
    list: &ListPaneState,
    flat: &[FlatRow],
    items: &[PickerEntry<'_>],
    area: Rect,
    cx: &mut FrameCtx<'_>,
) {
    let Some(layout) = list.layout() else {
        return;
    };
    let end = list.offset.saturating_add(area.height as usize);
    for visible in 0..layout.item_count() {
        let Some(source) = list.source_index(visible) else {
            continue;
        };
        let Some(row) = flat.get(source) else {
            continue;
        };
        let Some(entry) = items.get(row.source) else {
            continue;
        };
        if row.group || !entry.is_row_expandable() || entry.is_persistently_expanded() {
            continue;
        }
        let top = layout.virtual_y(visible);
        if top < list.offset || top >= end {
            continue;
        }
        let indent = row.indent.saturating_add(entry.row_indent());
        let x = area
            .x
            .saturating_add(2)
            .saturating_add(indent.saturating_mul(2));
        if x >= area.right() {
            continue;
        }
        cx.register(
            HitId::namespaced(NS_PICKER_EXPAND, row.id as u32),
            Rect {
                x,
                y: area
                    .y
                    .saturating_add(top.saturating_sub(list.offset) as u16),
                width: 2u16.min(area.right().saturating_sub(x)),
                height: 1,
            },
        );
    }
}

pub fn parse_picker_row(id: HitId) -> Option<usize> {
    parse_row_id_ns(NS_PICKER, id)
}
pub fn parse_picker_group(id: HitId) -> Option<usize> {
    (id.namespace() == NS_PICKER_GROUP).then_some(id.index() as usize)
}
pub fn parse_picker_expander(id: HitId) -> Option<usize> {
    (id.namespace() == NS_PICKER_EXPAND).then_some(id.index() as usize)
}
pub fn is_picker_scroll(id: HitId) -> bool {
    list::is_scroll_id(NS_PICKER, id)
}

fn inner_width(width: u16, indent: u16) -> u16 {
    width.saturating_sub(indent.saturating_mul(2)).max(1)
}
fn line_width(line: &Line<'_>) -> u16 {
    line.spans
        .iter()
        .map(|span| span.content.width())
        .sum::<usize>()
        .min(u16::MAX as usize) as u16
}

/// Reserve at least one cell for the label and keep right-aligned metadata
/// inside the row even when either component is longer than the viewport.
fn metadata_widths(width: u16, right: u16, badge: u16) -> (u16, u16, u16) {
    let available = width.saturating_sub(1);
    let right = right.min(available);
    let remaining = available.saturating_sub(right);
    let badge = badge.min(remaining.saturating_sub(if right > 0 { 1 } else { 0 }));
    let spacing = if right > 0 && badge > 0 { 1 } else { 0 };
    (right, badge, spacing)
}

fn line_height(line: &Line<'_>, width: u16) -> usize {
    let width = width.max(1) as usize;
    let mut height = 1usize;
    let mut used = 0usize;
    for span in &line.spans {
        for grapheme in span.content.graphemes(true) {
            if grapheme == "\n" {
                height += 1;
                used = 0;
                continue;
            }
            let next = grapheme.width();
            if used > 0 && used.saturating_add(next) > width {
                height += 1;
                used = 0;
            }
            used = used.saturating_add(next);
        }
    }
    height
}

fn paint_component(
    line: &Line<'_>,
    buf: &mut Buffer,
    body: Rect,
    start: usize,
    line_offset: usize,
    context: RenderContext<'_>,
    dimmed: bool,
) {
    let width = body.width;
    let height = line_height(line, width);
    if line_offset >= start.saturating_add(height)
        || line_offset.saturating_add(body.height as usize) <= start
    {
        return;
    }
    let local = line_offset.saturating_sub(start);
    let y = body
        .y
        .saturating_add(start.saturating_sub(line_offset) as u16);
    let visible = height
        .saturating_sub(local)
        .min(body.bottom().saturating_sub(y) as usize) as u16;
    draw_line_slice(
        line,
        buf,
        Rect {
            x: body.x,
            y,
            width,
            height: visible,
        },
        local,
        context,
        true,
        dimmed,
    );
}

fn draw_line_slice(
    line: &Line<'_>,
    buf: &mut Buffer,
    area: Rect,
    line_offset: usize,
    context: RenderContext<'_>,
    highlight: bool,
    dimmed: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let mut physical = 0usize;
    let mut x = area.x;
    let right = area.right();
    let mut global = 0usize;
    for span in &line.spans {
        for (offset, grapheme) in span.content.grapheme_indices(true) {
            if grapheme == "\n" {
                physical += 1;
                x = area.x;
                continue;
            }
            let width = grapheme.width() as u16;
            if x == area.x && width > area.width {
                continue;
            }
            if x > area.x && x.saturating_add(width) > right {
                physical += 1;
                x = area.x;
            }
            if physical >= line_offset
                && physical < line_offset.saturating_add(area.height as usize)
            {
                let hit = highlight
                    && context.match_ranges.iter().any(|range| {
                        range.start < global + offset + grapheme.len()
                            && global + offset < range.end
                    });
                let base = context.style.patch(line.style);
                let mut style = if hit {
                    context.match_style
                } else {
                    base.patch(span.style)
                };
                if dimmed {
                    style = style.add_modifier(Modifier::DIM);
                }
                buf.set_string_safe(
                    x,
                    area.y.saturating_add((physical - line_offset) as u16),
                    grapheme,
                    style,
                );
            }
            x = x.saturating_add(width);
        }
        global = global.saturating_add(span.content.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> [PickerEntry<'static>; 3] {
        [
            PickerEntry::group(PickerGroup::new(10, "Commands")),
            PickerEntry::row(
                PickerRow::new(41, "café")
                    .description("a deliberately long description that wraps"),
            ),
            PickerEntry::row(PickerRow::new(99, "compile").badge("build")),
        ]
    }

    #[test]
    fn collapsed_children_reappear_for_search_without_wrong_selection() {
        let mut state = PickerState::default();
        let entries = items();
        state.toggle_group(10);
        assert_eq!(state.selected_id(&entries), None);
        state.set_query("café");
        assert_eq!(state.selected_id(&entries), Some(41));
        state.set_query("missing");
        assert_eq!(state.selected_id(&entries), None);
    }

    #[test]
    fn invalid_regex_recovers_and_keeps_unicode_row_selectable() {
        let mut state = PickerState::default();
        let entries = items();
        state.set_search_kind(SearchKind::Regex);
        state.set_query("(");
        assert!(state.search_error().is_some());
        assert_eq!(state.selected_id(&entries), None);
        state.set_query("é");
        assert!(state.search_error().is_none());
        assert_eq!(state.selected_id(&entries), Some(41));
    }

    #[test]
    fn row_expansion_changes_visible_detail_height_and_keeps_later_rows_reachable() {
        use gbtui_core::{AnimationClock, Theme};
        use gbtui_runtime::HitMap;
        use std::time::Instant;

        let fields = [PickerField::new("field: additional detail")];
        let entries = [
            PickerEntry::row(
                PickerRow::new(41, "command")
                    .summary("summary")
                    .description("details that wrap across multiple terminal rows")
                    .fields(&fields),
            ),
            PickerEntry::row(PickerRow::new(99, "later")),
        ];
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut animation = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut animation, &mut hits);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 12, 2));
        let mut state = PickerState::default();

        render_picker_content(
            &mut state,
            &entries,
            Rect::new(0, 0, 12, 2),
            &mut buffer,
            &mut cx,
        );
        let collapsed: String = (0..12).map(|x| buffer[(x, 1)].symbol()).collect();
        assert!(collapsed.contains("summary"));
        assert!(!state.is_row_expanded(41));

        assert!(state.handle_hit(
            HitId::namespaced(NS_PICKER_EXPAND, 41),
            0,
            Rect::new(0, 0, 12, 2),
            &entries
        ));
        render_picker_content(
            &mut state,
            &entries,
            Rect::new(0, 0, 12, 2),
            &mut buffer,
            &mut cx,
        );
        let expanded: String = (0..12).map(|x| buffer[(x, 1)].symbol()).collect();
        assert!(expanded.contains("details"));
        assert!(state.is_row_expanded(41));
        assert!(state.list.total_height() > 2);
        state.handle_scroll(99, Rect::new(0, 0, 12, 2), &entries);
        render_picker_content(
            &mut state,
            &entries,
            Rect::new(0, 0, 12, 2),
            &mut buffer,
            &mut cx,
        );
        assert_eq!(
            hits.hit(1, 1),
            gbtui_runtime::HitResult::Hit(HitId::namespaced(NS_PICKER, 99))
        );
    }

    #[test]
    fn collapsed_nested_groups_hide_the_subtree_until_search_reveals_the_match_path() {
        let entries = [
            PickerEntry::group(PickerGroup::new(10, "root")),
            PickerEntry::group(PickerGroup::new(11, "nested").indent(1)),
            PickerEntry::row(PickerRow::new(41, "needle")),
        ];
        let mut state = PickerState::default();
        state.toggle_group(10);
        assert_eq!(state.visual_len(&entries), 1);
        state.set_query("nested");
        assert_eq!(state.visual_len(&entries), 2);
        assert_eq!(state.selected_id(&entries), None);
        state.set_query("needle");
        assert_eq!(state.visual_len(&entries), 3);
        assert_eq!(state.selected_id(&entries), Some(41));
    }

    #[test]
    fn always_expanded_rows_keep_details_visible_when_host_toggles_them() {
        let entries = [PickerEntry::row(
            PickerRow::new(41, "slash")
                .summary("summary")
                .description("description")
                .always_expanded(),
        )];
        let mut state = PickerState::default();
        assert_eq!(state.selected_id(&entries), Some(41));
        assert!(state.is_row_expanded(41));
        state.toggle_row(41);
        assert!(state.selected_id(&entries).is_some());
        assert!(state.is_row_expanded(41));
    }

    #[test]
    fn input_before_first_paint_keeps_a_selectable_identity() {
        use crossterm::event::{KeyEvent, KeyModifiers};

        let mut state = PickerState::default();
        let entries = items();
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &entries,
            4
        ));
        assert_eq!(state.selected_id(&entries), Some(99));
        state.set_query("missing");
        assert_eq!(state.selected_id(&entries), None);
    }

    #[test]
    fn inline_search_fields_filter_borrowed_host_text() {
        let source = String::from("source tag");
        let arguments = String::from("argument hint");
        let description = String::from("multiline description");
        let entries = [
            PickerEntry::row(
                PickerRow::new(41, "open")
                    .search_field(&source)
                    .search_field(&arguments)
                    .search_field(&description),
            ),
            PickerEntry::row(PickerRow::new(99, "close")),
        ];
        let mut state = PickerState::default();
        state.set_query("description");
        assert_eq!(state.selected_id(&entries), Some(41));
    }

    #[test]
    fn fuzzy_search_mode_preserves_picker_source_order() {
        let entries = [
            PickerEntry::row(PickerRow::new(7, "甲a😀b")),
            PickerEntry::row(PickerRow::new(8, "ab")),
        ];
        let mut state = PickerState::default();
        state.set_search_mode(SearchMode::Search);
        state.set_query("ab");
        state.prepare(&entries);
        assert_eq!(
            state.flat.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![7, 8]
        );
    }

    #[test]
    fn search_mode_n_and_navigate_matching_rows() {
        use crossterm::event::{KeyEvent, KeyModifiers};

        let entries = [
            PickerEntry::row(PickerRow::new(41, "alpha")),
            PickerEntry::row(PickerRow::new(7, "skip")),
            PickerEntry::row(PickerRow::new(99, "again")),
        ];
        let mut state = PickerState::default();
        state.set_search_mode(SearchMode::Search);
        state.set_query("a");
        assert_eq!(state.selected_id(&entries), Some(41));
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
            &entries,
            4
        ));
        assert_eq!(state.selected_id(&entries), Some(99));
        assert!(state.handle_key(
            &KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT),
            &entries,
            4
        ));
        assert_eq!(state.selected_id(&entries), Some(41));
    }
}
