//! Multiline prompt over [`TextArea`]. No slash / `@` parser or resource loading.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/views/prompt_widget/** (STRIP)`
//!
//! ```text
//!  ❯ type here, text wraps
//!    continuation of long input...
//! ```
//!
//! Focus stays the caller's job. An open completion or in-memory history addon
//! owns `Esc` / `Tab` / navigation before input can bubble to the caller.
//! Chrome (accent line, focus box) is optional via [`PromptStyle::chrome`].
//!
//! # Keys
//!
//! `Enter` submit (via [`Prompt::route_enter`]) · `Shift`/`Alt`+Enter newline ·
//! trailing `\` + Enter newline · `Ctrl-R` smart-case fuzzy history · `↑`/`↓`
//! browse history with draft restore · `Ctrl-C` clears if non-empty.

use std::collections::HashMap;
use std::ops::Range;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use gbtui_core::{HitId, SafeBuf, ThemeRole, ThemeState, is_inline_paste_key, is_paste_key, key};
use gbtui_runtime::input::is_mod_enter;
use gbtui_runtime::terminal::glyphs::{PROMPT_ARROW_WIDTH, accent_bar, prompt_arrow};
use gbtui_runtime::{FrameCtx, Pointer};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::StatefulWidgetRef;
use unicode_width::UnicodeWidthStr;

use crate::dropdown::{
    BODY as DROPDOWN_BODY, DropdownAction, DropdownItem, DropdownState, is_scrollbar, parse_row,
    popup_area, render_dropdown,
};
use crate::ghost::GhostText;
use crate::paint::fill;
use crate::search::{SearchKind, SearchQuery};
use crate::textarea::textarea::TextAreaSnapshot;
use crate::textarea::{
    ElementId, ElementKind, MouseAction, TextArea, TextAreaState, TextElementEventKind,
};

pub const BODY: HitId = HitId::named("gbtui.prompt.body");

/// Result of [`Prompt::handle_key`] and [`Prompt::handle_pointer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptEvent {
    /// The underlying document changed.
    Edited,
    /// An open completion was accepted; use [`Prompt::take_completion_acceptance`]
    /// if the host needs the accepted candidate metadata.
    CompletionAccepted,
    /// An open completion was dismissed without changing the document.
    CompletionClosed,
    /// A history entry became the current draft.
    HistoryRecalled,
    /// History navigation was cancelled and its original draft restored.
    HistoryRestored,
    /// A collapsed paste was expanded into its original text.
    PasteExpanded(ElementId),
    /// A live attachment was activated for host-owned preview or opening.
    AttachmentActivated(AttachmentId),
    /// UI state changed without changing the document.
    Handled,
    Ignored,
}

impl PromptEvent {
    /// Whether the prompt consumed the input or changed visible prompt UI.
    pub fn is_handled(&self) -> bool {
        !matches!(self, Self::Ignored)
    }

    /// Whether the prompt document changed.
    pub fn is_edited(&self) -> bool {
        matches!(
            self,
            Self::Edited
                | Self::CompletionAccepted
                | Self::HistoryRecalled
                | Self::HistoryRestored
                | Self::PasteExpanded(_)
        )
    }
}

/// Result of [`Prompt::route_enter`] for “bare Enter submits” contexts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnterOutcome {
    NewlineInserted,
    Submit,
    /// Prompt UI accepted completion/history instead of submitting.
    Handled,
    PassThrough,
}

/// Immutable request snapshot supplied to a completion host.
///
/// Return results with this exact snapshot through
/// [`Prompt::set_completions`]; stale drafts, cursors, and generations are
/// rejected before any completion UI changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRequest {
    pub draft: String,
    pub cursor: usize,
    pub generation: u64,
}

/// A host-provided completion candidate.
///
/// `replacement` indexes bytes in the associated [`CompletionRequest::draft`].
/// It must be a valid UTF-8 boundary range and cannot partially cross an
/// atomic textarea element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionCandidate {
    pub replacement: Range<usize>,
    pub insert_text: String,
    pub label: String,
    pub detail: Option<String>,
}

/// How [`Prompt::handle_paste`] treats long pasted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PastePolicy {
    /// Insert every pasted byte as ordinary editable text.
    #[default]
    Disabled,
    /// Insert a real atomic element when the input reaches `min_bytes`.
    ///
    /// Its backing text remains the original paste and can be restored by
    /// double-clicking the element.
    Collapse { min_bytes: usize },
}

/// Stable host-owned attachment identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttachmentId(pub String);

impl AttachmentId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Host-defined visual class for an attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentKind {
    File,
    Image,
    Custom(String),
}

/// Metadata for one host-owned attachment.
///
/// The prompt stores only this metadata. Resource loading, image decoding,
/// previewing, and transfer remain entirely the host's responsibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptAttachment {
    pub id: AttachmentId,
    pub kind: AttachmentKind,
    pub label: String,
    pub insert_text: String,
}

/// A currently live attachment with its textarea element identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveAttachment {
    pub element_id: ElementId,
    pub attachment: PromptAttachment,
}

/// Send-ready prompt data with current host attachment references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSubmission {
    /// Original buffer text, not chip labels.
    pub text: String,
    pub attachments: Vec<LiveAttachment>,
}

/// Visual flags. Focus (Esc/Tab) is still the caller’s job.
#[derive(Debug, Clone)]
pub struct PromptStyle {
    pub focused: bool,
    pub show_prefix: bool,
    pub chrome: bool,
    pub show_accent_line: bool,
    /// Grok rounded box (`╭─╮` / `│` / `╰─╯`). Independent of the accent rail.
    pub show_borders: bool,
    /// Right-aligned label in the bottom border (model · flags).
    pub caption: Option<&'static str>,
    pub placeholder: &'static str,
    pub placeholder_when_focused: bool,
    pub vpad_top: u16,
}

impl Default for PromptStyle {
    fn default() -> Self {
        Self {
            focused: true,
            show_prefix: true,
            chrome: true,
            show_accent_line: true,
            show_borders: false,
            caption: None,
            placeholder: "type something",
            placeholder_when_focused: false,
            vpad_top: 0,
        }
    }
}

const PASTE_ELEMENT_KIND: ElementKind = ElementKind(0x5041);
const ATTACHMENT_ELEMENT_KIND: ElementKind = ElementKind(0x4154);

#[derive(Debug)]
struct HistoryMatch {
    index: usize,
    score: i64,
    highlights: Vec<Range<usize>>,
}

/// Multiline composer over [`TextArea`](crate::TextArea).
pub struct Prompt {
    pub textarea: TextArea,
    textarea_state: TextAreaState,
    textarea_area: Rect,
    textarea_revision: u64,
    completion_generation: u64,
    completion_cursor: Option<usize>,
    completions: Vec<CompletionCandidate>,
    completion_dropdown: DropdownState,
    completion_popup: Option<Rect>,
    accepted_completion: Option<CompletionCandidate>,
    history: Vec<String>,
    history_draft: Option<TextAreaSnapshot>,
    history_index: Option<usize>,
    history_search_active: bool,
    history_search: String,
    history_matches: Vec<HistoryMatch>,
    history_query: SearchQuery,
    history_dropdown: DropdownState,
    history_popup: Option<Rect>,
    paste_policy: PastePolicy,
    attachments: HashMap<ElementId, PromptAttachment>,
}

impl Default for Prompt {
    fn default() -> Self {
        Self::new()
    }
}

