//! Settings example: Modal / Overlay / List / Picker / ChoiceList / Prompt /
//! ShortcutsBar / Spinner / Link / StatusBar / Toast / Tabs / Chip / Dropdown /
//! Cheatsheet. Hover/click go through FrameCtx.
//!
//! Choices: `j`/`k` navigate, Space toggles, Enter submits; on Other, Enter
//! enters the free editor and Enter/Esc commit/cancel. Form: Enter expands
//! multi-selects, Space toggles options, Esc closes them, and `j` from the last
//! field reaches the pinned Apply/Reset actions.

use std::time::Instant;

use gbtui::key;
use gbtui::widgets::{
    BANNER_HIDE, DROPDOWN_BODY, FormActionRow, MODAL_BODY, NS_CHEAT, NS_CHOICE, NS_FORM, NS_PICKER,
    NS_ROW, ShortcutsBar, TOAST_BODY, TOAST_CLOSE, click_scrollbar, dropdown_popup_area,
    handle_checklist_key, handle_overlay_key, handle_overlay_nav_key, is_dropdown_scrollbar,
    is_scroll_id, parse_cheatsheet_row, parse_check_row, parse_choice_row, parse_dropdown_row,
    parse_form_action, parse_form_option, parse_form_row, parse_hint_id, parse_picker_expander,
    parse_picker_group, parse_picker_row, parse_row_id, parse_tab, pending_on_key,
    render_cheatsheet, render_checklist, render_choices, render_dropdown, render_form_with_actions,
    render_list, render_picker, render_preview, render_progress,
};
use gbtui::{
    ActionDef, ActionRegistry, App, AppEvent, Banner, Button, CheatsheetEntry, CheatsheetState,
    CheckAction, CheckItem, CheckStatus, Chip, Choice, ChoiceAction, ChoiceListState,
    DropdownAction, DropdownItem, DropdownState, EnterOutcome, FormAction, FormRow, FormState,
    FrameCtx, GhostAction, GhostText, HintItem, HitId, Link, ListPaneState, Modal, ModalAction,
    ModalShortcut, ModalState, Outcome, OverlayAction, OverlayState, PendingOutcome, PickerEntry,
    PickerGroup, PickerRow, PickerState, Pointer, PreviewConfig, Prompt, PromptStyle, Spinner,
    StatusBar, SystemClipboard, Tabs, ThemeDefinition, ThemeKind, ThemeRole, ThemeState,
    TickDemand, Toast, UsageGauge, default_unreliable_key, hint_item, hint_items, run_blocking,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::widgets::{Block, Borders, Clear};

const SAVE: HitId = HitId::named("gbtui.settings.save");
const CANCEL: HitId = HitId::named("gbtui.settings.cancel");
const SAVE_BTN: HitId = HitId::named("gbtui.settings.save_btn");
const CANCEL_BTN: HitId = HitId::named("gbtui.settings.cancel_btn");

const CATEGORIES: [&str; 3] = ["Theme", "Editor", "Mouse"];
const THEMES: [&str; 3] = ["Night", "Day", "Auto"];
const TABS: [&str; 5] = ["List", "Picker", "Choices", "Form", "Prompt"];
const DOCS: HitId = HitId::named("gbtui.settings.docs");
const CHIP_THEME: HitId = HitId::named("gbtui.settings.chip.theme");
const CHIP_THEME_CLOSE: HitId = HitId::named("gbtui.settings.chip.theme.close");
const NS_CHIP_CHOICE: u32 = 0x4343;
const THEME_DETAILS: [&str; 3] = ["dark", "light", "system"];
const FORM_FEATURES: [&str; 3] = ["lint on save", "format on save", "show minimap"];
const FORM_NAME_ERROR: &str = "Host validation: use at least three UTF-8 characters.";

const FORM_ACTIONS: [FormActionRow<'static>; 2] = [
    FormActionRow::new(1, "Apply"),
    FormActionRow::new(2, "Reset"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DemoAction {
    Overlay,
    OpenModal,
    OpenPicker,
    OpenChoices,
    Spin,
    Help,
    Dropdown,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DemoCtx {
    Home,
    Overlay,
    Always,
}

struct SettingsApp {
    overlay: OverlayState,
    modal_open: bool,
    modal: ModalState,
    categories: ListPaneState,
    themes: ListPaneState,
    pending_theme: Option<ThemeDefinition>,
    picker: PickerState,
    choices: ChoiceListState,
    choice_items: Vec<Choice<'static>>,
    form_features: [bool; 3],
    status: String,
    overlay_list: Rect,
    modal_list: Rect,
    shortcuts: [ModalShortcut; 2],
    spinner: Spinner,
    prompt: Prompt,
    pending_quit: Option<gbtui::widgets::PendingConfirm>,
    hints: Vec<HintItem>,
    registry: ActionRegistry<DemoAction, DemoCtx>,
    toast: Toast,
    dropdown: DropdownState,
    drop_anchor: Rect,
    dropdown_area: Rect,
    cheatsheet_open: bool,
    cheatsheet: CheatsheetState,
    cheatsheet_modal: ModalState,
    hide_theme_chip: bool,
    form: FormState,
    form_vim: bool,
    form_indent: i64,
    form_name: String,
    checks: Vec<CheckItem>,
    check_list: ListPaneState,
    ghost: GhostText,
    banner_visible: bool,
}

impl SettingsApp {
    fn registry() -> ActionRegistry<DemoAction, DemoCtx> {
        ActionRegistry::new(vec![
            ActionDef::new(DemoAction::Overlay, "overlay", key!('o'), DemoCtx::Home)
                .alts(vec![key!('o', CONTROL)])
                .describe("Toggle the overlay pane")
                .hint(0)
                .pinned(),
            ActionDef::new(DemoAction::OpenModal, "modal", key!(Enter), DemoCtx::Home)
                .describe("Open the preferences modal")
                .hint(1),
            ActionDef::new(DemoAction::OpenPicker, "picker", key!('/'), DemoCtx::Home)
                .describe("Open the picker tab")
                .hint(2),
            ActionDef::new(DemoAction::OpenChoices, "choices", key!('c'), DemoCtx::Home)
                .describe("Open the choices tab")
                .hint(3),
            ActionDef::new(DemoAction::Spin, "spin", key!('s'), DemoCtx::Home)
                .describe("Toggle the demo spinner")
                .hint(4),
            ActionDef::new(DemoAction::Help, "help", key!('.', CONTROL), DemoCtx::Home)
                .alts(vec![key!('?')])
                .describe("Open the keyboard cheatsheet")
                .hint(5),
            ActionDef::new(DemoAction::Dropdown, "menu", key!('d'), DemoCtx::Home)
                .describe("Open the theme dropdown")
                .hint(6),
            ActionDef::new(
                DemoAction::OpenModal,
                "modal",
                key!(Enter),
                DemoCtx::Overlay,
            )
            .describe("Open modal for the selected overlay row"),
            ActionDef::new(DemoAction::Quit, "quit", key!('q'), DemoCtx::Always)
                .describe("Quit (press twice)")
                .hint(10)
                .confirm()
                .pinned(),
        ])
        .unreliable_keys(default_unreliable_key)
    }

    fn new() -> Self {
        let mut overlay = OverlayState::visible();
        overlay.focused = true;
        let mut choices = ChoiceListState::multi();
        choices.set_other(Some(String::new()));
        let registry = Self::registry();
        let hints = hint_items(&registry, &[DemoCtx::Home, DemoCtx::Always]);
        Self {
            overlay,
            modal_open: false,
            modal: ModalState::popup(),
            categories: ListPaneState::default(),
            themes: ListPaneState::default(),
            pending_theme: None,
            picker: PickerState::default(),
            choices,
            choice_items: vec![
                Choice::new("vim", "vim keys")
                    .description("Navigation stays in the list until an editor is opened.")
                    .preview("Use h/j/k/l-compatible keybindings."),
                Choice::new("mouse", "mouse support")
                    .description("Hover and click through FrameCtx hit testing."),
                Choice::new("bell", "terminal bell")
                    .description("Notify when background work completes.")
                    .preview("Preview: a host-controlled optional setting."),
            ],
            status: "o overlay · enter modal · / picker · c choices · q quit".into(),
            overlay_list: Rect::default(),
            modal_list: Rect::default(),
            shortcuts: [
                ModalShortcut {
                    id: CANCEL,
                    key: key!(Esc),
                    label: "close",
                },
                ModalShortcut {
                    id: SAVE,
                    key: key!('s', CONTROL),
                    label: "save",
                },
            ],
            spinner: Spinner::new(),
            prompt: {
                let mut prompt = Prompt::new();
                prompt.set_clipboard_provider(Box::new(SystemClipboard::new()));
                prompt
            },
            pending_quit: None,
            hints,
            registry,
            toast: Toast::new(),
            dropdown: DropdownState::default(),
            drop_anchor: Rect::default(),
            dropdown_area: Rect::default(),
            cheatsheet_open: false,
            cheatsheet: CheatsheetState::default(),
            cheatsheet_modal: ModalState::popup(),
            hide_theme_chip: false,
            form: FormState::default(),
            form_vim: false,
            form_indent: 4,
            form_name: "gbtui".into(),
            form_features: [true, false, true],
            checks: vec![
                CheckItem::new("write docs"),
                CheckItem {
                    label: "implement form".into(),
                    status: CheckStatus::Active,
                },
                CheckItem {
                    label: "ship P10".into(),
                    status: CheckStatus::Pending,
                },
            ],
            check_list: ListPaneState::default(),
            ghost: GhostText::new().with_text("save the night theme"),
            banner_visible: true,
        }
    }

    fn dispatch(&mut self, id: DemoAction, key: &crossterm::event::KeyEvent) -> Outcome {
        match id {
            DemoAction::Overlay => {
                self.overlay.toggle();
                Outcome::Changed
            }
            DemoAction::OpenModal => {
                let tab = if self.overlay.visible && self.overlay.focused {
                    self.categories.selected.min(2)
                } else {
                    0
                };
                self.open_modal(tab);
                Outcome::Changed
            }
            DemoAction::OpenPicker => {
                self.open_modal(1);
                Outcome::Changed
            }
            DemoAction::OpenChoices => {
                self.open_modal(2);
                Outcome::Changed
            }
            DemoAction::Spin => {
                self.spinner.running = !self.spinner.running;
                Outcome::Changed
            }
            DemoAction::Help => {
                self.cheatsheet_open = true;
                self.cheatsheet_modal = ModalState::popup();
                self.cheatsheet = CheatsheetState::default();
                self.status = "cheatsheet".into();
                Outcome::Changed
            }
            DemoAction::Dropdown => {
                self.dropdown
                    .open_at(self.themes.selected.min(THEMES.len() - 1));
                Outcome::Changed
            }
            DemoAction::Quit => match pending_on_key(
                &mut self.pending_quit,
                Instant::now(),
                key,
                key!('q'),
                "quit",
            ) {
                Some(PendingOutcome::Confirmed) => Outcome::Quit,
                Some(PendingOutcome::Armed) => {
                    self.status = "press q again to quit".into();
                    Outcome::Changed
                }
                _ => Outcome::Changed,
            },
        }
    }

    fn chrome(&self) -> Modal<'_> {
        Modal::new("Preferences")
            .tabs(&TABS)
            .shortcuts(&self.shortcuts)
    }

    fn picker_items() -> [PickerEntry<'static>; 5] {
        [
            PickerEntry::group(PickerGroup::new(10, "Commands").collapsible(false)),
            PickerEntry::row(PickerRow::new(0, "Open theme list").right("1")),
            PickerEntry::row(PickerRow::new(1, "Open picker").right("/")),
            PickerEntry::group(PickerGroup::new(20, "Help").collapsible(false)),
            PickerEntry::row(PickerRow::new(2, "About gbtui").dimmed()),
        ]
    }

    fn open_modal(&mut self, tab: usize) {
        self.modal_open = true;
        self.modal = ModalState::popup();
        self.modal.active_tab = tab.min(TABS.len() - 1);
        self.status = "Esc close · click outside · ✗ · Ctrl-S save".into();
    }

    fn close_modal(&mut self) {
        self.modal_open = false;
        self.status = "closed".into();
    }

    fn save(&mut self) {
        let theme = THEMES.get(self.themes.selected).copied().unwrap_or("?");
        self.pending_theme = ThemeKind::from_name(theme).map(ThemeDefinition::from);
        self.status = format!("saved {theme}");
        self.toast.show(self.status.clone(), Instant::now());
        self.modal_open = false;
    }

    fn cheatsheet_entries(&self) -> Vec<CheatsheetEntry> {
        let mut out = Vec::new();
        for (ctx, header) in [
            (DemoCtx::Home, "Home"),
            (DemoCtx::Overlay, "Overlay"),
            (DemoCtx::Always, "Always"),
        ] {
            let defs: Vec<&ActionDef<DemoAction, DemoCtx>> = self
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

    fn dropdown_items() -> [DropdownItem<'static>; 3] {
        [
            DropdownItem::new(THEMES[0]).detail(THEME_DETAILS[0]),
            DropdownItem::new(THEMES[1]).detail(THEME_DETAILS[1]),
            DropdownItem::new(THEMES[2]).detail(THEME_DETAILS[2]),
        ]
    }

    fn apply_dropdown(&mut self, action: DropdownAction) -> Outcome {
        match action {
            DropdownAction::Unhandled => Outcome::Unchanged,
            DropdownAction::Closed | DropdownAction::Changed => Outcome::Changed,
            DropdownAction::Confirmed(i) => {
                self.themes.selected = i.min(THEMES.len() - 1);
                let name = THEMES[self.themes.selected];
                self.pending_theme = ThemeKind::from_name(name).map(ThemeDefinition::from);
                self.status = format!("theme {name}");
                self.toast.show(self.status.clone(), Instant::now());
                self.hide_theme_chip = false;
                Outcome::Changed
            }
        }
    }

    fn close_cheatsheet(&mut self) {
        self.cheatsheet_open = false;
        self.status = "closed cheatsheet".into();
    }

    fn cheatsheet_len(&self) -> usize {
        let entries = self.cheatsheet_entries();
        self.cheatsheet.visual_len(&entries)
    }

    fn apply_form(&mut self, action: FormAction) -> Outcome {
        match action {
            FormAction::Unhandled => Outcome::Unchanged,
            FormAction::Changed
            | FormAction::EditStart(_)
            | FormAction::EditCancel(_)
            | FormAction::MultiSelectOpen(_)
            | FormAction::MultiSelectClose(_) => Outcome::Changed,
            FormAction::Toggle(_) => {
                self.form_vim = !self.form_vim;
                Outcome::Changed
            }
            FormAction::OpenSelect(_) => {
                self.dropdown
                    .open_at(self.themes.selected.min(THEMES.len() - 1));
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
            FormAction::ToggleMulti { option, .. } => {
                if let Some(selected) = self.form_features.get_mut(option) {
                    *selected = !*selected;
                }
                Outcome::Changed
            }
            FormAction::ActivateAction(1) => {
                if self.form_name.chars().count() < 3 {
                    self.status = format!("cannot apply: {FORM_NAME_ERROR}");
                } else {
                    self.status = format!(
                        "applied {} feature(s)",
                        self.form_features
                            .iter()
                            .filter(|selected| **selected)
                            .count()
                    );
                }
                Outcome::Changed
            }
            FormAction::ActivateAction(2) => {
                self.form_vim = false;
                self.form_indent = 4;
                self.form_name = "gbtui".into();
                self.form_features = [true, false, true];
                self.status = "reset settings".into();
                Outcome::Changed
            }
            FormAction::ActivateAction(_) => Outcome::Unchanged,
        }
    }

    fn form_page(&self) -> usize {
        self.modal_list
            .height
            .saturating_sub(FORM_ACTIONS.len() as u16)
            .max(1) as usize
    }

    fn apply_form_key(&mut self, key: &crossterm::event::KeyEvent) -> Outcome {
        let name = self.form_name.clone();
        let name_error = (name.chars().count() < 3).then_some(FORM_NAME_ERROR);
        let theme = THEMES[self.themes.selected.min(THEMES.len() - 1)];
        let rows = [
            FormRow::header("Editor"),
            FormRow::toggle(1, "vim keys", self.form_vim),
            FormRow::select(2, "theme", theme),
            FormRow::stepper(3, "indent", self.form_indent, 1, 8),
            FormRow {
                error: name_error,
                ..FormRow::field(4, "name", name.as_str())
            },
            FormRow::multi_select(5, "features", &FORM_FEATURES, &self.form_features),
        ];
        let page = self.form_page();
        let action = self
            .form
            .handle_key_with_actions(key, &rows, &FORM_ACTIONS, page);
        self.apply_form(action)
    }

    fn apply_overlay(&mut self, action: OverlayAction) -> Outcome {
        match action {
            OverlayAction::Ignored => Outcome::Unchanged,
            OverlayAction::Changed | OverlayAction::Unfocus | OverlayAction::UnfocusAlt => {
                Outcome::Changed
            }
        }
    }

    fn apply_modal_action(&mut self, action: ModalAction) -> Outcome {
        match action {
            ModalAction::Close => {
                self.close_modal();
                Outcome::Changed
            }
            ModalAction::Tab(_) => Outcome::Changed,
            ModalAction::Shortcut(id) if id == SAVE => {
                self.save();
                Outcome::Changed
            }
            ModalAction::Shortcut(id) if id == CANCEL => {
                self.close_modal();
                Outcome::Changed
            }
            ModalAction::Shortcut(_) | ModalAction::Unhandled => Outcome::Unchanged,
        }
    }

    fn draw_preferences(&mut self, area: Rect, f: &mut Frame, cx: &mut FrameCtx) {
        let chrome = Modal::new("Preferences")
            .tabs(&TABS)
            .shortcuts(&self.shortcuts);
        let Some(body) = chrome.render(area, f.buffer_mut(), cx, &mut self.modal) else {
            return;
        };
        match self.modal.active_tab {
            0 => {
                let [list_area, prog, _, btns] = Layout::vertical([
                    Constraint::Min(3),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(3),
                ])
                .areas(body);
                render_list(&mut self.themes, &THEMES, list_area, f.buffer_mut(), cx);
                let frac = if THEMES.is_empty() {
                    0.0
                } else {
                    (self.themes.selected as f32 + 1.0) / THEMES.len() as f32
                };
                render_progress(prog, frac, f.buffer_mut(), cx);
                self.modal_list = list_area;
                let [save, _, cancel] = Layout::horizontal([
                    Constraint::Length(10),
                    Constraint::Length(2),
                    Constraint::Length(10),
                ])
                .flex(Flex::Center)
                .areas(btns);
                Button::new("Save")
                    .id(SAVE_BTN)
                    .focused(false)
                    .render(save, f.buffer_mut(), cx);
                Button::new("Cancel").id(CANCEL_BTN).focused(false).render(
                    cancel,
                    f.buffer_mut(),
                    cx,
                );
            }
            1 => {
                render_picker(
                    &mut self.picker,
                    &Self::picker_items(),
                    body,
                    f.buffer_mut(),
                    cx,
                );
                self.modal_list = body;
                let items = Self::picker_items();
                if let Some(i) = self.picker.selected_id(&items) {
                    let text = format!(
                        "Picker row {i}\nOpens a command or help entry.\nUsed to demo the preview overlay.\nLine four.\nLine five.\nLine six."
                    );
                    let cfg = PreviewConfig {
                        preview_lines: 2,
                        hint: Some("preview"),
                        ..PreviewConfig::default()
                    };
                    render_preview(&text, body, f.buffer_mut(), cx, &cfg);
                }
            }
            2 => {
                render_choices(
                    &mut self.choices,
                    &self.choice_items,
                    body,
                    f.buffer_mut(),
                    cx,
                );
                self.modal_list = body;
            }
            3 => {
                let [form_area, checks] =
                    Layout::vertical([Constraint::Min(5), Constraint::Length(5)]).areas(body);
                let name = self.form_name.clone();
                let name_error = (name.chars().count() < 3).then_some(FORM_NAME_ERROR);
                let theme = THEMES[self.themes.selected.min(THEMES.len() - 1)];
                let rows = [
                    FormRow::header("Editor"),
                    FormRow::toggle(1, "vim keys", self.form_vim),
                    FormRow::select(2, "theme", theme),
                    FormRow::stepper(3, "indent", self.form_indent, 1, 8),
                    FormRow {
                        error: name_error,
                        ..FormRow::field(4, "name", name.as_str())
                    },
                    FormRow::multi_select(5, "features", &FORM_FEATURES, &self.form_features),
                ];
                render_form_with_actions(
                    &mut self.form,
                    &rows,
                    &FORM_ACTIONS,
                    form_area,
                    f.buffer_mut(),
                    cx,
                );
                render_checklist(
                    &mut self.check_list,
                    &self.checks,
                    checks,
                    f.buffer_mut(),
                    cx,
                );
                self.modal_list = form_area;
            }
            _ => {
                let style = PromptStyle {
                    focused: true,
                    placeholder: "type and enter",
                    placeholder_when_focused: true,
                    ..PromptStyle::default()
                };
                self.prompt
                    .render_with_ghost(body, f.buffer_mut(), cx, &style, Some(&self.ghost));
                self.modal_list = body;
            }
        }
    }
}

impl App for SettingsApp {
    type Msg = ();

    fn take_theme_request(&mut self) -> Option<ThemeDefinition> {
        self.pending_theme.take()
    }

    fn tick_demand(&self) -> TickDemand {
        if self.spinner.running {
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
        if self.modal_open
            && self.modal.active_tab == 4
            && !self.cheatsheet_open
            && !self.dropdown.open
        {
            match &ev {
                AppEvent::Hit(id, pointer) => {
                    if self.prompt.handle_hit(*id, *pointer).is_handled() {
                        return Outcome::Changed;
                    }
                }
                AppEvent::Key(key) if self.prompt.has_active_addon() => {
                    return if self.prompt.handle_key(key).is_handled() {
                        Outcome::Changed
                    } else {
                        Outcome::Unchanged
                    };
                }
                _ => {}
            }
        }
        match ev {
            AppEvent::TerminalInfo(info) => {
                if let Some(version) = info.xtversion {
                    self.status = format!("terminal: {}", version.as_str());
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            AppEvent::Hit(_, Pointer::Move { .. }) | AppEvent::Miss(Pointer::Move { .. }) => {
                Outcome::Changed
            }

            AppEvent::Hit(id, Pointer::Down { .. }) if id == TOAST_CLOSE => {
                self.toast.dismiss();
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if id == TOAST_BODY => Outcome::Unchanged,

            AppEvent::Miss(Pointer::Down { .. }) if self.cheatsheet_open => {
                self.close_cheatsheet();
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if self.cheatsheet_open && id == MODAL_BODY => {
                Outcome::Unchanged
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if self.cheatsheet_open => {
                let chrome = Modal::new("Shortcuts");
                if let Some(action) = chrome.hit_action(id) {
                    if action == ModalAction::Close {
                        self.close_cheatsheet();
                    }
                    return Outcome::Changed;
                }
                if let Some(i) = parse_cheatsheet_row(id) {
                    let n = self.cheatsheet_len();
                    self.cheatsheet.list.select_visible(i, n, 12);
                }
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Scroll { delta_lines, .. }) if self.cheatsheet_open => {
                let n = self.cheatsheet_len();
                if parse_cheatsheet_row(id).is_some() || is_scroll_id(NS_CHEAT, id) {
                    self.cheatsheet.list.handle_scroll(delta_lines, n, 12);
                }
                Outcome::Changed
            }
            AppEvent::Key(key) if self.cheatsheet_open => {
                if key!(Esc).matches(&key) {
                    self.close_cheatsheet();
                    return Outcome::Changed;
                }
                let entries = self.cheatsheet_entries();
                self.cheatsheet.handle_key(&key, &entries, 12);
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
            AppEvent::Hit(id, Pointer::Down { .. }) if id == DROPDOWN_BODY => Outcome::Unchanged,
            AppEvent::Hit(id, Pointer::Down { .. }) if self.dropdown.open => {
                if let Some(i) = parse_dropdown_row(id) {
                    self.dropdown.selected = i;
                    return self.apply_dropdown(DropdownAction::Confirmed(i));
                }
                self.dropdown.close();
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Scroll { delta_lines, .. }) if self.dropdown.open => {
                if parse_dropdown_row(id).is_some()
                    || id == DROPDOWN_BODY
                    || is_dropdown_scrollbar(id)
                {
                    self.dropdown.handle_scroll(delta_lines, THEMES.len());
                }
                Outcome::Changed
            }
            AppEvent::Key(key) if self.dropdown.open => {
                let action = self.dropdown.handle_key(&key, THEMES.len());
                self.apply_dropdown(action)
            }

            AppEvent::Miss(Pointer::Down { .. }) if self.modal_open => {
                self.close_modal();
                Outcome::Changed
            }

            AppEvent::Hit(id, Pointer::Down { .. }) if id == MODAL_BODY => Outcome::Unchanged,

            AppEvent::Hit(id, Pointer::Down { row, .. }) if self.modal_open => {
                if let Some(action) = self.chrome().hit_action(id) {
                    return self.apply_modal_action(action);
                }
                if id == SAVE_BTN {
                    self.save();
                    return Outcome::Changed;
                }
                if id == CANCEL_BTN {
                    self.close_modal();
                    return Outcome::Changed;
                }
                match self.modal.active_tab {
                    0 => {
                        if let Some(i) = parse_row_id(id) {
                            self.themes.select_visible(i, THEMES.len(), 8);
                        } else if is_scroll_id(NS_ROW, id) {
                            click_scrollbar(&mut self.themes, row, self.modal_list, THEMES.len());
                        }
                    }
                    1 => {
                        let items = Self::picker_items();
                        self.picker.handle_hit(id, row, self.modal_list, &items);
                    }
                    2 => {
                        if let Some(item_id) = parse_choice_row(id) {
                            let _ = self.choices.click(item_id, &self.choice_items, 8);
                        } else if is_scroll_id(NS_CHOICE, id) {
                            let vis = self.choices.vis_len(&self.choice_items);
                            click_scrollbar(&mut self.choices.list, row, self.modal_list, vis);
                        }
                    }
                    3 => {
                        let name = self.form_name.clone();
                        let name_error = (name.chars().count() < 3).then_some(FORM_NAME_ERROR);
                        let theme = THEMES[self.themes.selected.min(THEMES.len() - 1)];
                        let rows = [
                            FormRow::header("Editor"),
                            FormRow::toggle(1, "vim keys", self.form_vim),
                            FormRow::select(2, "theme", theme),
                            FormRow::stepper(3, "indent", self.form_indent, 1, 8),
                            FormRow {
                                error: name_error,
                                ..FormRow::field(4, "name", name.as_str())
                            },
                            FormRow::multi_select(
                                5,
                                "features",
                                &FORM_FEATURES,
                                &self.form_features,
                            ),
                        ];
                        if let Some(action_id) = parse_form_action(id) {
                            let action = self.form.click_action(action_id, &FORM_ACTIONS);
                            return self.apply_form(action);
                        }
                        if let Some(option) = parse_form_option(id) {
                            let action = self.form.click_multi_option(option, &rows);
                            return self.apply_form(action);
                        }
                        if let Some(i) = parse_form_row(id) {
                            let page = self.form_page();
                            let action = self.form.click_row(i, &rows, page);
                            return self.apply_form(action);
                        } else if let Some(i) = parse_check_row(id) {
                            self.check_list.select_visible(i, self.checks.len(), 6);
                            if let Some(item) = self.checks.get_mut(i) {
                                item.status = item.status.cycle();
                            }
                        }
                    }
                    _ => {}
                }
                Outcome::Changed
            }

            AppEvent::Hit(id, Pointer::Down { row, .. }) => {
                if id == BANNER_HIDE {
                    self.banner_visible = false;
                    return Outcome::Changed;
                }
                if id == DOCS {
                    self.status = "docs".into();
                    return Outcome::Changed;
                }
                if id == CHIP_THEME_CLOSE {
                    self.hide_theme_chip = true;
                    return Outcome::Changed;
                }
                if id.namespace() == NS_CHIP_CHOICE {
                    if let Some(choice) = self.choice_items.get(id.index() as usize) {
                        self.choices.picked.remove(choice.id());
                    }
                    return Outcome::Changed;
                }
                if let Some(i) = parse_tab(id) {
                    self.themes.selected = i.min(THEMES.len() - 1);
                    self.hide_theme_chip = false;
                    return Outcome::Changed;
                }
                if let Some(i) = parse_hint_id(id) {
                    let defs = self.registry.hints(&[DemoCtx::Home, DemoCtx::Always]);
                    if let Some(def) = defs.get(i) {
                        return self.dispatch(def.id, &def.default_key.to_key_event());
                    }
                    return Outcome::Unchanged;
                }
                if let Some(i) = parse_row_id(id) {
                    self.categories.select_visible(i, CATEGORIES.len(), 8);
                    self.overlay.focused = true;
                } else if is_scroll_id(NS_ROW, id) {
                    click_scrollbar(
                        &mut self.categories,
                        row,
                        self.overlay_list,
                        CATEGORIES.len(),
                    );
                }
                Outcome::Changed
            }

            AppEvent::Hit(id, Pointer::Scroll { delta_lines, .. }) => {
                if self.modal_open {
                    match self.modal.active_tab {
                        0 if parse_row_id(id).is_some() || is_scroll_id(NS_ROW, id) => {
                            self.themes.handle_scroll(delta_lines, THEMES.len(), 8);
                        }
                        1 if parse_picker_row(id).is_some()
                            || parse_picker_group(id).is_some()
                            || parse_picker_expander(id).is_some()
                            || is_scroll_id(NS_PICKER, id) =>
                        {
                            self.picker.handle_scroll(
                                delta_lines,
                                self.modal_list,
                                &Self::picker_items(),
                            );
                        }
                        2 if parse_choice_row(id).is_some() || is_scroll_id(NS_CHOICE, id) => {
                            let n = self.choices.vis_len(&self.choice_items);
                            self.choices.list.handle_scroll(delta_lines, n, 8);
                        }
                        3 if parse_form_row(id).is_some() || is_scroll_id(NS_FORM, id) => {
                            self.form.handle_scroll(delta_lines);
                        }
                        _ => {}
                    }
                } else if parse_row_id(id).is_some() || is_scroll_id(NS_ROW, id) {
                    self.categories
                        .handle_scroll(delta_lines, CATEGORIES.len(), 8);
                }
                Outcome::Changed
            }

            AppEvent::Key(key) if self.modal_open => {
                let child_owns_keys = match self.modal.active_tab {
                    2 => self.choices.is_editing_other(),
                    3 => self.form.editing.is_some() || self.form.expanded_multi.is_some(),
                    4 => self.prompt.has_active_addon(),
                    _ => false,
                };
                if !child_owns_keys {
                    let action = Modal::new("Preferences")
                        .tabs(&TABS)
                        .shortcuts(&self.shortcuts)
                        .handle_key(&key, &mut self.modal);
                    if action != ModalAction::Unhandled {
                        return self.apply_modal_action(action);
                    }
                }
                match self.modal.active_tab {
                    0 => {
                        if key!(Enter).matches(&key) {
                            self.save();
                        } else {
                            self.themes.handle_key(&key, THEMES.len(), 8);
                        }
                    }
                    1 => {
                        let items = Self::picker_items();
                        if key!(Enter).matches(&key) {
                            if let Some(i) = self.picker.selected_id(&items) {
                                self.status = format!("picked #{i}");
                            }
                        } else {
                            self.picker.handle_key(&key, &items, 8);
                        }
                    }
                    2 => match self.choices.handle_key(&key, &self.choice_items, 8) {
                        ChoiceAction::Submit => {
                            self.status = format!("choices {:?}", self.choices.picked);
                            self.modal_open = false;
                        }
                        ChoiceAction::OtherEditStart => {
                            self.status = "Other: type freely · enter commit · esc cancel".into();
                        }
                        ChoiceAction::OtherEditCommit => {
                            self.status = format!("other={:?}", self.choices.other());
                        }
                        ChoiceAction::OtherEditCancel => {
                            self.status = "discarded other draft".into();
                        }
                        ChoiceAction::Changed | ChoiceAction::Ignored => {}
                    },
                    3 => {
                        let action = self.apply_form_key(&key);
                        if action == Outcome::Unchanged {
                            match handle_checklist_key(
                                &mut self.check_list,
                                &key,
                                self.checks.len(),
                                6,
                            ) {
                                CheckAction::Cycled(i) => {
                                    if let Some(item) = self.checks.get_mut(i) {
                                        item.status = item.status.cycle();
                                    }
                                }
                                CheckAction::Changed | CheckAction::Unhandled => {}
                            }
                        }
                    }
                    _ => {
                        match self.ghost.handle_key(
                            &key,
                            self.prompt.text(),
                            self.prompt.cursor_at_end(),
                        ) {
                            GhostAction::Accepted(rest) => {
                                self.prompt.insert_str(&rest);
                                return Outcome::Changed;
                            }
                            GhostAction::Dismissed => return Outcome::Changed,
                            GhostAction::Unhandled => {}
                        }
                        match self.prompt.route_enter(&key) {
                            EnterOutcome::Submit => {
                                if let Some(text) = self.prompt.try_send() {
                                    self.status = format!("prompt: {text}");
                                    self.prompt.set_text("");
                                }
                            }
                            EnterOutcome::NewlineInserted | EnterOutcome::Handled => {}
                            EnterOutcome::PassThrough => {
                                let _ = self.prompt.handle_key(&key);
                            }
                        }
                    }
                }
                Outcome::Changed
            }

            AppEvent::Key(key) if self.overlay.focused && self.overlay.visible => {
                if let Some(action) = handle_overlay_key(&mut self.overlay, &key) {
                    return self.apply_overlay(action);
                }
                if let Some(action) = handle_overlay_nav_key(&mut self.overlay, &key) {
                    return self.apply_overlay(action);
                }
                if let Some(id) = self.registry.lookup(&key, DemoCtx::Overlay) {
                    return self.dispatch(id, &key);
                }
                if self.categories.handle_key(&key, CATEGORIES.len(), 8) {
                    return Outcome::Changed;
                }
                if let Some(id) = self
                    .registry
                    .lookup_in(&key, [DemoCtx::Home, DemoCtx::Always])
                {
                    return self.dispatch(id, &key);
                }
                Outcome::Unchanged
            }

            AppEvent::Key(key) => {
                if let Some(id) = self
                    .registry
                    .lookup_in(&key, [DemoCtx::Home, DemoCtx::Always])
                {
                    return self.dispatch(id, &key);
                }
                if key!(Esc).matches(&key) {
                    return Outcome::Quit;
                }
                Outcome::Unchanged
            }
            AppEvent::Tick => {
                if self
                    .pending_quit
                    .as_ref()
                    .is_some_and(|p| p.expired(Instant::now()))
                {
                    self.pending_quit = None;
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            _ => Outcome::Unchanged,
        }
    }

    fn draw(&mut self, f: &mut Frame, cx: &mut FrameCtx) {
        let area = f.area();
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(" settings ")
            .borders(Borders::ALL)
            .style(cx.style(ThemeRole::Surface, ThemeState::Normal));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mut body_y = inner.y;
        if self.banner_visible && !self.modal_open && !self.cheatsheet_open {
            let h = Banner::info("gbtui P10")
                .message("Form · Preview · Ghost · Checklist")
                .render(inner, f.buffer_mut(), cx);
            body_y = body_y.saturating_add(h);
        }

        let overlay_w = if self.overlay.fullscreen {
            inner.width
        } else {
            26.min(inner.width)
        };
        if self.overlay.visible && overlay_w > 0 {
            let ov = Rect {
                x: inner.x,
                y: body_y,
                width: overlay_w,
                height: inner.bottom().saturating_sub(body_y).saturating_sub(3),
            };
            let ov_block = Block::default()
                .title(if self.overlay.focused {
                    " overlay "
                } else {
                    " overlay (unfocused) "
                })
                .borders(Borders::ALL)
                .border_style(cx.style(
                    ThemeRole::Border,
                    if self.overlay.focused {
                        ThemeState::Focused
                    } else {
                        ThemeState::Normal
                    },
                ));
            let list_area = ov_block.inner(ov);
            f.render_widget(ov_block, ov);
            render_list(
                &mut self.categories,
                &CATEGORIES,
                list_area,
                f.buffer_mut(),
                cx,
            );
            self.overlay_list = list_area;
        }

        let spin_x = inner.right().saturating_sub(12);
        self.spinner.render(spin_x, inner.y, f.buffer_mut(), cx);
        Link::new("docs", "https://example.com").id(DOCS).render(
            Rect::new(spin_x.saturating_sub(6), inner.y, 5, 1),
            f.buffer_mut(),
            cx,
        );

        let content_x = if self.overlay.visible {
            inner.x.saturating_add(overlay_w).saturating_add(1)
        } else {
            inner.x
        };
        let tabs_area = Rect {
            x: content_x,
            y: inner.y.saturating_add(1),
            width: inner.right().saturating_sub(content_x).min(40),
            height: 1,
        };
        if !self.modal_open && !self.cheatsheet_open && tabs_area.width > 8 {
            Tabs::new(&THEMES)
                .active(self.themes.selected.min(THEMES.len() - 1))
                .render(tabs_area, f.buffer_mut(), cx);
            self.drop_anchor = tabs_area;
        }

        let mut chip_x = inner.x;
        let chip_y = inner.bottom().saturating_sub(3);
        if chip_y > inner.y && !self.modal_open && !self.cheatsheet_open {
            if !self.hide_theme_chip {
                let w = Chip::new(THEMES[self.themes.selected.min(THEMES.len() - 1)])
                    .id(CHIP_THEME)
                    .close_id(CHIP_THEME_CLOSE)
                    .render(
                        Rect::new(
                            chip_x,
                            chip_y,
                            inner.width.saturating_sub(chip_x - inner.x),
                            1,
                        ),
                        f.buffer_mut(),
                        cx,
                    );
                chip_x = chip_x.saturating_add(w.saturating_add(1));
            }
            let mut picked: Vec<_> = self.choices.picked.iter().collect();
            picked.sort_unstable();
            for choice_id in picked {
                let Some((i, choice)) = self
                    .choice_items
                    .iter()
                    .enumerate()
                    .find(|(_, choice)| choice.id() == choice_id)
                else {
                    continue;
                };
                let remain = inner.right().saturating_sub(chip_x);
                if remain < 8 {
                    break;
                }
                let w = Chip::new(choice.label())
                    .id(HitId::namespaced(NS_CHIP_CHOICE, i as u32))
                    .close_id(HitId::namespaced(NS_CHIP_CHOICE, i as u32))
                    .render(Rect::new(chip_x, chip_y, remain, 1), f.buffer_mut(), cx);
                chip_x = chip_x.saturating_add(w.saturating_add(1));
            }
        }

        let status_row = Rect {
            x: inner.x,
            y: inner.bottom().saturating_sub(2),
            width: inner.width,
            height: 1,
        };
        let mode = if self.cheatsheet_open {
            "cheatsheet"
        } else if self.modal_open {
            "modal"
        } else if self.dropdown.open {
            "menu"
        } else {
            "home"
        };
        let center = if self.modal_open {
            TABS.get(self.modal.active_tab).copied()
        } else {
            THEMES.get(self.themes.selected).copied()
        };
        let mut bar = StatusBar::new(mode);
        if let Some(c) = center {
            bar = bar.center(c);
        }
        bar = bar.right(self.status.as_str());
        let gw = 12.min(status_row.width / 3);
        let bar_area = Rect {
            x: status_row.x,
            y: status_row.y,
            width: status_row.width.saturating_sub(gw),
            height: 1,
        };
        bar.render(bar_area, f.buffer_mut(), cx);
        UsageGauge::new(8_500, 1_000_000).render(
            Rect {
                x: bar_area.right(),
                y: status_row.y,
                width: gw,
                height: 1,
            },
            f.buffer_mut(),
            cx,
        );

        let hints_row = Rect {
            x: inner.x,
            y: inner.bottom().saturating_sub(1),
            width: inner.width,
            height: 1,
        };
        let now = cx.anim().now();
        let pending = self
            .pending_quit
            .as_ref()
            .filter(|p| !p.expired(now))
            .map(|p| p.hint());
        ShortcutsBar::new(&self.hints)
            .with_pending(pending)
            .render(hints_row, f.buffer_mut(), cx);

        if self.modal_open {
            self.draw_preferences(area, f, cx);
        }

        if self.dropdown.open {
            if let Some(r) = self.form.select_anchor {
                self.drop_anchor = r;
            }
            let pop = dropdown_popup_area(self.drop_anchor, area, THEMES.len(), &self.dropdown);
            self.dropdown_area = pop;
            render_dropdown(
                &mut self.dropdown,
                Self::dropdown_items().into_iter(),
                pop,
                f.buffer_mut(),
                cx,
            );
        }

        if self.cheatsheet_open {
            let chrome = Modal::new("Shortcuts");
            if let Some(body) = chrome.render(area, f.buffer_mut(), cx, &mut self.cheatsheet_modal)
            {
                let entries = self.cheatsheet_entries();
                render_cheatsheet(&mut self.cheatsheet, &entries, body, f.buffer_mut(), cx);
            }
        }

        self.toast.render(area, f.buffer_mut(), cx);
    }
}

fn main() -> std::io::Result<()> {
    run_blocking(SettingsApp::new(), ThemeKind::Auto)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_cancels_nested_editors_before_closing_modal() {
        let mut app = SettingsApp::new();
        app.open_modal(2);
        for code in [
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyCode::Char('j'),
            crossterm::event::KeyCode::Esc,
        ] {
            app.handle(AppEvent::Key(code.into()));
        }
        assert!(app.modal_open);
        assert!(!app.choices.is_editing_other());
        assert_eq!(app.choices.other(), Some(""));

        app.open_modal(3);
        app.form.list.selected = 3;
        app.handle(AppEvent::Key(crossterm::event::KeyCode::Enter.into()));
        assert_eq!(app.form.editing, Some(4));
        app.handle(AppEvent::Key(crossterm::event::KeyCode::Esc.into()));
        assert!(app.modal_open);
        assert_eq!(app.form.editing, None);
        assert_eq!(app.form_name, "gbtui");
    }
}
