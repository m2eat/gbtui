//! Theme laboratory: real widgets, all semantic states, contrast, live switching.
//! Run with `cargo run -p gbtui --example themes --features theme-config -- [theme.toml]`.

use std::path::PathBuf;

use gbtui::crossterm::event::{KeyCode, KeyEventKind};
use gbtui::ratatui::Frame;
use gbtui::ratatui::layout::Rect;
use gbtui::ratatui::style::{Modifier, Style};
use gbtui::ratatui::widgets::{Block, Paragraph};
use gbtui::{
    App, AppEvent, BackgroundPolicy, Button, FrameCtx, HitId, Link, Outcome, Pointer, Prompt,
    PromptStyle, ThemeDefinition, ThemeKind, ThemeRegistry, ThemeRole, ThemeState, run_blocking,
};

struct Themes {
    registry: ThemeRegistry,
    selected: String,
    background: BackgroundPolicy,
    pending: Option<ThemeDefinition>,
    file: Option<PathBuf>,
    external: String,
    status: String,
    scroll: usize,
    state: usize,
    prompt: Prompt,
    typing: bool,
}

impl Themes {
    fn select(&mut self, name: &str) {
        if let Some(definition) = self.registry.get(name) {
            self.selected = name.to_owned();
            self.pending = Some(definition.clone().with_background(self.background));
            self.status = format!("Selected {name}; input and scroll state retained");
        }
    }

    fn cycle_background(&mut self) {
        self.background = match self.background {
            BackgroundPolicy::Solid => BackgroundPolicy::TerminalDefault,
            BackgroundPolicy::TerminalDefault => BackgroundPolicy::Transparent,
            BackgroundPolicy::Transparent => BackgroundPolicy::Solid,
        };
        let name = self.selected.clone();
        self.select(&name);
    }

    fn reload(&mut self) {
        let result = match &self.file {
            Some(path) => std::fs::read_to_string(path).map_err(|error| error.to_string()),
            None => Ok(include_str!("aurora.toml").to_owned()),
        }
        .and_then(|text| ThemeDefinition::from_toml(&text).map_err(|error| error.to_string()));
        match result {
            Ok(definition) => {
                self.external = definition.name().to_owned();
                self.background = definition.background();
                self.registry.insert(definition);
                self.select(&self.external.clone());
            }
            Err(error) => {
                self.status = format!("Reload rejected; previous theme retained: {error}")
            }
        }
    }
}

impl App for Themes {
    type Msg = ();

    fn take_theme_request(&mut self) -> Option<ThemeDefinition> {
        self.pending.take()
    }

