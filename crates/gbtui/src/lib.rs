//! gbtui — terminal UI runtime and widgets.
//!
//! Facade crate. Application code should depend on this package and use the
//! re-exports below. Layered crates (`gbtui-core`, `gbtui-runtime`, …) are
//! public for advanced users who want a smaller dependency set.
//!
//! ```text
//! gbtui-agent (optional; not this crate)
//!   └─ gbtui
//!         ├─ gbtui-actions     (optional keymap registry)
//!         ├─ gbtui-widgets     (Button, TextArea, Modal, …)
//!         │     └─ gbtui-runtime   (event loop, FrameCtx, HitMap)
//!         │           ├─ gbtui-input
//!         │           └─ gbtui-terminal
//!         │                 └─ gbtui-core
//!         └─ (actions → core only)
//! ```
//!
//! This is **not** Grok Build. `sources/grok-build/` in this repository is a
//! read-only reference tree used during extraction.

#![forbid(unsafe_code)]

pub use gbtui_actions as actions;
pub use gbtui_core as core;
pub use gbtui_input as input;
pub use gbtui_runtime as runtime;
pub use gbtui_terminal as terminal;
pub use gbtui_widgets as widgets;

pub use gbtui_actions::{ActionDef, ActionRegistry, UnreliableKey};
#[cfg(feature = "theme-config")]
pub use gbtui_core::ThemeConfigError;
pub use gbtui_core::renderable::{RenderContext, Renderable};
pub use gbtui_core::{
    BackgroundPolicy, HitId, ResolvedTheme, TextMatcher, Theme, ThemeAppearance, ThemeColor,
    ThemeContext, ThemeDefinition, ThemeKind, ThemeRegistry, ThemeResolver, ThemeRole, ThemeState,
    TickDemand, crossterm, key, next_index_after, prev_index_before, ratatui,
};
pub use gbtui_runtime::{
    App, AppEvent, FrameCtx, Outcome, Pointer, Pump, RuntimeTheme, run, run_blocking, run_inline,
    run_inline_blocking,
};
pub use gbtui_terminal::TerminalInfo;
pub use gbtui_widgets::{
    ActivityRow, Banner, BannerKind, Button, CheatsheetEntry, CheatsheetState, CheckAction,
    CheckItem, CheckStatus, Chip, Choice, ChoiceAction, ChoiceListState, ChoiceMode,
    DropdownAction, DropdownItem, DropdownScrollPolicy, DropdownState, EnterOutcome, FormAction,
    FormActionRow, FormKind, FormRow, FormState, GhostAction, GhostText, HintItem, LineEditor,
    Link, LinkActivation, ListItem, ListLayoutCache, ListPaneState, Modal, ModalAction, ModalKind,
    ModalShortcut, ModalSizing, ModalState, OverlayAction, OverlayState, PendingOutcome,
    PickerEntry, PickerField, PickerGroup, PickerRow, PickerState, PreviewConfig, Prompt,
    PromptEvent, PromptStyle, ShortcutsBar, Spinner, StatusBar, SystemClipboard, Tabs, Toast,
    UsageGauge, WrapMode,
};
pub use gbtui_widgets::{
    AttachmentId, AttachmentKind, CompletionCandidate, CompletionRequest, LiveAttachment,
    PastePolicy, PromptAttachment, PromptSubmission, SearchKind, SearchMatch, SearchMode,
    SearchQuery,
};
pub use gbtui_widgets::{
    ContentCopyTarget, ContentDocument, ContentDocumentId, ContentRow, ContentView, ContentViewer,
};

#[cfg(feature = "images")]
pub use gbtui_terminal::image::PreparedImage;
#[cfg(feature = "images")]
pub use gbtui_widgets::Image;

/// Map an [`ActionDef`] to a shortcuts-bar [`HintItem`].
///
/// Lives on the facade so `gbtui-actions` stays free of widgets.
pub fn hint_item<Id, Ctx>(
    registry: &ActionRegistry<Id, Ctx>,
    def: &ActionDef<Id, Ctx>,
) -> HintItem {
    let mut item = HintItem::new(registry.advertised_key(def), def.label);
    item.custom_display = def.hint_key_display;
    item.description = Some(std::borrow::Cow::Borrowed(def.description));
    if def.pinned {
        item = item.pinned();
    }
    item
}

/// Hints for `contexts`, already converted for [`ShortcutsBar`].
pub fn hint_items<Id, Ctx>(registry: &ActionRegistry<Id, Ctx>, contexts: &[Ctx]) -> Vec<HintItem>
where
    Id: Copy + PartialEq,
    Ctx: PartialEq,
{
    registry
        .hints(contexts)
        .into_iter()
        .map(|def| hint_item(registry, def))
        .collect()
}

/// Default unreliable-key hook: Ctrl+. when the terminal likely drops it.
pub fn ctrl_dot_unreliable() -> bool {
    gbtui_terminal::terminal_context().ctrl_dot_unreliable()
}

/// [`UnreliableKey`] for [`ActionRegistry::unreliable_keys`].
pub fn default_unreliable_key(key: gbtui_core::KeyShortcut) -> bool {
    key == key!('.', CONTROL) && ctrl_dot_unreliable()
}

/// Facade crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
