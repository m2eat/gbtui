//! Grok Build–shaped agent shell, fake session only (no LLM / ACP / network).
//!
//! Opens on a canned turn so the layout matches the real pager: user band,
//! `Read` / `Edit` / `$` tool headers, `Thought for`, boxed prompt, usage
//! gauge, slash overlay, permission stack.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEventKind, MouseButton};
use gbtui::key;
use gbtui::widgets::{
    ACTIVITY_STOP, CheckItem, CheckStatus, ClipboardProvider, ListPaneState, ShortcutsBar,
    handle_checklist_key, parse_check_row, render_checklist,
};
use gbtui::{
    ActionDef, ActionRegistry, ActivityRow, App, AppEvent, ContentCopyTarget, ContentDocumentId,
    ContentViewer, EnterOutcome, FrameCtx, HintItem, HitId, Outcome, Pointer, Prompt, PromptStyle,
    ResolvedTheme, StatusBar, SystemClipboard, Theme, ThemeDefinition, ThemeKind, ThemeResolver,
    ThemeRole, ThemeState, TickDemand, UsageGauge, hint_items, run_blocking,
};
use gbtui_agent::{
    AgentLayout, AgentLayoutParams, CodeDocument, CodeView, MarkdownDocument, PERM_ALLOW,
    PERM_DENY, PermAction, PermissionCard, PermissionState, SlashAction, SlashItem, SlashPalette,
    TRANSCRIPT_BODY, ToolKind, ToolRow, ToolStatus, TranscriptDirection, TranscriptDisplay,
    TranscriptDisplayTarget, TranscriptItem, TranscriptNavigation, TranscriptSelection,
    TranscriptState, extra_id, parse_item_id, parse_tool_id, render_transcript_with_selection,
    slash_query,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Block, Clear, Paragraph};

const TOOL_TTL: Duration = Duration::from_millis(400);
const STREAM_CPS: f32 = 28.0;
const REPLY: &str = "## Streaming reply\n\n**Incremental Markdown** keeps [link labels](https://example.com) clean while it grows.\n\n| Cache | Result |\n| --- | --- |\n| push | append only |\n| finish | matches batch |\n\n```rust\nlet rendered = cache.finish();\n```\n";
const PERM_ALWAYS: HitId = extra_id(0);
const CODE_SAMPLE: &str = "pub fn smart_case_matches(query: &str, haystack: &str) -> bool {\n    let sensitive = query.chars().any(char::is_uppercase);\n    if sensitive {\n        haystack.contains(query)\n    } else {\n        haystack.to_lowercase().contains(&query.to_lowercase())\n    }\n}\n";