impl Prompt {
    pub fn new() -> Self {
        let mut textarea = TextArea::new();
        textarea.show_scrollbar = false;
        let textarea_revision = textarea.document_revision();
        Self {
            textarea,
            textarea_state: TextAreaState::default(),
            textarea_area: Rect::default(),
            textarea_revision,
            completion_generation: 0,
            completion_cursor: None,
            completions: Vec::new(),
            completion_dropdown: DropdownState::default(),
            completion_popup: None,
            accepted_completion: None,
            history: Vec::new(),
            history_draft: None,
            history_index: None,
            history_search_active: false,
            history_search: String::new(),
            history_matches: Vec::new(),
            history_query: {
                let mut query = SearchQuery::new();
                query.set_kind(SearchKind::Fuzzy);
                query
            },
            history_dropdown: DropdownState::default(),
            history_popup: None,
            paste_policy: PastePolicy::default(),
            attachments: HashMap::new(),
        }
    }

    pub fn text(&self) -> &str {
        self.textarea.text()
    }

    pub fn set_text(&mut self, text: &str) {
        self.textarea.set_text(text);
        self.textarea.set_cursor(self.textarea.text().len());
        self.finish_document_edit();
    }

    /// Swap the composer clipboard. The default is
    /// [`crate::InternalClipboard`].
    pub fn set_clipboard_provider(
        &mut self,
        provider: Box<dyn crate::textarea::ClipboardProvider>,
    ) {
        self.textarea.set_clipboard_provider(provider);
    }

    pub fn cursor_at_end(&self) -> bool {
        self.textarea.cursor() >= self.textarea.text().len()
    }

    pub fn insert_str(&mut self, text: &str) {
        self.textarea.insert_str_replacing_selection(text);
        self.finish_document_edit();
    }

    pub fn is_empty(&self) -> bool {
        self.textarea.text().trim().is_empty() && !self.has_live_attachments()
    }

    pub fn can_send(&self) -> bool {
        !self.is_empty()
    }

    fn has_live_attachments(&self) -> bool {
        self.textarea.elements().iter().any(|element| {
            element.kind == ATTACHMENT_ELEMENT_KIND && self.attachments.contains_key(&element.id)
        })
    }

    /// Snapshot the current document for an asynchronous completion host.
    pub fn completion_request(&mut self) -> CompletionRequest {
        self.sync_document_revision();
        CompletionRequest {
            draft: self.textarea.text().to_owned(),
            cursor: self.textarea.cursor(),
            generation: self.completion_generation,
        }
    }

    /// Install completion results if they still describe the current draft.
    ///
    /// Returns `false` for stale requests or malformed replacement ranges.
    pub fn set_completions(
        &mut self,
        request: &CompletionRequest,
        candidates: Vec<CompletionCandidate>,
    ) -> bool {
        self.sync_document_revision();
        if request.generation != self.completion_generation
            || request.cursor != self.textarea.cursor()
            || request.draft != self.textarea.text()
            || candidates
                .iter()
                .any(|candidate| !self.valid_completion_candidate(candidate))
        {
            return false;
        }

        self.completions = candidates;
        self.completion_popup = None;
        self.completion_cursor = (!self.completions.is_empty()).then_some(request.cursor);
        if self.completions.is_empty() {
            self.completion_dropdown.close();
        } else {
            self.completion_dropdown.open_at(0);
        }
        true
    }

    /// Take metadata for the most recently accepted completion.
    pub fn take_completion_acceptance(&mut self) -> Option<CompletionCandidate> {
        self.accepted_completion.take()
    }

    pub fn set_paste_policy(&mut self, policy: PastePolicy) {
        self.paste_policy = policy;
    }

    /// Replace the in-memory history supplied by the host.
    pub fn set_history(&mut self, history: Vec<String>) {
        self.history = history;
        self.clear_history_mode();
    }

    /// Append one host-owned history entry. Prompt history is never persisted.
    pub fn record_history(&mut self, entry: impl Into<String>) {
        let entry = entry.into();
        if !entry.trim().is_empty() {
            self.history.push(entry);
        }
    }

    /// Insert a host-owned attachment as an atomic textarea element.
    ///
    /// An empty `insert_text` uses `label` (or `"attachment"` for an empty
    /// label) as its backing text, so the inserted metadata remains a live,
    /// clickable element and can be submitted on its own.
    pub fn insert_attachment(&mut self, mut attachment: PromptAttachment) -> ElementId {
        if attachment.insert_text.is_empty() {
            attachment.insert_text = if attachment.label.is_empty() {
                "attachment".into()
            } else {
                attachment.label.clone()
            };
        }
        let range = self
            .textarea
            .selection_range()
            .unwrap_or_else(|| self.textarea.cursor()..self.textarea.cursor());
        let display = Line::from(format!("[{}]", attachment.label));
        let id = self.textarea.replace_range_with_element_verbatim(
            range,
            &attachment.insert_text,
            ATTACHMENT_ELEMENT_KIND,
            Some(display),
        );
        self.attachments.insert(id, attachment);
        self.finish_document_edit();
        id
    }

    /// Read send-ready text and the attachment elements currently live in it.
    pub fn submission(&self) -> PromptSubmission {
        let attachments = self
            .textarea
            .elements()
            .iter()
            .filter(|element| element.kind == ATTACHMENT_ELEMENT_KIND)
            .filter_map(|element| {
                self.attachments
                    .get(&element.id)
                    .cloned()
                    .map(|attachment| LiveAttachment {
                        element_id: element.id,
                        attachment,
                    })
            })
            .collect();
        PromptSubmission {
            text: self.textarea.text().to_owned(),
            attachments,
        }
    }

    /// Return a send-ready submission unless a continuation newline was inserted.
    pub fn try_submission(&mut self) -> Option<PromptSubmission> {
        if self.apply_backslash_continuation() || self.is_empty() {
            return None;
        }
        Some(self.submission())
    }

    pub fn try_send(&mut self) -> Option<String> {
        self.try_submission().map(|submission| submission.text)
    }

    fn apply_backslash_continuation(&mut self) -> bool {
        let cursor = self.textarea.cursor();
        if cursor > 0 && self.textarea.text().as_bytes().get(cursor - 1) == Some(&b'\\') {
            self.textarea.delete_backward(1);
            self.textarea.insert_str("\n");
            self.finish_document_edit();
            true
        } else {
            false
        }
    }

    fn sync_document_revision(&mut self) -> bool {
        let revision = self.textarea.document_revision();
        let document_changed = revision != self.textarea_revision;
        if document_changed {
            self.textarea_revision = revision;
            self.completion_generation = self.completion_generation.saturating_add(1);
        }
        if document_changed
            || self
                .completion_cursor
                .is_some_and(|cursor| cursor != self.textarea.cursor())
        {
            self.close_completion();
        }
        document_changed
    }

    fn finish_document_edit(&mut self) {
        self.sync_document_revision();
        self.clear_history_mode();
    }

    fn close_completion(&mut self) {
        self.completions.clear();
        self.completion_dropdown.close();
        self.completion_popup = None;
        self.completion_cursor = None;
    }

    fn clear_history_mode(&mut self) {
        self.history_draft = None;
        self.history_index = None;
        self.history_search_active = false;
        self.history_search.clear();
        self.history_matches.clear();
        self.history_dropdown.close();
        self.history_popup = None;
    }

    fn valid_completion_candidate(&self, candidate: &CompletionCandidate) -> bool {
        let text = self.textarea.text();
        if candidate.replacement.start > candidate.replacement.end
            || candidate.replacement.end > text.len()
            || !text.is_char_boundary(candidate.replacement.start)
            || !text.is_char_boundary(candidate.replacement.end)
        {
            return false;
        }
        self.textarea.elements().iter().all(|element| {
            let range = &element.range;
            let start_inside = candidate.replacement.start > range.start
                && candidate.replacement.start < range.end;
            let end_inside =
                candidate.replacement.end > range.start && candidate.replacement.end < range.end;
            let overlaps =
                candidate.replacement.start < range.end && range.start < candidate.replacement.end;
            !start_inside
                && !end_inside
                && (!overlaps
                    || (candidate.replacement.start <= range.start
                        && candidate.replacement.end >= range.end))
        })
    }

