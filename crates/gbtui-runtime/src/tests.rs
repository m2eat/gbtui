use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use gbtui_core::{
    HitId, Theme, ThemeAppearance, ThemeContext, ThemeDefinition, ThemeKind, TickDemand,
};
use gbtui_input::{ScrollConfig, ScrollInputMode};

use gbtui_terminal::{HyperlinkCapabilities, Terminal};
use ratatui::Frame;
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::{TerminalOptions, Viewport};

use crate::app::{App, AppEvent, Outcome};
use crate::frame::FrameCtx;
use crate::pointer::Pointer;
use crate::pump::{Pump, RESIZE_DEBOUNCE};

const BOTTOM: HitId = HitId::named("bottom");
const TOP: HitId = HitId::named("top");
const HOVER_BG: Color = Color::Rgb(1, 2, 3);

struct ProbeApp {
    bottom: Rect,
    top: Option<Rect>,
    occlude: Option<Rect>,
    last_hit: Option<HitId>,
    last_miss: bool,
    want_tick: TickDemand,
}

impl ProbeApp {
    fn single_button() -> Self {
        Self {
            bottom: Rect::new(2, 2, 8, 1),
            top: None,
            occlude: None,
            last_hit: None,
            last_miss: false,
            want_tick: TickDemand::None,
        }
    }
}

impl App for ProbeApp {
    type Msg = ();

    fn handle(&mut self, ev: AppEvent<()>) -> Outcome {
        match ev {
            AppEvent::Hit(id, Pointer::Down { .. }) => {
                self.last_hit = Some(id);
                self.last_miss = false;
                Outcome::Changed
            }
            AppEvent::Miss(Pointer::Down { .. }) => {
                self.last_hit = None;
                self.last_miss = true;
                Outcome::Changed
            }
            AppEvent::Hit(_, Pointer::Move { .. }) | AppEvent::Miss(Pointer::Move { .. }) => {
                Outcome::Changed
            }
            AppEvent::Resize { .. } => Outcome::Changed,
            AppEvent::Tick => Outcome::Changed,
            _ => Outcome::Unchanged,
        }
    }

    fn draw(&mut self, f: &mut Frame, cx: &mut FrameCtx) {
        // Chrome first, then occlude (dropdown/modal), then overlay targets.
        cx.register(BOTTOM, self.bottom);
        let bg = if cx.hovered(self.bottom) {
            HOVER_BG
        } else {
            cx.theme.bg_base
        };
        f.buffer_mut()[(self.bottom.x, self.bottom.y)].set_bg(bg);
        if let Some(area) = self.occlude {
            cx.occlude(area);
        }
        if let Some(top) = self.top {
            cx.register(TOP, top);
        }
    }

    fn tick_demand(&self) -> TickDemand {
        self.want_tick
    }
}

fn day_theme_request() -> ThemeDefinition {
    ThemeDefinition::new("test-day", ThemeKind::Day)
}

struct ThemeRequestApp {
    request_after_handle: bool,
    pending_request: Option<ThemeDefinition>,
}

impl ThemeRequestApp {
    fn after_event() -> Self {
        Self {
            request_after_handle: true,
            pending_request: None,
        }
    }

    fn at_startup() -> Self {
        Self {
            request_after_handle: false,
            pending_request: Some(day_theme_request()),
        }
    }
}

impl App for ThemeRequestApp {
    type Msg = ();

    fn handle(&mut self, _ev: AppEvent<()>) -> Outcome {
        if self.request_after_handle {
            self.pending_request = Some(day_theme_request());
        }
        Outcome::Unchanged
    }

    fn draw(&mut self, _f: &mut Frame, cx: &mut FrameCtx) {
        cx.register(BOTTOM, Rect::new(2, 2, 8, 1));
    }

    fn take_theme_request(&mut self) -> Option<ThemeDefinition> {
        self.pending_request.take()
    }
}

struct CaretApp;

impl App for CaretApp {
    type Msg = ();

    fn handle(&mut self, _ev: AppEvent<()>) -> Outcome {
        Outcome::Unchanged
    }

    fn draw(&mut self, _f: &mut Frame, cx: &mut FrameCtx) {
        cx.set_cursor(3, 2);
    }
}

struct PointerProbeApp {
    pointers: Vec<Pointer>,
    focus_lost: usize,
}