    fn handle(&mut self, event: AppEvent<()>) -> Outcome {
        match event {
            AppEvent::Key(key) if key.kind != KeyEventKind::Release => {
                if self.typing {
                    if key.code == KeyCode::Esc {
                        self.typing = false;
                    } else {
                        self.prompt.handle_key(&key);
                    }
                    return Outcome::Changed;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Outcome::Quit,
                    KeyCode::Char('1') => self.select("night"),
                    KeyCode::Char('2') => self.select("day"),
                    KeyCode::Char('3') => self.select("auto"),
                    KeyCode::Char('4') => self.select(&self.external.clone()),
                    KeyCode::Char('b') => self.cycle_background(),
                    KeyCode::Char('r') => self.reload(),
                    KeyCode::Char('i') => self.typing = true,
                    KeyCode::Char('s') => self.state = (self.state + 1) % ThemeState::ALL.len(),
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.scroll = (self.scroll + 1).min(ThemeRole::ALL.len().saturating_sub(1));
                    }
                    KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
                    _ => return Outcome::Unchanged,
                }
                Outcome::Changed
            }
            AppEvent::Hit(id, Pointer::Down { .. }) if id == HitId::named("theme.background") => {
                self.cycle_background();
                Outcome::Changed
            }
            AppEvent::Hit(_, Pointer::Move { .. }) | AppEvent::Miss(Pointer::Move { .. }) => {
                Outcome::Changed
            }
            _ => Outcome::Unchanged,
        }
    }

    fn draw(&mut self, frame: &mut Frame, cx: &mut FrameCtx) {
        let area = frame.area();
        frame.render_widget(
            Block::default().style(cx.style(ThemeRole::Surface, ThemeState::Normal)),
            area,
        );
        if area.is_empty() {
            return;
        }
        let row = |y: u16| Rect::new(area.x, area.y.saturating_add(y), area.width, 1);
        let version = cx.theme_version();
        frame.render_widget(
            Paragraph::new(format!(
                "Theme: {}  background: {:?}  version: {version}",
                self.selected, self.background
            ))
            .style(
                cx.style(ThemeRole::Text, ThemeState::Normal)
                    .add_modifier(Modifier::BOLD),
            ),
            row(0),
        );
        if area.height > 1 {
            frame.render_widget(
                Paragraph::new(
                    "1/2/3/4 theme | b bg | r reload | i input | s state | j/k scroll | q quit",
                )
                .style(cx.style(ThemeRole::Muted, ThemeState::Normal)),
                row(1),
            );
        }
        if area.height > 3 {
            let button_width = (area.width / 3).min(24);
            Button::new("Background")
                .id(HitId::named("theme.background"))
                .render(
                    Rect::new(area.x, area.y + 3, button_width, 1),
                    frame.buffer_mut(),
                    cx,
                );
            Button::new("Focused")
                .id(HitId::named("theme.focused"))
                .focused(true)
                .render(
                    Rect::new(area.x + button_width, area.y + 3, button_width, 1),
                    frame.buffer_mut(),
                    cx,
                );
            Link::new("Documentation", "https://docs.rs/gbtui")
                .id(HitId::named("theme.link"))
                .render(
                    Rect::new(
                        area.x + button_width * 2,
                        area.y + 3,
                        area.width.saturating_sub(button_width * 2),
                        1,
                    ),
                    frame.buffer_mut(),
                    cx,
                );
        }
        if area.height > 5 {
            self.prompt.render(
                Rect::new(area.x, area.y + 5, area.width, 1),
                frame.buffer_mut(),
                cx,
                &PromptStyle {
                    focused: self.typing,
                    placeholder: "Press i to edit; Esc keeps text and leaves input mode",
                    placeholder_when_focused: true,
                    ..PromptStyle::default()
                },
            );
        }
        let grid_top = 7u16;
        let grid_height = area.height.saturating_sub(grid_top + 2);
        let states = if area.width >= 100 {
            ThemeState::ALL
        } else {
            &ThemeState::ALL[self.state..self.state + 1]
        };
        let label_width = area.width.min(14);
        let state_width = area.width.saturating_sub(label_width) / states.len() as u16;
        for (index, &role) in ThemeRole::ALL
            .iter()
            .skip(self.scroll)
            .take(grid_height as usize)
            .enumerate()
        {
            let y = area.y + grid_top + index as u16;
            frame.render_widget(
                Paragraph::new(role.name()).style(cx.style(ThemeRole::Text, ThemeState::Normal)),
                Rect::new(area.x, y, label_width, 1),
            );
            for (column, &state) in states.iter().enumerate() {
                let contrast = cx
                    .resolved_theme()
                    .and_then(|theme| theme.contrast(role, state));
                let ratio = match contrast {
                    Some(value) if value < 4.5 => format!("{value:.1}!"),
                    Some(value) => format!("{value:.1}"),
                    None => "?".into(),
                };
                frame.render_widget(
                    Paragraph::new(format!("{} {ratio}", state.name()))
                        .style(cx.style(role, state)),
                    Rect::new(
                        area.x + label_width + column as u16 * state_width,
                        y,
                        state_width,
                        1,
                    ),
                );
            }
        }
        if area.height > 2 {
            frame.render_widget(
                Paragraph::new("Contrast estimate: ! below 4.5, ? unknown terminal RGB. ANSI palette is terminal-owned.")
                    .style(cx.style(ThemeRole::Muted, ThemeState::Normal)),
                row(area.height - 2),
            );
            frame.render_widget(
                Paragraph::new(&*self.status)
                    .style(Style::default().patch(cx.style(ThemeRole::Text, ThemeState::Normal))),
                row(area.height - 1),
            );
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = std::env::args_os().nth(1).map(PathBuf::from);
    let text = match &file {
        Some(path) => std::fs::read_to_string(path)?,
        None => include_str!("aurora.toml").to_owned(),
    };
    let external = ThemeDefinition::from_toml(&text)?;
    let external_name = external.name().to_owned();
    let mut registry = ThemeRegistry::default();
    registry.insert(external.clone());
    let definition = if file.is_some() {
        external
    } else {
        ThemeDefinition::from(ThemeKind::Auto).with_background(BackgroundPolicy::TerminalDefault)
    };
    let app = Themes {
        selected: definition.name().to_owned(),
        background: definition.background(),
        registry,
        pending: None,
        file,
        external: external_name,
        status: "Live theme laboratory; solid/terminal/transparent preserve foreground colors"
            .into(),
        scroll: 0,
        state: 0,
        prompt: Prompt::new(),
        typing: false,
    };
    run_blocking(app, definition)?;
    Ok(())
}
