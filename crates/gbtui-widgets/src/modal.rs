//! Modal chrome: border, title, close, optional tabs, footer shortcuts.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/views/modal_window.rs (STRIP)`
//!
//! ```text
//!  ┌─ title ──────────────[✗]─┐
//!  │ tab1  tab2               │
//!  │                          │
//!  │         body             │
//!  │                          │
//!  │     Esc:close  Enter:ok  │
//!  └──────────────────────────┘
//! ```
//!
//! # Keys
//!
//! `Esc` close · `Tab`/`→` next tab · `Shift+Tab`/`←` previous tab ·
//! footer shortcuts match [`ModalShortcut::key`].
//!
//! Popup kind occludes the full `frame` then registers `BODY` so inside
//! clicks are not treated as outside-close. No global embedded flag: pass
//! [`ModalKind`].

use crossterm::event::KeyEvent;
use gbtui_core::{HitId, KeyShortcut, SafeBuf, ThemeRole, ThemeState, key};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Clear, Widget};
use unicode_width::UnicodeWidthStr;

use crate::paint::fill;
use crate::tabs::{self, Tabs};

pub use crate::tabs::{NS_TAB, tab_id};

pub const NS_SHORTCUT: u32 = 0x4D02;
pub const CLOSE: HitId = HitId::named("gbtui.modal.close");
pub const BODY: HitId = HitId::named("gbtui.modal.body");

/// Popup is centered and occludes the frame; Embedded fills the given area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModalKind {
    #[default]
    Popup,
    Embedded,
}

/// Popup size as a fraction of the frame, with min/max clamps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModalSizing {
    pub width_pct: f32,
    pub max_width: u16,
    pub min_width: u16,
    pub v_margin: u16,
    pub h_pad: u16,
    pub v_pad: u16,
    pub footer_lines: u16,
}

impl Default for ModalSizing {
    fn default() -> Self {
        Self {
            width_pct: 0.9,
            max_width: 140,
            min_width: 60,
            v_margin: 7,
            h_pad: 2,
            v_pad: 2,
            footer_lines: 2,
        }
    }
}

impl ModalSizing {
    pub fn medium() -> Self {
        Self {
            width_pct: 0.60,
            max_width: 120,
            min_width: 44,
            v_margin: 4,
            h_pad: 2,
            v_pad: 1,
            footer_lines: 2,
        }
    }
}

/// Footer hint: key + label, clickable via [`id`](Self::id).
#[derive(Debug, Clone, Copy)]
pub struct ModalShortcut {
    pub id: HitId,
    pub key: KeyShortcut,
    pub label: &'static str,
}

/// Result of [`Modal::handle_key`] / [`Modal::hit_action`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalAction {
    Close,
    Tab(usize),
    Shortcut(HitId),
    Unhandled,
}

/// Kind, active tab, and last painted popup rect.
#[derive(Debug, Clone)]
pub struct ModalState {
    pub kind: ModalKind,
    pub active_tab: usize,
    pub popup: Option<Rect>,
}

impl ModalState {
    pub fn popup() -> Self {
        Self {
            kind: ModalKind::Popup,
            active_tab: 0,
            popup: None,
        }
    }

    pub fn embedded() -> Self {
        Self {
            kind: ModalKind::Embedded,
            active_tab: 0,
            popup: None,
        }
    }
}

/// Modal chrome. Call [`render`](Self::render) then paint the body into the
/// returned inner rect.
pub struct Modal<'a> {
    pub title: &'a str,
    pub tabs: &'a [&'a str],
    pub shortcuts: &'a [ModalShortcut],
    pub sizing: ModalSizing,
}

