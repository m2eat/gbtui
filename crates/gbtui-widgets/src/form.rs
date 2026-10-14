//! Settings-style form rows. The application owns values; this module paints
//! and reports actions. Select opens the existing [`crate::DropdownState`].
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/views/settings_modal (STRIP)` —
//! row kinds only. No `SettingKey`, disk, or consent.
//!
//! ```text
//! ── Editor ──
//!  vim keys          on
//!  theme             Night ▾
//!  indent            4
//!  name              gbtui
//! ```
//!
//! # Keys
//!
//! Enter starts Field edit · Esc cancels edit · Select emits `OpenSelect`.
//! Expanded multi-selects use `↑`/`↓`/`j`/`k` and Space/Enter to toggle;
//! `Esc` returns to the field list. [`render_form_with_actions`] keeps its
//! host-owned action rows pinned at the bottom of the supplied area.

use crate::button::Button;
use crate::line_editor::{LineEditOutcome, LineEditor};
use crate::list::{self, ListPaneState};
use crate::paint::fill;
use crossterm::event::{KeyCode, KeyEvent};
use gbtui_core::{HitId, SafeBuf, ThemeRole, ThemeState, key, line_utils};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

pub const NS_FORM: u32 = 0x4652;
pub const NS_FORM_ACTION: u32 = 0x4653;
pub const NS_FORM_OPTION: u32 = 0x4654;

/// One form row. `id` is unused for headers. Errors are host-owned display
/// text and do not perform validation in this module.
#[derive(Debug, Clone, Copy)]
pub struct FormRow<'a> {
    pub id: u32,
    pub kind: FormKind<'a>,
    pub error: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub enum FormKind<'a> {
    Header(&'a str),
    Toggle {
        label: &'a str,
        on: bool,
    },
    Select {
        label: &'a str,
        value: &'a str,
    },
    Stepper {
        label: &'a str,
        value: i64,
        min: i64,
        max: i64,
    },
    Field {
        label: &'a str,
        value: &'a str,
    },
    /// A host-owned multi-select. Options are expanded only while this row is
    /// active, so ordinary forms retain their compact layout.
    MultiSelect {
        label: &'a str,
        options: &'a [&'a str],
        selected: &'a [bool],
    },
}

impl<'a> FormRow<'a> {
    pub fn header(label: &'a str) -> Self {
        Self {
            id: 0,
            kind: FormKind::Header(label),
            error: None,
        }
    }

    pub fn toggle(id: u32, label: &'a str, on: bool) -> Self {
        Self {
            id,
            kind: FormKind::Toggle { label, on },
            error: None,
        }
    }

    pub fn select(id: u32, label: &'a str, value: &'a str) -> Self {
        Self {
            id,
            kind: FormKind::Select { label, value },
            error: None,
        }
    }

    pub fn stepper(id: u32, label: &'a str, value: i64, min: i64, max: i64) -> Self {
        Self {
            id,
            kind: FormKind::Stepper {
                label,
                value,
                min,
                max,
            },
            error: None,
        }
    }

    pub fn field(id: u32, label: &'a str, value: &'a str) -> Self {
        Self {
            id,
            kind: FormKind::Field { label, value },
            error: None,
        }
    }

    pub fn multi_select(
        id: u32,
        label: &'a str,
        options: &'a [&'a str],
        selected: &'a [bool],
    ) -> Self {
        Self {
            id,
            kind: FormKind::MultiSelect {
                label,
                options,
                selected,
            },
            error: None,
        }
    }

    pub fn with_error(mut self, error: &'a str) -> Self {
        self.error = Some(error);
        self
    }

    fn is_header(&self) -> bool {
        matches!(self.kind, FormKind::Header(_))
    }
}

/// A host-owned bottom action. The renderer returns its ID on keyboard or hit
/// activation; it never runs a callback.
#[derive(Debug, Clone, Copy)]
pub struct FormActionRow<'a> {
    pub id: u32,
    pub label: &'a str,
}

