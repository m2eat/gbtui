//! Reusable terminal widgets for gbtui.
//!
//! Widgets draw with ratatui and talk to the mouse/hover/animation runtime
//! through `gbtui_runtime` (FrameCtx). They must not run their own event
//! loop or keep a parallel hover map.
//!
//! # Extraction
//!
//! - `SOURCE: crates/codegen/xai-ratatui-textarea/** (COPY)` — P1a
//! - Modal / overlay / list / picker / choice / prompt / chrome / P9
//!   status-toast-tabs-chip-dropdown-cheatsheet-clipboard: see `docs/source-map.md`

#![forbid(unsafe_code)]

pub use gbtui_core as core;
pub use gbtui_runtime as runtime;

mod paint;

/// Clickable button. Hover/click go through FrameCtx.
pub mod button;
pub use button::Button;

/// Overlay pane visibility / focus / fullscreen state machine.
pub mod overlay;
pub use overlay::{OverlayAction, OverlayState, handle_overlay_key, handle_overlay_nav_key};

/// Modal chrome: border, title, close, tabs, footer shortcuts.
pub mod modal;
pub use modal::{
    BODY as MODAL_BODY, CLOSE as MODAL_CLOSE, Modal, ModalAction, ModalKind, ModalShortcut,
    ModalSizing, ModalState, NS_SHORTCUT, NS_TAB, shortcut_hit_id, tab_id,
};

/// Ghost / autosuggest remainder.
pub mod ghost;
pub use ghost::{GhostAction, GhostText};

/// Settings-style form rows.
pub mod form;
pub use form::{
    FormAction, FormActionRow, FormKind, FormRow, FormState, NS_FORM, NS_FORM_ACTION,
    NS_FORM_OPTION, parse_action as parse_form_action, parse_option as parse_form_option,
    parse_row as parse_form_row, render_form, render_form_with_actions, row_at_visible,
};

/// Multiline preview overlay.
pub mod preview;
pub use preview::{BODY as PREVIEW_BODY, PreviewConfig, render_preview};

/// Source-mapped content viewer with host-owned styled rows.
pub mod viewer;
pub use viewer::{
    ContentCopyTarget, ContentDocument, ContentDocumentId, ContentRow, ContentView, ContentViewer,
    NS_CONTENT_VIEWER,
};

/// Alert banner.
pub mod banner;
pub use banner::{BODY as BANNER_BODY, Banner, BannerKind, HIDE as BANNER_HIDE};

/// Compact usage gauge.
pub mod gauge;
pub use gauge::{UsageGauge, fmt_compact, gauge_color};

/// Status-icon checklist.
pub mod checklist;
pub use checklist::{
    CheckAction, CheckItem, CheckStatus, NS_CHECK, handle_checklist_key,
    parse_row as parse_check_row, render_checklist,
};

/// Activity row (spinner + label + stop).
pub mod activity;
pub use activity::{ActivityRow, STOP as ACTIVITY_STOP};

/// Horizontal tab strip (also used by [`Modal`]).
pub mod tabs;
pub use tabs::{Tabs, parse_tab};

/// Left / center / right status row.
pub mod status;
pub use status::StatusBar;

/// Ephemeral one-line toast.
pub mod toast;
pub use toast::{BODY as TOAST_BODY, CLOSE as TOAST_CLOSE, Toast};

/// Dismissible label chip.
pub mod chip;
pub use chip::Chip;

/// Borrowed label/detail dropdown.
pub mod dropdown;
pub use dropdown::{
    BODY as DROPDOWN_BODY, DropdownAction, DropdownItem, DropdownScrollPolicy, DropdownState,
    NS_DROP, SCROLLBAR as DROPDOWN_SCROLLBAR, is_scrollbar as is_dropdown_scrollbar,
    parse_row as parse_dropdown_row, popup_area as dropdown_popup_area, render_dropdown,
    row_id as dropdown_row_id,
};

/// Keyboard cheatsheet body (Modal + List).
pub mod cheatsheet;
pub use cheatsheet::{
    CheatsheetEntry, CheatsheetState, NS_CHEAT, parse_row as parse_cheatsheet_row,
    render_cheatsheet, selected_hint_index,
};

/// System clipboard (`arboard` + optional OSC 52).
pub mod clipboard;
pub use clipboard::{SystemClipboard, osc52_set_sequence};

/// Prepared matching policy for lists and pickers.
pub mod search;
pub use search::{SearchKind, SearchMatch, SearchMode, SearchQuery};

/// Scrollable selectable list.
pub mod list;
pub use list::{
    ListItem, ListLayoutCache, ListPaneState, ListPaneStyle, NS_ROW, NS_SCROLL, WrapMode,
    click_scrollbar, is_scroll_id, parse_row_id, parse_row_id_ns, render_list, render_list_items,
    render_list_items_ns, render_list_ns, row_id, row_id_ns, scroll_id,
};

/// Rich fuzzy picker with grouped, expandable rows.
pub mod picker;
pub use picker::{
    NS_PICKER, NS_PICKER_EXPAND, NS_PICKER_GROUP, PickerEntry, PickerField, PickerGroup, PickerRow,
    PickerState, is_picker_scroll, parse_picker_expander, parse_picker_group, parse_picker_row,
    render_picker, render_picker_content,
};

/// Single- / multi-select choice list.
pub mod choice;
pub use choice::{
    Choice, ChoiceAction, ChoiceListState, ChoiceMode, NS_CHOICE, parse_choice_row, render_choices,
};

/// Single-line editor over the textarea [`EditBuffer`].
pub mod line_editor;
pub use line_editor::{LineEditOutcome, LineEditor, sanitize_single_line};

/// Multiline prompt with host-driven completion, history, paste and attachments.
pub mod prompt;
pub use prompt::{
    AttachmentId, AttachmentKind, BODY as PROMPT_BODY, CompletionCandidate, CompletionRequest,
    EnterOutcome, LiveAttachment, PastePolicy, Prompt, PromptAttachment, PromptEvent, PromptStyle,
    PromptSubmission,
};

/// 1/8-cell progress bar.
pub mod progress;
pub use progress::{progress_bar_spans, render_progress, render_progress_bar};

/// Braille spinner + wave accent bar.
pub mod spinner;
pub use spinner::{Spinner, render_wave_bar};

/// Shortcuts bar + pending double-press confirm.
pub mod shortcuts;
pub use shortcuts::{
    CompactConfig, HintItem, NS_HINT, PendingConfirm, PendingHint, PendingOutcome, ShortcutsBar,
    compute_effective_hints, hint_id, parse_hint_id, pending_on_key,
};

/// Hover/click hyperlink.
pub mod link;
pub use link::{Link, LinkActivation};

/// Optional terminal image with text fallback.
#[cfg(feature = "images")]
pub mod image;
#[cfg(feature = "images")]
pub use image::Image;

/// Text editor widget and edit buffer.
///
/// `SOURCE: crates/codegen/xai-ratatui-textarea/** (COPY)`
pub mod textarea;
pub use textarea::{
    ApplyEditPlanError, ClipboardProvider, EditBuffer, EditCommand, EditDelta, EditOutcome,
    EditPlan, ElementId, ElementKind, InternalClipboard, MouseAction, PostEditCursorAffinity,
    SingleLineViewport, TextArea, TextAreaState, TextElement, TextElementEvent,
    TextElementEventKind, WordStyle, classify_key_event, is_altgr, is_undo_input,
};
pub use textarea::{editor, render, wrapping};

/// Crate version, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