impl<'a> Modal<'a> {
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            tabs: &[],
            shortcuts: &[],
            sizing: ModalSizing::medium(),
        }
    }

    pub fn tabs(mut self, tabs: &'a [&'a str]) -> Self {
        self.tabs = tabs;
        self
    }

    pub fn shortcuts(mut self, shortcuts: &'a [ModalShortcut]) -> Self {
        self.shortcuts = shortcuts;
        self
    }

    pub fn sizing(mut self, sizing: ModalSizing) -> Self {
        self.sizing = sizing;
        self
    }

    /// Paint chrome and return the inner body rect (or `None` if too small).
    pub fn render(
        &self,
        frame: Rect,
        buf: &mut Buffer,
        cx: &mut FrameCtx<'_>,
        state: &mut ModalState,
    ) -> Option<Rect> {
        let popup = match state.kind {
            ModalKind::Embedded => frame,
            ModalKind::Popup => {
                let (w, h) = popup_dims(frame, &self.sizing);
                if w < 20 || h < 6 {
                    state.popup = None;
                    return None;
                }
                Rect {
                    x: frame.x + (frame.width.saturating_sub(w)) / 2,
                    y: frame.y + (frame.height.saturating_sub(h)) / 2,
                    width: w,
                    height: h,
                }
            }
        };
        state.popup = Some(popup);

        if state.kind == ModalKind::Popup {
            cx.occlude(frame);
        }
        cx.register(BODY, popup);

        // `fill` writes blank symbols even when the resolved surface background
        // is Reset, preventing transparent popups from retaining prior-frame text.
        Clear.render(popup, buf);
        let bg = cx.style(ThemeRole::Surface, ThemeState::Normal);
        fill(buf, popup, bg);

        let mut inner = popup;
        if state.kind == ModalKind::Popup {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", self.title))
                .border_style(bg.patch(cx.style(ThemeRole::Border, ThemeState::Normal)))
                .title_style(
                    bg.patch(cx.style(ThemeRole::Text, ThemeState::Normal))
                        .add_modifier(Modifier::BOLD),
                )
                .style(bg);
            inner = block.inner(popup);
            block.render(popup, buf);
            paint_close(popup, buf, cx);
        } else if !self.title.is_empty() {
            buf.set_string_safe(
                popup.x + self.sizing.h_pad,
                popup.y,
                self.title,
                bg.patch(cx.style(ThemeRole::Text, ThemeState::Normal))
                    .add_modifier(Modifier::BOLD),
            );
            inner.y = inner.y.saturating_add(1);
            inner.height = inner.height.saturating_sub(1);
        }

        inner.x = inner.x.saturating_add(self.sizing.h_pad);
        inner.width = inner
            .width
            .saturating_sub(self.sizing.h_pad.saturating_mul(2));
        if self.sizing.v_pad > 0 && inner.height > self.sizing.v_pad {
            inner.y = inner.y.saturating_add(self.sizing.v_pad);
            inner.height = inner.height.saturating_sub(self.sizing.v_pad);
        }

        if !self.tabs.is_empty() && inner.height > 1 {
            Tabs::new(self.tabs)
                .active(state.active_tab)
                .render(inner, buf, cx);
            inner.y = inner.y.saturating_add(1);
            inner.height = inner.height.saturating_sub(1);
        }

        if !self.shortcuts.is_empty() && inner.height > self.sizing.footer_lines {
            let footer = Rect {
                x: inner.x,
                y: inner.bottom().saturating_sub(self.sizing.footer_lines),
                width: inner.width,
                height: self.sizing.footer_lines,
            };
            inner.height = inner.height.saturating_sub(self.sizing.footer_lines);
            paint_shortcuts(self.shortcuts, footer, buf, cx);
        }

        Some(inner)
    }

    pub fn handle_key(&self, key: &KeyEvent, state: &mut ModalState) -> ModalAction {
        if key!(Esc).matches(key) {
            return ModalAction::Close;
        }
        if !self.tabs.is_empty()
            && let Some(i) = Tabs::handle_key(key, state.active_tab, self.tabs.len())
        {
            state.active_tab = i;
            return ModalAction::Tab(state.active_tab);
        }
        for s in self.shortcuts {
            if s.key.matches(key) {
                return ModalAction::Shortcut(s.id);
            }
        }
        ModalAction::Unhandled
    }

    /// Map a hit id produced by this modal to an action.
    pub fn hit_action(&self, id: HitId) -> Option<ModalAction> {
        if id == CLOSE {
            return Some(ModalAction::Close);
        }
        if let Some(i) = tabs::parse_tab(id) {
            return Some(ModalAction::Tab(i));
        }
        if let Some(s) = self.shortcuts.iter().find(|s| s.id == id) {
            return Some(ModalAction::Shortcut(s.id));
        }
        if id.namespace() == NS_SHORTCUT {
            return self
                .shortcuts
                .get(id.index() as usize)
                .map(|s| ModalAction::Shortcut(s.id));
        }
        None
    }
}

