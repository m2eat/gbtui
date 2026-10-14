//! Single- and multi-select choice list with rich options and an optional
//! free-form answer.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/views/question_view.rs (REWRITE)`
//!
//! # Keys
//!
//! In navigation mode, `↑`/`↓`/`j`/`k` move, `Space` toggles an option, and
//! `Enter` submits. On the free-form row, `Enter` explicitly starts editing.
//! While editing, every editor key—including `j`, `k`, space, and Unicode
//! text—belongs to the [`LineEditor`]; `Enter` commits the draft and returns to
//! navigation, while `Esc` discards it.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crossterm::event::KeyEvent;
use gbtui_core::renderable::RenderContext;
use gbtui_core::{HitId, Renderable, SafeBuf, key, line_utils};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::line_editor::{LineEditOutcome, LineEditor};
use crate::list::{ListItem, ListPaneState, parse_row_id_ns, render_list_items_ns};
use crate::search::SearchQuery;

pub const NS_CHOICE: u32 = 0x4348;
const FREEFORM_ITEM_ID: usize = 0;

/// Radio vs checkbox behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceMode {
    Single,
    Multi,
}

/// A host-supplied choice. `id` must be unique within one rendered list.
///
/// Text accepts borrowed or owned data. Descriptions and previews stay
/// collapsed until the option is focused, then use the list's physical-row
/// layout and scrolling.
#[derive(Debug, Clone)]
pub struct Choice<'a> {
    id: Cow<'a, str>,
    label: Cow<'a, str>,
    description: Option<Cow<'a, str>>,
    preview: Option<Cow<'a, str>>,
    revision: u64,
}

impl<'a> Choice<'a> {
    pub fn new(id: impl Into<Cow<'a, str>>, label: impl Into<Cow<'a, str>>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            preview: None,
            revision: 0,
        }
    }

    pub fn description(mut self, description: impl Into<Cow<'a, str>>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn preview(mut self, preview: impl Into<Cow<'a, str>>) -> Self {
        self.preview = Some(preview.into());
        self
    }

    /// Exact host-owned invalidation token for mutable borrowed content.
    pub fn revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

/// Single- / multi-select list with an optional free-form last row.
#[derive(Debug, Clone)]
pub struct ChoiceListState {
    pub list: ListPaneState,
    pub mode: ChoiceMode,
    /// Chosen host IDs, never source indexes. Reordering or filtering cannot
    /// silently turn a selection into a different option.
    pub picked: HashSet<String>,
    other: Option<String>,
    other_editor: LineEditor,
    editing_other: bool,
    item_ids: HashMap<String, usize>,
    next_item_id: usize,
}

impl ChoiceListState {
    pub fn single() -> Self {
        Self::new(ChoiceMode::Single)
    }

    pub fn multi() -> Self {
        Self::new(ChoiceMode::Multi)
    }

    fn new(mode: ChoiceMode) -> Self {
        Self {
            list: ListPaneState::default(),
            mode,
            picked: HashSet::new(),
            other: None,
            other_editor: LineEditor::default(),
            editing_other: false,
            item_ids: HashMap::new(),
            next_item_id: FREEFORM_ITEM_ID + 1,
        }
    }

    /// Add or replace the optional free-form answer. The value is only changed
    /// by an editor `Enter` commit, never by an abandoned draft.
    pub fn set_other(&mut self, other: Option<impl Into<String>>) {
        self.other = other.map(Into::into);
        self.other_editor
            .set_text(self.other.as_deref().unwrap_or_default());
        self.editing_other = false;
        self.list.invalidate_layout();
    }

    pub fn other(&self) -> Option<&str> {
        self.other.as_deref()
    }

    pub fn is_editing_other(&self) -> bool {
        self.editing_other
    }