const SLASH: [SlashItem<'static>; 7] = [
    SlashItem {
        name: "help",
        source_tag: Some("demo"),
        argument_hint: None,
        description: Some("show commands"),
    },
    SlashItem {
        name: "clear",
        source_tag: Some("demo"),
        argument_hint: None,
        description: Some("clear transcript"),
    },
    SlashItem {
        name: "compact",
        source_tag: Some("demo"),
        argument_hint: None,
        description: Some("compact context\n(fake session only)"),
    },
    SlashItem {
        name: "model",
        source_tag: Some("demo"),
        argument_hint: Some("<name>"),
        description: Some("show model (fake session)"),
    },
    SlashItem {
        name: "permit",
        source_tag: Some("demo"),
        argument_hint: None,
        description: Some("show fake permission card"),
    },
    SlashItem {
        name: "todo",
        source_tag: Some("demo"),
        argument_hint: None,
        description: Some("focus todo list"),
    },
    SlashItem {
        name: "quit",
        source_tag: Some("demo"),
        argument_hint: None,
        description: Some("exit shell"),
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentAction {
    Quit,
    Mode,
    Shortcuts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentCtx {
    Always,
}

enum Line {
    User {
        text: String,
        time: Option<String>,
    },
    Assistant {
        text: String,
        streaming: bool,
    },
    Thinking {
        text: String,
        expanded: bool,
        elapsed: Option<Duration>,
        streaming: bool,
    },
    System {
        text: String,
    },
    Tool {
        name: String,
        summary: String,
        detail: Option<String>,
        status: ToolStatus,
        kind: ToolKind,
        range: Option<(u32, u32)>,
        diffstat: Option<(u32, u32)>,
        expanded: bool,
        elapsed: Duration,
    },
    Marker {
        text: String,
    },
    Hook {
        name: String,
        count: u32,
    },
}

enum Turn {
    Idle,
    RunningTool {
        idx: usize,
        started: Instant,
    },
    Streaming {
        idx: usize,
        full: String,
        started: Instant,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewerDocument {
    Code,
    Markdown,
}

struct AgentApp {
    prompt: Prompt,
    search_prompt: Prompt,
    search_active: bool,
    items: Vec<Line>,
    transcript: TranscriptState,
    selection: TranscriptSelection,
    pending_transcript_click: Option<HitId>,
    clipboard: SystemClipboard,
    scrollback: Rect,
    slash: SlashPalette,
    perm: Option<PermissionState>,
    todos: Vec<CheckItem>,
    todo_list: ListPaneState,
    show_todo: bool,
    activity: bool,
    activity_started: Option<Instant>,
    turn: Turn,
    hints: Vec<HintItem>,
    registry: ActionRegistry<AgentAction, AgentCtx>,
    cwd: String,
    usage: UsageGauge,
    viewer: ContentViewer,
    viewer_search_prompt: Prompt,
    viewer_search_active: bool,
    viewer_open: bool,
    viewer_document: ViewerDocument,
    markdown_document: MarkdownDocument,
    code_document: CodeDocument,
    code_view: CodeView,
    viewer_area: Rect,
    viewer_status: Option<String>,
    viewer_theme: ResolvedTheme,
    pending_theme: Option<ThemeDefinition>,
    light_theme: bool,
}

impl AgentApp {
    fn new() -> Self {
        let registry = ActionRegistry::new(vec![
            ActionDef::new(
                AgentAction::Mode,
                "mode",
                key!(Tab, SHIFT),
                AgentCtx::Always,
            )
            .hint(0),
            ActionDef::new(
                AgentAction::Shortcuts,
                "shortcuts",
                key!('.', CONTROL),
                AgentCtx::Always,
            )
            .hint(1),
            ActionDef::new(AgentAction::Quit, "quit", key!('q'), AgentCtx::Always)
                .alts(vec![key!(Esc)])
                .hint(2)
                .pinned(),
        ]);
        let hints = hint_items(&registry, &[AgentCtx::Always]);
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "/Volumes/WorkSpace/Projects/Idea/gbtui".into());
        let mut app = Self {
            prompt: Prompt::new(),
            search_prompt: Prompt::new(),
            search_active: false,
            items: canned_session(),
            transcript: TranscriptState::new(),
            selection: TranscriptSelection::new(),
            pending_transcript_click: None,
            clipboard: SystemClipboard::new(),
            scrollback: Rect::default(),
            slash: SlashPalette::new(),
            perm: None,
            todos: vec![
                CheckItem {
                    label: "add failing test".into(),
                    status: CheckStatus::Done,
                },
                CheckItem {
                    label: "implement matcher".into(),
                    status: CheckStatus::Done,
                },
                CheckItem {
                    label: "write rustdoc".into(),
                    status: CheckStatus::Pending,
                },
            ],
            todo_list: ListPaneState::default(),
            show_todo: false,
            activity: false,
            activity_started: None,
            turn: Turn::Idle,
            hints,
            registry,
            cwd,
            usage: UsageGauge::new(275_000, 500_000),
            viewer: ContentViewer::new(),
            viewer_search_prompt: Prompt::new(),
            viewer_search_active: false,
            viewer_open: false,
            viewer_document: ViewerDocument::Code,
            markdown_document: MarkdownDocument::new(ContentDocumentId(1), 1, REPLY)
                .with_metadata("sample streaming markdown"),
            code_document: CodeDocument::new(ContentDocumentId(2), 1, CODE_SAMPLE)
                .with_language("rust")
                .with_path("crates/gbtui-core/src/search.rs")
                .with_metadata("crates/gbtui-core/src/search.rs"),
            code_view: CodeView::new(),
            viewer_area: Rect::default(),
            viewer_status: None,
            viewer_theme: ThemeResolver::explicit(Theme::night()).resolved().clone(),
            pending_theme: None,
            light_theme: false,
        };
        app.transcript.follow = true;
        app
    }

    fn items_as_transcript(items: &[Line]) -> Vec<TranscriptItem<'_>> {
        items
            .iter()
            .enumerate()
            .map(|(index, line)| Self::transcript_item(line, index))
            .collect()
    }

    fn transcript_item(line: &Line, index: usize) -> TranscriptItem<'_> {
        match line {
            Line::User { text, time } => TranscriptItem::User {
                author: None,
                text,
                time: time.as_deref(),
            },
            Line::Assistant { text, streaming } => TranscriptItem::Assistant {
                author: None,
                text,
                time: None,
                streaming: *streaming,
            },
            Line::Thinking {
                text,
                expanded,
                elapsed,
                streaming,
            } => TranscriptItem::Thinking {
                text,
                expanded: *expanded,
                elapsed: *elapsed,
                streaming: *streaming,
            },
            Line::System { text } => TranscriptItem::System { text },
            Line::Marker { text } => TranscriptItem::Marker { text },
            Line::Hook { name, count } => TranscriptItem::Hook {
                name,
                count: *count,
            },
            Line::Tool {
                name,
                summary,
                detail,
                status,
                kind,
                range,
                diffstat,
                expanded,
                elapsed,
            } => {
                let mut row = ToolRow::new(name, summary)
                    .kind(*kind)
                    .status(*status)
                    .expanded(*expanded)
                    .elapsed(*elapsed)
                    .index(index as u32)
                    .maybe_detail(detail.as_deref());
                if let Some((a, b)) = *range {
                    row = row.range(a, b);
                }
                if let Some((plus, minus)) = *diffstat {
                    row = row.diffstat(plus, minus);
                }
                TranscriptItem::Tool(row)
            }
        }
    }

    fn transcript_metrics(&mut self) -> (usize, u16) {
        let borrowed = Self::items_as_transcript(&self.items);
        (
            self.transcript
                .transcript_height(&borrowed, self.scrollback.width),
            self.scrollback.height,
        )
    }

    fn handle_transcript_item_pointer(&mut self, id: HitId, pointer: Pointer) -> bool {
        let Some(index) = parse_item_id(id).map(|index| index as usize) else {
            return false;
        };
        let thinking = matches!(self.items.get(index), Some(Line::Thinking { .. }));
        let selection_changed = {
            let borrowed = Self::items_as_transcript(&self.items);
            self.selection
                .handle_pointer(pointer, &self.transcript, &borrowed, self.scrollback)
        };
        let mut changed = selection_changed;
        match pointer {
            Pointer::Down {
                button: MouseButton::Left,
                ..
            } => {
                self.pending_transcript_click = thinking.then_some(id);
                changed = true;
            }
            Pointer::Drag {
                button: MouseButton::Left,
                ..
            } => {
                changed |= self.pending_transcript_click.take().is_some();
            }
            Pointer::Up {
                button: MouseButton::Left,
                ..
            } if self.pending_transcript_click == Some(id) => {
                self.pending_transcript_click = None;
                changed |= self.toggle_transcript_item(id);
            }
            _ => {}
        }
        changed
    }

    fn handle_tool_header_pointer(&mut self, id: HitId, pointer: Pointer) -> bool {
        let selection_changed = {
            let borrowed = Self::items_as_transcript(&self.items);
            self.selection
                .handle_pointer(pointer, &self.transcript, &borrowed, self.scrollback)
        };
        let mut changed = selection_changed;
        match pointer {
            Pointer::Down {
                button: MouseButton::Left,
                ..
            } => {
                self.pending_transcript_click = Some(id);
                changed = true;
            }
            Pointer::Drag {
                button: MouseButton::Left,
                ..
            } => {
                changed |= self.pending_transcript_click.take().is_some();
            }
            Pointer::Up {
                button: MouseButton::Left,
                ..
            } if self.pending_transcript_click == Some(id) => {
                self.pending_transcript_click = None;
                changed |= self.toggle_transcript_item(id);
            }
            _ => {}
        }
        changed
    }

    fn toggle_transcript_item(&mut self, id: HitId) -> bool {
        let index = parse_tool_id(id)
            .or_else(|| parse_item_id(id))
            .map(|index| index as usize);
        let Some(index) = index else {
            return false;
        };
        let borrowed = Self::items_as_transcript(&self.items);
        self.transcript.toggle_display(&borrowed, index)
    }

    fn toggle_last_thinking(&mut self) -> bool {
        let Some(index) = self.items.iter().rposition(|line| {
            matches!(
                line,
                Line::Thinking {
                    streaming: false,
                    ..
                }
            )
        }) else {
            return false;
        };
        let borrowed = Self::items_as_transcript(&self.items);
        self.transcript.toggle_display(&borrowed, index)
    }

    fn toggle_display_target(&mut self, target: TranscriptDisplayTarget) -> bool {
        let borrowed = Self::items_as_transcript(&self.items);
        let mut any = false;
        let mut all_expanded = true;
        for (index, item) in borrowed.iter().enumerate() {
            let selected = match target {
                TranscriptDisplayTarget::Item(item) => index == item,
                TranscriptDisplayTarget::All => {
                    matches!(item, TranscriptItem::Thinking { .. })
                        || matches!(item, TranscriptItem::Tool(row) if row.detail.is_some_and(|detail| !detail.is_empty()))
                }
                TranscriptDisplayTarget::Thinking => {
                    matches!(item, TranscriptItem::Thinking { .. })
                }
            };
            if selected {
                any = true;
                all_expanded &=
                    self.transcript.display_mode(index, item) == TranscriptDisplay::Expanded;
            }
        }
        if !any {
            return false;
        }
        self.transcript.set_display(
            &borrowed,
            target,
            if all_expanded {
                TranscriptDisplay::Collapsed
            } else {
                TranscriptDisplay::Expanded
            },
        )
    }

    fn open_search(&mut self) {
        self.search_prompt.set_text(self.transcript.search_query());
        self.search_active = true;
    }

    fn sync_transcript_search(&mut self) -> bool {
        self.transcript.set_search_query(self.search_prompt.text())
    }

    fn navigate_transcript(
        &mut self,
        target: TranscriptNavigation,
        direction: TranscriptDirection,
    ) -> bool {
        let borrowed = Self::items_as_transcript(&self.items);
        self.transcript
            .navigate(
                &borrowed,
                self.scrollback.width,
                self.scrollback.height,
                target,
                direction,
            )
            .is_some()
    }

    fn navigate_search(&mut self, direction: TranscriptDirection) -> bool {
        let borrowed = Self::items_as_transcript(&self.items);
        match direction {
            TranscriptDirection::Forward => self.transcript.next_search_match(
                &borrowed,
                self.scrollback.width,
                self.scrollback.height,
            ),
            TranscriptDirection::Backward => self.transcript.previous_search_match(
                &borrowed,
                self.scrollback.width,
                self.scrollback.height,
            ),
        }
        .is_some()
    }

    fn copy_transcript_selection(&mut self) -> bool {
        let text = {
            let borrowed = Self::items_as_transcript(&self.items);
            self.selection.selected_text(&self.transcript, &borrowed)
        };
        if let Some(text) = text {
            self.clipboard.set(&text);
            return true;
        }
        false
    }

    fn sync_slash(&mut self) {
        match slash_query(self.prompt.text()) {
            Some(q) => self.slash.open_with(q),
            None => self.slash.close(),
        }
    }

    fn run_slash(&mut self, name: &str) -> Outcome {
        self.prompt.set_text("");
        self.slash.close();
        match name {
            "help" => {
                self.items.push(Line::System {
                    text: "/help  /clear  /compact  /model  /permit  /todo  /quit".into(),
                });
                Outcome::Changed
            }
            "clear" => {
                self.items.clear();
                self.selection.clear();
                self.pending_transcript_click = None;
                self.turn = Turn::Idle;
                self.activity = false;
                Outcome::Changed
            }
            "compact" => {
                self.items.push(Line::System {
                    text: "compact skipped (demo · no session store)".into(),
                });
                Outcome::Changed
            }
            "model" => {
                self.items.push(Line::System {
                    text: "model: grok-4 (demo)".into(),
                });
                Outcome::Changed
            }
            "permit" => {
                let mut state = PermissionState::embedded();
                state.extra.push(("Allow for session".into(), PERM_ALWAYS));
                self.perm = Some(state);
                Outcome::Changed
            }
            "todo" => {
                self.show_todo = !self.show_todo;
                Outcome::Changed
            }
            "quit" => Outcome::Quit,
            other => {
                self.items.push(Line::System {
                    text: format!("unknown /{other}"),
                });
                Outcome::Changed
            }
        }
    }

    fn send(&mut self, now: Instant) {
        let Some(text) = self.prompt.try_send() else {
            return;
        };
        self.prompt.set_text("");
        self.slash.close();
        if let Some(rest) = text.strip_prefix('/') {
            let cmd = rest.split_whitespace().next().unwrap_or(rest);
            let _ = self.run_slash(cmd);
            return;
        }
        self.items.push(Line::User {
            text: text.clone(),
            time: Some("now".into()),
        });
        self.items.push(Line::Tool {
            name: "bash".into(),
            summary: "cargo test --workspace".into(),
            detail: Some("running…".into()),
            status: ToolStatus::Running,
            kind: ToolKind::Run,
            range: None,
            diffstat: None,
            expanded: false,
            elapsed: Duration::ZERO,
        });
        let idx = self.items.len() - 1;
        self.turn = Turn::RunningTool { idx, started: now };
        self.activity = true;
        self.activity_started = Some(now);
        self.transcript.follow = true;
    }

    fn resolve_perm(&mut self, allowed: bool) {
        self.items.push(Line::System {
            text: if allowed {
                "allowed".into()
            } else {
                "denied".into()
            },
        });
        self.perm = None;
    }

    fn advance_turn(&mut self, now: Instant) {
        match self.turn {
            Turn::Idle => {}
            Turn::RunningTool { idx, started } => {
                if let Some(Line::Tool {
                    status, elapsed, ..
                }) = self.items.get_mut(idx)
                {
                    *elapsed = now.saturating_duration_since(started);
                    if now.saturating_duration_since(started) >= TOOL_TTL {
                        *status = ToolStatus::Done;
                        self.items.push(Line::Assistant {
                            text: String::new(),
                            streaming: true,
                        });
                        let aidx = self.items.len() - 1;
                        self.turn = Turn::Streaming {
                            idx: aidx,
                            full: REPLY.into(),
                            started: now,
                        };
                    }
                }
            }
            Turn::Streaming {
                idx,
                ref full,
                started,
            } => {
                let n = ((now.saturating_duration_since(started).as_secs_f32() * STREAM_CPS)
                    as usize)
                    .min(full.chars().count());
                let shown: String = full.chars().take(n).collect();
                let done = n >= full.chars().count();
                if let Some(Line::Assistant { text, streaming }) = self.items.get_mut(idx) {
                    *text = shown;
                    if done {
                        *streaming = false;
                    }
                }
                if done {
                    self.turn = Turn::Idle;
                    self.activity = false;
                    self.activity_started = None;
                }
            }
        }
    }

    fn tick_fast(&self) -> bool {
        self.activity
            || matches!(self.turn, Turn::RunningTool { .. } | Turn::Streaming { .. })
            || self.items.iter().any(|l| {
                matches!(
                    l,
                    Line::Assistant {
                        streaming: true,
                        ..
                    } | Line::Tool {
                        status: ToolStatus::Running,
                        ..
                    }
                )
            })
    }

    fn sync_viewer_search(&mut self) -> bool {
        self.viewer
            .set_search_query(self.viewer_search_prompt.text())
    }

    fn toggle_viewer_projection(&mut self) -> bool {
        let theme = &self.viewer_theme;
        let width = self.viewer_area.width.max(1);
        match self.viewer_document {
            ViewerDocument::Code => {
                let document = self.code_view.document_resolved(&self.code_document, theme);
                self.viewer.toggle_view(document)
            }
            ViewerDocument::Markdown => {
                let document = self
                    .markdown_document
                    .document_resolved(theme, width, false);
                self.viewer.toggle_view(document)
            }
        }
    }

    fn navigate_viewer_search(&mut self, backwards: bool) -> bool {
        let theme = &self.viewer_theme;
        let width = self.viewer_area.width.max(1);
        let height = self.viewer_area.height;
        match self.viewer_document {
            ViewerDocument::Code => {
                let document = self.code_view.document_resolved(&self.code_document, theme);
                self.viewer.prepare(document, width);
                self.viewer.next_search_match(document, height, backwards)
            }
            ViewerDocument::Markdown => {
                let document = self
                    .markdown_document
                    .document_resolved(theme, width, false);
                self.viewer.prepare(document, width);
                self.viewer.next_search_match(document, height, backwards)
            }
        }
    }

    fn copy_viewer(&mut self, target: ContentCopyTarget) -> bool {
        let theme = &self.viewer_theme;
        let width = self.viewer_area.width.max(1);
        let text = match self.viewer_document {
            ViewerDocument::Code => {
                let document = self.code_view.document_resolved(&self.code_document, theme);
                self.viewer.extract(document, target)
            }
            ViewerDocument::Markdown => {
                let document = self
                    .markdown_document
                    .document_resolved(theme, width, false);
                self.viewer.extract(document, target)
            }
        };
        let Some(text) = text else {
            self.viewer_status = Some("No text is available for that copy target".into());
            return false;
        };
        self.clipboard.set(&text);
        self.viewer_status = Some(match target {
            ContentCopyTarget::Selection => "Selected rows sent to SystemClipboard".into(),
            ContentCopyTarget::Source => "Source sent to SystemClipboard".into(),
            ContentCopyTarget::Rendered => "Rendered text sent to SystemClipboard".into(),
            ContentCopyTarget::Metadata => "Metadata sent to SystemClipboard".into(),
        });
        true
    }

    fn handle_viewer_key(&mut self, key: crossterm::event::KeyEvent) -> Outcome {
        if self.viewer_search_active {
            if self.viewer_search_prompt.has_active_addon() {
                let event = self.viewer_search_prompt.handle_key(&key);
                if event.is_handled() {
                    self.sync_viewer_search();
                    return Outcome::Changed;
                }
                return Outcome::Unchanged;
            }
            if key!(Esc).matches(&key) {
                self.viewer_search_active = false;
                return Outcome::Changed;
            }
            match self.viewer_search_prompt.route_enter(&key) {
                EnterOutcome::Submit => {
                    self.viewer_search_active = false;
                    Outcome::Changed
                }
                EnterOutcome::NewlineInserted | EnterOutcome::Handled => {
                    self.sync_viewer_search();
                    Outcome::Changed
                }
                EnterOutcome::PassThrough => {
                    let event = self.viewer_search_prompt.handle_key(&key);
                    if event.is_handled() {
                        self.sync_viewer_search();
                        Outcome::Changed
                    } else {
                        Outcome::Unchanged
                    }
                }
            }
        } else if key!(Esc).matches(&key) {
            self.viewer_open = false;
            self.viewer_status = None;
            Outcome::Changed
        } else if key!('/').matches(&key) {
            self.viewer_search_prompt
                .set_text(self.viewer.search_query());
            self.viewer_search_active = true;
            Outcome::Changed
        } else if key!('r').matches(&key) {
            if self.toggle_viewer_projection() {
                Outcome::Changed
            } else {
                Outcome::Unchanged
            }
        } else if key!('w').matches(&key) {
            self.viewer.toggle_wrapping();
            Outcome::Changed
        } else if key!('m').matches(&key) {
            self.viewer_document = ViewerDocument::Markdown;
            self.viewer_status = Some("Markdown sample".into());
            Outcome::Changed
        } else if key!('c').matches(&key) {
            self.viewer_document = ViewerDocument::Code;
            self.viewer_status = Some("Code sample with non-copyable gutter".into());
            Outcome::Changed
        } else if key!('n').matches(&key) {
            if self.navigate_viewer_search(false) {
                Outcome::Changed
            } else {
                Outcome::Unchanged
            }
        } else if key!('N').matches(&key) {
            if self.navigate_viewer_search(true) {
                Outcome::Changed
            } else {
                Outcome::Unchanged
            }
        } else if key!('c', CONTROL | SHIFT).matches(&key) {
            self.copy_viewer(ContentCopyTarget::Selection);
            Outcome::Changed
        } else if key!('c', CONTROL | ALT).matches(&key) {
            self.copy_viewer(ContentCopyTarget::Source);
            Outcome::Changed
        } else if key!('r', CONTROL | ALT).matches(&key) {
            self.copy_viewer(ContentCopyTarget::Rendered);
            Outcome::Changed
        } else if key!('m', CONTROL | ALT).matches(&key) {
            self.copy_viewer(ContentCopyTarget::Metadata);
            Outcome::Changed
        } else if self.viewer.handle_key(&key, self.viewer_area.height) {
            Outcome::Changed
        } else {
            Outcome::Unchanged
        }
    }

    fn handle_viewer_event(&mut self, ev: AppEvent<()>) -> Outcome {
        if let AppEvent::Hit(id, pointer) = &ev
            && self.viewer_search_active
            && self
                .viewer_search_prompt
                .handle_hit(*id, *pointer)
                .is_handled()
        {
            self.sync_viewer_search();
            return Outcome::Changed;
        }
        match ev {
            AppEvent::Hit(_, Pointer::Move { .. }) | AppEvent::Miss(Pointer::Move { .. }) => {
                Outcome::Changed
            }
            AppEvent::Hit(_, Pointer::Scroll { delta_lines, .. }) => {
                if self.viewer.scroll(delta_lines, self.viewer_area.height) {
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            AppEvent::Hit(id, pointer) => {
                if self
                    .viewer
                    .handle_pointer(id, pointer, self.viewer_area, false)
                {
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            AppEvent::Miss(pointer @ Pointer::Cancel { .. }) => {
                if self.viewer.handle_pointer(
                    HitId::named("content-viewer-cancel"),
                    pointer,
                    self.viewer_area,
                    false,
                ) {
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            AppEvent::Key(key) => self.handle_viewer_key(key),
            AppEvent::Paste(text) if self.viewer_search_active => {
                if self.viewer_search_prompt.handle_paste(&text).is_handled() {
                    self.sync_viewer_search();
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            AppEvent::Resize { .. } => Outcome::Changed,
            _ => Outcome::Unchanged,
        }
    }

    fn render_content_viewer(&mut self, area: Rect, f: &mut Frame, cx: &mut FrameCtx<'_>) {
        let panel = Rect {
            x: area.x.saturating_add((area.width > 4) as u16),
            y: area.y.saturating_add((area.height > 4) as u16),
            width: area.width.saturating_sub(2).max(1),
            height: area.height.saturating_sub(2).max(1),
        };
        cx.occlude(panel);
        f.render_widget(Clear, panel);
        let title = match self.viewer_document {
            ViewerDocument::Code => " Content viewer · code + gutter ",
            ViewerDocument::Markdown => " Content viewer · Markdown ",
        };
        let block = Block::bordered()
            .title(title)
            .style(cx.style(ThemeRole::Elevated, ThemeState::Normal));
        let inner = block.inner(panel);
        f.render_widget(block, panel);
        if inner.height == 0 || inner.width == 0 {
            return;
        }
        let requested_footer = if self.viewer_search_active { 3 } else { 1 };
        let footer_height = requested_footer.min(inner.height.saturating_sub(1));
        let [content, footer] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(footer_height)]).areas(inner);
        self.viewer_area = content;
        match self.viewer_document {
            ViewerDocument::Code => {
                let document = self
                    .code_view
                    .document_resolved(&self.code_document, &self.viewer_theme);
                self.viewer.render(document, content, f.buffer_mut(), cx);
            }
            ViewerDocument::Markdown => {
                let document = self.markdown_document.document_resolved(
                    &self.viewer_theme,
                    content.width.max(1),
                    false,
                );
                self.viewer.render(document, content, f.buffer_mut(), cx);
            }
        }
        if footer.height == 0 {
            return;
        }
        if self.viewer_search_active {
            let style = PromptStyle {
                focused: true,
                placeholder: "Search viewer",
                placeholder_when_focused: true,
                show_accent_line: false,
                show_borders: true,
                caption: Some("Enter closes · n/N navigate · Esc returns to viewer"),
                chrome: true,
                ..PromptStyle::default()
            };
            self.viewer_search_prompt
                .render(footer, f.buffer_mut(), cx, &style);
        } else {
            let status = self.viewer_status.as_deref().unwrap_or(
                "v range · Shift+↑/↓ or Shift+click extends · Ctrl+Shift+C copies range · Esc closes",
            );
            f.render_widget(
                Paragraph::new(status).style(
                    cx.style(ThemeRole::Elevated, ThemeState::Normal)
                        .patch(cx.style(ThemeRole::Muted, ThemeState::Normal)),
                ),
                footer,
            );
        }
    }

    fn perm_card() -> PermissionCard<'static> {
        PermissionCard::new(
            "Run cargo test --workspace",
            "The working tree has uncommitted changes.",
        )
    }
}

fn canned_session() -> Vec<Line> {
    vec![
        Line::User {
            text: "write a failing test for TextMatcher then make it pass".into(),
            time: Some("3:13 PM".into()),
        },
        Line::Thinking {
            text: "I'll inspect search.rs, add a unit test, then implement smart-case substring matching.".into(),
            expanded: false,
            elapsed: Some(Duration::from_millis(800)),
            streaming: false,
        },
        Line::Tool {
            name: "read".into(),
            summary: "crates/gbtui-core/src/search.rs".into(),
            detail: Some(
                "pub struct TextMatcher {\n    query: String,\n    sensitive: bool,\n}"
                    .into(),
            ),
            status: ToolStatus::Done,
            kind: ToolKind::Read,
            range: Some((1, 40)),
            diffstat: None,
            expanded: false,
            elapsed: Duration::from_millis(280),
        },
        Line::Tool {
            name: "edit".into(),
            summary: "crates/gbtui-core/src/search.rs".into(),
            detail: Some(
                "diff --git a/crates/gbtui-core/src/search.rs b/crates/gbtui-core/src/search.rs\n--- a/crates/gbtui-core/src/search.rs\n+++ b/crates/gbtui-core/src/search.rs\n@@ -40,1 +40,2 @@ impl TextMatcher {\n-    self.query == hay\n+    self.query.is_empty() || hay.contains(&self.query)\n+    // lower-case queries remain insensitive\n@@ -88,2 +89,2 @@ mod tests {\n     #[test]\n-    fn exact_match_only() {}\n+    fn uppercase_query_is_sensitive() {}\n"
                    .into(),
            ),
            status: ToolStatus::Done,
            kind: ToolKind::Edit,
            range: None,
            diffstat: Some((12, 3)),
            expanded: false,
            elapsed: Duration::from_millis(640),
        },
        Line::Tool {
            name: "bash".into(),
            summary: "cargo test -p gbtui-core search".into(),
            detail: Some(
                "running 5 tests\ntest search::smart_case_insensitive_without_uppercase ... ok\ntest search::uppercase_query_is_sensitive ... ok\n\ntest result: ok. 5 passed"
                    .into(),
            ),
            status: ToolStatus::Done,
            kind: ToolKind::Run,
            range: None,
            diffstat: None,
            expanded: false,
            elapsed: Duration::from_millis(1420),
        },
        Line::Assistant {
            text: "The matcher is **green**.\n\n# What changed\n- smart-case substring (uppercase in the query forces sensitive)\n- `n` / `N` wrap through hits\n- see the [search contract](https://example.com/search)\n\n| Path | Status |\n| --- | --- |\n| matcher | passing |\n| copy mapping | rich |\n\n```rust\nfn is_match(&self, hay: &str) -> bool {\n    self.query.is_empty() || hay.contains(&self.query)\n}\n```\n\nNo `regex` crate — still a core primitive.\n\nTry `cargo run -p agent` to reopen this shell.".into(),
            streaming: false,
        },
        Line::Marker {
            text: "Worked for 14m33s".into(),
        },
        Line::Hook {
            name: "session_end".into(),
            count: 2,
        },
        Line::Hook {
            name: "stop".into(),
            count: 3,
        },
        Line::Hook {
            name: "session_start".into(),
            count: 3,
        },
        Line::Tool {
            name: "web_search".into(),
            summary: "fake host result: transcript navigation".into(),
            detail: Some("sample-web-search: local fixture only".into()),
            status: ToolStatus::Done,
            kind: ToolKind::WebSearch,
            range: None,
            diffstat: None,
            expanded: true,
            elapsed: Duration::from_millis(30),
        },
        Line::Tool {
            name: "web_fetch".into(),
            summary: "fake://docs/transcript".into(),
            detail: Some(
                "sample-web-fetch: fixture body\nNo network request was made; this is host-provided demo text."
                    .into(),
            ),
            status: ToolStatus::Done,
            kind: ToolKind::WebFetch,
            range: None,
            diffstat: None,
            expanded: true,
            elapsed: Duration::from_millis(20),
        },
        Line::Tool {
            name: "memory_search".into(),
            summary: "fake session note".into(),
            detail: Some("sample-memory-search: remembered local preference".into()),
            status: ToolStatus::Done,
            kind: ToolKind::MemorySearch,
            range: None,
            diffstat: None,
            expanded: true,
            elapsed: Duration::from_millis(4),
        },
        Line::Tool {
            name: "use_tool".into(),
            summary: "fake formatter adapter".into(),
            detail: Some("sample-use-tool: adapter response is static".into()),
            status: ToolStatus::Done,
            kind: ToolKind::UseTool,
            range: None,
            diffstat: None,
            expanded: true,
            elapsed: Duration::from_millis(6),
        },
        Line::Tool {
            name: "message".into(),
            summary: "fake teammate update".into(),
            detail: Some("sample-message: no session or delivery backend".into()),
            status: ToolStatus::Done,
            kind: ToolKind::Message,
            range: None,
            diffstat: None,
            expanded: true,
            elapsed: Duration::from_millis(2),
        },
        Line::Tool {
            name: "hook".into(),
            summary: "fake lifecycle callback".into(),
            detail: Some("sample-hook: rendered host event".into()),
            status: ToolStatus::Done,
            kind: ToolKind::Hook,
            range: None,
            diffstat: None,
            expanded: true,
            elapsed: Duration::from_millis(1),
        },
    ]
}

impl App for AgentApp {
    type Msg = ();

    fn take_theme_request(&mut self) -> Option<ThemeDefinition> {
        self.pending_theme.take()
    }

    fn tick_demand(&self) -> TickDemand {
        if self.tick_fast() {
            TickDemand::Fast
        } else {
            TickDemand::None
        }
    }

    fn handle(&mut self, ev: AppEvent<()>) -> Outcome {
        if let AppEvent::Key(key) = &ev
            && key.kind != KeyEventKind::Release
            && key.code == KeyCode::F(4)
        {
            self.light_theme = !self.light_theme;
            self.pending_theme = Some(ThemeDefinition::from(if self.light_theme {
                ThemeKind::Day
            } else {
                ThemeKind::Night
            }));
            return Outcome::Changed;
        }
        if self.viewer_open {
            return self.handle_viewer_event(ev);
        }
        if matches!(
            &ev,
            AppEvent::Hit(_, Pointer::Cancel { .. }) | AppEvent::Miss(Pointer::Cancel { .. })
        ) {
            self.pending_transcript_click = None;
            let canceled = {
                let borrowed = Self::items_as_transcript(&self.items);
                let pointer = match &ev {
                    AppEvent::Hit(_, pointer) | AppEvent::Miss(pointer) => *pointer,
                    _ => unreachable!("the match above only accepts pointer cancellation"),
                };
                self.selection
                    .handle_pointer(pointer, &self.transcript, &borrowed, self.scrollback)
            };
            return if canceled {
                Outcome::Changed
            } else {
                Outcome::Unchanged
            };
        }
        if let AppEvent::Hit(id, pointer) = &ev {
            let event = if self.search_active {
                self.search_prompt.handle_hit(*id, *pointer)
            } else {
                self.prompt.handle_hit(*id, *pointer)
            };
            if event.is_handled() {
                if self.search_active {
                    self.sync_transcript_search();
                } else {
                    self.sync_slash();
                }
                return Outcome::Changed;
            }
        }
        match ev {
            AppEvent::Hit(_, Pointer::Move { .. }) | AppEvent::Miss(Pointer::Move { .. }) => {
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if id == PERM_ALLOW || id == PERM_ALWAYS => {
                self.resolve_perm(true);
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if id == PERM_DENY => {
                self.resolve_perm(false);
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if id == ACTIVITY_STOP => {
                self.turn = Turn::Idle;
                self.activity = false;
                self.activity_started = None;
                Outcome::Changed
            }
            AppEvent::Hit(
                id,
                pointer @ (Pointer::Down { .. }
                | Pointer::Drag { .. }
                | Pointer::Up { .. }
                | Pointer::Cancel { .. }),
            ) if parse_item_id(id).is_some() => {
                if self.perm.is_some()
                    || self.slash.is_open()
                    || self.handle_transcript_item_pointer(id, pointer)
                {
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            AppEvent::Hit(
                id,
                pointer @ (Pointer::Down { .. }
                | Pointer::Drag { .. }
                | Pointer::Up { .. }
                | Pointer::Cancel { .. }),
            ) if parse_tool_id(id).is_some() => {
                if self.perm.is_some()
                    || self.slash.is_open()
                    || self.handle_tool_header_pointer(id, pointer)
                {
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            AppEvent::Hit(id, pointer @ Pointer::Down { .. }) => {
                if let Some(i) = parse_check_row(id) {
                    if let Some(item) = self.todos.get_mut(i) {
                        item.status = item.status.cycle();
                    }
                    return Outcome::Changed;
                }
                if let Some((_, name)) = self.slash.pick_hit(id, pointer.position().1, &SLASH) {
                    return self.run_slash(name);
                }
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Scroll { delta_lines, .. }) => {
                if self.slash.is_open() && SlashPalette::handles_hit(id) {
                    self.slash.handle_scroll(delta_lines, &SLASH);
                    return Outcome::Changed;
                }
                if id != TRANSCRIPT_BODY
                    && parse_item_id(id).is_none()
                    && parse_tool_id(id).is_none()
                {
                    return Outcome::Unchanged;
                }
                let (content_rows, viewport_rows) = self.transcript_metrics();
                self.transcript
                    .handle_scroll(delta_lines, content_rows, viewport_rows);
                Outcome::Changed
            }
            AppEvent::Tick => {
                self.advance_turn(Instant::now());
                Outcome::Changed
            }
            AppEvent::Resize { .. } => Outcome::Changed,
            AppEvent::Paste(text) => {
                if self.perm.is_some() {
                    return Outcome::Unchanged;
                }
                if self.search_active {
                    if self.search_prompt.handle_paste(&text).is_handled() {
                        self.sync_transcript_search();
                        return Outcome::Changed;
                    }
                    return Outcome::Unchanged;
                }
                if self.prompt.handle_paste(&text).is_handled() {
                    self.sync_slash();
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            AppEvent::Key(key) => {
                if let Some(state) = self.perm.as_mut() {
                    match Self::perm_card().handle_key(&key, state) {
                        PermAction::Allow | PermAction::Extra(_) => self.resolve_perm(true),
                        PermAction::Deny => self.resolve_perm(false),
                        PermAction::Unhandled => {}
                    }
                    return Outcome::Changed;
                }
                if self.search_active {
                    if self.search_prompt.has_active_addon() {
                        let event = self.search_prompt.handle_key(&key);
                        if event.is_handled() {
                            self.sync_transcript_search();
                            return Outcome::Changed;
                        }
                        return Outcome::Unchanged;
                    }
                    if key!(Esc).matches(&key) {
                        self.search_active = false;
                        return Outcome::Changed;
                    }
                    match self.search_prompt.route_enter(&key) {
                        EnterOutcome::Submit => {
                            self.search_active = false;
                            return Outcome::Changed;
                        }
                        EnterOutcome::NewlineInserted | EnterOutcome::Handled => {
                            self.sync_transcript_search();
                            return Outcome::Changed;
                        }
                        EnterOutcome::PassThrough => {
                            let event = self.search_prompt.handle_key(&key);
                            if event.is_handled() {
                                self.sync_transcript_search();
                                return Outcome::Changed;
                            }
                            return Outcome::Unchanged;
                        }
                    }
                }
                if self.prompt.has_active_addon() {
                    let event = self.prompt.handle_key(&key);
                    if event.is_handled() {
                        self.sync_slash();
                        return Outcome::Changed;
                    }
                    return Outcome::Unchanged;
                }
                if key!('f', CONTROL).matches(&key) {
                    self.open_search();
                    return Outcome::Changed;
                }
                if key!('o', CONTROL).matches(&key) {
                    self.viewer_open = true;
                    self.viewer_search_active = false;
                    self.viewer_status = Some(
                        "Code · v then Shift+↑/↓ or Shift+click range · r raw/rendered · w wrap · / search · Ctrl+Shift+C copy"
                            .into(),
                    );
                    return Outcome::Changed;
                }
                if self.slash.is_open() || slash_query(self.prompt.text()).is_some() {
                    match self.slash.handle_key(&key, &SLASH) {
                        SlashAction::Picked { name, .. } => return self.run_slash(name),
                        SlashAction::Closed | SlashAction::Changed => return Outcome::Changed,
                        SlashAction::Unhandled => {}
                    }
                }
                if self.selection.has_selection() && key!('c', CONTROL | SHIFT).matches(&key) {
                    return if self.copy_transcript_selection() {
                        Outcome::Changed
                    } else {
                        Outcome::Unchanged
                    };
                }
                if key!(Esc).matches(&key) && self.selection.has_selection() {
                    self.selection.clear();
                    return Outcome::Changed;
                }
                if key!('e', CONTROL | SHIFT).matches(&key) {
                    return if self.toggle_display_target(TranscriptDisplayTarget::All) {
                        Outcome::Changed
                    } else {
                        Outcome::Unchanged
                    };
                }
                if key!('e', CONTROL | ALT).matches(&key) {
                    return if self.toggle_display_target(TranscriptDisplayTarget::Thinking) {
                        Outcome::Changed
                    } else {
                        Outcome::Unchanged
                    };
                }
                if key!('e', CONTROL).matches(&key) {
                    return if self.toggle_last_thinking() {
                        Outcome::Changed
                    } else {
                        Outcome::Unchanged
                    };
                }
                if self.prompt.is_empty() && !self.transcript.search_query().is_empty() {
                    if key!('n').matches(&key) {
                        return if self.navigate_search(TranscriptDirection::Forward) {
                            Outcome::Changed
                        } else {
                            Outcome::Unchanged
                        };
                    }
                    if key!('N').matches(&key) {
                        return if self.navigate_search(TranscriptDirection::Backward) {
                            Outcome::Changed
                        } else {
                            Outcome::Unchanged
                        };
                    }
                }
                if self.prompt.is_empty() {
                    let navigation = if key!(']').matches(&key) {
                        Some((TranscriptNavigation::UserTurn, TranscriptDirection::Forward))
                    } else if key!('[').matches(&key) {
                        Some((
                            TranscriptNavigation::UserTurn,
                            TranscriptDirection::Backward,
                        ))
                    } else if key!('}', SHIFT).matches(&key) || key!('}').matches(&key) {
                        Some((
                            TranscriptNavigation::AssistantResponse,
                            TranscriptDirection::Forward,
                        ))
                    } else if key!('{', SHIFT).matches(&key) || key!('{').matches(&key) {
                        Some((
                            TranscriptNavigation::AssistantResponse,
                            TranscriptDirection::Backward,
                        ))
                    } else {
                        None
                    };
                    if let Some((target, direction)) = navigation {
                        return if self.navigate_transcript(target, direction) {
                            Outcome::Changed
                        } else {
                            Outcome::Unchanged
                        };
                    }
                }
                match self.prompt.route_enter(&key) {
                    EnterOutcome::Submit => {
                        self.send(Instant::now());
                        Outcome::Changed
                    }
                    EnterOutcome::NewlineInserted | EnterOutcome::Handled => Outcome::Changed,
                    EnterOutcome::PassThrough => {
                        if let Some(AgentAction::Quit) =
                            self.registry.lookup(&key, AgentCtx::Always)
                            && (key!(Esc).matches(&key) || self.prompt.is_empty())
                        {
                            return Outcome::Quit;
                        }
                        if self.prompt.is_empty() {
                            match handle_checklist_key(
                                &mut self.todo_list,
                                &key,
                                self.todos.len(),
                                8,
                            ) {
                                gbtui::CheckAction::Cycled(i) => {
                                    if let Some(item) = self.todos.get_mut(i) {
                                        item.status = item.status.cycle();
                                    }
                                    return Outcome::Changed;
                                }
                                gbtui::CheckAction::Changed => return Outcome::Changed,
                                gbtui::CheckAction::Unhandled => {}
                            }
                            let (content_rows, viewport_rows) = self.transcript_metrics();
                            if self
                                .transcript
                                .handle_key(&key, content_rows, viewport_rows)
                            {
                                return Outcome::Changed;
                            }
                        }
                        let event = self.prompt.handle_key(&key);
                        if event.is_handled() {
                            self.sync_slash();
                            Outcome::Changed
                        } else {
                            Outcome::Unchanged
                        }
                    }
                }
            }
            _ => Outcome::Unchanged,
        }
    }

    fn draw(&mut self, f: &mut Frame, cx: &mut FrameCtx) {
        if let Some(theme) = cx.resolved_theme()
            && theme != &self.viewer_theme
        {
            self.viewer_theme = theme.clone();
        }
        let area = f.area();
        f.render_widget(Clear, area);
        f.render_widget(
            ratatui::widgets::Block::default()
                .style(cx.style(ThemeRole::Surface, ThemeState::Normal)),
            area,
        );

        let perm_h = self.perm.as_ref().map_or(0, |state| {
            Self::perm_card().desired_height(area.width, state)
        });
        let todo_h = if self.show_todo && !self.todos.is_empty() {
            (self.todos.len() as u16).saturating_add(1).min(6)
        } else {
            0
        };
        let turn_h = if self.activity { 1 } else { 0 };
        let layout = AgentLayout::compute(
            area,
            AgentLayoutParams {
                todo_h,
                turn_h,
                banner_h: if self.search_active { 3 } else { 0 },
                follow_h: 0,
                perm_h,
                prompt_h: 4,
            },
        );

        let gauge_w = 14u16.min(layout.status.width / 3);
        let [status_l, status_r] =
            Layout::horizontal([Constraint::Min(8), Constraint::Length(gauge_w)])
                .areas(layout.status);
        StatusBar::new(&self.cwd).render(status_l, f.buffer_mut(), cx);
        self.usage.render(status_r, f.buffer_mut(), cx);

        if todo_h > 0 {
            render_checklist(
                &mut self.todo_list,
                &self.todos,
                layout.todo,
                f.buffer_mut(),
                cx,
            );
        }

        self.scrollback = layout.scrollback;
        let borrowed = Self::items_as_transcript(&self.items);
        render_transcript_with_selection(
            &mut self.transcript,
            &mut self.selection,
            &borrowed,
            layout.scrollback,
            f.buffer_mut(),
            cx,
        );

        if self.activity {
            let total = self
                .activity_started
                .map(|t| cx.anim().now().saturating_duration_since(t))
                .unwrap_or_default();
            ActivityRow::new("working")
                .total(total)
                .render(layout.turn_status, f.buffer_mut(), cx);
        }

        if let Some(state) = self.perm.as_mut()
            && layout.perm.height > 0
        {
            Self::perm_card().render(layout.perm, f.buffer_mut(), cx, state);
        }

        if self.search_active && layout.banner.height > 0 {
            let search_style = PromptStyle {
                focused: true,
                placeholder: "Search transcript",
                placeholder_when_focused: true,
                show_accent_line: false,
                show_borders: true,
                caption: Some("Search · Enter closes · n/N moves matches"),
                chrome: true,
                ..PromptStyle::default()
            };
            self.search_prompt
                .render(layout.banner, f.buffer_mut(), cx, &search_style);
        }

        let style = PromptStyle {
            focused: self.perm.is_none() && !self.search_active,
            placeholder: "",
            placeholder_when_focused: false,
            show_accent_line: false,
            show_borders: true,
            caption: Some(
                "Fake session · Ctrl+F search · Ctrl+O content viewer · n/N hits · []/{} turns · Ctrl+E fold",
            ),
            chrome: true,
            vpad_top: 1,
            ..PromptStyle::default()
        };
        self.prompt
            .render(layout.prompt, f.buffer_mut(), cx, &style);

        if self.slash.is_open() {
            let h = 8u16.min(layout.scrollback.height.saturating_add(1)).max(3);
            let y = layout.prompt.y.saturating_sub(h).max(layout.scrollback.y);
            let pal = Rect {
                x: layout.prompt.x,
                y,
                width: layout.prompt.width,
                height: h.min(layout.prompt.y.saturating_sub(y)),
            };
            if pal.height >= 2 {
                self.slash.render(&SLASH, pal, f.buffer_mut(), cx);
            }
        }

        ShortcutsBar::new(&self.hints).render(layout.shortcuts, f.buffer_mut(), cx);
        if self.viewer_open {
            self.render_content_viewer(area, f, cx);
        }
    }
}

fn main() -> std::io::Result<()> {
    run_blocking(AgentApp::new(), ThemeKind::Auto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use gbtui::Pump;
    use gbtui_agent::{item_id, tool_id};
    use ratatui::backend::TestBackend;

    fn pump(app: AgentApp) -> Pump<AgentApp, TestBackend> {
        let backend = TestBackend::new(80, 28);
        let terminal = gbtui::terminal::Terminal::new(backend).unwrap();
        Pump::new(app, terminal, Theme::night(), Instant::now())
    }

    fn screen(p: &mut Pump<AgentApp, TestBackend>) -> String {
        p.paint().unwrap();
        let buf = p.terminal_mut().backend().buffer();
        let a = buf.area();
        (0..a.height)
            .map(|y| {
                (0..a.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn idle_canned_session_does_not_arm_tick() {
        let mut p = pump(AgentApp::new());
        p.paint().unwrap();
        assert_eq!(p.tick_deadline(Instant::now()), None);
        assert_eq!(p.app.tick_demand(), TickDemand::None);
    }

    #[test]
    fn scrolling_to_top_shows_user_and_read() {
        let mut p = pump(AgentApp::new());
        p.app.transcript.follow = false;
        p.app.transcript.offset = 0;
        let s = screen(&mut p);
        assert!(s.contains("Read "), "Read header missing at top:\n{s}");
        assert!(s.contains("(1-40)"), "read range missing at top:\n{s}");
        assert!(
            s.contains("TextMatcher"),
            "user prompt missing at top:\n{s}"
        );
        assert!(s.contains("Thought"), "thinking missing at top:\n{s}");
        assert!(s.contains("for 0.8s"), "thought elapsed missing:\n{s}");
        assert!(s.contains("Edit "), "Edit header missing at top:\n{s}");
        assert!(s.contains("+12"), "edit diffstat missing:\n{s}");
    }

    #[test]
    fn slash_help_appends_system() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let mut p = pump(AgentApp::new());
        p.paint().unwrap();
        for c in ['/', 'h', 'e', 'l', 'p'] {
            p.drive(
                Instant::now(),
                Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)),
            );
        }
        p.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(
            p.app
                .items
                .iter()
                .any(|l| matches!(l, Line::System { text } if text.contains("/help"))),
            "expected system help line"
        );
        p.paint().unwrap();
        assert_eq!(p.app.tick_demand(), TickDemand::None);
    }

    #[test]
    fn send_starts_running_tool_and_fast_tick() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let mut p = pump(AgentApp::new());
        p.app.prompt.set_text("run tests");
        p.paint().unwrap();
        p.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert_eq!(p.app.tick_demand(), TickDemand::Fast);
        assert!(p.app.items.iter().any(|l| matches!(
            l,
            Line::Tool {
                status: ToolStatus::Running,
                ..
            }
        )));
    }

    #[test]
    fn thinking_header_click_toggles_but_drag_does_not() {
        let mut app = AgentApp::new();
        app.items = vec![Line::Thinking {
            text: "details".into(),
            expanded: false,
            elapsed: None,
            streaming: false,
        }];
        app.transcript.follow = false;
        app.scrollback = Rect::new(0, 0, 40, 4);
        let id = item_id(0);

        assert!(app.handle_transcript_item_pointer(
            id,
            Pointer::Down {
                button: MouseButton::Left,
                col: 3,
                row: 0,
                clicks: 1,
                modifiers: KeyModifiers::NONE,
            },
        ));
        assert!(app.handle_transcript_item_pointer(
            id,
            Pointer::Up {
                button: MouseButton::Left,
                col: 3,
                row: 0,
            },
        ));
        let borrowed = AgentApp::items_as_transcript(&app.items);
        assert_eq!(
            app.transcript.display_mode(0, &borrowed[0]),
            TranscriptDisplay::Expanded
        );

        assert!(app.transcript.set_display(
            &borrowed,
            TranscriptDisplayTarget::Item(0),
            TranscriptDisplay::Collapsed
        ));
        assert!(app.handle_transcript_item_pointer(
            id,
            Pointer::Down {
                button: MouseButton::Left,
                col: 3,
                row: 0,
                clicks: 1,
                modifiers: KeyModifiers::NONE,
            },
        ));
        assert!(app.handle_transcript_item_pointer(
            id,
            Pointer::Drag {
                button: MouseButton::Left,
                col: 4,
                row: 0,
            },
        ));
        let _ = app.handle_transcript_item_pointer(
            id,
            Pointer::Up {
                button: MouseButton::Left,
                col: 4,
                row: 0,
            },
        );
        let borrowed = AgentApp::items_as_transcript(&app.items);
        assert_eq!(
            app.transcript.display_mode(0, &borrowed[0]),
            TranscriptDisplay::Collapsed
        );
    }

    #[test]
    fn tool_header_click_toggles_but_drag_does_not() {
        let mut app = AgentApp::new();
        app.items = vec![Line::Tool {
            name: "Read".into(),
            summary: "file.rs".into(),
            detail: Some("body".into()),
            status: ToolStatus::Done,
            kind: ToolKind::Read,
            range: None,
            diffstat: None,
            expanded: false,
            elapsed: Duration::ZERO,
        }];
        let id = tool_id(0);

        assert!(app.handle_tool_header_pointer(
            id,
            Pointer::Down {
                button: MouseButton::Left,
                col: 2,
                row: 0,
                clicks: 1,
                modifiers: KeyModifiers::NONE,
            },
        ));
        assert!(app.handle_tool_header_pointer(
            id,
            Pointer::Up {
                button: MouseButton::Left,
                col: 2,
                row: 0,
            },
        ));
        let borrowed = AgentApp::items_as_transcript(&app.items);
        assert_eq!(
            app.transcript.display_mode(0, &borrowed[0]),
            TranscriptDisplay::Expanded
        );

        assert!(app.transcript.set_display(
            &borrowed,
            TranscriptDisplayTarget::Item(0),
            TranscriptDisplay::Collapsed
        ));
        assert!(app.handle_tool_header_pointer(
            id,
            Pointer::Down {
                button: MouseButton::Left,
                col: 2,
                row: 0,
                clicks: 1,
                modifiers: KeyModifiers::NONE,
            },
        ));
        assert!(app.handle_tool_header_pointer(
            id,
            Pointer::Drag {
                button: MouseButton::Left,
                col: 4,
                row: 0,
            },
        ));
        let _ = app.handle_tool_header_pointer(
            id,
            Pointer::Up {
                button: MouseButton::Left,
                col: 4,
                row: 0,
            },
        );
        let borrowed = AgentApp::items_as_transcript(&app.items);
        assert_eq!(
            app.transcript.display_mode(0, &borrowed[0]),
            TranscriptDisplay::Collapsed
        );
    }

    #[test]
    fn copy_chord_writes_the_host_clipboard() {
        let mut app = AgentApp::new();
        app.items = vec![Line::User {
            text: "copy me".into(),
            time: None,
        }];
        app.transcript.follow = false;
        app.scrollback = Rect::new(0, 0, 20, 2);
        app.clipboard = SystemClipboard::memory_only();
        let id = item_id(0);
        assert!(app.handle_transcript_item_pointer(
            id,
            Pointer::Down {
                button: MouseButton::Left,
                col: 0,
                row: 0,
                clicks: 1,
                modifiers: KeyModifiers::NONE,
            },
        ));
        assert!(app.handle_transcript_item_pointer(
            id,
            Pointer::Drag {
                button: MouseButton::Left,
                col: 12,
                row: 0,
            },
        ));
        assert!(app.handle_transcript_item_pointer(
            id,
            Pointer::Up {
                button: MouseButton::Left,
                col: 12,
                row: 0,
            },
        ));
        assert_eq!(
            app.handle(AppEvent::Key(crossterm::event::KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ))),
            Outcome::Changed
        );
        assert_eq!(app.clipboard.get().as_deref(), Some("copy me"));
    }
    #[test]
    fn search_entry_navigates_visible_matches_and_turns() {
        let mut app = AgentApp::new();
        app.scrollback = Rect::new(0, 0, 80, 8);
        let borrowed = AgentApp::items_as_transcript(&app.items);
        app.transcript.prepare(&borrowed, 80, &Theme::night());

        assert_eq!(
            app.handle(AppEvent::Key(crossterm::event::KeyEvent::new(
                KeyCode::Char('f'),
                KeyModifiers::CONTROL,
            ))),
            Outcome::Changed
        );
        assert_eq!(
            app.handle(AppEvent::Paste("green".into())),
            Outcome::Changed
        );
        assert_eq!(app.transcript.search_query(), "green");
        assert_eq!(
            app.handle(AppEvent::Key(crossterm::event::KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            ))),
            Outcome::Changed
        );
        assert!(!app.search_active);
        assert_eq!(
            app.handle(AppEvent::Key(crossterm::event::KeyEvent::new(
                KeyCode::Char('n'),
                KeyModifiers::NONE,
            ))),
            Outcome::Changed
        );
        assert_eq!(
            app.handle(AppEvent::Key(crossterm::event::KeyEvent::new(
                KeyCode::Char(']'),
                KeyModifiers::NONE,
            ))),
            Outcome::Changed
        );
    }

    #[test]
    fn viewer_range_copy_uses_code_source_not_gutters() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

        let mut p = pump(AgentApp::new());
        p.app.clipboard = SystemClipboard::memory_only();
        p.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
        );
        p.paint().unwrap();
        p.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE)),
        );
        p.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT)),
        );
        p.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            )),
        );

        assert_eq!(
            p.app.clipboard.get().as_deref(),
            Some(
                "pub fn smart_case_matches(query: &str, haystack: &str) -> bool {\n    let sensitive = query.chars().any(char::is_uppercase);"
            )
        );
    }
}