pub fn shortcut_hit_id(i: usize) -> HitId {
    HitId::namespaced(NS_SHORTCUT, i as u32)
}

fn popup_dims(area: Rect, sizing: &ModalSizing) -> (u16, u16) {
    let w = ((area.width as f32 * sizing.width_pct) as u16)
        .clamp(sizing.min_width.min(area.width), sizing.max_width)
        .min(area.width);
    let h = area
        .height
        .saturating_sub(sizing.v_margin.saturating_mul(2));
    (w, h.max(6))
}

fn paint_close(popup: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) {
    let label = "[✗]";
    let w = label.width() as u16;
    if popup.width <= w + 2 {
        return;
    }
    let area = Rect {
        x: popup.right().saturating_sub(w + 1),
        y: popup.y,
        width: w,
        height: 1,
    };
    cx.register(CLOSE, area);
    let state = ThemeState::from_flags(false, false, false, false, cx.hovered(area));
    let style = if state == ThemeState::Hovered {
        cx.style(ThemeRole::Surface, ThemeState::Normal)
            .patch(cx.style(ThemeRole::Hover, state))
            .patch(cx.style(ThemeRole::Error, state))
    } else {
        cx.style(ThemeRole::Surface, ThemeState::Normal)
            .patch(cx.style(ThemeRole::Muted, state))
    };
    buf.set_string_safe(area.x, area.y, label, style);
}

