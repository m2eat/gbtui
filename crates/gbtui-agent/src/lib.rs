//! Optional agent-CLI chrome stacked on the [`gbtui`] facade.
//!
//! This crate is **not** part of `gbtui`. Conversation transcripts, tool
//! rows, permission cards, and slash palettes live here so the core runtime
//! stays protocol-free.
//!
//! Visual chrome is a 1:1 STRIP of Grok Build's agent view (user band,
//! `Thought` / `Read` / `Edit` / `$` headers, permission stack, slash overlay)
//! without ACP, session store, or a markdown crate.
//!
//! ```text
//! your agent CLI
//!     → gbtui-agent     (this crate)
//!         → gbtui       (runtime + generic widgets)
//! ```
//!
//! `gbtui` must not depend on this crate. Prompt has no slash / `@` parser;
//! the host opens [`SlashPalette`] when [`slash_query`] says so.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/scrollback/blocks/* (STRIP paint, not types)`
//! `SOURCE: crates/codegen/xai-grok-pager/src/views/permission_view.rs (STRIP layout)`
//! `SOURCE: crates/codegen/xai-grok-pager/src/views/slash_dropdown.rs (STRIP layout)`
//! Files above are **not copied**. ACP / `BlockContent` stay out.

#![forbid(unsafe_code)]

mod diff;
mod layout;
mod markdown;
mod mention;
mod paint;
mod permission;
mod slash;
mod tool_row;
mod transcript;
mod viewer;
pub use diff::{
    DiffFile, DiffFileItem, DiffHunk, DiffHunkHeader, DiffLine, DiffLineKind, DiffMetadata,
    DiffMetadataKind, UnifiedDiff, parse_unified_diff,
};

pub use layout::{AgentLayout, AgentLayoutParams, SCROLLBACK_MIN};
pub use mention::{
    MentionChip, NS_MENTION, NS_MENTION_CLOSE, mention_close_id, mention_id,
    parse_mention_close_id, parse_mention_id, render_mention_bar,
};
pub use permission::{
    NS_PERM_EXTRA, PERM_ALLOW, PERM_BODY, PERM_DENY, PermAction, PermissionCard, PermissionState,
    extra_id, parse_extra_id,
};
pub use slash::{SlashAction, SlashItem, SlashPalette, slash_query};
pub use tool_row::{NS_TOOL, ToolKind, ToolRow, ToolRowState, ToolStatus, parse_tool_id, tool_id};
pub use transcript::{
    BODY as TRANSCRIPT_BODY, NS_TRANSCRIPT, TranscriptDirection, TranscriptDisplay,
    TranscriptDisplayTarget, TranscriptItem, TranscriptNavigation, TranscriptSearchMatch,
    TranscriptSelection, TranscriptState, TranscriptTextPosition, item_id, parse_item_id,
    render_transcript, render_transcript_with_selection,
};
pub use viewer::{CodeDocument, CodeView, MarkdownDocument};

/// Crate version, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