impl App for PointerProbeApp {
    type Msg = ();

    fn handle(&mut self, ev: AppEvent<()>) -> Outcome {
        match ev {
            AppEvent::Hit(_, pointer) => self.pointers.push(pointer),
            AppEvent::FocusLost => self.focus_lost = self.focus_lost.saturating_add(1),
            _ => {}
        }
        Outcome::Unchanged
    }

    fn draw(&mut self, _f: &mut Frame, cx: &mut FrameCtx) {
        cx.register(BOTTOM, Rect::new(2, 2, 8, 1));
    }
}

const SCROLL_TARGET: HitId = HitId::named("scroll-target");
const REPLACED_SCROLL_TARGET: HitId = HitId::named("replaced-scroll-target");
const SCROLL_REGION: HitId = HitId::named("scroll-region");
const REPLACED_SCROLL_REGION: HitId = HitId::named("replaced-scroll-region");

struct ScrollProbeApp {
    target: Option<HitId>,
    scroll_region: Option<HitId>,
    occlude: bool,
    scrolls: Vec<(HitId, i32, u16, u16)>,
}

impl ScrollProbeApp {
    fn target() -> Self {
        Self {
            target: Some(SCROLL_TARGET),
            scroll_region: Some(SCROLL_REGION),
            occlude: false,
            scrolls: Vec::new(),
        }
    }
}

impl App for ScrollProbeApp {
    type Msg = ();

    fn handle(&mut self, ev: AppEvent<()>) -> Outcome {
        match ev {
            AppEvent::Hit(
                id,
                Pointer::Scroll {
                    delta_lines,
                    col,
                    row,
                },
            ) => {
                self.scrolls.push((id, delta_lines, col, row));
                Outcome::Changed
            }
            _ => Outcome::Unchanged,
        }
    }

    fn draw(&mut self, _f: &mut Frame, cx: &mut FrameCtx) {
        let area = Rect::new(2, 2, 8, 1);
        if let Some(id) = self.target {
            cx.register(id, area);
        }
        if let Some(id) = self.scroll_region {
            cx.register_scroll(id, area);
        }
        if self.occlude {
            cx.occlude(area);
        }
    }
}

#[derive(Clone, Default)]
struct RecordingWriter(Arc<Mutex<Vec<u8>>>);

impl RecordingWriter {
    fn snapshot(&self) -> Vec<u8> {
        self.0.lock().expect("recording writer mutex").clone()
    }
}

impl Write for RecordingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("recording writer mutex")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct LinkApp {
    url: Arc<str>,
}

impl App for LinkApp {
    type Msg = ();

    fn handle(&mut self, _ev: AppEvent<()>) -> Outcome {
        Outcome::Unchanged
    }

    fn draw(&mut self, f: &mut Frame, cx: &mut FrameCtx) {
        let area = Rect::new(0, 0, 2, 1);
        f.buffer_mut()
            .set_string(area.x, area.y, "AB", ratatui::style::Style::default());
        cx.register_link(area, Arc::clone(&self.url), Some(7));
    }
}

fn native_osc8() -> HyperlinkCapabilities {
    HyperlinkCapabilities::native()
}

fn new_pump(app: ProbeApp, w: u16, h: u16, now: Instant) -> Pump<ProbeApp, TestBackend> {
    let backend = TestBackend::new(w, h);
    let terminal = Terminal::new(backend).unwrap();
    Pump::new(app, terminal, Theme::night(), now)
}

fn new_theme_request_pump(
    app: ThemeRequestApp,
    now: Instant,
) -> Pump<ThemeRequestApp, TestBackend> {
    let terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();
    Pump::new(app, terminal, Theme::night(), now)
}

fn assert_event_theme_request(
    dispatch: impl FnOnce(&mut Pump<ThemeRequestApp, TestBackend>, Instant) -> Outcome,
) {
    let now = Instant::now();
    let mut pump = new_theme_request_pump(ThemeRequestApp::after_event(), now);
    pump.paint().unwrap();
    let version = pump.theme_version();

    assert_eq!(dispatch(&mut pump, now), Outcome::Changed);
    assert_eq!(pump.theme(), &Theme::day());
    assert!(pump.theme_version() > version);
}