    fn accept_completion(&mut self, index: usize) -> PromptEvent {
        self.sync_document_revision();
        if self.completion_cursor != Some(self.textarea.cursor()) || index >= self.completions.len()
        {
            self.close_completion();
            return PromptEvent::CompletionClosed;
        }
        let candidate = self.completions.swap_remove(index);
        if !self.valid_completion_candidate(&candidate) {
            self.close_completion();
            return PromptEvent::CompletionClosed;
        }
        self.textarea
            .replace_range(candidate.replacement.clone(), &candidate.insert_text);
        self.sync_document_revision();
        self.accepted_completion = Some(candidate);
        PromptEvent::CompletionAccepted
    }

    fn refresh_history_matches(&mut self) {
        self.history_query.set_query(&self.history_search);
        self.history_matches.clear();
        self.history_matches.reserve(self.history.len());
        for (index, entry) in self.history.iter().enumerate() {
            if let Some(matched) = self.history_query.find(entry) {
                self.history_matches.push(HistoryMatch {
                    index,
                    score: matched.score,
                    highlights: matched.ranges,
                });
            }
        }
        self.history_matches.sort_unstable_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| right.index.cmp(&left.index))
        });
        self.history_dropdown.open_at(0);
    }

    fn refresh_history_browse(&mut self) {
        self.history_matches.clear();
        self.history_matches
            .extend((0..self.history.len()).rev().map(|index| HistoryMatch {
                index,
                score: 0,
                highlights: Vec::new(),
            }));
        self.history_dropdown.open_at(0);
    }

    fn begin_history_search(&mut self) -> PromptEvent {
        if self.history.is_empty() {
            return PromptEvent::Ignored;
        }
        self.close_completion();
        if self.history_draft.is_none() {
            self.history_draft = Some(self.textarea.capture_snapshot());
        }
        self.history_index = None;
        self.history_search_active = true;
        self.history_search.clear();
        self.refresh_history_matches();
        PromptEvent::Handled
    }

    fn restore_history_draft(&mut self) -> PromptEvent {
        if let Some(draft) = self.history_draft.take() {
            self.textarea.restore_snapshot(&draft);
            self.sync_document_revision();
        }
        self.clear_history_mode();
        PromptEvent::HistoryRestored
    }

    fn load_history_entry(&mut self, index: usize) -> PromptEvent {
        let Some(entry) = self.history.get(index) else {
            return PromptEvent::Handled;
        };
        self.textarea.set_text(entry);
        self.textarea.set_cursor(self.textarea.text().len());
        self.textarea.clear_history();
        self.sync_document_revision();
        self.history_index = Some(index);
        if let Some(position) = self
            .history_matches
            .iter()
            .position(|candidate| candidate.index == index)
        {
            self.history_dropdown.selected = position;
        }
        PromptEvent::HistoryRecalled
    }

    fn accept_history_search(&mut self) -> PromptEvent {
        let Some(index) = self
            .history_matches
            .get(self.history_dropdown.selected)
            .map(|matched| matched.index)
        else {
            return self.restore_history_draft();
        };
        let event = self.load_history_entry(index);
        self.clear_history_mode();
        event
    }

    fn handle_history_navigation(&mut self, key: &KeyEvent) -> Option<PromptEvent> {
        if !key.modifiers.is_empty() || !matches!(key.code, KeyCode::Up | KeyCode::Down) {
            return None;
        }
        if self.history.is_empty() {
            return None;
        }

        let cursor = self.textarea.cursor();
        if self.history_index.is_none() {
            let before = &self.textarea.text()[..cursor];
            if key.code == KeyCode::Down || (key.code == KeyCode::Up && before.contains('\n')) {
                return None;
            }
            self.close_completion();
            self.history_draft = Some(self.textarea.capture_snapshot());
            self.history_search_active = false;
            self.history_search.clear();
            self.refresh_history_browse();
        }

        match key.code {
            KeyCode::Up => {
                let index = self
                    .history_index
                    .unwrap_or(self.history.len())
                    .saturating_sub(1);
                if self.history_index == Some(index) {
                    Some(PromptEvent::Handled)
                } else {
                    Some(self.load_history_entry(index))
                }
            }
            KeyCode::Down => {
                let index = self.history_index?;
                if index + 1 < self.history.len() {
                    Some(self.load_history_entry(index + 1))
                } else {
                    Some(self.restore_history_draft())
                }
            }
            _ => None,
        }
    }

    fn handle_history_search_key(&mut self, key: &KeyEvent) -> PromptEvent {
        if (key.code == KeyCode::Esc && key.modifiers.is_empty()) || key!('r', CONTROL).matches(key)
        {
            return self.restore_history_draft();
        }
        if matches!(key.code, KeyCode::Tab) && key.modifiers.is_empty() {
            return self.accept_history_search();
        }
        match key.code {
            KeyCode::Backspace if key.modifiers.is_empty() => {
                if let Some((offset, _)) = self.history_search.char_indices().next_back() {
                    self.history_search.truncate(offset);
                    self.refresh_history_matches();
                }
                return PromptEvent::Handled;
            }
            KeyCode::Char(c) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
                self.history_search.push(c);
                self.refresh_history_matches();
                return PromptEvent::Handled;
            }
            _ => {}
        }
        match self
            .history_dropdown
            .handle_key(key, self.history_matches.len())
        {
            DropdownAction::Confirmed(_) => return self.accept_history_search(),
            DropdownAction::Closed => return self.restore_history_draft(),
            DropdownAction::Changed => return PromptEvent::Handled,
            DropdownAction::Unhandled => {}
        }
        PromptEvent::Handled
    }

    fn handle_addon_key(&mut self, key: &KeyEvent) -> Option<PromptEvent> {
        self.sync_document_revision();
        if self.history_search_active {
            return Some(self.handle_history_search_key(key));
        }
        if key!('r', CONTROL).matches(key) {
            return Some(self.begin_history_search());
        }
        if self.completion_dropdown.open {
            if key.code == KeyCode::Tab && key.modifiers.is_empty() {
                return Some(self.accept_completion(self.completion_dropdown.selected));
            }
            if matches!(key.code, KeyCode::Char(_))
                && key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
            {
                return None;
            }
            return match self
                .completion_dropdown
                .handle_key(key, self.completions.len())
            {
                DropdownAction::Confirmed(index) => Some(self.accept_completion(index)),
                DropdownAction::Closed => Some(PromptEvent::CompletionClosed),
                DropdownAction::Changed => Some(PromptEvent::Handled),
                DropdownAction::Unhandled => None,
            };
        }
        if self.history_index.is_some() && key.code == KeyCode::Esc && key.modifiers.is_empty() {
            return Some(self.restore_history_draft());
        }
        self.handle_history_navigation(key)
    }

    /// Whether completion, history search, or history navigation owns keys.
    pub fn has_active_addon(&self) -> bool {
        self.completion_dropdown.open || self.history_search_active || self.history_index.is_some()
    }

    pub fn route_enter(&mut self, key: &KeyEvent) -> EnterOutcome {
        if let Some(event) = self.handle_addon_key(key)
            && event.is_handled()
        {
            return EnterOutcome::Handled;
        }
        if is_mod_enter(key) {
            self.textarea.insert_str_replacing_selection("\n");
            self.finish_document_edit();
            return EnterOutcome::NewlineInserted;
        }
        if key.code == KeyCode::Enter
            && key.modifiers.is_empty()
            && self.apply_backslash_continuation()
        {
            return EnterOutcome::NewlineInserted;
        }
        if key.code == KeyCode::Enter && key.modifiers.is_empty() {
            return EnterOutcome::Submit;
        }
        EnterOutcome::PassThrough
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> PromptEvent {
        if let Some(event) = self.handle_addon_key(key) {
            return event;
        }
        // History navigation has already loaded the selected draft. Leave bare
        // Enter for the host's `route_enter` send path rather than inserting a
        // newline when a caller pre-routes active addons through this method.
        if self.history_index.is_some() && key.code == KeyCode::Enter && key.modifiers.is_empty() {
            return PromptEvent::Ignored;
        }
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
            || (key.code == KeyCode::Esc && key.modifiers.is_empty())
        {
            self.textarea.clear_selection();
            return PromptEvent::Ignored;
        }
        if is_mod_enter(key) {
            self.textarea.insert_str_replacing_selection("\n");
            self.finish_document_edit();
            return PromptEvent::Edited;
        }
        if key!('c', CONTROL).matches(key) {
            return if self.textarea.text().is_empty() {
                PromptEvent::Ignored
            } else {
                self.textarea.set_text("");
                self.finish_document_edit();
                PromptEvent::Edited
            };
        }
        if is_paste_key(key) || is_inline_paste_key(key) {
            return PromptEvent::Ignored;
        }

        let cursor = self.textarea.cursor();
        self.textarea.input(*key);
        if self.sync_document_revision() {
            self.clear_history_mode();
            PromptEvent::Edited
        } else if self.textarea.cursor() != cursor {
            self.close_completion();
            PromptEvent::Handled
        } else {
            PromptEvent::Ignored
        }
    }

    pub fn handle_paste(&mut self, text: &str) -> PromptEvent {
        if text.is_empty() {
            return PromptEvent::Ignored;
        }
        let range = self
            .textarea
            .selection_range()
            .unwrap_or_else(|| self.textarea.cursor()..self.textarea.cursor());
        match self.paste_policy {
            PastePolicy::Disabled => self.textarea.replace_range(range, text),
            PastePolicy::Collapse { min_bytes } if text.len() >= min_bytes => {
                let lines = text.lines().count().max(1);
                let display = Line::from(format!("[pasted {lines} lines · {} bytes]", text.len()));
                self.textarea.replace_range_with_element_verbatim(
                    range,
                    text,
                    PASTE_ELEMENT_KIND,
                    Some(display),
                );
            }
            PastePolicy::Collapse { .. } => self.textarea.replace_range(range, text),
        }
        self.finish_document_edit();
        PromptEvent::Edited
    }

    pub fn handle_mouse(&mut self, event: MouseEvent) -> MouseAction {
        self.textarea
            .handle_mouse(event, self.textarea_area, self.textarea_state)
    }

    fn handle_element_event(&mut self) -> Option<PromptEvent> {
        let event = self.textarea.poll_element_event()?;
        let kind = self
            .textarea
            .elements()
            .iter()
            .find(|element| element.id == event.id)
            .map(|element| element.kind)?;
        match (kind, event.kind) {
            (PASTE_ELEMENT_KIND, TextElementEventKind::DoubleClick) => {
                self.textarea.inline_element(event.id).then(|| {
                    self.sync_document_revision();
                    PromptEvent::PasteExpanded(event.id)
                })
            }
            (
                ATTACHMENT_ELEMENT_KIND,
                TextElementEventKind::Click | TextElementEventKind::DoubleClick,
            ) => self
                .attachments
                .get(&event.id)
                .map(|attachment| PromptEvent::AttachmentActivated(attachment.id.clone())),
            _ => Some(PromptEvent::Handled),
        }
    }

    fn dispatch_pointer_mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) -> bool {
        self.handle_mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }) != MouseAction::Nothing
    }

    /// Place/drag/scroll the caret from a FrameCtx pointer (no `MouseEventKind` in the app).
    pub fn handle_pointer(&mut self, pointer: Pointer) -> PromptEvent {
        if matches!(pointer, Pointer::Cancel { .. }) {
            self.textarea.cancel_pointer_interaction();
            return PromptEvent::Handled;
        }
        let (column, row) = pointer.position();
        let mut handled = false;
        match pointer {
            Pointer::Move { .. } => {
                handled |= self.dispatch_pointer_mouse(MouseEventKind::Moved, column, row);
            }
            Pointer::Down {
                button, modifiers, ..
            } => {
                handled |= self.handle_mouse(MouseEvent {
                    kind: MouseEventKind::Down(button),
                    column,
                    row,
                    modifiers,
                }) != MouseAction::Nothing;
            }
            Pointer::Up { button, .. } => {
                handled |= self.dispatch_pointer_mouse(MouseEventKind::Up(button), column, row);
            }
            Pointer::Drag { button, .. } => {
                handled |= self.dispatch_pointer_mouse(MouseEventKind::Drag(button), column, row);
            }
            Pointer::Scroll { delta_lines, .. } if delta_lines < 0 => {
                for _ in 0..delta_lines.unsigned_abs() {
                    handled |= self.dispatch_pointer_mouse(MouseEventKind::ScrollUp, column, row);
                }
            }
            Pointer::Scroll { delta_lines, .. } => {
                for _ in 0..delta_lines.unsigned_abs() {
                    handled |= self.dispatch_pointer_mouse(MouseEventKind::ScrollDown, column, row);
                }
            }
            Pointer::Cancel { .. } => return PromptEvent::Handled,
        }
        self.handle_element_event().unwrap_or(if handled {
            PromptEvent::Handled
        } else {
            PromptEvent::Ignored
        })
    }

    fn popup_contains(popup: Rect, pointer: Pointer) -> bool {
        let (col, row) = pointer.position();
        col >= popup.x && col < popup.right() && row >= popup.y && row < popup.bottom()
    }

    fn handle_completion_hit(&mut self, id: HitId, pointer: Pointer) -> PromptEvent {
        if is_scrollbar(id)
            && self.completion_popup.is_some_and(|popup| {
                self.completion_dropdown.handle_scrollbar_pointer(
                    pointer,
                    popup,
                    self.completions.len(),
                )
            })
        {
            return PromptEvent::Handled;
        }
        let row = parse_row(id).filter(|&index| index < self.completions.len());
        match pointer {
            Pointer::Down {
                button: MouseButton::Left,
                ..
            } if row.is_some() => {
                let index = row.expect("row checked");
                self.completion_dropdown.selected = index;
                self.accept_completion(index)
            }
            Pointer::Move { .. } if row.is_some() => {
                self.completion_dropdown.selected = row.expect("row checked");
                PromptEvent::Handled
            }
            Pointer::Scroll { delta_lines, .. } => {
                self.completion_dropdown
                    .handle_scroll(delta_lines, self.completions.len());
                PromptEvent::Handled
            }
            Pointer::Down { .. } if id == DROPDOWN_BODY => {
                self.close_completion();
                PromptEvent::CompletionClosed
            }
            _ if id == DROPDOWN_BODY => PromptEvent::Handled,
            _ => PromptEvent::Handled,
        }
    }

    fn handle_history_hit(&mut self, id: HitId, pointer: Pointer) -> PromptEvent {
        if is_scrollbar(id)
            && self.history_popup.is_some_and(|popup| {
                self.history_dropdown.handle_scrollbar_pointer(
                    pointer,
                    popup,
                    self.history_matches.len(),
                )
            })
        {
            return PromptEvent::Handled;
        }
        let row = parse_row(id).filter(|&index| index < self.history_matches.len());
        match pointer {
            Pointer::Down {
                button: MouseButton::Left,
                ..
            } if row.is_some() => {
                self.history_dropdown.selected = row.expect("row checked");
                if self.history_search_active {
                    self.accept_history_search()
                } else {
                    let index = self.history_matches[self.history_dropdown.selected].index;
                    self.load_history_entry(index)
                }
            }
            Pointer::Move { .. } if row.is_some() => {
                self.history_dropdown.selected = row.expect("row checked");
                PromptEvent::Handled
            }
            Pointer::Scroll { delta_lines, .. } => {
                self.history_dropdown
                    .handle_scroll(delta_lines, self.history_matches.len());
                PromptEvent::Handled
            }
            Pointer::Down { .. } if id == DROPDOWN_BODY => self.restore_history_draft(),
            _ if id == DROPDOWN_BODY => PromptEvent::Handled,
            _ => PromptEvent::Handled,
        }
    }

    /// Route a FrameCtx hit. Only the prompt body and its cached addon popups
    /// are consumed; unrelated hit IDs return [`PromptEvent::Ignored`].
    pub fn handle_hit(&mut self, id: HitId, pointer: Pointer) -> PromptEvent {
        if matches!(pointer, Pointer::Cancel { .. }) {
            if is_scrollbar(id) {
                if let Some(popup) = self.completion_popup {
                    self.completion_dropdown.handle_scrollbar_pointer(
                        pointer,
                        popup,
                        self.completions.len(),
                    );
                }
                if let Some(popup) = self.history_popup {
                    self.history_dropdown.handle_scrollbar_pointer(
                        pointer,
                        popup,
                        self.history_matches.len(),
                    );
                }
            }
            self.textarea.cancel_pointer_interaction();
            return PromptEvent::Handled;
        }
        if let Some(popup) = self.completion_popup
            && (is_scrollbar(id)
                || (Self::popup_contains(popup, pointer)
                    && (id == DROPDOWN_BODY
                        || parse_row(id).is_some_and(|index| index < self.completions.len()))))
        {
            return self.handle_completion_hit(id, pointer);
        }
        if let Some(popup) = self.history_popup
            && (is_scrollbar(id)
                || (Self::popup_contains(popup, pointer)
                    && (id == DROPDOWN_BODY
                        || parse_row(id)
                            .is_some_and(|index| index < self.history_matches.len().max(1)))))
        {
            return self.handle_history_hit(id, pointer);
        }
        if id != BODY {
            return PromptEvent::Ignored;
        }
        let had_addon = self.has_active_addon();
        if self.completion_dropdown.open {
            self.close_completion();
        }
        if self.history_search_active || self.history_index.is_some() {
            let _ = self.restore_history_draft();
        }
        let event = self.handle_pointer(pointer);
        if event.is_handled() {
            event
        } else if had_addon {
            PromptEvent::Handled
        } else {
            PromptEvent::Ignored
        }
    }

    pub fn desired_height(&self, width: u16, style: &PromptStyle) -> u16 {
        let chrome = u16::from(style.chrome && style.show_accent_line);
        let inner = width.saturating_sub(
            chrome
                + if style.show_prefix {
                    PROMPT_ARROW_WIDTH
                } else {
                    0
                },
        );
        let w = inner.max(1) as usize;
        let text = self.textarea.text();
        let lines = if text.is_empty() {
            1
        } else {
            text.split('\n')
                .map(|line| {
                    let ww = line.width().max(1);
                    ww.div_ceil(w).max(1)
                })
                .sum::<usize>()
                .min(u16::MAX as usize) as u16
        };
        let top = if style.show_borders {
            1
        } else {
            style.vpad_top
        };
        let bot = u16::from(style.show_borders);
        lines.saturating_add(top).saturating_add(bot).max(1)
    }

    /// Paint. Registers [`BODY`](BODY). Does not match `MouseEventKind`.
    pub fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        cx: &mut FrameCtx<'_>,
        style: &PromptStyle,
    ) {
        self.render_with_ghost(area, buf, cx, style, None);
    }

    /// Same as [`render`](Self::render), with optional ghost remainder.
    pub fn render_with_ghost(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        cx: &mut FrameCtx<'_>,
        style: &PromptStyle,
        ghost: Option<&GhostText>,
    ) {
        self.sync_document_revision();
        if area.width == 0 || area.height == 0 {
            self.completion_popup = None;
            self.history_popup = None;
            return;
        }
        cx.register(BODY, area);
        let input_state = ThemeState::from_flags(false, false, false, style.focused, false);
        let bg = cx
            .style(ThemeRole::Surface, ThemeState::Normal)
            .patch(cx.style(ThemeRole::Input, input_state));
        fill(buf, area, bg);
        if let Some(theme) = cx.resolved_theme() {
            self.textarea.apply_resolved_theme(theme);
        } else {
            self.textarea.apply_theme(cx.theme);
        }

        let mut content = area;
        if style.chrome && style.show_borders && area.height >= 3 && area.width >= 4 {
            paint_prompt_box(area, style, bg, input_state, buf, cx);
            content.x = content.x.saturating_add(1);
            content.width = content.width.saturating_sub(2);
            content.y = content.y.saturating_add(1);
            content.height = content.height.saturating_sub(2);
        } else if style.chrome && style.show_accent_line && area.width > 1 {
            let accent_role = if style.focused {
                ThemeRole::Accent
            } else {
                ThemeRole::Muted
            };
            let accent = bg.patch(cx.style(accent_role, input_state));
            for y in area.y..area.bottom() {
                buf.set_string_safe(area.x, y, accent_bar(), accent);
            }
            content.x = content.x.saturating_add(1);
            content.width = content.width.saturating_sub(1);
            let mut text_area = content;
            if style.vpad_top > 0 && text_area.height > style.vpad_top {
                text_area.y = text_area.y.saturating_add(style.vpad_top);
                text_area.height = text_area.height.saturating_sub(style.vpad_top);
            }
            content = text_area;
        } else {
            let mut text_area = content;
            if style.vpad_top > 0 && text_area.height > style.vpad_top {
                text_area.y = text_area.y.saturating_add(style.vpad_top);
                text_area.height = text_area.height.saturating_sub(style.vpad_top);
            }
            content = text_area;
        }

        let text_area = content;

        let prefix_w = if style.show_prefix {
            PROMPT_ARROW_WIDTH
        } else {
            0
        };
        if style.show_prefix && text_area.width > prefix_w {
            let prefix_style = if style.show_borders {
                bg.patch(cx.style(ThemeRole::Muted, ThemeState::Normal))
            } else if style.focused {
                bg.patch(cx.style(ThemeRole::Accent, ThemeState::Focused))
                    .add_modifier(Modifier::BOLD)
            } else {
                bg.patch(cx.style(ThemeRole::Muted, ThemeState::Normal))
            };
            buf.set_string_safe(text_area.x, text_area.y, prompt_arrow(), prefix_style);
        }

        let ta = Rect {
            x: text_area.x + prefix_w,
            y: text_area.y,
            width: text_area.width.saturating_sub(prefix_w),
            height: text_area.height,
        };
        self.textarea_area = ta;
        if ta.width == 0 || ta.height == 0 {
            return;
        }

        let empty = self.textarea.text().is_empty();
        let caret = if style.focused {
            let pos = self.textarea.cursor_pos_with_state(ta, self.textarea_state);
            if empty {
                pos.or(Some((ta.x, ta.y)))
            } else {
                pos
            }
        } else {
            None
        };
        let ghost_rest = ghost
            .filter(|_| self.cursor_at_end() && style.focused)
            .and_then(|g| g.ghost_for(self.textarea.text()));
        let show_ph = empty
            && ghost_rest.is_none()
            && !style.placeholder.is_empty()
            && (style.placeholder_when_focused || !style.focused);
        if show_ph {
            buf.set_string_safe(
                ta.x,
                ta.y,
                gbtui_core::line_utils::truncate_str(style.placeholder, ta.width as usize),
                bg.patch(cx.style(ThemeRole::Muted, ThemeState::Normal)),
            );
        } else if !empty {
            StatefulWidgetRef::render_ref(&&self.textarea, ta, buf, &mut self.textarea_state);
        }
        if let Some(rest) = ghost_rest
            && let Some((x, y)) = caret
        {
            let budget = ta.right().saturating_sub(x) as usize;
            if budget > 0 {
                buf.set_string_safe(
                    x,
                    y,
                    gbtui_core::line_utils::truncate_str(rest, budget),
                    bg.patch(cx.style(ThemeRole::Muted, ThemeState::Normal))
                        .add_modifier(Modifier::ITALIC),
                );
            }
        }
        if let Some((x, y)) = caret {
            cx.set_cursor(x, y);
        }
        self.completion_popup = None;
        self.history_popup = None;
        if self.completion_dropdown.open && !self.completions.is_empty() {
            let popup = popup_area(
                ta,
                buf.area,
                self.completions.len(),
                &self.completion_dropdown,
            );
            self.completion_popup = Some(popup);
            let (completions, dropdown) = (&self.completions, &mut self.completion_dropdown);
            render_dropdown(
                dropdown,
                completions.iter().map(|candidate| {
                    let item = DropdownItem::new(candidate.label.as_str());
                    match candidate.detail.as_deref() {
                        Some(detail) => item.detail(detail),
                        None => item,
                    }
                }),
                popup,
                buf,
                cx,
            );
        } else if self.history_search_active || self.history_index.is_some() {
            let popup = popup_area(
                ta,
                buf.area,
                self.history_matches.len().max(1),
                &self.history_dropdown,
            );
            self.history_popup = Some(popup);
            if self.history_matches.is_empty() {
                render_dropdown(
                    &mut self.history_dropdown,
                    std::iter::once(
                        DropdownItem::new("no matching history").detail("Search history"),
                    ),
                    popup,
                    buf,
                    cx,
                );
            } else {
                let mode = if self.history_search_active {
                    "Search history"
                } else {
                    "Browse history"
                };
                let (history, matches, dropdown) = (
                    &self.history,
                    &self.history_matches,
                    &mut self.history_dropdown,
                );
                render_dropdown(
                    dropdown,
                    matches.iter().map(|matched| {
                        DropdownItem::new(history[matched.index].as_str())
                            .detail(mode)
                            .highlights(&matched.highlights)
                    }),
                    popup,
                    buf,
                    cx,
                );
            }
        }
    }
}