    pub fn vis_len(&self, choices: &[Choice<'_>]) -> usize {
        choices.len() + usize::from(self.other.is_some())
    }

    pub fn is_picked(&self, choice: &Choice<'_>) -> bool {
        self.picked.contains(choice.id())
    }

    /// The selected option, if navigation is on an ordinary choice rather
    /// than the free-form row.
    pub fn selected_choice<'choices, 'content>(
        &self,
        choices: &'choices [Choice<'content>],
    ) -> Option<&'choices Choice<'content>> {
        self.selected_source(choices)
            .and_then(|source| choices.get(source))
    }

    pub fn handle_key(
        &mut self,
        key: &KeyEvent,
        choices: &[Choice<'_>],
        page: usize,
    ) -> ChoiceAction {
        if self.editing_other {
            if key!(Esc).matches(key) {
                self.editing_other = false;
                self.other_editor
                    .set_text(self.other.as_deref().unwrap_or_default());
                self.list.invalidate_layout();
                return ChoiceAction::OtherEditCancel;
            }
            if key!(Enter).matches(key) {
                let draft = self.other_editor.text().to_owned();
                let changed = self.other.as_deref() != Some(draft.as_str());
                self.other = Some(draft);
                self.editing_other = false;
                self.list.invalidate_layout();
                return if changed {
                    ChoiceAction::OtherEditCommit
                } else {
                    ChoiceAction::Changed
                };
            }
            return match self.other_editor.handle_key(key) {
                LineEditOutcome::Unhandled => ChoiceAction::Ignored,
                LineEditOutcome::HandledNoChange | LineEditOutcome::CursorChanged => {
                    ChoiceAction::Changed
                }
                LineEditOutcome::TextChanged => {
                    self.list.invalidate_layout();
                    ChoiceAction::Changed
                }
            };
        }

        let vis = self.vis_len(choices);
        if self.list.handle_key(key, vis, page) {
            if self.mode == ChoiceMode::Single
                && let Some(choice) = self.selected_choice(choices)
            {
                self.pick_single(choice.id());
            }
            return ChoiceAction::Changed;
        }
        if key!(Enter).matches(key) {
            if self.is_other_selected(choices) {
                self.other_editor
                    .set_text(self.other.as_deref().unwrap_or_default());
                self.editing_other = true;
                self.list.invalidate_layout();
                return ChoiceAction::OtherEditStart;
            }
            if let Some(choice_id) = self
                .selected_choice(choices)
                .map(|choice| choice.id().to_owned())
            {
                if self.mode == ChoiceMode::Single {
                    self.pick_single(&choice_id);
                }
                return ChoiceAction::Submit;
            }
        }
        if key!(' ').matches(key)
            && let Some(choice_id) = self
                .selected_choice(choices)
                .map(|choice| choice.id().to_owned())
        {
            self.toggle(&choice_id);
            return ChoiceAction::Changed;
        }
        ChoiceAction::Ignored
    }

    /// Select and toggle an ordinary hit row. Free-form hits only move
    /// navigation; editing still starts explicitly with `Enter`.
    pub fn click(&mut self, item_id: usize, choices: &[Choice<'_>], page: usize) -> ChoiceAction {
        if !self.list.select_item(item_id, page) {
            return ChoiceAction::Ignored;
        }
        if let Some(choice_id) = self
            .selected_choice(choices)
            .map(|choice| choice.id().to_owned())
        {
            self.toggle(&choice_id);
        }
        ChoiceAction::Changed
    }

    fn selected_source(&self, choices: &[Choice<'_>]) -> Option<usize> {
        if let Some(item_id) = self.list.selected_id() {
            return choices
                .iter()
                .position(|choice| self.item_ids.get(choice.id()).copied() == Some(item_id));
        }
        self.list
            .source_index(self.list.selected)
            .filter(|source| *source < choices.len())
            .or_else(|| (self.list.selected < choices.len()).then_some(self.list.selected))
    }

    fn is_other_selected(&self, choices: &[Choice<'_>]) -> bool {
        if self.other.is_none() {
            return false;
        }
        if let Some(item_id) = self.list.selected_id() {
            return item_id == FREEFORM_ITEM_ID;
        }
        self.list
            .source_index(self.list.selected)
            .map_or(self.list.selected >= choices.len(), |source| {
                source >= choices.len()
            })
    }

    fn pick_single(&mut self, id: &str) {
        self.picked.clear();
        self.picked.insert(id.to_owned());
    }

    fn toggle(&mut self, id: &str) {
        match self.mode {
            ChoiceMode::Single => self.pick_single(id),
            ChoiceMode::Multi => {
                if !self.picked.remove(id) {
                    self.picked.insert(id.to_owned());
                }
            }
        }
    }

    fn sync_item_ids(&mut self, choices: &[Choice<'_>]) {
        for choice in choices {
            if self.item_ids.contains_key(choice.id()) {
                continue;
            }
            let item_id = self.next_item_id;
            self.next_item_id = self
                .next_item_id
                .saturating_add(1)
                .max(FREEFORM_ITEM_ID + 1);
            self.item_ids.insert(choice.id().to_owned(), item_id);
        }
    }

    fn item_id(&self, choice: &Choice<'_>) -> usize {
        self.item_ids
            .get(choice.id())
            .copied()
            .expect("choice IDs are synchronized before use")
    }
}

/// Result of [`ChoiceListState::handle_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceAction {
    Ignored,
    Changed,
    /// Navigation `Enter` submitted the current ordinary choice.
    Submit,
    OtherEditStart,
    OtherEditCommit,
    OtherEditCancel,
}

struct ChoiceContent<'a> {
    item_id: usize,
    marker: &'static str,
    label: &'a str,
    description: Option<&'a str>,
    preview: Option<&'a str>,
    focused: bool,
    revision: u64,
}

impl ChoiceContent<'_> {
    fn wrapped_height(text: Option<&str>, width: u16) -> usize {
        text.map_or(0, |text| {
            gbtui_core::wrapping::wrap_text(text, width.max(1) as usize)
                .len()
                .max(1)
        })
    }

    fn paint(&self, area: Rect, buf: &mut Buffer, line_offset: usize, style: Style) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        if !self.focused {
            if line_offset == 0 {
                buf.set_string_safe(area.x, area.y, self.marker, style);
                let x = area.x.saturating_add(self.marker.width() as u16);
                buf.set_string_safe(
                    x,
                    area.y,
                    line_utils::truncate_str(self.label, area.right().saturating_sub(x) as usize),
                    style,
                );
            }
            return;
        }

        if line_offset == 0 {
            buf.set_string_safe(area.x, area.y, self.marker, style);
            let x = area.x.saturating_add(self.marker.width() as u16);
            buf.set_string_safe(
                x,
                area.y,
                line_utils::truncate_str(self.label, area.right().saturating_sub(x) as usize),
                style,
            );
        }
        let mut skipped = line_offset.saturating_sub(1);
        let mut y = area.y.saturating_add(u16::from(line_offset == 0));
        for text in [self.description, self.preview].into_iter().flatten() {
            if y >= area.bottom() {
                break;
            }
            let wrapped =
                gbtui_core::wrapping::wrap_text(text, area.width.saturating_sub(2) as usize);
            if skipped >= wrapped.len() {
                skipped -= wrapped.len();
                continue;
            }
            for line in wrapped.iter().skip(skipped) {
                if y >= area.bottom() {
                    return;
                }
                buf.set_string_safe(area.x, y, "  ", style);
                buf.set_string_safe(area.x.saturating_add(2), y, line, style);
                y += 1;
            }
            skipped = 0;
        }
    }
}

impl Renderable for ChoiceContent<'_> {
    fn render_slice(&self, area: Rect, buf: &mut Buffer, line_offset: usize) {
        self.paint(area, buf, line_offset, Style::default());
    }