fn new_pointer_pump(now: Instant) -> Pump<PointerProbeApp, TestBackend> {
    let terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();
    Pump::new(
        PointerProbeApp {
            pointers: Vec::new(),
            focus_lost: 0,
        },
        terminal,
        Theme::night(),
        now,
    )
}

fn mouse(kind: MouseEventKind, col: u16, row: u16) -> Event {
    mouse_with_modifiers(kind, col, row, KeyModifiers::NONE)
}

fn mouse_with_modifiers(
    kind: MouseEventKind,
    col: u16,
    row: u16,
    modifiers: KeyModifiers,
) -> Event {
    Event::Mouse(crossterm::event::MouseEvent {
        kind,
        column: col,
        row,
        modifiers,
    })
}

fn new_scroll_pump(now: Instant) -> Pump<ScrollProbeApp, TestBackend> {
    let terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();
    Pump::new(ScrollProbeApp::target(), terminal, Theme::night(), now).with_scroll_config(
        ScrollConfig {
            mode: ScrollInputMode::Trackpad,
            ..ScrollConfig::default()
        },
    )
}

#[test]
fn later_registration_wins_clicks() {
    let now = Instant::now();
    let mut app = ProbeApp::single_button();
    app.top = Some(Rect::new(4, 2, 8, 1));
    let mut pump = new_pump(app, 20, 8, now);
    pump.paint().unwrap();
    pump.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), 5, 2));
    assert_eq!(pump.app.last_hit, Some(TOP));
}

#[test]
fn occlude_swallows_clicks_underneath() {
    let now = Instant::now();
    let mut app = ProbeApp::single_button();
    app.occlude = Some(Rect::new(0, 0, 20, 8));
    app.top = Some(Rect::new(10, 4, 3, 1));
    let mut pump = new_pump(app, 20, 8, now);
    pump.paint().unwrap();
    pump.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), 3, 2));
    assert!(pump.app.last_miss);
    assert_eq!(pump.app.last_hit, None);
    pump.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), 11, 4));
    assert_eq!(pump.app.last_hit, Some(TOP));
}

#[test]
fn dropdown_occlude_blocks_button_underneath() {
    let now = Instant::now();
    let mut app = ProbeApp::single_button();
    app.occlude = Some(Rect::new(0, 0, 20, 8));
    let mut pump = new_pump(app, 20, 8, now);
    pump.paint().unwrap();
    pump.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), 4, 2));
    assert!(pump.app.last_miss, "occluded button must not click");
}

#[test]
fn hover_is_visible_on_the_draw_after_move() {
    let now = Instant::now();
    let mut pump = new_pump(ProbeApp::single_button(), 20, 8, now);
    pump.paint().unwrap();
    assert_ne!(pump.terminal_mut().backend().buffer()[(2, 2)].bg, HOVER_BG);
    pump.drive(now, mouse(MouseEventKind::Moved, 3, 2));
    pump.paint().unwrap();
    assert_eq!(pump.terminal_mut().backend().buffer()[(2, 2)].bg, HOVER_BG);
}

#[test]
fn pointer_down_forwards_its_modifier_snapshot() {
    let now = Instant::now();
    let mut pump = new_pointer_pump(now);
    pump.paint().unwrap();
    let _ = pump.drive(
        now,
        mouse_with_modifiers(
            MouseEventKind::Down(MouseButton::Left),
            3,
            2,
            KeyModifiers::SHIFT,
        ),
    );

    assert_eq!(
        pump.app.pointers,
        vec![Pointer::Down {
            button: MouseButton::Left,
            col: 3,
            row: 2,
            clicks: 1,
            modifiers: KeyModifiers::SHIFT,
        }]
    );
}