fn paint_shortcuts(
    shortcuts: &[ModalShortcut],
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) {
    let mut parts: Vec<String> = Vec::new();
    for s in shortcuts {
        parts.push(format!("{}:{}", s.key, s.label));
    }
    let line = parts.join("   ");
    let w = line.width() as u16;
    let mut x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y;
    for (i, s) in shortcuts.iter().enumerate() {
        let text = format!("{}:{}", s.key, s.label);
        let tw = text.width() as u16;
        let r = Rect {
            x,
            y,
            width: tw,
            height: 1,
        };
        cx.register(shortcut_hit_id(i), r);
        cx.register(s.id, r);
        let state = ThemeState::from_flags(false, false, false, false, cx.hovered(r));
        let style = if state == ThemeState::Hovered {
            cx.style(ThemeRole::Surface, ThemeState::Normal)
                .patch(cx.style(ThemeRole::Hover, state))
                .patch(cx.style(ThemeRole::Accent, state))
                .add_modifier(Modifier::BOLD)
        } else {
            cx.style(ThemeRole::Surface, ThemeState::Normal)
                .patch(cx.style(ThemeRole::Muted, state))
        };
        buf.set_string_safe(x, y, &text, style);
        x = x.saturating_add(tw + 3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;
    use gbtui_core::{
        AnimationClock, BackgroundPolicy, Theme, ThemeContext, ThemeDefinition, ThemeResolver,
    };
    use gbtui_runtime::HitMap;
    use ratatui::style::{Color, Style};
    use std::time::Instant;

    fn cx<'a>(
        theme: &'a Theme,
        mouse: Option<(u16, u16)>,
        anim: &'a mut AnimationClock,
        hits: &'a mut HitMap,
    ) -> FrameCtx<'a> {
        FrameCtx::new(theme, mouse, anim, hits)
    }

    #[test]
    fn popup_registers_close_and_occludes() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = cx(&theme, Some((0, 0)), &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        let mut state = ModalState::popup();
        let body =
            Modal::new("settings").render(Rect::new(0, 0, 80, 24), &mut buf, &mut cx, &mut state);
        assert!(body.is_some());
        assert!(state.popup.is_some());
        assert_eq!(hits.hit(0, 0), gbtui_runtime::HitResult::Occluded);
    }

    #[test]
    fn close_hover_uses_error_color() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        let mut state = ModalState::popup();
        let mouse = {
            let mut cx = cx(&theme, None, &mut anim, &mut hits);
            Modal::new("x").render(Rect::new(0, 0, 80, 24), &mut buf, &mut cx, &mut state);
            let popup = state.popup.unwrap();
            (popup.right().saturating_sub(3), popup.y)
        };
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = cx(&theme, Some(mouse), &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        Modal::new("x").render(Rect::new(0, 0, 80, 24), &mut buf, &mut cx, &mut state);
        assert_eq!(buf[(mouse.0, mouse.1)].fg, theme.error);
    }

    #[test]
    fn esc_closes() {
        let m = Modal::new("x");
        let mut state = ModalState::popup();
        let key = KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE);
        assert_eq!(m.handle_key(&key, &mut state), ModalAction::Close);
    }

    #[test]
    fn tab_cycles() {
        let m = Modal::new("x").tabs(&["a", "b"]);
        let mut state = ModalState::popup();
        let key = KeyEvent::new(KeyCode::Tab, crossterm::event::KeyModifiers::NONE);
        assert_eq!(m.handle_key(&key, &mut state), ModalAction::Tab(1));
        assert_eq!(m.handle_key(&key, &mut state), ModalAction::Tab(0));
    }

    #[test]
    fn embedded_does_not_occlude_outside_its_frame() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = cx(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        let mut state = ModalState::embedded();
        Modal::new("x").render(Rect::new(10, 8, 40, 10), &mut buf, &mut cx, &mut state);
        assert_eq!(hits.hit(0, 0), gbtui_runtime::HitResult::Miss);
        assert_eq!(hits.hit(12, 9), gbtui_runtime::HitResult::Hit(BODY));
    }

    #[test]
    fn transparent_modal_clears_stale_symbols() {
        let definition = ThemeDefinition::from_palette("transparent", Theme::night())
            .with_background(BackgroundPolicy::Transparent);
        let resolver = ThemeResolver::new(definition, ThemeContext::default());
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(resolver.resolved().palette(), None, &mut anim, &mut hits)
            .with_resolved_theme(resolver.resolved());
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        buf.set_string(10, 3, "stale", Style::default().fg(Color::Red));

        let mut state = ModalState::embedded();
        Modal::new("").render(area, &mut buf, &mut cx, &mut state);

        assert_eq!(buf[(10, 3)].symbol(), " ");
    }
}

#[cfg(test)]
mod pump_tests {
    use super::*;
    use std::time::Instant;

    use crossterm::event::{Event, KeyModifiers, MouseButton, MouseEventKind};
    use gbtui_core::Theme;
    use gbtui_runtime::terminal::Terminal;
    use gbtui_runtime::{App, AppEvent, HitResult, Outcome, Pointer, Pump};
    use ratatui::backend::TestBackend;

    const UNDER: HitId = HitId::named("under");
    const SAVE: HitId = HitId::named("gbtui.modal.save");

    struct ModalApp {
        modal: ModalState,
        closed: bool,
        last_hit: Option<HitId>,
        last_miss_down: bool,
        under_clicked: bool,
        tabs: [&'static str; 2],
        shortcuts: [ModalShortcut; 2],
    }

    impl ModalApp {
        fn new() -> Self {
            Self {
                modal: ModalState::popup(),
                closed: false,
                last_hit: None,
                last_miss_down: false,
                under_clicked: false,
                tabs: ["List", "Picker"],
                shortcuts: [
                    ModalShortcut {
                        id: CLOSE,
                        key: key!(Esc),
                        label: "close",
                    },
                    ModalShortcut {
                        id: SAVE,
                        key: key!(Enter),
                        label: "save",
                    },
                ],
            }
        }

