//! Rich fuzzy picker over a static file list. Enter prints the selected path.
//!
//! `F2` cycles fuzzy / regex / substring and `F3` switches Filter / Search.
//! `←`/`→` collapse or expand the selected detail row. Type `(` after selecting
//! regex to see a visible compilation error. Group headers are clickable; the
//! long descriptions demonstrate physical scrolling.

#[cfg(test)]
use gbtui::Theme;
use gbtui::crossterm::event::KeyCode;
use gbtui::key;
use gbtui::widgets::{
    ShortcutsBar, parse_hint_id, parse_picker_expander, parse_picker_group, parse_picker_row,
    render_picker,
};
use gbtui::{
    ActionDef, ActionRegistry, App, AppEvent, FrameCtx, HintItem, Outcome, PickerEntry,
    PickerGroup, PickerRow, PickerState, Pointer, ThemeKind, ThemeRole, ThemeState, TickDemand,
    hint_items, run_blocking,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

const FILES: [&str; 12] = [
    "crates/gbtui/src/lib.rs",
    "crates/gbtui-core/src/theme.rs",
    "crates/gbtui-runtime/src/pump.rs",
    "crates/gbtui-widgets/src/picker.rs",
    "crates/gbtui-widgets/src/prompt.rs",
    "crates/gbtui-actions/src/registry.rs",
    "examples/settings/src/main.rs",
    "examples/chat/src/main.rs",
    "docs/plan.md",
    "docs/architecture.md",
    "README.md",
    "Cargo.toml",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickAction {
    Confirm,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickCtx {
    Picker,
}

struct PickerApp {
    picker: PickerState,
    items: Vec<PickerEntry<'static>>,
    picker_area: Rect,
    hints: Vec<HintItem>,
    registry: ActionRegistry<PickAction, PickCtx>,
}

impl PickerApp {
    fn new() -> Self {
        let mut items = Vec::with_capacity(FILES.len() + 4);
        items.push(PickerEntry::group(PickerGroup::new(
            100,
            "Workspace source",
        )));
        for (index, path) in FILES.iter().take(6).enumerate() {
            let mut row = PickerRow::new(index, path)
                .right("Rust")
                .badge("src")
                .summary("workspace module")
                .search_text(path);
            if index == 0 {
                row = row
                    .label(Line::from(vec![
                        Span::styled("crates/", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw("gbtui/src/lib.rs"),
                    ]))
                    .description("A deliberately long description demonstrates that a rich picker row can occupy multiple physical terminal lines without making later rows unreachable.")
                    .always_expanded();
            }
            items.push(PickerEntry::row(row));
        }
        items.push(PickerEntry::group(PickerGroup::new(
            200,
            "Examples and docs",
        )));
        for (index, path) in FILES.iter().enumerate().skip(6) {
            if index == 8 {
                items.push(PickerEntry::group(
                    PickerGroup::new(201, "Documentation").indent(1),
                ));
            } else if index == 11 {
                items.push(PickerEntry::group(PickerGroup::new(202, "Workspace root")));
            }
            items.push(PickerEntry::row(
                PickerRow::new(index, path)
                    .right(if path.ends_with(".md") { "docs" } else { "example" })
                    .summary("searchable file entry")
                    .description("Long rich content wraps at narrow widths; collapse this group, then search for a child to reveal it again.")
                    .search_text(path),
            ));
        }
        let registry = ActionRegistry::new(vec![
            ActionDef::new(PickAction::Confirm, "open", key!(Enter), PickCtx::Picker).hint(0),
            ActionDef::new(PickAction::Quit, "quit", key!(Esc), PickCtx::Picker)
                .alts(vec![key!('q')])
                .hint(1)
                .pinned(),
        ]);
        let hints = hint_items(&registry, &[PickCtx::Picker]);
        Self {
            picker: PickerState::default(),
            items,
            picker_area: Rect::default(),
            hints,
            registry,
        }
    }

    fn confirm(&mut self) -> Outcome {
        let Some(index) = self.picker.selected_id(&self.items) else {
            return Outcome::Unchanged;
        };
        if let Some(path) = FILES.get(index) {
            eprintln!("{path}");
        }
        Outcome::Quit
    }
}

impl App for PickerApp {
    type Msg = ();

    fn tick_demand(&self) -> TickDemand {
        TickDemand::None
    }

    fn handle(&mut self, event: AppEvent<()>) -> Outcome {
        match event {
            AppEvent::Hit(_, Pointer::Move { .. }) | AppEvent::Miss(Pointer::Move { .. }) => {
                Outcome::Changed
            }
            AppEvent::Miss(Pointer::Down { .. }) => Outcome::Unchanged,
            AppEvent::Hit(id, Pointer::Down { row, .. }) => {
                if let Some(hint) = parse_hint_id(id) {
                    return match hint {
                        0 => self.confirm(),
                        _ => Outcome::Quit,
                    };
                }
                if self
                    .picker
                    .handle_hit(id, row, self.picker_area, &self.items)
                {
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            AppEvent::Hit(id, Pointer::Scroll { delta_lines, .. }) => {
                if parse_picker_row(id).is_some()
                    || parse_picker_group(id).is_some()
                    || parse_picker_expander(id).is_some()
                    || gbtui::widgets::is_picker_scroll(id)
                {
                    self.picker
                        .handle_scroll(delta_lines, self.picker_area, &self.items);
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            AppEvent::Key(key) if key.code == KeyCode::F(2) => {
                self.picker.cycle_search_kind();
                Outcome::Changed
            }
            AppEvent::Key(key) if key.code == KeyCode::F(3) => {
                self.picker.toggle_search_mode();
                Outcome::Changed
            }
            AppEvent::Key(key) => {
                if let Some(id) = self.registry.lookup(&key, PickCtx::Picker) {
                    return match id {
                        PickAction::Confirm => self.confirm(),
                        PickAction::Quit => Outcome::Quit,
                    };
                }
                if self.picker.handle_key(
                    &key,
                    &self.items,
                    self.picker_area.height.saturating_sub(1).max(1) as usize,
                ) {
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            _ => Outcome::Unchanged,
        }
    }

    fn draw(&mut self, frame: &mut Frame, cx: &mut FrameCtx) {
        let area = frame.area();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default().style(cx.style(ThemeRole::Surface, ThemeState::Normal)),
            area,
        );
        frame.render_widget(
            Paragraph::new("background (occluded)")
                .style(cx.style(ThemeRole::Muted, ThemeState::Normal)),
            Rect {
                x: area.x + 1,
                y: area.y + 1,
                width: area.width.saturating_sub(2),
                height: 1,
            },
        );
        cx.occlude(area);
        let [popup] = Layout::vertical([Constraint::Percentage(70)])
            .flex(Flex::Center)
            .areas(area);
        let [popup] = Layout::horizontal([Constraint::Percentage(70)])
            .flex(Flex::Center)
            .areas(popup);
        let block = Block::default()
            .title(" files · F2 matcher · F3 mode ")
            .borders(Borders::ALL)
            .style(cx.style(ThemeRole::Surface, ThemeState::Normal))
            .border_style(cx.style(ThemeRole::Border, ThemeState::Normal));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        cx.register(gbtui::widgets::MODAL_BODY, popup);
        let [picker_area, bar] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(inner);
        render_picker(
            &mut self.picker,
            &self.items,
            picker_area,
            frame.buffer_mut(),
            cx,
        );
        self.picker_area = picker_area;
        ShortcutsBar::new(&self.hints).render(bar, frame.buffer_mut(), cx);
    }
}

fn main() -> std::io::Result<()> {
    run_blocking(PickerApp::new(), ThemeKind::Auto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    use crossterm::event::{Event, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
    use gbtui::Pump;
    use gbtui::runtime::HitResult;
    use ratatui::backend::TestBackend;

    fn pump(app: PickerApp) -> Pump<PickerApp, TestBackend> {
        let terminal = gbtui::terminal::Terminal::new(TestBackend::new(80, 24)).unwrap();
        Pump::new(app, terminal, Theme::night(), Instant::now())
    }

    fn mouse(kind: MouseEventKind, col: u16, row: u16) -> Event {
        Event::Mouse(crossterm::event::MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn idle_does_not_tick() {
        let mut pump = pump(PickerApp::new());
        pump.paint().unwrap();
        assert_eq!(pump.tick_deadline(Instant::now()), None);
    }

    #[test]
    fn background_is_occluded() {
        let mut pump = pump(PickerApp::new());
        pump.paint().unwrap();
        assert_eq!(pump.hits().hit(0, 0), HitResult::Occluded);
        pump.drive(
            Instant::now(),
            mouse(MouseEventKind::Down(MouseButton::Left), 0, 0),
        );
        assert_eq!(pump.app.picker.selected_id(&pump.app.items), Some(0));
    }

    #[test]
    fn j_moves_selection_between_rich_rows() {
        let mut pump = pump(PickerApp::new());
        pump.paint().unwrap();
        pump.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
        );
        assert_eq!(pump.app.picker.selected_id(&pump.app.items), Some(1));
    }

    #[test]
    fn typing_filters_and_confirming_no_result_is_unchanged() {
        let mut pump = pump(PickerApp::new());
        pump.drive(
            Instant::now(),
            Event::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)),
        );
        assert_eq!(pump.app.picker.query(), "z");
        assert_eq!(pump.app.picker.visual_len(&pump.app.items), 0);
        assert_eq!(pump.app.confirm(), Outcome::Unchanged);
    }
}