#[test]
fn focus_loss_cancels_capture_and_repaints_even_when_the_app_is_unchanged() {
    let now = Instant::now();
    let mut pointer_pump = new_pointer_pump(now);
    pointer_pump.paint().unwrap();
    let _ = pointer_pump.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), 3, 2));
    assert_eq!(pointer_pump.drive(now, Event::FocusLost), Outcome::Changed);
    let _ = pointer_pump.drive(now, Event::FocusGained);
    let _ = pointer_pump.drive(now, mouse(MouseEventKind::Moved, 4, 2));
    assert_eq!(pointer_pump.app.focus_lost, 1);
    assert_eq!(
        pointer_pump.app.pointers,
        vec![
            Pointer::Down {
                button: MouseButton::Left,
                col: 3,
                row: 2,
                clicks: 1,
                modifiers: KeyModifiers::NONE,
            },
            Pointer::Cancel {
                button: MouseButton::Left,
                col: 3,
                row: 2,
            },
            Pointer::Move { col: 4, row: 2 },
        ]
    );

    let mut hover_pump = new_pump(ProbeApp::single_button(), 20, 8, now);
    hover_pump.paint().unwrap();
    let _ = hover_pump.drive(now, mouse(MouseEventKind::Moved, 3, 2));
    hover_pump.paint().unwrap();
    assert_eq!(
        hover_pump.terminal_mut().backend().buffer()[(2, 2)].bg,
        HOVER_BG
    );
    assert_eq!(hover_pump.drive(now, Event::FocusLost), Outcome::Changed);
    hover_pump.paint().unwrap();
    assert_ne!(
        hover_pump.terminal_mut().backend().buffer()[(2, 2)].bg,
        HOVER_BG
    );
}

#[test]
fn focus_loss_resets_multiclick_history() {
    let now = Instant::now();
    let mut pump = new_pointer_pump(now);
    pump.paint().unwrap();
    let _ = pump.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), 3, 2));
    let _ = pump.drive(now, Event::FocusLost);
    let _ = pump.drive(
        now + Duration::from_millis(1),
        mouse(MouseEventKind::Down(MouseButton::Left), 3, 2),
    );

    assert!(matches!(
        pump.app.pointers.last(),
        Some(Pointer::Down { clicks: 1, .. })
    ));
}

#[test]
fn move_without_mouse_up_cancels_a_stale_capture_without_emitting_drag() {
    let now = Instant::now();
    let mut pump = new_pointer_pump(now);
    pump.paint().unwrap();
    let _ = pump.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), 3, 2));
    let _ = pump.drive(now, mouse(MouseEventKind::Moved, 4, 2));

    assert_eq!(
        pump.app.pointers,
        vec![
            Pointer::Down {
                button: MouseButton::Left,
                col: 3,
                row: 2,
                clicks: 1,
                modifiers: KeyModifiers::NONE,
            },
            Pointer::Cancel {
                button: MouseButton::Left,
                col: 4,
                row: 2,
            },
            Pointer::Move { col: 4, row: 2 },
        ]
    );
}

#[test]
fn drag_with_a_held_button_remains_captured() {
    let now = Instant::now();
    let mut pump = new_pointer_pump(now);
    pump.paint().unwrap();
    let _ = pump.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), 3, 2));
    let _ = pump.drive(now, mouse(MouseEventKind::Drag(MouseButton::Left), 4, 2));

    assert!(matches!(
        pump.app.pointers.last(),
        Some(Pointer::Drag {
            button: MouseButton::Left,
            col: 4,
            row: 2,
        })
    ));
}

#[test]
fn idle_does_not_arm_tick_deadline() {
    let now = Instant::now();
    let mut pump = new_pump(ProbeApp::single_button(), 20, 8, now);
    pump.paint().unwrap();
    assert_eq!(pump.tick_deadline(now), None);
}

#[test]
fn scroll_clock_flushes_all_pending_lines_without_animation_demand() {
    let now = Instant::now();
    let mut pump = new_scroll_pump(now);
    pump.paint().unwrap();
    for offset in 1..=4 {
        let _ = pump.drive(
            now + Duration::from_millis(offset),
            mouse(MouseEventKind::ScrollDown, 3, 2),
        );
    }

    let flush_at = pump
        .tick_deadline(now + Duration::from_millis(4))
        .expect("pending scroll lines arm a clock even when the app is idle");
    assert_eq!(pump.tick(flush_at), Outcome::Changed);
    assert_eq!(pump.app.scrolls, vec![(SCROLL_REGION, 4, 3, 2)]);

    let finish_at = pump
        .tick_deadline(flush_at)
        .expect("active scroll stream remains armed through its gap");
    let _ = pump.tick(finish_at);
    assert_eq!(pump.tick_deadline(finish_at), None);
}

