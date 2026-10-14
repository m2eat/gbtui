//! Inline chat shell: history in native scrollback, prompt pinned at the bottom.
//!
//! Hover and click are routed through `FrameCtx` pointers.

#[cfg(test)]
use gbtui::Theme;
use gbtui::key;
use gbtui::widgets::ShortcutsBar;
use gbtui::{
    ActionDef, ActionRegistry, App, AppEvent, AttachmentId, AttachmentKind, CompletionCandidate,
    EnterOutcome, FrameCtx, GhostAction, GhostText, HintItem, Outcome, PastePolicy, Pointer,
    Prompt, PromptAttachment, PromptEvent, PromptStyle, ThemeKind, ThemeRole, ThemeState,
    TickDemand, hint_items, run_inline_blocking,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::{Clear, Paragraph};

const INLINE_HEIGHT: u16 = 14;
const SAMPLE_ATTACHMENT_ID: &str = "chat-example-attachment";
const SAMPLE_HISTORY: [&str; 12] = [
    "review the Unicode rendering",
    "render the dropdown scrollbar",
    "restore the attachment draft",
    "browse the newest history",
    "search history with ctrl-r",
    "show the description column",
    "keep completion requests fresh",
    "paste keeps original content",
    "preview the selected history row",
    "scroll through more than eight rows",
    "highlight grapheme-safe matches",
    "remember host-owned in-memory history",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChatAction {
    Quit,
    InsertAttachment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChatCtx {
    Always,
}

struct ChatApp {
    prompt: Prompt,
    history: String,
    pending_emit: Option<String>,
    status: String,
    hints: Vec<HintItem>,
    registry: ActionRegistry<ChatAction, ChatCtx>,
    ghost: GhostText,
}

impl ChatApp {
    fn new() -> Self {
        let registry = ActionRegistry::new(vec![
            ActionDef::new(ChatAction::Quit, "quit", key!('q'), ChatCtx::Always)
                .alts(vec![key!(Esc)])
                .hint(0)
                .pinned(),
            ActionDef::new(
                ChatAction::InsertAttachment,
                "attach example metadata",
                key!('g', CONTROL),
                ChatCtx::Always,
            )
            .hint(1),
        ]);
        let hints = hint_items(&registry, &[ChatCtx::Always]);
        let mut prompt = Prompt::new();
        prompt.set_paste_policy(PastePolicy::Collapse { min_bytes: 32 });
        prompt.set_history(SAMPLE_HISTORY.iter().map(|entry| (*entry).into()).collect());
        Self {
            prompt,
            history: String::new(),
            pending_emit: None,
            status: "type h for 12 completions · ctrl-r fuzzy search · ↑/↓ browse · ctrl-g attach · q quit".into(),
            hints,
            registry,
            ghost: GhostText::new().accept_tab(true).with_text("tell me more"),
        }
    }

    fn refresh_completions(&mut self) {
        const STATIC: [(&str, &str); 12] = [
            ("hello", "static greeting"),
            ("help", "static help request"),
            ("history", "memory-history demo"),
            ("highlight", "grapheme-safe matching"),
            ("hover", "same-frame pointer feedback"),
            ("horizon", "long candidate label example"),
            ("housekeeping", "host-owned state reminder"),
            ("howdy", "short friendly greeting"),
            ("hyperlink", "terminal capability sample"),
            ("haptic", "generic completion candidate"),
            ("hologram", "generic completion candidate"),
            ("hyphenate", "generic completion candidate"),
        ];

        let request = self.prompt.completion_request();
        let before_cursor = &request.draft[..request.cursor];
        let start = before_cursor
            .char_indices()
            .rev()
            .find_map(|(index, ch)| ch.is_whitespace().then_some(index + ch.len_utf8()))
            .unwrap_or(0);
        let token = &before_cursor[start..];
        let candidates = STATIC
            .iter()
            .filter(|(value, _)| !token.is_empty() && value.starts_with(token) && *value != token)
            .map(|(value, detail)| CompletionCandidate {
                replacement: start..request.cursor,
                insert_text: (*value).into(),
                label: (*value).into(),
                detail: Some((*detail).into()),
            })
            .collect();
        let _ = self.prompt.set_completions(&request, candidates);
    }

    fn insert_example_attachment(&mut self) {
        self.prompt.insert_attachment(PromptAttachment {
            id: AttachmentId::new(SAMPLE_ATTACHMENT_ID),
            kind: AttachmentKind::File,
            label: "example-notes.txt".into(),
            insert_text: "example-notes.txt".into(),
        });
        self.status = "inserted example attachment metadata".into();
    }

    fn send(&mut self) {
        let Some(submission) = self.prompt.try_submission() else {
            return;
        };
        let attachment_count = submission.attachments.len();
        let text = submission.text;
        self.prompt.record_history(text.clone());
        self.prompt.set_text("");
        self.refresh_completions();

        let line = format!("submitted: {text:?} · attachments: {attachment_count}\r\n");
        self.history.push_str(&line);
        self.pending_emit = Some(line);
        self.status = format!(
            "submitted {} bytes · {attachment_count} attachment(s)",
            text.len()
        );
        self.ghost.clear();
    }

    fn apply_prompt_event(&mut self, event: PromptEvent) -> Outcome {
        if let PromptEvent::AttachmentActivated(id) = &event {
            self.status = format!("activated attachment metadata: {}", id.0);
        }
        if matches!(&event, PromptEvent::Edited | PromptEvent::PasteExpanded(_)) {
            self.refresh_completions();
        }
        if event.is_handled() {
            Outcome::Changed
        } else {
            Outcome::Unchanged
        }
    }
}

impl App for ChatApp {
    type Msg = ();

    fn tick_demand(&self) -> TickDemand {
        TickDemand::None
    }

    fn take_scrollback_emit(&mut self) -> Option<String> {
        self.pending_emit.take()
    }

    fn scrollback_history(&self) -> Option<&str> {
        Some(self.history.as_str())
    }

    fn handle(&mut self, ev: AppEvent<()>) -> Outcome {
        match ev {
            AppEvent::Hit(id, pointer) => {
                let event = self.prompt.handle_hit(id, pointer);
                if event.is_handled() {
                    return self.apply_prompt_event(event);
                }
                match pointer {
                    Pointer::Move { .. } => Outcome::Changed,
                    Pointer::Down { .. } => match gbtui::widgets::parse_hint_id(id) {
                        Some(0) => Outcome::Quit,
                        Some(1) => {
                            self.insert_example_attachment();
                            self.refresh_completions();
                            Outcome::Changed
                        }
                        _ => Outcome::Unchanged,
                    },
                    _ => Outcome::Unchanged,
                }
            }
            AppEvent::Miss(Pointer::Move { .. }) => Outcome::Changed,
            AppEvent::Resize { .. } => Outcome::Changed,
            AppEvent::Paste(text) => {
                let event = self.prompt.handle_paste(&text);
                self.apply_prompt_event(event)
            }
            AppEvent::Key(key) => {
                if self.prompt.has_active_addon() {
                    let event = self.prompt.handle_key(&key);
                    if event.is_handled() {
                        return self.apply_prompt_event(event);
                    }
                }
                if key!('g', CONTROL).matches(&key) {
                    self.insert_example_attachment();
                    self.refresh_completions();
                    return Outcome::Changed;
                }
                match self
                    .ghost
                    .handle_key(&key, self.prompt.text(), self.prompt.cursor_at_end())
                {
                    GhostAction::Accepted(rest) => {
                        self.prompt.insert_str(&rest);
                        self.refresh_completions();
                        return Outcome::Changed;
                    }
                    GhostAction::Dismissed => return Outcome::Changed,
                    GhostAction::Unhandled => {}
                }
                match self.prompt.route_enter(&key) {
                    EnterOutcome::Submit => {
                        self.send();
                        Outcome::Changed
                    }
                    EnterOutcome::NewlineInserted => {
                        self.refresh_completions();
                        Outcome::Changed
                    }
                    EnterOutcome::Handled => Outcome::Changed,
                    EnterOutcome::PassThrough => {
                        if let Some(ChatAction::Quit) = self.registry.lookup(&key, ChatCtx::Always)
                            && (key!(Esc).matches(&key) || self.prompt.is_empty())
                        {
                            return Outcome::Quit;
                        }
                        let event = self.prompt.handle_key(&key);
                        self.apply_prompt_event(event)
                    }
                }
            }
            _ => Outcome::Unchanged,
        }
    }

    fn draw(&mut self, f: &mut Frame, cx: &mut FrameCtx) {
        let area = f.area();
        f.render_widget(Clear, area);
        let bg = cx.style(ThemeRole::Surface, ThemeState::Normal);
        f.render_widget(ratatui::widgets::Block::default().style(bg), area);

        let style = PromptStyle {
            focused: true,
            placeholder: "message",
            placeholder_when_focused: true,
            ..PromptStyle::default()
        };
        let prompt_height = self
            .prompt
            .desired_height(area.width, &style)
            .max(2)
            .min(area.height.saturating_sub(2).max(1));
        let [showcase_row, _dropdown_room, prompt_area, bar] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(prompt_height),
            Constraint::Length(1),
        ])
        .areas(area);
        f.render_widget(
            Paragraph::new(
                " prompt addons: 12 completions · fuzzy memory history · ↑/↓ browse · atomic paste · metadata attachment",
            )
            .style(cx.style(ThemeRole::Muted, ThemeState::Normal)),
            showcase_row,
        );

        ShortcutsBar::new(&self.hints)
            .with_right_text(Some(self.status.as_str()))
            .render(bar, f.buffer_mut(), cx);

        self.prompt
            .render_with_ghost(prompt_area, f.buffer_mut(), cx, &style, Some(&self.ghost));
    }
}

fn main() -> std::io::Result<()> {
    run_inline_blocking(ChatApp::new(), ThemeKind::Auto, INLINE_HEIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui::Pump;
    use ratatui::backend::TestBackend;
    use std::time::Instant;

    fn pump(app: ChatApp) -> Pump<ChatApp, TestBackend> {
        let backend = TestBackend::new(40, 12);
        let terminal = gbtui::terminal::Terminal::new(backend).unwrap();
        Pump::new(app, terminal, Theme::night(), Instant::now())
    }

    #[test]
    fn idle_does_not_arm_tick() {
        let mut p = pump(ChatApp::new());
        p.paint().unwrap();
        assert_eq!(p.tick_deadline(Instant::now()), None);
        assert_eq!(p.app.tick_demand(), TickDemand::None);
    }

    #[test]
    fn static_completion_accepts_without_submitting() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = ChatApp::new();
        app.handle(AppEvent::Key(KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::NONE,
        )));
        app.handle(AppEvent::Key(KeyEvent::new(
            KeyCode::Char('e'),
            KeyModifiers::NONE,
        )));
        assert!(app.prompt.has_active_addon());

        app.handle(AppEvent::Key(KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.prompt.text(), "hello");
        assert!(app.history.is_empty());
    }
    #[test]
    fn submit_reports_content_and_attachment_count() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let mut app = ChatApp::new();
        app.prompt.set_text("hi ");
        app.insert_example_attachment();
        let mut p = pump(app);
        p.paint().unwrap();
        p.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert_eq!(p.app.tick_demand(), TickDemand::None);
        assert!(
            p.app
                .history
                .contains("submitted: \"hi example-notes.txt\"")
        );
        assert!(p.app.history.contains("attachments: 1"));
    }
}
