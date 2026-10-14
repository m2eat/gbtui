//! Workbench demo: a fake project inbox that *uses* the runtime, instead of
//! cataloguing widgets.
//!
//! Distinctive gbtui behaviour on one screen:
//! - same-frame hover (list / chips / tabs / buttons — no `MouseEventKind`)
//! - occlusion (modal / dropdown / cheatsheet / preview swallow clicks)
//! - `TickDemand::None` when idle; Fast only while syncing; Slow for toast
//! - `ActionRegistry` drives both keys and the shortcuts bar

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
#[cfg(test)]
use gbtui::Theme;
use gbtui::core::{SafeBuf, line_utils, wrapping::wrap_text};
use gbtui::key;
use gbtui::widgets::list::WrapMode;
use gbtui::widgets::{
    ACTIVITY_STOP, DROPDOWN_BODY, MODAL_BODY, NS_CHEAT, NS_FORM, NS_PICKER, ShortcutsBar,
    TOAST_BODY, TOAST_CLOSE, click_scrollbar, dropdown_popup_area, handle_checklist_key,
    handle_overlay_key, handle_overlay_nav_key, is_dropdown_scrollbar, is_scroll_id,
    parse_cheatsheet_row, parse_check_row, parse_choice_row, parse_dropdown_row, parse_form_row,
    parse_picker_expander, parse_picker_group, parse_picker_row, parse_row_id_ns, parse_tab,
    pending_on_key, render_cheatsheet, render_checklist, render_choices, render_dropdown,
    render_form, render_list_ns, render_picker, render_preview,
};
use gbtui::{
    ActionDef, ActionRegistry, App, AppEvent, CheatsheetEntry, CheatsheetState, CheckAction,
    CheckItem, CheckStatus, Chip, Choice, ChoiceAction, ChoiceListState, DropdownAction,
    DropdownItem, DropdownState, EnterOutcome, FormAction, FormRow, FormState, FrameCtx,
    GhostAction, GhostText, HintItem, HitId, ListPaneState, Modal, ModalAction, ModalShortcut,
    ModalSizing, ModalState, Outcome, OverlayAction, OverlayState, PendingOutcome, PickerEntry,
    PickerGroup, PickerRow, PickerState, Pointer, PreviewConfig, Prompt, PromptStyle, StatusBar,
    SystemClipboard, Tabs, TextMatcher, ThemeDefinition, ThemeKind, ThemeRole, ThemeState,
    TickDemand, Toast, UsageGauge, default_unreliable_key, hint_item, hint_items, run_blocking,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::Clear;

const TABS: [&str; 3] = ["Inbox", "Files", "Settings"];
const PROJECTS: [&str; 3] = ["gbtui", "gbtui-agent", "docs"];
const THEMES: [&str; 2] = ["Night", "Day"];
const NS_PROJECT: u32 = 0x5350;
const NS_ISSUE: u32 = 0x5351;
const SYNC_TTL: Duration = Duration::from_millis(450);

const CHIP_OPEN: HitId = HitId::named("studio.chip.open");
const CHIP_OPEN_X: HitId = HitId::named("studio.chip.open.x");
const CHIP_MINE: HitId = HitId::named("studio.chip.mine");
const CHIP_MINE_X: HitId = HitId::named("studio.chip.mine.x");
const NEW_OK: HitId = HitId::named("studio.new.ok");
const NEW_CANCEL: HitId = HitId::named("studio.new.cancel");

/// Letters / digits / `?` with only Shift held — these must type into Prompt
/// / Picker, not fire `o`/`s`/`n` actions.
fn is_typeable(key: &KeyEvent) -> bool {
    let mods = key.modifiers - KeyModifiers::SHIFT;
    if mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) {
        return false;
    }
    matches!(
        key.code,
        KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete
    )
}

fn compact_path(raw: &str) -> String {
    let p = std::path::Path::new(raw);
    let parts: Vec<&str> = p.iter().filter_map(|s| s.to_str()).collect();
    if parts.len() <= 3 {
        return raw.to_string();
    }
    format!("…/{}", parts[parts.len() - 2..].join("/"))
}