#[test]
fn focus_loss_cancels_an_active_scroll_deadline() {
    let now = Instant::now();
    let mut pump = new_scroll_pump(now);
    pump.paint().unwrap();
    let _ = pump.drive(
        now + Duration::from_millis(1),
        mouse(MouseEventKind::ScrollDown, 3, 2),
    );
    assert!(pump.tick_deadline(now + Duration::from_millis(1)).is_some());

    assert_eq!(
        pump.drive(now + Duration::from_millis(2), Event::FocusLost),
        Outcome::Changed
    );
    assert_eq!(pump.tick_deadline(now + Duration::from_millis(2)), None);
}

#[test]
fn scroll_reversal_uses_the_latest_scroll_position_without_old_tail() {
    let now = Instant::now();
    let mut pump = new_scroll_pump(now);
    pump.paint().unwrap();
    let _ = pump.drive(
        now + Duration::from_millis(1),
        mouse(MouseEventKind::ScrollDown, 3, 2),
    );
    let _ = pump.drive(
        now + Duration::from_millis(2),
        mouse(MouseEventKind::ScrollUp, 7, 2),
    );

    let flush_at = pump
        .tick_deadline(now + Duration::from_millis(2))
        .expect("reversed stream stays armed");
    let _ = pump.tick(flush_at);
    assert_eq!(pump.app.scrolls, vec![(SCROLL_REGION, -1, 7, 2)]);
}

#[test]
fn scroll_tail_keeps_its_region_when_rows_change() {
    let now = Instant::now();
    let mut pump = new_scroll_pump(now);
    pump.paint().unwrap();
    let _ = pump.drive(
        now + Duration::from_millis(1),
        mouse(MouseEventKind::ScrollDown, 3, 2),
    );
    pump.app.target = Some(REPLACED_SCROLL_TARGET);
    pump.paint().unwrap();

    let flush_at = pump
        .tick_deadline(now + Duration::from_millis(1))
        .expect("pending stream arms a clock");
    let _ = pump.tick(flush_at);
    assert_eq!(pump.app.scrolls, vec![(SCROLL_REGION, 1, 3, 2)]);
}

#[test]
fn scroll_tail_cancels_when_its_region_changes_or_resizes() {
    let now = Instant::now();
    let mut pump = new_scroll_pump(now);
    pump.paint().unwrap();
    let _ = pump.drive(
        now + Duration::from_millis(1),
        mouse(MouseEventKind::ScrollDown, 3, 2),
    );
    pump.app.scroll_region = Some(REPLACED_SCROLL_REGION);
    pump.paint().unwrap();

    let flush_at = pump
        .tick_deadline(now + Duration::from_millis(1))
        .expect("pending stream arms a clock");
    let _ = pump.tick(flush_at);
    assert!(pump.app.scrolls.is_empty());
    assert_eq!(pump.tick_deadline(flush_at), None);

    let mut resize_pump = new_scroll_pump(now);
    resize_pump.paint().unwrap();
    let _ = resize_pump.drive(
        now + Duration::from_millis(30),
        mouse(MouseEventKind::ScrollDown, 3, 2),
    );
    let _ = resize_pump.drive(now + Duration::from_millis(31), Event::Resize(30, 10));
    assert_eq!(
        resize_pump.tick_deadline(now + Duration::from_millis(31)),
        None
    );
}

#[test]
fn scroll_tail_never_penetrates_a_new_occluder() {
    let now = Instant::now();
    let mut pump = new_scroll_pump(now);
    pump.paint().unwrap();
    let _ = pump.drive(
        now + Duration::from_millis(1),
        mouse(MouseEventKind::ScrollDown, 3, 2),
    );
    pump.app.occlude = true;
    pump.paint().unwrap();

    let flush_at = pump
        .tick_deadline(now + Duration::from_millis(1))
        .expect("pending stream arms a clock");
    let _ = pump.tick(flush_at);
    assert!(pump.app.scrolls.is_empty());
    assert_eq!(pump.tick_deadline(flush_at), None);
}

#[test]
fn resize_storm_draws_once_after_debounce() {
    let now = Instant::now();
    let mut pump = new_pump(ProbeApp::single_button(), 20, 8, now);
    pump.paint().unwrap();
    let base = pump.draw_count();
    for i in 0..8 {
        pump.drive(
            now + Duration::from_millis(i),
            Event::Resize(40 + i as u16, 12),
        );
        assert_eq!(pump.draw_count(), base, "resize must not paint immediately");
    }
    assert!(pump.resize_deadline().is_some());
    let last = now + Duration::from_millis(7);
    let flushed = pump.flush_resize(last + RESIZE_DEBOUNCE);
    assert_eq!(flushed, Outcome::Changed);
    pump.paint().unwrap();
    assert_eq!(pump.draw_count(), base + 1);
}