fn paint_prompt_box(
    area: Rect,
    style: &PromptStyle,
    base: Style,
    input_state: ThemeState,
    buf: &mut Buffer,
    cx: &FrameCtx<'_>,
) {
    let st = base.patch(cx.style(ThemeRole::Border, input_state));
    let left = area.x;
    let right = area.right().saturating_sub(1);
    let top = area.y;
    let bot = area.bottom().saturating_sub(1);
    for x in left..=right {
        let ch = if x == left {
            "╭"
        } else if x == right {
            "╮"
        } else {
            "─"
        };
        buf.set_string_safe(x, top, ch, st);
        let ch = if x == left {
            "╰"
        } else if x == right {
            "╯"
        } else {
            "─"
        };
        buf.set_string_safe(x, bot, ch, st);
    }
    for y in top.saturating_add(1)..bot {
        buf.set_string_safe(left, y, "│", st);
        buf.set_string_safe(right, y, "│", st);
    }
    if let Some(caption) = style.caption.filter(|s| !s.is_empty()) {
        let label = format!(" {caption} ");
        let max_w = area.width.saturating_sub(6) as usize;
        if max_w >= 6 {
            let trunc = gbtui_core::line_utils::truncate_str(&label, max_w);
            let w = trunc.width() as u16;
            let x = area.right().saturating_sub(3 + w);
            buf.set_string_safe(
                x,
                bot,
                &trunc,
                base.patch(cx.style(ThemeRole::Muted, ThemeState::Normal)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use gbtui_core::{AnimationClock, Theme, TickDemand};
    use gbtui_runtime::HitMap;
    use std::time::Instant;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn esc_is_ignored() {
        let mut p = Prompt::new();
        assert_eq!(
            p.handle_key(&key(KeyCode::Esc, KeyModifiers::NONE)),
            PromptEvent::Ignored
        );
    }

    #[test]
    fn typing_edits() {
        let mut p = Prompt::new();
        assert_eq!(
            p.handle_key(&key(KeyCode::Char('a'), KeyModifiers::NONE)),
            PromptEvent::Edited
        );
        assert_eq!(p.text(), "a");
    }

    #[test]
    fn bare_enter_submits() {
        let mut p = Prompt::new();
        p.set_text("hi");
        assert_eq!(
            p.route_enter(&key(KeyCode::Enter, KeyModifiers::NONE)),
            EnterOutcome::Submit
        );
    }

    #[test]
    fn shift_enter_inserts_newline() {
        let mut p = Prompt::new();
        p.set_text("hi");
        assert_eq!(
            p.route_enter(&key(KeyCode::Enter, KeyModifiers::SHIFT)),
            EnterOutcome::NewlineInserted
        );
        assert!(p.text().contains('\n'));
    }

    #[test]
    fn backslash_enter_continues() {
        let mut p = Prompt::new();
        p.set_text("hi\\");
        assert_eq!(
            p.route_enter(&key(KeyCode::Enter, KeyModifiers::NONE)),
            EnterOutcome::NewlineInserted
        );
        assert_eq!(p.text(), "hi\n");
    }

    #[test]
    fn ctrl_c_clears() {
        let mut p = Prompt::new();
        p.set_text("draft");
        assert_eq!(
            p.handle_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            PromptEvent::Edited
        );
        assert!(p.text().is_empty());
    }

    #[test]
    fn render_registers_body_and_idle_is_none() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
        let mut p = Prompt::new();
        {
            let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
            p.render(
                Rect::new(0, 0, 40, 4),
                &mut buf,
                &mut cx,
                &PromptStyle::default(),
            );
            assert!(cx.cursor().is_some(), "focused prompt must place a caret");
        }
        assert_eq!(hits.hit(2, 1), gbtui_runtime::HitResult::Hit(BODY));
        assert_eq!(anim.demand(), TickDemand::None);
    }

    #[test]
    fn unfocused_does_not_set_cursor() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
        let mut p = Prompt::new();
        p.set_text("hi");
        let style = PromptStyle {
            focused: false,
            ..PromptStyle::default()
        };
        p.render(Rect::new(0, 0, 40, 4), &mut buf, &mut cx, &style);
        assert!(cx.cursor().is_none());
    }

    #[test]
    fn ghost_starts_at_caret_not_after_it() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
        let mut p = Prompt::new();
        let mut ghost = crate::GhostText::new();
        ghost.set("hello");
        p.render_with_ghost(
            Rect::new(0, 0, 40, 4),
            &mut buf,
            &mut cx,
            &PromptStyle::default(),
            Some(&ghost),
        );
        let pos = cx.cursor().expect("caret");
        assert_eq!(
            buf[(pos.x, pos.y)].symbol(),
            "h",
            "caret sits on the first ghost cell, not past the remainder"
        );
    }

    #[test]
    fn grok_box_paints_rounded_corners_and_caption() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
        let mut p = Prompt::new();
        let style = PromptStyle {
            show_accent_line: false,
            show_borders: true,
            caption: Some("Grok 4.6 · always-approve"),
            vpad_top: 1,
            ..PromptStyle::default()
        };
        p.render(Rect::new(0, 0, 40, 4), &mut buf, &mut cx, &style);
        assert_eq!(buf[(0, 0)].symbol(), "╭");
        assert_eq!(buf[(39, 0)].symbol(), "╮");
        assert_eq!(buf[(0, 3)].symbol(), "╰");
        assert_eq!(buf[(39, 3)].symbol(), "╯");
        let bottom: String = (0..40).map(|x| buf[(x, 3)].symbol().to_string()).collect();
        assert!(bottom.contains("Grok 4.6"), "{bottom}");
    }

    fn candidate(replacement: Range<usize>, insert_text: &str) -> CompletionCandidate {
        CompletionCandidate {
            replacement,
            insert_text: insert_text.into(),
            label: insert_text.into(),
            detail: None,
        }
    }

    #[test]
    fn stale_completion_result_is_rejected() {
        let mut p = Prompt::new();
        p.set_text("hel");
        let request = p.completion_request();
        p.insert_str("p");

        assert!(!p.set_completions(&request, vec![candidate(0..3, "hello")]));
        assert!(!p.has_active_addon());
    }

    #[test]
    fn completion_replaces_unicode_byte_range_and_enter_does_not_submit() {
        let mut p = Prompt::new();
        p.set_text("hé");
        let request = p.completion_request();

        assert!(!p.set_completions(&request, vec![candidate(1..2, "E")]));
        assert!(p.set_completions(&request, vec![candidate(1..3, "E")]));
        assert_eq!(
            p.route_enter(&key(KeyCode::Enter, KeyModifiers::NONE)),
            EnterOutcome::Handled
        );
        assert_eq!(p.text(), "hE");
        assert_eq!(
            p.take_completion_acceptance()
                .expect("accepted")
                .insert_text,
            "E"
        );
    }

    #[test]
    fn completion_navigation_tab_and_enter_are_handled_by_handle_key() {
        let mut p = Prompt::new();
        p.set_text("x");
        let request = p.completion_request();
        assert!(p.set_completions(
            &request,
            vec![candidate(0..1, "first"), candidate(0..1, "second")]
        ));
        assert_eq!(
            p.handle_key(&key(KeyCode::Down, KeyModifiers::NONE)),
            PromptEvent::Handled
        );
        assert_eq!(
            p.handle_key(&key(KeyCode::Up, KeyModifiers::NONE)),
            PromptEvent::Handled
        );
        assert_eq!(
            p.handle_key(&key(KeyCode::Tab, KeyModifiers::NONE)),
            PromptEvent::CompletionAccepted
        );
        assert_eq!(p.text(), "first");

        p.set_text("x");
        let request = p.completion_request();
        assert!(p.set_completions(&request, vec![candidate(0..1, "entered")]));
        assert_eq!(
            p.handle_key(&key(KeyCode::Enter, KeyModifiers::NONE)),
            PromptEvent::CompletionAccepted
        );
        assert_eq!(p.text(), "entered");
    }

    #[test]
    fn completion_cannot_replace_part_of_an_atomic_element() {
        let mut p = Prompt::new();
        p.insert_attachment(PromptAttachment {
            id: AttachmentId::new("atomic"),
            kind: AttachmentKind::File,
            label: "atomic".into(),
            insert_text: "token".into(),
        });
        let request = p.completion_request();

        assert!(!p.set_completions(&request, vec![candidate(1..3, "x")]));
    }

    #[test]
    fn completion_escape_closes_without_editing() {
        let mut p = Prompt::new();
        p.set_text("hel");
        let request = p.completion_request();
        assert!(p.set_completions(&request, vec![candidate(0..3, "hello")]));

        assert_eq!(
            p.handle_key(&key(KeyCode::Esc, KeyModifiers::NONE)),
            PromptEvent::CompletionClosed
        );
        assert_eq!(p.text(), "hel");
        assert!(!p.has_active_addon());
    }

    #[test]
    fn empty_history_does_not_open_an_invisible_addon() {
        let mut p = Prompt::new();
        assert_eq!(
            p.handle_key(&key(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            PromptEvent::Ignored
        );
        assert!(!p.has_active_addon());
    }

    #[test]
    fn direct_textarea_edits_stale_completions_and_remove_nonlive_attachments() {
        let mut p = Prompt::new();
        p.set_text("hel");
        let request = p.completion_request();
        assert!(p.set_completions(&request, vec![candidate(0..3, "hello")]));
        p.textarea.set_cursor(0);
        let _ = p.completion_request();
        assert!(!p.has_active_addon());

        let end = p.text().len();
        p.textarea.set_cursor(end);
        let request = p.completion_request();
        p.textarea.insert_str("p");
        assert!(!p.set_completions(&request, vec![candidate(0..3, "hello")]));

        p.insert_attachment(PromptAttachment {
            id: AttachmentId::new("temporary"),
            kind: AttachmentKind::File,
            label: "temporary.txt".into(),
            insert_text: "temporary.txt".into(),
        });
        p.textarea.set_text("");
        assert!(p.submission().attachments.is_empty());
    }

    #[test]
    fn completion_dropdown_hit_routes_through_prompt() {
        let mut p = Prompt::new();
        p.set_text("hel");
        let request = p.completion_request();
        assert!(p.set_completions(&request, vec![candidate(0..3, "hello")]));

        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 8));
        {
            let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
            p.render(
                Rect::new(0, 4, 40, 2),
                &mut buf,
                &mut cx,
                &PromptStyle::default(),
            );
        }
        let popup = p.completion_popup.expect("completion popup");
        let id = match hits.hit(popup.x, popup.y) {
            gbtui_runtime::HitResult::Hit(id) => id,
            other => panic!("expected completion row hit, got {other:?}"),
        };
        assert_eq!(
            p.handle_hit(
                id,
                Pointer::Down {
                    button: MouseButton::Left,
                    col: popup.x,
                    row: popup.y,
                    clicks: 1,
                    modifiers: KeyModifiers::NONE,
                },
            ),
            PromptEvent::CompletionAccepted
        );
        assert_eq!(p.text(), "hello");
    }

    #[test]
    fn history_browse_previews_and_restores_the_exact_attachment_draft() {
        let mut p = Prompt::new();
        p.set_text("draft ");
        p.insert_attachment(PromptAttachment {
            id: AttachmentId::new("draft-attachment"),
            kind: AttachmentKind::File,
            label: "draft.txt".into(),
            insert_text: "draft.txt".into(),
        });
        let draft = p.text().to_owned();
        p.record_history("older");
        p.record_history("newer");

        assert_eq!(
            p.handle_key(&key(KeyCode::Up, KeyModifiers::NONE)),
            PromptEvent::HistoryRecalled
        );
        assert_eq!(p.text(), "newer");
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut buf = Buffer::empty(Rect::new(0, 0, 48, 12));
        {
            let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
            p.render(
                Rect::new(0, 4, 48, 2),
                &mut buf,
                &mut cx,
                &PromptStyle::default(),
            );
        }
        let popup = p.history_popup.expect("visible browse popup");
        let row: String = (popup.x..popup.right())
            .map(|x| buf[(x, popup.y)].symbol().to_owned())
            .collect();
        assert!(row.contains("Browse history"), "{row}");

        assert_eq!(
            p.handle_key(&key(KeyCode::Down, KeyModifiers::NONE)),
            PromptEvent::HistoryRestored
        );
        assert_eq!(p.text(), draft);
        assert_eq!(
            p.submission().attachments[0].attachment.id.0,
            "draft-attachment"
        );

        p.handle_key(&key(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(
            p.handle_key(&key(KeyCode::Char('!'), KeyModifiers::NONE)),
            PromptEvent::Edited
        );
        assert!(!p.has_active_addon());
    }

    #[test]
    fn history_search_treats_j_and_k_as_query_text() {
        let mut p = Prompt::new();
        p.set_history(vec!["joker".into(), "alpha".into()]);
        p.handle_key(&key(KeyCode::Char('r'), KeyModifiers::CONTROL));
        for ch in ['j', 'k'] {
            p.handle_key(&key(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        p.handle_key(&key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(p.text(), "joker");
    }

    #[test]
    fn completion_does_not_steal_printable_navigation_letters() {
        let mut p = Prompt::new();
        p.set_text("h");
        let request = p.completion_request();
        p.set_completions(&request, vec![candidate(0..1, "hello")]);
        p.handle_key(&key(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(p.text(), "hj");
        assert!(!p.has_active_addon());
    }

    #[test]
    fn history_search_can_replace_an_open_completion() {
        let mut p = Prompt::new();
        p.set_history(vec!["cargo test".into()]);
        p.set_text("h");
        let request = p.completion_request();
        p.set_completions(&request, vec![candidate(0..1, "hello")]);
        p.handle_key(&key(KeyCode::Char('r'), KeyModifiers::CONTROL));
        p.handle_key(&key(KeyCode::Char('c'), KeyModifiers::NONE));
        p.handle_key(&key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(p.text(), "cargo test");
    }

    #[test]
    fn ctrl_r_uses_smart_case_fuzzy_ranking_and_grapheme_safe_highlights() {
        let mut p = Prompt::new();
        p.set_history(vec!["a---b".into(), "ab".into()]);
        p.set_text("draft");

        assert_eq!(
            p.handle_key(&key(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            PromptEvent::Handled
        );
        p.handle_key(&key(KeyCode::Char('a'), KeyModifiers::NONE));
        p.handle_key(&key(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(
            p.history_matches.first().map(|matched| matched.index),
            Some(1)
        );
        assert_eq!(p.history_matches[0].highlights, vec![0..2]);

        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut buf = Buffer::empty(Rect::new(0, 0, 48, 12));
        {
            let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
            p.render(
                Rect::new(0, 4, 48, 2),
                &mut buf,
                &mut cx,
                &PromptStyle::default(),
            );
        }
        let popup = p.history_popup.expect("search popup");
        assert_eq!(
            buf[(popup.x + 2, popup.y)].style().fg,
            Some(theme.fuzzy_accent)
        );

        assert_eq!(
            p.handle_key(&key(KeyCode::Enter, KeyModifiers::NONE)),
            PromptEvent::HistoryRecalled
        );
        assert_eq!(p.text(), "ab");
        p.set_history(vec!["A\u{301}B".into(), "ab".into()]);
        p.set_text("draft");
        p.handle_key(&key(KeyCode::Char('r'), KeyModifiers::CONTROL));
        p.handle_key(&key(KeyCode::Char('A'), KeyModifiers::SHIFT));
        p.handle_key(&key(KeyCode::Char('B'), KeyModifiers::SHIFT));
        assert_eq!(
            p.history_matches
                .iter()
                .map(|matched| matched.index)
                .collect::<Vec<_>>(),
            vec![0]
        );
        assert_eq!(p.history_matches[0].highlights, vec![0..4]);
    }

    #[test]
    fn history_navigation_leaves_bare_enter_for_send_router() {
        let mut p = Prompt::new();
        p.record_history("saved");
        p.set_text("draft");
        assert_eq!(
            p.handle_key(&key(KeyCode::Up, KeyModifiers::NONE)),
            PromptEvent::HistoryRecalled
        );

        assert_eq!(
            p.handle_key(&key(KeyCode::Enter, KeyModifiers::NONE)),
            PromptEvent::Ignored
        );
        assert_eq!(p.text(), "saved");
        assert_eq!(
            p.route_enter(&key(KeyCode::Enter, KeyModifiers::NONE)),
            EnterOutcome::Submit
        );
    }

    #[test]
    fn history_search_escape_restores_original_draft() {
        let mut p = Prompt::new();
        p.set_history(vec!["needle".into()]);
        p.set_text("draft");

        assert_eq!(
            p.handle_key(&key(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            PromptEvent::Handled
        );
        assert!(p.has_active_addon());
        assert_eq!(
            p.handle_key(&key(KeyCode::Char('n'), KeyModifiers::NONE)),
            PromptEvent::Handled
        );
        assert_eq!(
            p.handle_key(&key(KeyCode::Esc, KeyModifiers::NONE)),
            PromptEvent::HistoryRestored
        );
        assert_eq!(p.text(), "draft");
        assert!(!p.has_active_addon());
    }

    #[test]
    fn collapsed_paste_expands_on_double_click_and_undo_restores_element() {
        let mut p = Prompt::new();
        p.set_paste_policy(PastePolicy::Collapse { min_bytes: 4 });
        let pasted = "long\t\npaste";
        p.handle_paste(pasted);
        let id = p.textarea.elements()[0].id;

        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        p.render(
            Rect::new(0, 0, 40, 4),
            &mut buf,
            &mut cx,
            &PromptStyle::default(),
        );
        let (col, row) = p
            .textarea
            .screen_position_of(0, p.textarea_area, p.textarea_state)
            .expect("element is visible");
        let down = |clicks| Pointer::Down {
            button: MouseButton::Left,
            col,
            row,
            clicks,
            modifiers: KeyModifiers::NONE,
        };
        assert!(p.handle_pointer(down(1)).is_handled());
        assert_eq!(p.handle_pointer(down(2)), PromptEvent::PasteExpanded(id));
        assert_eq!(p.text(), pasted);
        assert!(p.textarea.elements().is_empty());

        assert_eq!(
            p.handle_key(&key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            PromptEvent::Edited
        );
        assert_eq!(p.textarea.elements()[0].id, id);
    }

    #[test]
    fn attachment_submission_tracks_delete_and_undo() {
        let mut p = Prompt::new();
        let attachment = PromptAttachment {
            id: AttachmentId::new("example"),
            kind: AttachmentKind::File,
            label: "example.txt".into(),
            insert_text: String::new(),
        };
        let id = p.insert_attachment(attachment);
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
        {
            let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
            p.render(
                Rect::new(0, 0, 40, 4),
                &mut buf,
                &mut cx,
                &PromptStyle::default(),
            );
        }
        let (col, row) = p
            .textarea
            .screen_position_of(0, p.textarea_area, p.textarea_state)
            .expect("attachment is visible");
        assert_eq!(
            p.handle_pointer(Pointer::Down {
                button: MouseButton::Left,
                col,
                row,
                clicks: 1,
                modifiers: KeyModifiers::NONE,
            }),
            PromptEvent::AttachmentActivated(AttachmentId::new("example"))
        );

        let submission = p.try_submission().expect("attachment-only submission");
        assert_eq!(submission.text, "example.txt");
        assert_eq!(
            submission.attachments[0].attachment.insert_text,
            "example.txt"
        );
        assert_eq!(p.submission().attachments[0].element_id, id);

        assert_eq!(
            p.handle_key(&key(KeyCode::Delete, KeyModifiers::NONE)),
            PromptEvent::Edited
        );
        assert!(p.submission().attachments.is_empty());
        assert_eq!(
            p.handle_key(&key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            PromptEvent::Edited
        );
        assert_eq!(p.submission().attachments[0].element_id, id);
    }
}