impl<'a> FormActionRow<'a> {
    pub const fn new(id: u32, label: &'a str) -> Self {
        Self { id, label }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormAction {
    Unhandled,
    Changed,
    Toggle(u32),
    OpenSelect(u32),
    Step { id: u32, value: i64 },
    EditStart(u32),
    EditCommit { id: u32, text: String },
    EditCancel(u32),
    MultiSelectOpen(u32),
    MultiSelectClose(u32),
    ToggleMulti { id: u32, option: usize },
    ActivateAction(u32),
}

#[derive(Debug, Clone, Default)]
pub struct FormState {
    pub list: ListPaneState,
    pub editing: Option<u32>,
    pub editor: LineEditor,
    /// Last painted Select row, for anchoring a [`crate::DropdownState`].
    pub select_anchor: Option<Rect>,
    /// Expanded host multi-select, if any.
    pub expanded_multi: Option<u32>,
    pub multi_cursor: usize,
    /// The bottom action region owns keyboard focus independently of the
    /// scrollable fields above it.
    pub actions_focused: bool,
    pub action_selected: usize,
    /// Physical terminal-row offset for error/detail/expanded-option bodies.
    /// Kept separately from `list`, whose index remains the legacy field cursor.
    pub body_offset: usize,
    body_scroll_detached: bool,
}

impl FormState {
    pub fn handle_key(&mut self, key: &KeyEvent, rows: &[FormRow<'_>], page: usize) -> FormAction {
        self.handle_key_with_actions(key, rows, &[], page)
    }

    pub fn handle_key_with_actions(
        &mut self,
        key: &KeyEvent,
        rows: &[FormRow<'_>],
        actions: &[FormActionRow<'_>],
        page: usize,
    ) -> FormAction {
        self.body_scroll_detached = false;
        if let Some(id) = self.editing {
            if key!(Esc).matches(key) {
                self.editing = None;
                return FormAction::EditCancel(id);
            }
            if key!(Enter).matches(key) {
                let text = self.editor.text().to_string();
                self.editing = None;
                return FormAction::EditCommit { id, text };
            }
            return match self.editor.handle_key(key) {
                LineEditOutcome::Unhandled => FormAction::Unhandled,
                _ => FormAction::Changed,
            };
        }

        if let Some(id) = self.expanded_multi {
            let Some(FormKind::MultiSelect { options, .. }) =
                rows.iter().find(|row| row.id == id).map(|row| row.kind)
            else {
                self.expanded_multi = None;
                return FormAction::Changed;
            };
            if key!(Esc).matches(key) {
                self.expanded_multi = None;
                return FormAction::MultiSelectClose(id);
            }
            if key!(Down).matches(key) || key!('j').matches(key) {
                self.multi_cursor = self
                    .multi_cursor
                    .saturating_add(1)
                    .min(options.len().saturating_sub(1));
                return FormAction::Changed;
            }
            if key!(Up).matches(key) || key!('k').matches(key) {
                self.multi_cursor = self.multi_cursor.saturating_sub(1);
                return FormAction::Changed;
            }
            if (key!(Enter).matches(key) || key.code == KeyCode::Char(' '))
                && self.multi_cursor < options.len()
            {
                return FormAction::ToggleMulti {
                    id,
                    option: self.multi_cursor,
                };
            }
            return FormAction::Unhandled;
        }

        let vis_len = visible_len(rows);
        if self.actions_focused {
            if actions.is_empty() {
                self.actions_focused = false;
            } else if key!(Enter).matches(key) || key.code == KeyCode::Char(' ') {
                return FormAction::ActivateAction(
                    actions[self.action_selected.min(actions.len() - 1)].id,
                );
            } else if key!(Down).matches(key) || key!('j').matches(key) {
                self.action_selected = self
                    .action_selected
                    .saturating_add(1)
                    .min(actions.len() - 1);
                return FormAction::Changed;
            } else if key!(Up).matches(key) || key!('k').matches(key) || key!(Esc).matches(key) {
                if self.action_selected == 0 {
                    self.actions_focused = false;
                    if vis_len > 0 {
                        self.list.select_visible(vis_len - 1, vis_len, page);
                    }
                } else {
                    self.action_selected -= 1;
                }
                return FormAction::Changed;
            } else {
                return FormAction::Unhandled;
            }
        }
        if vis_len == 0 {
            if let Some(action) = actions.first()
                && key!(Enter).matches(key)
            {
                self.actions_focused = true;
                self.action_selected = 0;
                return FormAction::ActivateAction(action.id);
            }
            return FormAction::Unhandled;
        }
        if key!(Down).matches(key) || key!('j').matches(key) {
            if self.list.selected >= vis_len.saturating_sub(1) && !actions.is_empty() {
                self.actions_focused = true;
                self.action_selected = self.action_selected.min(actions.len() - 1);
            } else {
                let _ = self.list.handle_key(key, vis_len, page);
            }
            return FormAction::Changed;
        }
        if key!(Up).matches(key) || key!('k').matches(key) {
            let _ = self.list.handle_key(key, vis_len, page);
            return FormAction::Changed;
        }
        let Some(row) = row_at_visible(rows, self.list.selected.min(vis_len - 1)).copied() else {
            return FormAction::Unhandled;
        };
        match row.kind {
            FormKind::Header(_) => FormAction::Unhandled,
            FormKind::Toggle { .. }
                if key!(Enter).matches(key) || key.code == KeyCode::Char(' ') =>
            {
                FormAction::Toggle(row.id)
            }
            FormKind::Select { .. }
                if key!(Enter).matches(key) || key.code == KeyCode::Char(' ') =>
            {
                FormAction::OpenSelect(row.id)
            }
            FormKind::Stepper {
                value, min, max, ..
            } => {
                let next = if key!(Right).matches(key) || key.code == KeyCode::Char('+') {
                    value.saturating_add(1).min(max)
                } else if key!(Left).matches(key) || key.code == KeyCode::Char('-') {
                    value.saturating_sub(1).max(min)
                } else {
                    return FormAction::Unhandled;
                };
                FormAction::Step {
                    id: row.id,
                    value: next,
                }
            }
            FormKind::Field { value, .. } if key!(Enter).matches(key) => {
                self.editing = Some(row.id);
                self.editor.set_text(value);
                FormAction::EditStart(row.id)
            }
            FormKind::MultiSelect { options, .. }
                if key!(Enter).matches(key) || key.code == KeyCode::Char(' ') =>
            {
                self.expanded_multi = Some(row.id);
                self.multi_cursor = self.multi_cursor.min(options.len().saturating_sub(1));
                FormAction::MultiSelectOpen(row.id)
            }
            _ => FormAction::Unhandled,
        }
    }

    /// Scroll the physical form body; the renderer clamps against its measured
    /// errors, long values, and expanded options.
    pub fn handle_scroll(&mut self, delta_lines: i32) {
        if delta_lines == 0 {
            return;
        }
        self.body_scroll_detached = true;
        if delta_lines > 0 {
            self.body_offset = self.body_offset.saturating_add(delta_lines as usize);
        } else {
            self.body_offset = self
                .body_offset
                .saturating_sub(delta_lines.unsigned_abs() as usize);
        }
    }

    /// Select a form row from a registered hit and return the host action it
    /// implies. This preserves compact rows while opening a multi-select for
    /// pointer users.
    pub fn click_row(&mut self, visible: usize, rows: &[FormRow<'_>], page: usize) -> FormAction {
        let vis_len = visible_len(rows);
        let Some(row) = row_at_visible(rows, visible).copied() else {
            return FormAction::Unhandled;
        };
        self.body_scroll_detached = false;
        self.list.select_visible(visible, vis_len, page);
        self.actions_focused = false;
        if self.expanded_multi != Some(row.id) {
            self.expanded_multi = None;
        }
        match row.kind {
            FormKind::Toggle { .. } => FormAction::Toggle(row.id),
            FormKind::Select { .. } => FormAction::OpenSelect(row.id),
            FormKind::MultiSelect { options, .. } => {
                self.expanded_multi = Some(row.id);
                self.multi_cursor = self.multi_cursor.min(options.len().saturating_sub(1));
                FormAction::MultiSelectOpen(row.id)
            }
            FormKind::Header(_) | FormKind::Stepper { .. } | FormKind::Field { .. } => {
                FormAction::Changed
            }
        }
    }

    /// Turn a registered expanded-option hit into the host mutation request.
    pub fn click_multi_option(&mut self, option: usize, rows: &[FormRow<'_>]) -> FormAction {
        let Some(id) = self.expanded_multi else {
            return FormAction::Unhandled;
        };
        let Some(FormKind::MultiSelect { options, .. }) =
            rows.iter().find(|row| row.id == id).map(|row| row.kind)
        else {
            return FormAction::Unhandled;
        };
        if option >= options.len() {
            return FormAction::Unhandled;
        }
        self.body_scroll_detached = false;
        self.multi_cursor = option;
        FormAction::ToggleMulti { id, option }
    }

    /// Turn a registered pinned-action hit into an action ID for the host.
    pub fn click_action(&mut self, id: u32, actions: &[FormActionRow<'_>]) -> FormAction {
        let Some(index) = actions.iter().position(|action| action.id == id) else {
            return FormAction::Unhandled;
        };
        self.actions_focused = true;
        self.action_selected = index;
        FormAction::ActivateAction(id)
    }
}

fn visible_len(rows: &[FormRow<'_>]) -> usize {
    rows.iter().filter(|row| !row.is_header()).count()
}

/// Visible (non-header) index, matching [`FormState::list`] selection.
pub fn parse_row(id: HitId) -> Option<usize> {
    list::parse_row_id_ns(NS_FORM, id)
}

pub fn row_at_visible<'a, 'b>(rows: &'a [FormRow<'b>], vis: usize) -> Option<&'a FormRow<'b>> {
    rows.iter().filter(|row| !row.is_header()).nth(vis)
}

/// Decode a pinned action hit into the host-supplied action ID.
pub fn parse_action(id: HitId) -> Option<u32> {
    list::parse_row_id_ns(NS_FORM_ACTION, id).and_then(|id| u32::try_from(id).ok())
}

/// Decode an expanded multi-select option hit. Only one multi-select can be
/// expanded, so the field ID remains in [`FormState::expanded_multi`].
pub fn parse_option(id: HitId) -> Option<usize> {
    list::parse_row_id_ns(NS_FORM_OPTION, id)
}

pub fn render_form(
    state: &mut FormState,
    rows: &[FormRow<'_>],
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> usize {
    if area.height == 0 || area.width == 0 {
        return 0;
    }
    cx.register_scroll(list::scroll_id(NS_FORM), area);
    fill(buf, area, cx.style(ThemeRole::Surface, ThemeState::Normal));
    let vis_len = visible_len(rows);
    let page = area.height as usize;
    state.list.clamp(vis_len, page);
    state.select_anchor = None;

    let mut total = 0usize;
    let mut cursor = 0usize;
    let mut vis_i = 0usize;
    for row in rows {
        let selected = !row.is_header() && vis_i == state.list.selected;
        if selected {
            cursor = if matches!(row.kind, FormKind::MultiSelect { .. })
                && state.expanded_multi == Some(row.id)
            {
                total + 1 + cursor_detail_offset(*row, true, state, area.width)
            } else {
                total
            };
        }
        total += 1 + detail_height(*row, selected, state, area.width);
        if !row.is_header() {
            vis_i += 1;
        }
    }
    state.body_offset = state.body_offset.min(total.saturating_sub(page));
    if !state.body_scroll_detached {
        if cursor < state.body_offset {
            state.body_offset = cursor;
        } else if cursor >= state.body_offset.saturating_add(page) {
            state.body_offset = cursor + 1 - page;
        }
    }
    state.body_offset = state.body_offset.min(total.saturating_sub(page));

    let end = state.body_offset.saturating_add(page);
    let mut top = 0usize;
    vis_i = 0;
    for row in rows {
        let selectable = !row.is_header();
        let selected = selectable && vis_i == state.list.selected;
        let details = detail_height(*row, selected, state, area.width);
        let height = 1 + details;
        let bottom = top.saturating_add(height);
        if bottom > state.body_offset && top < end {
            if top >= state.body_offset {
                let y = area.y.saturating_add((top - state.body_offset) as u16);
                let r = Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                };
                if selectable {
                    cx.register(list::row_id_ns(NS_FORM, vis_i), r);
                }
                let row_state = ThemeState::from_flags(
                    false,
                    false,
                    selected,
                    false,
                    selectable && cx.hovered(r),
                );
                let row_role = match row_state {
                    ThemeState::Selected => ThemeRole::Selection,
                    ThemeState::Hovered => ThemeRole::Hover,
                    _ => ThemeRole::Text,
                };
                let row_style = cx
                    .style(ThemeRole::Surface, ThemeState::Normal)
                    .patch(cx.style(row_role, row_state));
                fill(buf, r, row_style);
                paint_row(*row, selected, state, r, row_style, buf, cx);
            }
            if details > 0 {
                let detail_top = top + 1;
                let visible_top = detail_top.max(state.body_offset);
                if visible_top < end {
                    let skip = visible_top - detail_top;
                    let height = (end - visible_top).min(details.saturating_sub(skip)) as u16;
                    let detail_area = Rect {
                        x: area.x,
                        y: area
                            .y
                            .saturating_add((visible_top - state.body_offset) as u16),
                        width: area.width,
                        height,
                    };
                    let _ = paint_details(*row, selected, state, detail_area, skip, buf, cx);
                }
            }
        }
        top = bottom;
        if selectable {
            vis_i += 1;
        }
    }
    page
}

/// Paint a form body while pinning host-supplied action rows at the bottom.
/// The body may scroll independently without making an action unreachable.
pub fn render_form_with_actions(
    state: &mut FormState,
    rows: &[FormRow<'_>],
    actions: &[FormActionRow<'_>],
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> usize {
    if area.width == 0 || area.height == 0 {
        return 0;
    }
    if actions.is_empty() {
        state.actions_focused = false;
        state.action_selected = 0;
    } else {
        state.action_selected = state.action_selected.min(actions.len() - 1);
    }
    let actions_h = actions.len().min(area.height as usize) as u16;
    let body = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: area.height.saturating_sub(actions_h),
    };
    let page = render_form(state, rows, body, buf, cx);
    let start_y = area.bottom().saturating_sub(actions_h);
    let first_action = state
        .action_selected
        .saturating_add(1)
        .saturating_sub(actions_h as usize)
        .min(actions.len().saturating_sub(actions_h as usize));
    for (line, (index, action)) in actions
        .iter()
        .enumerate()
        .skip(first_action)
        .take(actions_h as usize)
        .enumerate()
    {
        let r = Rect {
            x: area.x,
            y: start_y.saturating_add(line as u16),
            width: area.width,
            height: 1,
        };
        Button::new(action.label)
            .id(HitId::namespaced(NS_FORM_ACTION, action.id))
            .focused(state.actions_focused && state.action_selected == index)
            .render(r, buf, cx);
    }
    page
}

fn paint_row(
    row: FormRow<'_>,
    selected: bool,
    state: &mut FormState,
    r: Rect,
    st: Style,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    let editing = state.editing == Some(row.id);
    match row.kind {
        FormKind::Header(label) => {
            buf.set_string_safe(
                r.x,
                r.y,
                line_utils::truncate_str(&format!("── {label} ──"), r.width as usize),
                st.patch(cx.style(ThemeRole::Muted, ThemeState::Normal)),
            );
        }
        FormKind::Toggle { label, on } => {
            paint_kv(
                label,
                if on { "on" } else { "off" },
                selected,
                r,
                buf,
                st,
                None,
            );
        }
        FormKind::Select { label, value } => {
            paint_kv(label, value, selected, r, buf, st, Some("▾"));
            state.select_anchor = Some(r);
        }
        FormKind::Stepper { label, value, .. } => {
            paint_kv(label, &value.to_string(), selected, r, buf, st, None);
        }
        FormKind::Field { label, value } => {
            let shown = if editing { state.editor.text() } else { value };
            let slot = paint_kv(label, shown, selected, r, buf, st, None);
            if editing && let Some((x, slot_w)) = slot {
                let vp = state.editor.viewport(slot_w.max(1) as usize);
                let slice = &state.editor.text()[vp.visible_byte_range.clone()];
                buf.set_string_safe(x, r.y, line_utils::truncate_str(slice, slot_w as usize), st);
                let col = (vp.cursor_display_column as u16).min(slot_w.saturating_sub(1));
                cx.set_cursor(x.saturating_add(col).min(r.right().saturating_sub(1)), r.y);
            }
        }
        FormKind::MultiSelect {
            label,
            options,
            selected: selected_options,
        } => {
            let summary = multi_summary(options, selected_options);
            paint_kv(label, &summary, selected, r, buf, st, Some("▾"));
        }
    }
}

fn multi_summary(options: &[&str], selected: &[bool]) -> String {
    let labels: Vec<_> = options
        .iter()
        .zip(selected)
        .filter_map(|(label, selected)| selected.then_some(*label))
        .collect();
    if labels.is_empty() {
        "(none selected)".to_owned()
    } else {
        labels.join(", ")
    }
}

fn wrapped_detail_height(text: &str, width: u16) -> usize {
    gbtui_core::wrapping::wrap_text(text, width.saturating_sub(2) as usize)
        .len()
        .max(1)
}

fn detail_height(row: FormRow<'_>, selected: bool, state: &FormState, width: u16) -> usize {
    let mut height = row
        .error
        .map_or(0, |error| wrapped_detail_height(error, width));
    if selected && let FormKind::Field { value, .. } = row.kind {
        let shown = if state.editing == Some(row.id) {
            state.editor.text()
        } else {
            value
        };
        if shown.width() > (width / 2).max(4) as usize {
            height += wrapped_detail_height(shown, width);
        }
    }
    if let FormKind::MultiSelect { options, .. } = row.kind
        && state.expanded_multi == Some(row.id)
    {
        height += options.len();
    }
    height
}

fn cursor_detail_offset(row: FormRow<'_>, selected: bool, state: &FormState, width: u16) -> usize {
    if !selected {
        return 0;
    }
    let error_height = row
        .error
        .map_or(0, |error| wrapped_detail_height(error, width));
    if let FormKind::MultiSelect { options, .. } = row.kind
        && state.expanded_multi == Some(row.id)
    {
        return error_height + state.multi_cursor.min(options.len().saturating_sub(1));
    }
    0
}

fn paint_details(
    row: FormRow<'_>,
    selected: bool,
    state: &FormState,
    area: Rect,
    skip: usize,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> u16 {
    if area.width == 0 || area.height == 0 {
        return 0;
    }
    let base = if selected {
        cx.style(ThemeRole::Surface, ThemeState::Normal)
            .patch(cx.style(ThemeRole::Selection, ThemeState::Selected))
    } else {
        cx.style(ThemeRole::Surface, ThemeState::Normal)
    };
    let normal = base.patch(cx.style(ThemeRole::Muted, ThemeState::Normal));
    let error = base.patch(cx.style(ThemeRole::Error, ThemeState::Invalid));
    let mut logical = 0usize;
    let mut used = 0u16;
    let mut paint_wrapped = |text: &str, style: Style| {
        for line in gbtui_core::wrapping::wrap_text(text, area.width.saturating_sub(2) as usize) {
            if logical >= skip && used < area.height {
                let r = Rect {
                    x: area.x,
                    y: area.y.saturating_add(used),
                    width: area.width,
                    height: 1,
                };
                fill(buf, r, style);
                buf.set_string_safe(r.x, r.y, "  ", style);
                buf.set_string_safe(r.x.saturating_add(2), r.y, &line, style);
                used = used.saturating_add(1);
            }
            logical += 1;
        }
    };
    if let Some(error_text) = row.error {
        paint_wrapped(error_text, error);
    }
    if selected && let FormKind::Field { value, .. } = row.kind {
        let shown = if state.editing == Some(row.id) {
            state.editor.text()
        } else {
            value
        };
        if shown.width() > (area.width / 2).max(4) as usize {
            paint_wrapped(shown, normal);
        }
    }
    if let FormKind::MultiSelect {
        options,
        selected: selected_options,
        ..
    } = row.kind
        && state.expanded_multi == Some(row.id)
    {
        for (index, label) in options.iter().enumerate() {
            if logical >= skip && used < area.height {
                let focused = state.multi_cursor == index;
                let option_state = ThemeState::from_flags(false, false, false, focused, false);
                let style = base.patch(cx.style(ThemeRole::Selection, option_state));
                let r = Rect {
                    x: area.x,
                    y: area.y.saturating_add(used),
                    width: area.width,
                    height: 1,
                };
                fill(buf, r, style);
                let mark = if selected_options.get(index).copied().unwrap_or(false) {
                    "[x] "
                } else {
                    "[ ] "
                };
                buf.set_string_safe(r.x, r.y, if focused { "› " } else { "  " }, style);
                buf.set_string_safe(r.x.saturating_add(2), r.y, mark, style);
                buf.set_string_safe(
                    r.x.saturating_add(6),
                    r.y,
                    line_utils::truncate_str(label, r.width.saturating_sub(6) as usize),
                    style,
                );
                if let Ok(index) = u32::try_from(index) {
                    cx.register(HitId::namespaced(NS_FORM_OPTION, index), r);
                }
                used = used.saturating_add(1);
            }
            logical += 1;
        }
    }
    used
}
fn paint_kv(
    label: &str,
    value: &str,
    selected: bool,
    r: Rect,
    buf: &mut Buffer,
    st: Style,
    glyph: Option<&str>,
) -> Option<(u16, u16)> {
    let prefix = if selected { "▸ " } else { "  " };
    let glyph_w = glyph.map(|g| g.width() as u16).unwrap_or(0);
    let gap = 2u16;
    let value_budget = (r.width / 2).max(4).min(
        r.width
            .saturating_sub(prefix.width() as u16 + 4 + glyph_w + gap),
    );
    let shown = line_utils::truncate_str(value, value_budget as usize);
    let vw = (shown.width() as u16).max(1).min(value_budget);
    let vx = r.right().saturating_sub(vw + glyph_w);
    let label_budget = vx.saturating_sub(r.x + gap) as usize;
    buf.set_string_safe(
        r.x,
        r.y,
        line_utils::truncate_str(&format!("{prefix}{label}"), label_budget),
        st,
    );
    if vx >= r.x {
        buf.set_string_safe(vx, r.y, &shown, st);
        if let Some(g) = glyph {
            buf.set_string_safe(r.right().saturating_sub(glyph_w), r.y, g, st);
        }
        Some((vx, vw))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::HitMap;
    use std::time::Instant;

    fn rows() -> [FormRow<'static>; 3] {
        [
            FormRow::header("Editor"),
            FormRow::toggle(1, "vim keys", false),
            FormRow::stepper(2, "indent", 4, 1, 8),
        ]
    }

    #[test]
    fn space_toggles() {
        let mut s = FormState::default();
        let rows = rows();
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(s.handle_key(&key, &rows, 8), FormAction::Toggle(1));
    }

    #[test]
    fn right_steps() {
        let mut s = FormState::default();
        let rows = rows();
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(s.handle_key(&down, &rows, 8), FormAction::Changed);
        let right = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(
            s.handle_key(&right, &rows, 8),
            FormAction::Step { id: 2, value: 5 }
        );
    }

    #[test]
    fn render_registers_rows() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 6));
        let mut state = FormState::default();
        let rows = rows();
        render_form(&mut state, &rows, Rect::new(0, 0, 40, 6), &mut buf, &mut cx);
        match hits.hit(2, 1) {
            gbtui_runtime::HitResult::Hit(id) => assert_eq!(parse_row(id), Some(0)),
            other => panic!("expected toggle vis 0, got {other:?}"),
        }
    }

    #[test]
    fn long_value_is_truncated_not_dropped() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 2));
        let mut state = FormState::default();
        let rows = [FormRow::field(1, "name", "abcdefghijklmnopqrstuvwxyz")];
        render_form(&mut state, &rows, Rect::new(0, 0, 20, 2), &mut buf, &mut cx);
        let row: String = (0..20).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(
            row.contains('a') || row.contains('…'),
            "value must still paint: {row:?}"
        );
    }

    #[test]
    fn field_edit_sets_cursor() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
        let mut state = FormState::default();
        let rows = [FormRow::field(1, "name", "ab")];
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert!(matches!(
            state.handle_key(&enter, &rows, 4),
            FormAction::EditStart(1)
        ));
        render_form(&mut state, &rows, Rect::new(0, 0, 40, 4), &mut buf, &mut cx);
        assert!(cx.cursor().is_some());
    }

    #[test]
    fn expanded_multiselect_reports_host_mutation_and_esc_closes() {
        let options = ["one", "two"];
        let selected = [false, true];
        let rows = [FormRow::multi_select(9, "targets", &options, &selected)];
        let mut state = FormState::default();
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &rows, 4),
            FormAction::MultiSelectOpen(9)
        );
        assert_eq!(
            state.handle_key(
                &KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                &rows,
                4
            ),
            FormAction::ToggleMulti { id: 9, option: 0 }
        );
        assert_eq!(
            state.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &rows, 4),
            FormAction::MultiSelectClose(9)
        );
        assert_eq!(state.expanded_multi, None);
        assert_eq!(state.click_row(0, &rows, 4), FormAction::MultiSelectOpen(9));
        assert_eq!(
            state.click_multi_option(1, &rows),
            FormAction::ToggleMulti { id: 9, option: 1 }
        );
    }

    #[test]
    fn short_body_scrolls_selected_expanded_option_above_pinned_action() {
        let options = ["one", "two", "three", "four", "five"];
        let selected = [false; 5];
        let rows = [FormRow::multi_select(9, "targets", &options, &selected)];
        let actions = [FormActionRow::new(41, "Apply")];
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));
        let mut state = FormState {
            expanded_multi: Some(9),
            multi_cursor: 4,
            ..FormState::default()
        };
        render_form_with_actions(
            &mut state,
            &rows,
            &actions,
            Rect::new(0, 0, 20, 3),
            &mut buf,
            &mut cx,
        );
        assert!(state.body_offset > 0);
        let row: String = (0..20).map(|x| buf[(x, 1)].symbol().to_string()).collect();
        assert!(
            row.contains("five"),
            "selected option was not visible: {row:?}"
        );
        match hits.hit(1, 2) {
            gbtui_runtime::HitResult::Hit(id) => assert_eq!(parse_action(id), Some(41)),
            other => panic!("expected pinned action, got {other:?}"),
        }
    }
    #[test]
    fn wheel_detaches_body_until_keyboard_navigation_resumes() {
        let rows = [
            FormRow::toggle(0, "zero", false),
            FormRow::toggle(1, "one", false),
            FormRow::toggle(2, "two", false),
            FormRow::toggle(3, "three", false),
            FormRow::toggle(4, "four", false),
        ];
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let area = Rect::new(0, 0, 20, 2);
        let mut buf = Buffer::empty(area);
        let mut state = FormState::default();
        render_form(&mut state, &rows, area, &mut buf, &mut cx);
        state.handle_scroll(3);
        render_form(&mut state, &rows, area, &mut buf, &mut cx);
        assert_eq!(state.body_offset, 3);
        let top = (0..20).map(|x| buf[(x, 0)].symbol()).collect::<String>();
        assert!(top.contains("three"));
        assert_eq!(state.list.selected, 0);
        state.handle_key(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &rows, 2);
        render_form(&mut state, &rows, area, &mut buf, &mut cx);
        assert_eq!(state.body_offset, 1);
        let top = (0..20).map(|x| buf[(x, 0)].symbol()).collect::<String>();
        assert!(top.contains("one"));
    }

    #[test]
    fn short_form_keeps_actions_hittable_below_error_and_long_value() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 4));
        let rows = [
            FormRow::field(1, "name", "a very long host value").with_error("invalid host value")
        ];
        let actions = [
            FormActionRow::new(41, "Apply"),
            FormActionRow::new(42, "Cancel"),
        ];
        let mut state = FormState::default();
        render_form_with_actions(
            &mut state,
            &rows,
            &actions,
            Rect::new(0, 0, 20, 4),
            &mut buf,
            &mut cx,
        );
        let error_row: String = (0..20).map(|x| buf[(x, 1)].symbol().to_string()).collect();
        assert!(
            error_row.contains("invalid"),
            "error row was not painted: {error_row:?}"
        );
        match hits.hit(1, 2) {
            gbtui_runtime::HitResult::Hit(id) => assert_eq!(parse_action(id), Some(41)),
            other => panic!("expected pinned Apply hit, got {other:?}"),
        }
        match hits.hit(1, 3) {
            gbtui_runtime::HitResult::Hit(id) => assert_eq!(parse_action(id), Some(42)),
            other => panic!("expected pinned Cancel hit, got {other:?}"),
        }
    }

    #[test]
    fn short_viewport_shows_long_focused_value_above_pinned_action() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 4));
        let rows = [
            FormRow::field(1, "name", "a very long host value").with_error("invalid host value")
        ];
        let actions = [FormActionRow::new(41, "Apply")];
        let mut state = FormState::default();
        render_form_with_actions(
            &mut state,
            &rows,
            &actions,
            Rect::new(0, 0, 20, 4),
            &mut buf,
            &mut cx,
        );
        let expanded: String = (0..20).map(|x| buf[(x, 2)].symbol().to_string()).collect();
        assert!(
            expanded.contains("a very long"),
            "focused value was not expanded: {expanded:?}"
        );
        match hits.hit(1, 3) {
            gbtui_runtime::HitResult::Hit(id) => assert_eq!(parse_action(id), Some(41)),
            other => panic!("expected pinned action, got {other:?}"),
        }
    }

    #[test]
    fn last_field_navigates_to_pinned_action_and_activates_it() {
        let rows = [FormRow::field(1, "name", "gbtui")];
        let actions = [FormActionRow::new(41, "Apply")];
        let mut state = FormState::default();
        assert_eq!(
            state.handle_key_with_actions(
                &KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                &rows,
                &actions,
                1,
            ),
            FormAction::Changed
        );
        assert!(state.actions_focused);
        assert_eq!(
            state.handle_key_with_actions(
                &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &rows,
                &actions,
                1,
            ),
            FormAction::ActivateAction(41)
        );
    }
}