        fn modal(&self) -> Modal<'_> {
            Modal::new("settings")
                .tabs(&self.tabs)
                .shortcuts(&self.shortcuts)
        }
    }

    impl App for ModalApp {
        type Msg = ();

        fn handle(&mut self, ev: AppEvent<()>) -> Outcome {
            match ev {
                AppEvent::Hit(id, Pointer::Down { .. }) if id == UNDER => {
                    self.under_clicked = true;
                    self.last_hit = Some(id);
                    Outcome::Changed
                }
                AppEvent::Hit(id, Pointer::Down { .. }) => {
                    self.last_hit = Some(id);
                    self.last_miss_down = false;
                    if self.modal().hit_action(id) == Some(ModalAction::Close) {
                        self.closed = true;
                    }
                    Outcome::Changed
                }
                AppEvent::Miss(Pointer::Down { .. }) => {
                    self.last_miss_down = true;
                    self.closed = true;
                    Outcome::Changed
                }
                AppEvent::Hit(_, Pointer::Move { .. }) | AppEvent::Miss(Pointer::Move { .. }) => {
                    Outcome::Changed
                }
                _ => Outcome::Unchanged,
            }
        }

        fn draw(&mut self, f: &mut ratatui::Frame, cx: &mut FrameCtx) {
            cx.register(UNDER, Rect::new(0, 0, 4, 1));
            let chrome = Modal::new("settings")
                .tabs(&self.tabs)
                .shortcuts(&self.shortcuts);
            let _ = chrome.render(f.area(), f.buffer_mut(), cx, &mut self.modal);
        }
    }

    fn mouse(kind: MouseEventKind, col: u16, row: u16) -> Event {
        Event::Mouse(crossterm::event::MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn pump() -> Pump<ModalApp, TestBackend> {
        let backend = TestBackend::new(80, 24);
        let terminal = Terminal::new(backend).unwrap();
        Pump::new(ModalApp::new(), terminal, Theme::night(), Instant::now())
    }

    fn find_hit(pump: &Pump<ModalApp, TestBackend>, id: HitId) -> Option<(u16, u16)> {
        let area = Rect::new(0, 0, 80, 24);
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if pump.hits().hit(x, y) == HitResult::Hit(id) {
                    return Some((x, y));
                }
            }
        }
        None
    }

    #[test]
    fn outside_click_is_miss_and_closes() {
        let now = Instant::now();
        let mut p = pump();
        p.paint().unwrap();
        p.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), 0, 0));
        assert!(p.app.last_miss_down);
        assert!(p.app.closed);
        assert!(!p.app.under_clicked);
        assert_ne!(p.app.last_hit, Some(UNDER));
    }

    #[test]
    fn close_click_hits_close() {
        let now = Instant::now();
        let mut p = pump();
        p.paint().unwrap();
        let (x, y) = find_hit(&p, CLOSE).expect("close hit");
        p.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(p.app.last_hit, Some(CLOSE));
        assert!(p.app.closed);
    }

    #[test]
    fn footer_shortcut_click() {
        let now = Instant::now();
        let mut p = pump();
        p.paint().unwrap();
        let (x, y) = find_hit(&p, SAVE).expect("save shortcut");
        p.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(p.app.last_hit, Some(SAVE));
        assert!(!p.app.closed);
    }

    #[test]
    fn occlude_swallows_under_chrome() {
        let now = Instant::now();
        let mut p = pump();
        p.paint().unwrap();
        assert_eq!(p.hits().hit(0, 0), HitResult::Occluded);
        p.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), 1, 0));
        assert!(!p.app.under_clicked);
    }

    #[test]
    fn body_click_is_not_outside_close() {
        let now = Instant::now();
        let mut p = pump();
        p.paint().unwrap();
        let (x, y) = find_hit(&p, BODY).expect("body");
        p.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(p.app.last_hit, Some(BODY));
        assert!(!p.app.closed);
        assert!(!p.app.last_miss_down);
    }
}