fn new_shortcuts() -> [ModalShortcut; 2] {
    [
        ModalShortcut {
            id: NEW_OK,
            key: key!(Enter),
            label: "create",
        },
        ModalShortcut {
            id: NEW_CANCEL,
            key: key!(Esc),
            label: "cancel",
        },
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Inbox,
    Files,
    Settings,
}

impl Tab {
    fn from_index(i: usize) -> Self {
        match i {
            1 => Self::Files,
            2 => Self::Settings,
            _ => Self::Inbox,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Inbox => 0,
            Self::Files => 1,
            Self::Settings => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Overlay,
    Sync,
    New,
    Resolve,
    Help,
    TabInbox,
    TabFiles,
    TabSettings,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ctx {
    Inbox,
    Files,
    Settings,
    Overlay,
    Always,
}

struct Issue {
    id: u16,
    title: String,
    project: usize,
    open: bool,
    mine: bool,
    body: String,
    checks: Vec<CheckItem>,
}

struct Studio {
    tab: Tab,
    overlay: OverlayState,
    projects: ListPaneState,
    issues: Vec<Issue>,
    issue_list: ListPaneState,
    issue_list_area: Rect,
    checks: ListPaneState,
    picker: PickerState,
    picker_area: Rect,
    preview_open: bool,
    form: FormState,
    form_vim: bool,
    form_theme: usize,
    applied_theme: usize,
    form_indent: i64,
    form_name: String,
    dropdown: DropdownState,
    dropdown_area: Rect,
    prompt: Prompt,
    compose: Prompt,
    ghost: GhostText,
    new_open: bool,
    new_modal: ModalState,
    resolve_open: bool,
    resolve_modal: ModalState,
    resolve: ChoiceListState,
    cheatsheet_open: bool,
    cheatsheet: CheatsheetState,
    cheatsheet_modal: ModalState,
    toast: Toast,
    prompt_focused: bool,
    filter_open: bool,
    filter_mine: bool,
    syncing: Option<Instant>,
    usage: UsageGauge,
    cwd: String,
    hints: Vec<HintItem>,
    registry: ActionRegistry<Action, Ctx>,
    pending_quit: Option<gbtui::widgets::PendingConfirm>,
}

impl Studio {
    fn registry() -> ActionRegistry<Action, Ctx> {
        ActionRegistry::new(vec![
            ActionDef::new(Action::Overlay, "sidebar", key!('o', CONTROL), Ctx::Always)
                .alts(vec![key!('o')])
                .describe("Toggle the project overlay")
                .hint(0)
                .pinned(),
            ActionDef::new(Action::Sync, "sync", key!('s', CONTROL), Ctx::Always)
                .alts(vec![key!('s')])
                .describe("Fake a network sync (Fast tick, then idle)")
                .hint(1),
            ActionDef::new(Action::New, "new", key!('n', CONTROL), Ctx::Inbox)
                .alts(vec![key!('n')])
                .describe("Open the new-issue modal")
                .hint(2),
            ActionDef::new(Action::Resolve, "resolve", key!('r', CONTROL), Ctx::Inbox)
                .alts(vec![key!('r')])
                .describe("Resolve the selected issue"),
            ActionDef::new(Action::Help, "shortcuts", key!('.', CONTROL), Ctx::Always)
                .describe("Keyboard cheatsheet")
                .hint(4),
            ActionDef::new(Action::TabInbox, "inbox", key!('1', CONTROL), Ctx::Always),
            ActionDef::new(Action::TabFiles, "files", key!('2', CONTROL), Ctx::Always),
            ActionDef::new(
                Action::TabSettings,
                "settings",
                key!('3', CONTROL),
                Ctx::Always,
            ),
            ActionDef::new(Action::Quit, "quit", key!('q', CONTROL), Ctx::Always)
                .alts(vec![key!('q')])
                .describe("Quit (press twice)")
                .hint(10)
                .confirm()
                .pinned(),
        ])
        .unreliable_keys(default_unreliable_key)
    }

    fn new() -> Self {
        let registry = Self::registry();
        let hints = hint_items(&registry, &[Ctx::Inbox, Ctx::Always]);
        let mut overlay = OverlayState::visible();
        overlay.focused = false;
        let mut prompt = Prompt::new();
        prompt.set_clipboard_provider(Box::new(SystemClipboard::new()));
        Self {
            tab: Tab::Inbox,
            overlay,
            projects: ListPaneState::default(),
            issues: canned_issues(),
            issue_list: ListPaneState::with_wrap_mode(WrapMode::Wrap),
            issue_list_area: Rect::default(),
            checks: ListPaneState::default(),
            picker: PickerState::default(),
            picker_area: Rect::default(),
            preview_open: false,
            form: FormState::default(),
            form_vim: true,
            form_theme: 0,
            applied_theme: 0,
            form_indent: 4,
            form_name: "studio".into(),
            dropdown: DropdownState::default(),
            dropdown_area: Rect::default(),
            prompt,
            compose: Prompt::new(),
            ghost: GhostText::new().with_text("FrameCtx"),
            new_open: false,
            new_modal: ModalState::popup(),
            resolve_open: false,
            resolve_modal: ModalState::popup(),
            resolve: ChoiceListState::single(),
            cheatsheet_open: false,
            cheatsheet: CheatsheetState::default(),
            cheatsheet_modal: ModalState::popup(),
            toast: Toast::new(),
            prompt_focused: true,
            filter_open: true,
            filter_mine: false,
            syncing: None,
            usage: UsageGauge::new(42_000, 256_000),
            cwd: std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "~/gbtui".into()),
            hints,
            registry,
            pending_quit: None,
        }
    }

    fn ctx(&self) -> Ctx {
        if self.overlay.focused {
            Ctx::Overlay
        } else {
            match self.tab {
                Tab::Inbox => Ctx::Inbox,
                Tab::Files => Ctx::Files,
                Tab::Settings => Ctx::Settings,
            }
        }
    }

    fn refresh_hints(&mut self) {
        self.hints = hint_items(&self.registry, &[self.ctx(), Ctx::Always]);
    }

    fn blocking(&self) -> bool {
        self.new_open || self.resolve_open || self.cheatsheet_open || self.dropdown.open
    }

    fn is_typing(&self) -> bool {
        if self.blocking() || self.overlay.focused {
            return false;
        }
        match self.tab {
            Tab::Inbox => self.prompt_focused,
            Tab::Files => true,
            Tab::Settings => self.form.editing.is_some(),
        }
    }

    fn visible_issues(&self) -> Vec<usize> {
        let q = self.prompt.text();
        let matcher = (!q.is_empty()).then(|| TextMatcher::new(q));
        let project = self.projects.selected.min(PROJECTS.len() - 1);
        self.issues
            .iter()
            .enumerate()
            .filter(|(_, iss)| {
                if self.overlay.visible && iss.project != project {
                    return false;
                }
                if self.filter_open && !iss.open {
                    return false;
                }
                if self.filter_mine && !iss.mine {
                    return false;
                }
                matcher
                    .as_ref()
                    .is_none_or(|m| m.is_match(&iss.title) || m.is_match(&iss.body))
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn issue_page(&self) -> usize {
        self.issue_list_area.height.max(1) as usize
    }

    fn selected_issue_mut(&mut self) -> Option<&mut Issue> {
        let vis = self.visible_issues();
        let i = *vis.get(self.issue_list.selected.min(vis.len().saturating_sub(1)))?;
        self.issues.get_mut(i)
    }

    fn file_items() -> [PickerEntry<'static>; 6] {
        [
            PickerEntry::group(PickerGroup::new(10, "Runtime").collapsible(false)),
            PickerEntry::row(
                PickerRow::new(0, "crates/gbtui-runtime/src/frame.rs").right("FrameCtx"),
            ),
            PickerEntry::row(
                PickerRow::new(1, "crates/gbtui-runtime/src/hitmap.rs").right("occlude"),
            ),
            PickerEntry::group(PickerGroup::new(20, "Docs").collapsible(false)),
            PickerEntry::row(PickerRow::new(2, "docs/architecture.md")),
            PickerEntry::row(PickerRow::new(3, "README.md").dimmed()),
        ]
    }

    fn resolve_choices() -> [Choice<'static>; 3] {
        [
            Choice::new("done", "done")
                .description("Close the issue as completed.")
                .preview("The issue remains in the demo data."),
            Choice::new("wontfix", "won't fix")
                .description("Close without scheduling implementation."),
            Choice::new("dup", "duplicate").description("Close as covered by another issue."),
        ]
    }

    fn file_preview(index: usize) -> &'static str {
        match index {
            0 => {
                "FrameCtx\n  theme, mouse, anim, hits\n  hover is same-frame\n  clicks use the previous HitMap"
            }
            1 => {
                "HitMap\n  later registrations sit on top\n  occlude() swallows everything below\n  Modal/Dropdown must occlude first"
            }
            2 => {
                "Architecture\n  ratatui draws; gbtui is the runtime\n  TickDemand::None when idle\n  no retained widget tree"
            }
            _ => "gbtui — terminal UI runtime extracted from Grok Build.",
        }
    }

    fn apply_form(&mut self, action: FormAction) -> Outcome {
        match action {
            FormAction::Unhandled => Outcome::Unchanged,
            FormAction::Changed
            | FormAction::EditStart(_)
            | FormAction::EditCancel(_)
            | FormAction::MultiSelectOpen(_)
            | FormAction::MultiSelectClose(_)
            | FormAction::ToggleMulti { .. }
            | FormAction::ActivateAction(_) => Outcome::Changed,
            FormAction::Toggle(_) => {
                self.form_vim = !self.form_vim;
                Outcome::Changed
            }
            FormAction::OpenSelect(_) => {
                self.dropdown.open_at(self.form_theme.min(THEMES.len() - 1));
                Outcome::Changed
            }
            FormAction::Step { value, .. } => {
                self.form_indent = value;
                Outcome::Changed
            }
            FormAction::EditCommit { text, .. } => {
                self.form_name = text;
                Outcome::Changed
            }
        }
    }

    fn dispatch(&mut self, id: Action, key: &crossterm::event::KeyEvent) -> Outcome {
        match id {
            Action::Overlay => {
                self.overlay.toggle();
                self.issue_list.invalidate_layout();
                self.refresh_hints();
                Outcome::Changed
            }
            Action::Sync => {
                self.syncing = Some(Instant::now());
                Outcome::Changed
            }
            Action::New => {
                self.new_open = true;
                self.new_modal = ModalState::popup();
                self.compose.set_text("");
                Outcome::Changed
            }
            Action::Resolve => {
                if self.selected_issue_mut().is_some() {
                    self.resolve_open = true;
                    self.resolve_modal = ModalState::popup();
                    self.resolve = ChoiceListState::single();
                }
                Outcome::Changed
            }
            Action::Help => {
                self.cheatsheet_open = true;
                self.cheatsheet_modal = ModalState::popup();
                self.cheatsheet = CheatsheetState::default();
                Outcome::Changed
            }
            Action::TabInbox => {
                self.tab = Tab::Inbox;
                self.refresh_hints();
                Outcome::Changed
            }
            Action::TabFiles => {
                self.tab = Tab::Files;
                self.refresh_hints();
                Outcome::Changed
            }
            Action::TabSettings => {
                self.tab = Tab::Settings;
                self.refresh_hints();
                Outcome::Changed
            }
            Action::Quit => {
                if key!('q', CONTROL).matches(key) {
                    return Outcome::Quit;
                }
                match pending_on_key(
                    &mut self.pending_quit,
                    Instant::now(),
                    key,
                    key!('q'),
                    "quit",
                ) {
                    Some(PendingOutcome::Confirmed) => Outcome::Quit,
                    Some(PendingOutcome::Armed) => Outcome::Changed,
                    _ => Outcome::Changed,
                }
            }
        }
    }

    fn cheatsheet_entries(&self) -> Vec<CheatsheetEntry> {
        let mut out = Vec::new();
        for (ctx, header) in [
            (Ctx::Inbox, "Inbox"),
            (Ctx::Always, "Always"),
            (Ctx::Overlay, "Overlay"),
        ] {
            let defs: Vec<&ActionDef<Action, Ctx>> = self
                .registry
                .all()
                .iter()
                .filter(|d| d.context == ctx)
                .collect();
            if defs.is_empty() {
                continue;
            }
            out.push(CheatsheetEntry::Header(header));
            for def in defs {
                out.push(CheatsheetEntry::with_help(
                    hint_item(&self.registry, def),
                    def.description,
                ));
            }
        }
        out
    }

    fn create_issue(&mut self) {
        let title = self.compose.text().trim().to_string();
        self.new_open = false;
        if title.is_empty() {
            return;
        }
        let id = self.issues.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        self.issues.insert(
            0,
            Issue {
                id,
                title,
                project: self.projects.selected.min(PROJECTS.len() - 1),
                open: true,
                mine: true,
                body: "Created in the studio demo.".into(),
                checks: vec![CheckItem::new("write a failing test")],
            },
        );
        self.issue_list.selected = 0;
        self.issue_list.invalidate_layout();
        self.toast.show("issue created", Instant::now());
    }

    fn resolve_issue(&mut self, choice: &str) {
        self.resolve_open = false;
        let label = match choice {
            "wontfix" => "won't fix",
            "dup" => "duplicate",
            _ => "done",
        };
        if let Some(iss) = self.selected_issue_mut() {
            iss.open = false;
        }
        self.issue_list.invalidate_layout();
        self.toast
            .show(format!("resolved · {label}"), Instant::now());
    }

    fn finish_sync(&mut self, now: Instant) {
        if let Some(start) = self.syncing
            && now.saturating_duration_since(start) >= SYNC_TTL
        {
            self.syncing = None;
            self.toast
                .show("synced · still TickDemand::None after", now);
        }
    }
}

fn canned_issues() -> Vec<Issue> {
    vec![
        Issue {
            id: 12,
            title: "FrameCtx hover is same-frame, even when a narrow viewport wraps this issue across several physical terminal rows".into(),
            project: 0,
            open: true,
            mine: true,
            body: "Move the mouse over this row. The highlight changes on the draw that sees the pointer — the app never matches MouseEventKind.".into(),
            checks: vec![
                CheckItem {
                    label: "hover the list".into(),
                    status: CheckStatus::Done,
                },
                CheckItem {
                    label: "click without a mouse match".into(),
                    status: CheckStatus::Active,
                },
                CheckItem::new("occlude a modal over it"),
            ],
        },
        Issue {
            id: 8,
            title: "Idle must be TickDemand::None".into(),
            project: 0,
            open: true,
            mine: true,
            body: "Spinners request Fast. Toasts request Slow. This inbox requests nothing until you press s to sync.".into(),
            checks: vec![CheckItem::new("press s, watch the activity row, then idle")],
        },
        Issue {
            id: 4,
            title: "Dropdown and modal must occlude".into(),
            project: 0,
            open: true,
            mine: false,
            body: "Open n (new issue) or 3 (settings) then the theme ▾. Clicks on the list underneath must miss.".into(),
            checks: vec![CheckItem::new("open the new-issue modal")],
        },
        Issue {
            id: 19,
            title: "ActionRegistry has no product ActionId".into(),
            project: 1,
            open: true,
            mine: true,
            body: "Shortcuts at the bottom are hint_items(&registry). Ctrl+. opens the same table as a cheatsheet modal.".into(),
            checks: vec![CheckItem::new("press Ctrl+.")],
        },
        Issue {
            id: 11,
            title: "Overlay: focus, Tab out, Ctrl-F".into(),
            project: 0,
            open: false,
            mine: true,
            body: "o toggles the project sidebar. Tab unfocuses it so j/k move the issue list. Ctrl-F fullscreen.".into(),
            checks: vec![CheckItem::new("toggle the sidebar")],
        },
    ]
}

impl App for Studio {
    type Msg = ();

    fn take_theme_request(&mut self) -> Option<ThemeDefinition> {
        if self.applied_theme == self.form_theme {
            return None;
        }
        self.applied_theme = self.form_theme;
        Some(ThemeDefinition::from(if self.form_theme == 0 {
            ThemeKind::Night
        } else {
            ThemeKind::Day
        }))
    }

    fn tick_demand(&self) -> TickDemand {
        if self.syncing.is_some() {
            TickDemand::Fast
        } else if self.toast.is_visible(Instant::now())
            || self
                .pending_quit
                .as_ref()
                .is_some_and(|p| !p.expired(Instant::now()))
        {
            TickDemand::Slow
        } else {
            TickDemand::None
        }
    }

    fn handle(&mut self, ev: AppEvent<()>) -> Outcome {
        if let AppEvent::Key(key) = &ev
            && !key!('q').matches(key)
        {
            self.pending_quit = None;
        }
        if !self.cheatsheet_open && !self.resolve_open && !self.dropdown.open {
            if self.new_open {
                match &ev {
                    AppEvent::Hit(id, pointer) => {
                        if self.compose.handle_hit(*id, *pointer).is_handled() {
                            return Outcome::Changed;
                        }
                    }
                    AppEvent::Key(key) if self.compose.has_active_addon() => {
                        return if self.compose.handle_key(key).is_handled() {
                            Outcome::Changed
                        } else {
                            Outcome::Unchanged
                        };
                    }
                    _ => {}
                }
            } else if self.tab == Tab::Inbox {
                let event = match &ev {
                    AppEvent::Hit(id, pointer) => self.prompt.handle_hit(*id, *pointer),
                    AppEvent::Key(key) if self.prompt_focused && self.prompt.has_active_addon() => {
                        self.prompt.handle_key(key)
                    }
                    _ => gbtui::PromptEvent::Ignored,
                };
                if event.is_handled() {
                    self.prompt_focused = true;
                    self.overlay.focused = false;
                    if event.is_edited() {
                        self.issue_list.invalidate_layout();
                    }
                    return Outcome::Changed;
                }
            }
        }
        match ev {
            AppEvent::Hit(_, Pointer::Move { .. }) | AppEvent::Miss(Pointer::Move { .. }) => {
                Outcome::Changed
            }
            AppEvent::Tick => {
                self.finish_sync(Instant::now());
                Outcome::Changed
            }
            AppEvent::Resize { .. } => Outcome::Changed,

            AppEvent::Hit(id, Pointer::Down { .. }) if id == TOAST_CLOSE => {
                self.toast.dismiss();
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if id == TOAST_BODY => Outcome::Unchanged,
            AppEvent::Hit(id, Pointer::Down { .. }) if id == ACTIVITY_STOP => {
                self.syncing = None;
                Outcome::Changed
            }
            AppEvent::Miss(Pointer::Down { .. }) if self.cheatsheet_open => {
                self.cheatsheet_open = false;
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if self.cheatsheet_open && id == MODAL_BODY => {
                Outcome::Unchanged
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if self.cheatsheet_open => {
                if matches!(
                    Modal::new("Shortcuts").hit_action(id),
                    Some(ModalAction::Close)
                ) {
                    self.cheatsheet_open = false;
                }
                Outcome::Changed
            }

            AppEvent::Miss(Pointer::Down { .. }) if self.new_open => {
                self.new_open = false;
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if self.new_open && id == NEW_OK => {
                self.create_issue();
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if self.new_open && id == NEW_CANCEL => {
                self.new_open = false;
                Outcome::Changed
            }

            AppEvent::Miss(Pointer::Down { .. }) if self.resolve_open => {
                self.resolve_open = false;
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if self.resolve_open => {
                let choices = Self::resolve_choices();
                if let Some(item_id) = parse_choice_row(id) {
                    let _ = self.resolve.click(item_id, &choices, 8);
                    if let Some(choice_id) = self
                        .resolve
                        .selected_choice(&choices)
                        .map(|choice| choice.id().to_owned())
                    {
                        self.resolve_issue(&choice_id);
                    }
                } else if matches!(
                    Modal::new("Resolve").hit_action(id),
                    Some(ModalAction::Close)
                ) {
                    self.resolve_open = false;
                }
                Outcome::Changed
            }

            AppEvent::Miss(Pointer::Down { .. }) if self.dropdown.open => {
                self.dropdown.close();
                Outcome::Changed
            }
            AppEvent::Hit(id, pointer) if self.dropdown.open && is_dropdown_scrollbar(id) => {
                if self
                    .dropdown
                    .handle_scrollbar_pointer(pointer, self.dropdown_area, THEMES.len())
                {
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if self.dropdown.open => {
                if id == DROPDOWN_BODY {
                    return Outcome::Unchanged;
                }
                if let Some(i) = parse_dropdown_row(id) {
                    self.form_theme = i.min(THEMES.len() - 1);
                    self.dropdown.close();
                    self.toast
                        .show(format!("theme {}", THEMES[self.form_theme]), Instant::now());
                }
                Outcome::Changed
            }

            AppEvent::Hit(id, Pointer::Down { .. }) if id == CHIP_OPEN || id == CHIP_OPEN_X => {
                self.filter_open = !self.filter_open;
                self.issue_list.invalidate_layout();
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if id == CHIP_MINE || id == CHIP_MINE_X => {
                self.filter_mine = !self.filter_mine;
                self.issue_list.invalidate_layout();
                Outcome::Changed
            }

            AppEvent::Hit(id, Pointer::Down { .. }) if parse_tab(id).is_some() => {
                if let Some(i) = parse_tab(id) {
                    self.tab = Tab::from_index(i);
                    self.refresh_hints();
                }
                Outcome::Changed
            }

            AppEvent::Hit(id, Pointer::Down { row, .. }) => {
                if let Some(i) = parse_row_id_ns(NS_PROJECT, id) {
                    self.projects.selected = i;
                    self.issue_list.selected = 0;
                    self.issue_list.invalidate_layout();
                    self.prompt_focused = false;
                    self.overlay.focused = true;
                    return Outcome::Changed;
                }
                if let Some(i) = parse_row_id_ns(NS_ISSUE, id) {
                    let visible = self.visible_issues();
                    self.issue_list
                        .select_visible(i, visible.len(), self.issue_page());
                    self.prompt_focused = false;
                    self.overlay.focused = false;
                    return Outcome::Changed;
                }
                if is_scroll_id(NS_ISSUE, id) {
                    let visible = self.visible_issues();
                    click_scrollbar(
                        &mut self.issue_list,
                        row,
                        self.issue_list_area,
                        visible.len(),
                    );
                    return Outcome::Changed;
                }
                if let Some(i) = parse_check_row(id)
                    && let Some(iss) = self.selected_issue_mut()
                    && let Some(item) = iss.checks.get_mut(i)
                {
                    item.status = item.status.cycle();
                    return Outcome::Changed;
                }
                if self.tab == Tab::Files {
                    let items = Self::file_items();
                    if self.picker.handle_hit(id, row, self.picker_area, &items) {
                        if parse_picker_row(id).is_some()
                            && self.picker.selected_id(&items).is_some()
                        {
                            self.preview_open = true;
                        }
                        return Outcome::Changed;
                    }
                }
                if self.tab == Tab::Settings
                    && let Some(i) = parse_form_row(id)
                {
                    let name = self.form_name.clone();
                    let theme = THEMES[self.form_theme.min(THEMES.len() - 1)];
                    let rows = [
                        FormRow::header("Editor"),
                        FormRow::toggle(1, "vim keys", self.form_vim),
                        FormRow::select(2, "theme", theme),
                        FormRow::stepper(3, "indent", self.form_indent, 1, 8),
                        FormRow::field(4, "name", name.as_str()),
                    ];
                    let action = self.form.click_row(i, &rows, 8);
                    return self.apply_form(action);
                }
                Outcome::Changed
            }

            AppEvent::Hit(id, Pointer::Drag { row, .. }) if is_scroll_id(NS_ISSUE, id) => {
                let visible = self.visible_issues();
                click_scrollbar(
                    &mut self.issue_list,
                    row,
                    self.issue_list_area,
                    visible.len(),
                );
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Scroll { delta_lines, .. }) => {
                if self.cheatsheet_open {
                    if parse_cheatsheet_row(id).is_some() || is_scroll_id(NS_CHEAT, id) {
                        let entries = self.cheatsheet_entries();
                        let count = self.cheatsheet.visual_len(&entries);
                        self.cheatsheet.list.handle_scroll(delta_lines, count, 12);
                    }
                    return Outcome::Changed;
                }
                if self.dropdown.open {
                    self.dropdown.handle_scroll(delta_lines, THEMES.len());
                    return Outcome::Changed;
                }
                if self.tab == Tab::Settings
                    && (parse_form_row(id).is_some() || is_scroll_id(NS_FORM, id))
                {
                    self.form.handle_scroll(delta_lines);
                    return Outcome::Changed;
                }
                if parse_row_id_ns(NS_ISSUE, id).is_some() || is_scroll_id(NS_ISSUE, id) {
                    let n = self.visible_issues().len();
                    let page = self.issue_page();
                    self.issue_list.handle_scroll(delta_lines, n, page);
                } else if parse_row_id_ns(NS_PROJECT, id).is_some() || is_scroll_id(NS_PROJECT, id)
                {
                    self.projects.handle_scroll(delta_lines, PROJECTS.len(), 8);
                } else if parse_picker_row(id).is_some()
                    || parse_picker_group(id).is_some()
                    || parse_picker_expander(id).is_some()
                    || is_scroll_id(NS_PICKER, id)
                {
                    let items = Self::file_items();
                    self.picker
                        .handle_scroll(delta_lines, self.picker_area, &items);
                }
                Outcome::Changed
            }

            AppEvent::Paste(text) => {
                if self.new_open {
                    let _ = self.compose.handle_paste(&text);
                } else if self.tab == Tab::Inbox {
                    let _ = self.prompt.handle_paste(&text);
                    self.issue_list.invalidate_layout();
                }
                Outcome::Changed
            }

            AppEvent::Key(key) => {
                if self.cheatsheet_open {
                    let chrome = Modal::new("Shortcuts");
                    match chrome.handle_key(&key, &mut self.cheatsheet_modal) {
                        ModalAction::Close => {
                            self.cheatsheet_open = false;
                            return Outcome::Changed;
                        }
                        ModalAction::Unhandled => {}
                        _ => return Outcome::Changed,
                    }
                    let entries = self.cheatsheet_entries();
                    let _ = self.cheatsheet.handle_key(&key, &entries, 12);
                    return Outcome::Changed;
                }
                if self.new_open {
                    let shortcuts = new_shortcuts();
                    let chrome = Modal::new("New issue")
                        .shortcuts(&shortcuts)
                        .sizing(ModalSizing::medium());
                    match chrome.handle_key(&key, &mut self.new_modal) {
                        ModalAction::Close => {
                            self.new_open = false;
                            return Outcome::Changed;
                        }
                        ModalAction::Shortcut(id) if id == NEW_CANCEL => {
                            self.new_open = false;
                            return Outcome::Changed;
                        }
                        ModalAction::Shortcut(id) if id == NEW_OK => {
                            self.create_issue();
                            return Outcome::Changed;
                        }
                        _ => {}
                    }
                    match self.compose.route_enter(&key) {
                        EnterOutcome::Submit => {
                            self.create_issue();
                            return Outcome::Changed;
                        }
                        EnterOutcome::NewlineInserted | EnterOutcome::Handled => {
                            return Outcome::Changed;
                        }
                        EnterOutcome::PassThrough => {
                            if self.compose.handle_key(&key).is_handled() {
                                return Outcome::Changed;
                            }
                        }
                    }
                    return Outcome::Changed;
                }
                if self.resolve_open {
                    let chrome = Modal::new("Resolve").sizing(ModalSizing::medium());
                    if !self.resolve.is_editing_other()
                        && chrome.handle_key(&key, &mut self.resolve_modal) == ModalAction::Close
                    {
                        self.resolve_open = false;
                        return Outcome::Changed;
                    }
                    let choices = Self::resolve_choices();
                    match self.resolve.handle_key(&key, &choices, 8) {
                        ChoiceAction::Submit => {
                            let choice_id = self
                                .resolve
                                .selected_choice(&choices)
                                .map(|choice| choice.id().to_owned())
                                .or_else(|| self.resolve.picked.iter().next().cloned())
                                .unwrap_or_else(|| "done".to_owned());
                            self.resolve_issue(&choice_id);
                            return Outcome::Changed;
                        }
                        ChoiceAction::Ignored => {}
                        _ => return Outcome::Changed,
                    }
                    return Outcome::Changed;
                }
                if self.dropdown.open {
                    match self.dropdown.handle_key(&key, THEMES.len()) {
                        DropdownAction::Confirmed(i) => {
                            self.form_theme = i.min(THEMES.len() - 1);
                            self.toast
                                .show(format!("theme {}", THEMES[self.form_theme]), Instant::now());
                            return Outcome::Changed;
                        }
                        DropdownAction::Unhandled => {}
                        _ => return Outcome::Changed,
                    }
                }
                if let Some(action) = handle_overlay_key(&mut self.overlay, &key) {
                    let _ = action;
                    self.issue_list.invalidate_layout();
                    return Outcome::Changed;
                }
                if key!(Esc).matches(&key) {
                    if self.preview_open {
                        self.preview_open = false;
                        return Outcome::Changed;
                    }
                    if self.overlay.focused {
                        let _ = self.overlay.escape();
                        self.issue_list.invalidate_layout();
                        self.refresh_hints();
                        return Outcome::Changed;
                    }
                    if !self.prompt.is_empty() {
                        self.prompt.set_text("");
                        self.issue_list.invalidate_layout();
                        return Outcome::Changed;
                    }
                    if self.prompt_focused {
                        self.prompt_focused = false;
                        return Outcome::Changed;
                    }
                    return Outcome::Unchanged;
                }
                if self.overlay.focused
                    && let Some(action) = handle_overlay_nav_key(&mut self.overlay, &key)
                {
                    if matches!(action, OverlayAction::Unfocus | OverlayAction::UnfocusAlt) {
                        self.overlay.focused = false;
                    }
                    self.issue_list.invalidate_layout();
                    self.refresh_hints();
                    return Outcome::Changed;
                }
                if self.overlay.focused {
                    let _ = self.projects.handle_key(&key, PROJECTS.len(), 8);
                    if is_typeable(&key) {
                        return Outcome::Changed;
                    }
                    if let Some(id) = self.registry.lookup_in(&key, [Ctx::Overlay, Ctx::Always]) {
                        return self.dispatch(id, &key);
                    }
                    return Outcome::Changed;
                }
                if !(self.is_typing() && is_typeable(&key))
                    && let Some(id) = self.registry.lookup_in(&key, [self.ctx(), Ctx::Always])
                {
                    return self.dispatch(id, &key);
                }
                if self.tab == Tab::Inbox
                    && matches!(
                        key.code,
                        KeyCode::Up
                            | KeyCode::Down
                            | KeyCode::PageUp
                            | KeyCode::PageDown
                            | KeyCode::Home
                            | KeyCode::End
                    )
                {
                    let vis = self.visible_issues().len();
                    if self.issue_list.handle_key(&key, vis, self.issue_page()) {
                        return Outcome::Changed;
                    }
                }
                if self.tab == Tab::Files {
                    let items = Self::file_items();
                    if self.picker.handle_key(&key, &items, 10) {
                        return Outcome::Changed;
                    }
                    if key!(Enter).matches(&key) {
                        self.preview_open = !self.preview_open;
                        return Outcome::Changed;
                    }
                    if key!(Esc).matches(&key) && self.preview_open {
                        self.preview_open = false;
                        return Outcome::Changed;
                    }
                    return Outcome::Unchanged;
                }
                if self.tab == Tab::Settings {
                    let name = self.form_name.clone();
                    let theme = THEMES[self.form_theme.min(THEMES.len() - 1)];
                    let rows = [
                        FormRow::header("Editor"),
                        FormRow::toggle(1, "vim keys", self.form_vim),
                        FormRow::select(2, "theme", theme),
                        FormRow::stepper(3, "indent", self.form_indent, 1, 8),
                        FormRow::field(4, "name", name.as_str()),
                    ];
                    let action = self.form.handle_key(&key, &rows, 8);
                    return self.apply_form(action);
                }
                match self
                    .ghost
                    .handle_key(&key, self.prompt.text(), self.prompt.cursor_at_end())
                {
                    GhostAction::Accepted(rest) => {
                        self.prompt.insert_str(&rest);
                        self.issue_list.invalidate_layout();
                        return Outcome::Changed;
                    }
                    GhostAction::Dismissed => return Outcome::Changed,
                    GhostAction::Unhandled => {}
                }
                match self.prompt.route_enter(&key) {
                    EnterOutcome::Submit
                    | EnterOutcome::NewlineInserted
                    | EnterOutcome::Handled => Outcome::Changed,
                    EnterOutcome::PassThrough => {
                        if self.prompt.is_empty() && !self.prompt_focused {
                            let vis = self.visible_issues().len();
                            if self.issue_list.handle_key(&key, vis, self.issue_page()) {
                                return Outcome::Changed;
                            }
                            let n = self
                                .selected_issue_mut()
                                .map(|iss| iss.checks.len())
                                .unwrap_or(0);
                            match handle_checklist_key(&mut self.checks, &key, n, 6) {
                                CheckAction::Cycled(i) => {
                                    if let Some(iss) = self.selected_issue_mut()
                                        && let Some(item) = iss.checks.get_mut(i)
                                    {
                                        item.status = item.status.cycle();
                                    }
                                    return Outcome::Changed;
                                }
                                CheckAction::Changed => return Outcome::Changed,
                                CheckAction::Unhandled => {}
                            }
                            if let Some(i) = Tabs::handle_key(&key, self.tab.index(), TABS.len()) {
                                self.tab = Tab::from_index(i);
                                self.refresh_hints();
                                return Outcome::Changed;
                            }
                        }
                        let event = self.prompt.handle_key(&key);
                        if event.is_handled() {
                            if event.is_edited() {
                                self.issue_list.invalidate_layout();
                            }
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
        let area = f.area();
        f.render_widget(Clear, area);
        f.render_widget(
            ratatui::widgets::Block::default()
                .style(cx.style(ThemeRole::Surface, ThemeState::Normal)),
            area,
        );

        let act_h = u16::from(self.syncing.is_some());
        let prompt_h = if self.tab == Tab::Inbox { 3 } else { 0 };

        let [status, tabs, main, activity, prompt, shortcuts] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(act_h),
            Constraint::Length(prompt_h),
            Constraint::Length(1),
        ])
        .areas(area);

        let gauge_w = 14u16.min(status.width / 3);
        let [status_l, status_r] =
            Layout::horizontal([Constraint::Min(8), Constraint::Length(gauge_w)]).areas(status);
        let project = PROJECTS[self.projects.selected.min(PROJECTS.len() - 1)];
        StatusBar::new(&compact_path(&self.cwd))
            .center(TABS[self.tab.index()])
            .right(project)
            .render(status_l, f.buffer_mut(), cx);
        self.usage.render(status_r, f.buffer_mut(), cx);

        Tabs::new(&TABS)
            .active(self.tab.index())
            .render(tabs, f.buffer_mut(), cx);

        let overlay_w = if !self.overlay.visible {
            0
        } else if self.overlay.fullscreen {
            main.width
        } else {
            16u16.min(main.width / 4).max(12)
        };
        let content = if overlay_w == 0 {
            main
        } else {
            let [ov, rest] =
                Layout::horizontal([Constraint::Length(overlay_w), Constraint::Min(8)]).areas(main);
            let pane_role = if self.overlay.focused {
                ThemeRole::Elevated
            } else {
                ThemeRole::Surface
            };
            let pane_style = cx.style(pane_role, ThemeState::Normal);
            f.render_widget(ratatui::widgets::Block::default().style(pane_style), ov);
            let title_st = pane_style
                .patch(cx.style(
                    if self.overlay.focused {
                        ThemeRole::Text
                    } else {
                        ThemeRole::Muted
                    },
                    ThemeState::Normal,
                ))
                .add_modifier(ratatui::style::Modifier::BOLD);
            f.buffer_mut()
                .set_string_safe(ov.x.saturating_add(1), ov.y, "projects", title_st);
            let list_area = Rect {
                x: ov.x,
                y: ov.y.saturating_add(1),
                width: ov.width.saturating_sub(1),
                height: ov.height.saturating_sub(1),
            };
            render_list_ns(
                &mut self.projects,
                &PROJECTS,
                NS_PROJECT,
                list_area,
                f.buffer_mut(),
                cx,
            );
            rest
        };

        match self.tab {
            Tab::Inbox => {
                let chips_h = 1u16;
                let [chips, split] =
                    Layout::vertical([Constraint::Length(chips_h), Constraint::Min(3)])
                        .areas(content);
                let mut x = chips.x.saturating_add(1);
                if self.filter_open {
                    let w = Chip::new("open")
                        .id(CHIP_OPEN)
                        .close_id(CHIP_OPEN_X)
                        .render(
                            Rect {
                                x,
                                y: chips.y,
                                width: chips.right().saturating_sub(x),
                                height: 1,
                            },
                            f.buffer_mut(),
                            cx,
                        );
                    x = x.saturating_add(w.saturating_add(1));
                }
                if self.filter_mine {
                    Chip::new("mine")
                        .id(CHIP_MINE)
                        .close_id(CHIP_MINE_X)
                        .render(
                            Rect {
                                x,
                                y: chips.y,
                                width: chips.right().saturating_sub(x),
                                height: 1,
                            },
                            f.buffer_mut(),
                            cx,
                        );
                }
                let [list, preview] =
                    Layout::horizontal([Constraint::Percentage(42), Constraint::Min(24)])
                        .areas(split);
                let vis = self.visible_issues();
                let labels: Vec<String> = vis
                    .iter()
                    .map(|&i| {
                        let iss = &self.issues[i];
                        format!("#{}  {}", iss.id, iss.title)
                    })
                    .collect();
                let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                self.issue_list_area = list;
                render_list_ns(
                    &mut self.issue_list,
                    &refs,
                    NS_ISSUE,
                    list,
                    f.buffer_mut(),
                    cx,
                );
                if let Some(&idx) =
                    vis.get(self.issue_list.selected.min(vis.len().saturating_sub(1)))
                {
                    let iss = &self.issues[idx];
                    let check_h = (iss.checks.len() as u16).min(4);
                    let title = format!("#{}  {}", iss.id, iss.title);
                    let body_w = preview.width.saturating_sub(2).max(1) as usize;
                    let wrapped = wrap_text(&iss.body, body_w);
                    let body_h = (wrapped.len() as u16)
                        .min(preview.height.saturating_sub(1 + check_h).max(1));
                    let [head, body, checks, _rest] = Layout::vertical([
                        Constraint::Length(1),
                        Constraint::Length(body_h),
                        Constraint::Length(check_h),
                        Constraint::Min(0),
                    ])
                    .areas(preview);
                    let shown =
                        line_utils::truncate_str(&title, head.width.saturating_sub(2) as usize);
                    f.buffer_mut().set_string_safe(
                        head.x.saturating_add(1),
                        head.y,
                        &shown,
                        cx.style(ThemeRole::Text, ThemeState::Normal)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                    );
                    for (i, line) in wrapped.iter().take(body_h as usize).enumerate() {
                        f.buffer_mut().set_string_safe(
                            body.x.saturating_add(1),
                            body.y.saturating_add(i as u16),
                            line,
                            cx.style(ThemeRole::Muted, ThemeState::Normal),
                        );
                    }
                    if check_h > 0 {
                        render_checklist(&mut self.checks, &iss.checks, checks, f.buffer_mut(), cx);
                    }
                }
            }
            Tab::Files => {
                let items = Self::file_items();
                render_picker(&mut self.picker, &items, content, f.buffer_mut(), cx);
                self.picker_area = content;
                if self.preview_open
                    && let Some(src) = self.picker.selected_id(&items)
                {
                    render_preview(
                        Self::file_preview(src),
                        content,
                        f.buffer_mut(),
                        cx,
                        &PreviewConfig {
                            preview_lines: 4,
                            hint: Some("esc closes"),
                            ..PreviewConfig::default()
                        },
                    );
                }
            }
            Tab::Settings => {
                let name = self.form_name.clone();
                let theme = THEMES[self.form_theme.min(THEMES.len() - 1)];
                let rows = [
                    FormRow::header("Editor"),
                    FormRow::toggle(1, "vim keys", self.form_vim),
                    FormRow::select(2, "theme", theme),
                    FormRow::stepper(3, "indent", self.form_indent, 1, 8),
                    FormRow::field(4, "name", name.as_str()),
                ];
                render_form(&mut self.form, &rows, content, f.buffer_mut(), cx);
            }
        }

        if let Some(start) = self.syncing {
            gbtui::ActivityRow::new("syncing")
                .total(cx.anim().now().saturating_duration_since(start))
                .render(activity, f.buffer_mut(), cx);
        }

        if prompt_h > 0 {
            let focused = self.prompt_focused && !self.blocking() && !self.overlay.focused;
            let style = PromptStyle {
                focused,
                placeholder: "filter issues",
                placeholder_when_focused: false,
                show_borders: true,
                show_accent_line: false,
                caption: None,
                chrome: true,
                vpad_top: 0,
                ..PromptStyle::default()
            };
            self.prompt
                .render_with_ghost(prompt, f.buffer_mut(), cx, &style, Some(&self.ghost));
        }

        let pending = self
            .pending_quit
            .as_ref()
            .filter(|p| !p.expired(cx.anim().now()))
            .map(|p| p.hint());
        ShortcutsBar::new(&self.hints)
            .with_pending(pending)
            .render(shortcuts, f.buffer_mut(), cx);

        self.toast.render(area, f.buffer_mut(), cx);

        if self.dropdown.open {
            let anchor = self.form.select_anchor.unwrap_or(content);
            let pop = dropdown_popup_area(anchor, area, THEMES.len(), &self.dropdown);
            self.dropdown_area = pop;
            let items = [
                DropdownItem::new(THEMES[0]).detail("dark"),
                DropdownItem::new(THEMES[1]).detail("light"),
            ];
            render_dropdown(
                &mut self.dropdown,
                items.into_iter(),
                pop,
                f.buffer_mut(),
                cx,
            );
        }

        if self.new_open {
            let shortcuts = new_shortcuts();
            let chrome = Modal::new("New issue")
                .shortcuts(&shortcuts)
                .sizing(ModalSizing::medium());
            if let Some(body) = chrome.render(area, f.buffer_mut(), cx, &mut self.new_modal) {
                let style = PromptStyle {
                    focused: true,
                    placeholder: "issue title",
                    placeholder_when_focused: true,
                    show_borders: true,
                    chrome: true,
                    ..PromptStyle::default()
                };
                self.compose.render(body, f.buffer_mut(), cx, &style);
            }
        }

        if self.resolve_open {
            let chrome = Modal::new("Resolve").sizing(ModalSizing::medium());
            if let Some(body) = chrome.render(area, f.buffer_mut(), cx, &mut self.resolve_modal) {
                let choices = Self::resolve_choices();
                render_choices(&mut self.resolve, &choices, body, f.buffer_mut(), cx);
            }
        }

        if self.cheatsheet_open {
            let chrome = Modal::new("Shortcuts");
            if let Some(body) = chrome.render(area, f.buffer_mut(), cx, &mut self.cheatsheet_modal)
            {
                let entries = self.cheatsheet_entries();
                render_cheatsheet(&mut self.cheatsheet, &entries, body, f.buffer_mut(), cx);
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    run_blocking(Studio::new(), ThemeKind::Night)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui::Pump;
    use ratatui::backend::TestBackend;

    fn pump(app: Studio) -> Pump<Studio, TestBackend> {
        let backend = TestBackend::new(100, 32);
        let terminal = gbtui::terminal::Terminal::new(backend).unwrap();
        Pump::new(app, terminal, Theme::night(), Instant::now())
    }

    fn screen(p: &mut Pump<Studio, TestBackend>) -> String {
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
    fn idle_workbench_does_not_arm_tick() {
        let mut p = pump(Studio::new());
        p.paint().unwrap();
        assert_eq!(p.tick_deadline(Instant::now()), None);
        assert_eq!(p.app.tick_demand(), TickDemand::None);
    }

    #[test]
    fn paints_inbox_like_an_app() {
        let mut p = pump(Studio::new());
        let s = screen(&mut p);
        assert!(s.contains("Inbox"), "{s}");
        assert!(s.contains("#12") || s.contains("FrameCtx"), "{s}");
        assert!(
            s.contains("╭") && s.contains("╰"),
            "boxed prompt missing:\n{s}"
        );
        assert!(
            s.contains("Ctrl+") || s.contains("sync") || s.contains("sidebar"),
            "{s}"
        );
        assert!(
            s.contains("42K") || s.contains("256K") || s.contains('/'),
            "{s}"
        );
    }

    #[test]
    fn sync_requests_fast_then_idles() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let mut p = pump(Studio::new());
        p.paint().unwrap();
        p.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
        );
        assert!(
            p.app.syncing.is_none(),
            "plain 's' must type into the prompt, not sync"
        );
        assert!(p.app.prompt.text().contains('s'));
        p.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        );
        assert_eq!(p.app.tick_demand(), TickDemand::Fast);
        p.app
            .finish_sync(Instant::now() + SYNC_TTL + Duration::from_millis(10));
        assert!(p.app.syncing.is_none());
        p.paint().unwrap();
        assert_eq!(p.app.tick_demand(), TickDemand::Slow);
    }

    #[test]
    fn new_issue_modal_occludes_background() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let mut p = pump(Studio::new());
        p.paint().unwrap();
        p.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)),
        );
        assert!(p.app.new_open);
        p.paint().unwrap();
        let s = screen(&mut p);
        assert!(s.contains("New issue") || s.contains("create"), "{s}");
        p.drive(
            Instant::now(),
            Event::Mouse(crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert!(
            !p.app.new_open,
            "outside click should close the occluding modal"
        );
    }

    #[test]
    fn overlay_o_toggles_sidebar() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let mut p = pump(Studio::new());
        assert!(p.app.overlay.visible);
        p.paint().unwrap();
        p.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)),
        );
        assert!(
            !p.app.overlay.focused,
            "plain 'o' types into the prompt; sidebar stays unfocused"
        );
        p.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
        );
        assert!(p.app.overlay.focused || !p.app.overlay.visible);
        let s = screen(&mut p);
        assert!(
            s.contains("projects") || s.contains("gbtui") || !p.app.overlay.visible,
            "{s}"
        );
    }
}