#[test]
fn key_reaches_app() {
    let now = Instant::now();
    let mut pump = new_pump(ProbeApp::single_button(), 20, 8, now);
    let out = pump.drive(
        now,
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
    );
    assert_eq!(out, Outcome::Unchanged);
}

#[test]
fn unchanged_event_handlers_apply_theme_requests() {
    assert_event_theme_request(|pump, now| {
        pump.drive(
            now,
            Event::Key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE)),
        )
    });
    assert_event_theme_request(|pump, _| pump.drive_msg(()));
    assert_event_theme_request(|pump, now| pump.tick(now));
    assert_event_theme_request(|pump, now| {
        pump.drive(now, mouse(MouseEventKind::Down(MouseButton::Left), 3, 2))
    });
}

#[test]
fn startup_theme_request_applies_before_the_first_paint() {
    let now = Instant::now();
    let mut pump = new_theme_request_pump(ThemeRequestApp::at_startup(), now);
    let version = pump.theme_version();

    pump.paint().unwrap();

    assert_eq!(pump.theme(), &Theme::day());
    assert!(pump.theme_version() > version);
}

#[test]
fn definition_reresolves_for_context_changes_without_noop_version_bumps() {
    let now = Instant::now();
    let mut pump = new_theme_request_pump(ThemeRequestApp::after_event(), now);
    let auto = || ThemeDefinition::new("test-auto", ThemeKind::Auto);
    let light = || ThemeContext {
        appearance: Some(ThemeAppearance::Light),
        ..ThemeContext::default()
    };
    let dark = || ThemeContext {
        appearance: Some(ThemeAppearance::Dark),
        ..ThemeContext::default()
    };

    let _ = pump.set_theme_definition(auto());
    assert!(pump.update_theme_context(light()));
    assert_eq!(pump.theme(), &Theme::day());

    let light_version = pump.theme_version();
    assert!(!pump.update_theme_context(light()));
    assert_eq!(pump.theme_version(), light_version);

    assert!(pump.update_theme_context(dark()));
    assert_eq!(pump.theme(), &Theme::night());
    let dark_version = pump.theme_version();
    assert!(!pump.set_theme_definition(auto()));
    assert_eq!(pump.theme_version(), dark_version);

    assert!(pump.set_theme_definition(ThemeDefinition::new("hot-day", ThemeKind::Day)));
    assert!(pump.update_theme_context(light()));
    assert_eq!(pump.theme(), &Theme::day());
}

#[test]
fn pump_applies_framectx_cursor() {
    use ratatui::layout::Position;
    let now = Instant::now();
    let backend = TestBackend::new(20, 8);
    let terminal = Terminal::new(backend).unwrap();
    let mut pump = Pump::new(CaretApp, terminal, Theme::night(), now);
    pump.paint().unwrap();
    let pos = pump.terminal_mut().get_cursor_position().unwrap();
    assert_eq!(pos, Position { x: 3, y: 2 });
}

#[test]
fn headless_pump_paints_without_a_raw_writer() {
    let now = Instant::now();
    let terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();
    let app = LinkApp {
        url: "https://example.com".into(),
    };
    let mut pump =
        Pump::new(app, terminal, Theme::night(), now).with_hyperlink_capabilities(native_osc8());

    pump.paint().unwrap();

    assert_eq!(pump.terminal_mut().backend().buffer()[(0, 0)].symbol(), "A");
}

#[test]
fn live_pump_emits_osc8_through_the_raw_writer() {
    let now = Instant::now();
    let writer = RecordingWriter::default();
    let terminal = Terminal::with_options(
        CrosstermBackend::new(writer.clone()),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 20, 8)),
        },
    )
    .unwrap();
    let app = LinkApp {
        url: "https://example.com".into(),
    };
    let mut pump =
        Pump::new(app, terminal, Theme::night(), now).with_hyperlink_capabilities(native_osc8());

    pump.paint_live().unwrap();

    let output = String::from_utf8(writer.snapshot()).unwrap();
    assert!(output.contains("\x1b]8;id=1;https://example.com\x07"));
    assert!(output.contains("AB"));
}