    fn desired_height(&self, width: u16) -> usize {
        if self.focused {
            1 + Self::wrapped_height(self.description, width.saturating_sub(2))
                + Self::wrapped_height(self.preview, width.saturating_sub(2))
        } else {
            1
        }
    }

    fn render_slice_with_context(
        &self,
        area: Rect,
        buf: &mut Buffer,
        line_offset: usize,
        context: RenderContext<'_>,
    ) {
        self.paint(area, buf, line_offset, context.style);
    }
}

pub fn render_choices(
    state: &mut ChoiceListState,
    choices: &[Choice<'_>],
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> usize {
    state.sync_item_ids(choices);
    let focused_item_id = state
        .list
        .selected_id()
        .or_else(|| {
            state.selected_source(choices).and_then(|source| {
                choices
                    .get(source)
                    .map(|choice| state.item_id(choice))
                    .or_else(|| {
                        (source == choices.len() && state.other.is_some())
                            .then_some(FREEFORM_ITEM_ID)
                    })
            })
        })
        .or_else(|| choices.first().map(|choice| state.item_id(choice)));
    let other_text = state.other.as_deref().map(|other| {
        if state.editing_other {
            state.other_editor.text()
        } else {
            other
        }
    });
    let mut content = Vec::with_capacity(choices.len() + usize::from(other_text.is_some()));
    for choice in choices {
        let item_id = state.item_id(choice);
        let picked = state.is_picked(choice);
        let marker = match state.mode {
            ChoiceMode::Single if picked => "(•) ",
            ChoiceMode::Single => "( ) ",
            ChoiceMode::Multi if picked => "[x] ",
            ChoiceMode::Multi => "[ ] ",
        };
        let focused = focused_item_id == Some(item_id);
        content.push(ChoiceContent {
            item_id,
            marker,
            label: choice.label(),
            description: choice.description.as_deref(),
            preview: choice.preview.as_deref(),
            focused,
            revision: choice.revision ^ u64::from(picked) ^ (u64::from(focused) << 1),
        });
    }
    if let Some(other) = other_text {
        content.push(ChoiceContent {
            item_id: FREEFORM_ITEM_ID,
            marker: "[+] other: ",
            label: other,
            description: None,
            preview: None,
            focused: focused_item_id == Some(FREEFORM_ITEM_ID),
            revision: other.len() as u64 ^ (u64::from(state.editing_other) << 1),
        });
    }
    let items: Vec<_> = content
        .iter()
        .map(|content| {
            ListItem::new(content.item_id, content, content.label).revision(content.revision)
        })
        .collect();
    render_list_items_ns(
        &mut state.list,
        &items,
        &SearchQuery::new(),
        NS_CHOICE,
        area,
        buf,
        cx,
    )
}

/// Decode a choice row hit into the cached stable item ID accepted by
/// [`ChoiceListState::click`].
pub fn parse_choice_row(id: HitId) -> Option<usize> {
    parse_row_id_ns(NS_CHOICE, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::HitMap;
    use std::time::Instant;

    fn choices() -> [Choice<'static>; 2] {
        [Choice::new("one", "one"), Choice::new("two", "two")]
    }

    #[test]
    fn space_toggles_multi_by_stable_id() {
        let mut state = ChoiceListState::multi();
        let choices = choices();
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        state.handle_key(&key, &choices, 5);
        assert!(state.picked.contains("one"));
        state.handle_key(&key, &choices, 5);
        assert!(state.picked.is_empty());
    }

    #[test]
    fn single_submit_records_choice_id() {
        let mut state = ChoiceListState::single();
        let choices = choices();
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.handle_key(&key, &choices, 5), ChoiceAction::Submit);
        assert_eq!(state.picked, HashSet::from(["one".to_owned()]));
    }

    #[test]
    fn freeform_editor_keeps_navigation_keys_spaces_and_unicode_as_text() {
        let mut state = ChoiceListState::single();
        let choices = choices();
        state.set_other(Some(""));
        state.list.selected = choices.len();
        assert_eq!(
            state.handle_key(
                &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &choices,
                5
            ),
            ChoiceAction::OtherEditStart
        );
        for character in ['a', ' ', 'j', 'k', '界'] {
            assert_eq!(
                state.handle_key(
                    &KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
                    &choices,
                    5
                ),
                ChoiceAction::Changed
            );
        }
        assert_eq!(
            state.handle_key(
                &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &choices,
                5
            ),
            ChoiceAction::OtherEditCommit
        );
        assert_eq!(state.other(), Some("a jk界"));
    }

    #[test]
    fn freeform_cancel_restores_committed_value() {
        let mut state = ChoiceListState::single();
        let choices = choices();
        state.set_other(Some("kept"));
        state.list.selected = choices.len();
        let _ = state.handle_key(
            &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &choices,
            5,
        );
        let _ = state.handle_key(
            &KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE),
            &choices,
            5,
        );
        assert_eq!(
            state.handle_key(
                &KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                &choices,
                5
            ),
            ChoiceAction::OtherEditCancel
        );
        assert_eq!(state.other(), Some("kept"));
        assert!(!state.is_editing_other());
    }

    #[test]
    fn freeform_selection_is_preserved_through_first_rich_render() {
        let choices = choices();
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));
        let mut state = ChoiceListState::single();
        state.set_other(Some("note"));
        state.list.selected = choices.len();
        render_choices(
            &mut state,
            &choices,
            Rect::new(0, 0, 20, 3),
            &mut buf,
            &mut cx,
        );
        assert_eq!(state.list.selected_id(), Some(FREEFORM_ITEM_ID));
        match hits.hit(4, 2) {
            gbtui_runtime::HitResult::Hit(id) => {
                assert_eq!(parse_choice_row(id), Some(FREEFORM_ITEM_ID));
            }
            other => panic!("expected freeform hit, got {other:?}"),
        }
    }

    #[test]
    fn selected_id_does_not_change_when_choices_reorder() {
        let mut state = ChoiceListState::multi();
        state.toggle("two");
        let reordered = [Choice::new("two", "two"), Choice::new("one", "one")];
        assert!(state.is_picked(&reordered[0]));
        assert!(!state.is_picked(&reordered[1]));
        assert!(state.picked.contains("two"));
    }

    #[test]
    fn cached_selection_uses_stable_id_before_reordered_render() {
        let choices = choices();
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));
        let mut state = ChoiceListState::single();
        render_choices(
            &mut state,
            &choices,
            Rect::new(0, 0, 20, 3),
            &mut buf,
            &mut cx,
        );
        assert_eq!(
            state.handle_key(
                &KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                &choices,
                3,
            ),
            ChoiceAction::Changed
        );
        state.picked.clear();
        let reordered = [Choice::new("two", "two"), Choice::new("one", "one")];
        assert_eq!(
            state.handle_key(
                &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &reordered,
                3,
            ),
            ChoiceAction::Submit
        );
        assert_eq!(state.picked, HashSet::from(["two".to_owned()]));
    }

    #[test]
    fn rich_choice_paints_description_and_preview_below_label() {
        let choices = [Choice::new("rich", "Label")
            .description("Description")
            .preview("Preview")];
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let area = Rect::new(0, 0, 30, 4);
        let mut buf = Buffer::empty(area);
        render_choices(
            &mut ChoiceListState::single(),
            &choices,
            area,
            &mut buf,
            &mut cx,
        );
        let row = |y| (0..30).map(|x| buf[(x, y)].symbol()).collect::<String>();
        assert!(row(1).contains("Description"));
        assert!(row(2).contains("Preview"));
    }

    #[test]
    fn focused_rich_choice_wraps_and_scrolls_by_physical_rows() {
        let choices = [Choice::new("rich", "A label")
            .description("A long description wraps across a narrow physical viewport.")
            .preview("A preview also expands below the description.")];
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 18, 2));
        let mut state = ChoiceListState::single();
        render_choices(
            &mut state,
            &choices,
            Rect::new(0, 0, 18, 2),
            &mut buf,
            &mut cx,
        );
        assert!(
            state.list.total_height() > 2,
            "focused descriptions must expand"
        );
        match hits.hit(4, 1) {
            gbtui_runtime::HitResult::Hit(id) => assert!(parse_choice_row(id).is_some()),
            other => panic!("wrapped row must retain a physical hit: {other:?}"),
        }
        state.list.handle_scroll(1, state.vis_len(&choices), 1);
        assert!(state.list.offset > 0, "rich list scrolls physical rows");
    }
}
