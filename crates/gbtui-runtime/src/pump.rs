//! Event pump: inject crossterm events in tests; `run()` drives a live TTY.

use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use gbtui_core::{
    AnimationClock, HitId, ResolvedTheme, Theme, ThemeContext, ThemeDefinition, ThemeResolver,
    TickDemand,
};
use gbtui_input::{
    KeyboardNormalizer, MouseScrollState, OsModifierProbe, ScrollConfig, ScrollDirection,
};
use gbtui_terminal::{
    HyperlinkCapabilities, Terminal, TerminalInfo, hyperlink_capabilities, terminal_context,
};
use ratatui::backend::Backend;

use crate::app::{App, AppEvent, Outcome};
use crate::frame::FrameCtx;
use crate::hitmap::{HitMap, HitResult};
use crate::pointer::{MULTI_CLICK_TIMEOUT_MS, Pointer};

pub const RESIZE_DEBOUNCE: Duration = Duration::from_millis(16);

struct ClickTracker {
    last: Option<(HitId, Instant, u8)>,
}

impl ClickTracker {
    fn down(&mut self, id: HitId, now: Instant) -> u8 {
        let clicks = match self.last {
            Some((prev, at, n))
                if prev == id && now.duration_since(at).as_millis() < MULTI_CLICK_TIMEOUT_MS =>
            {
                n.saturating_add(1)
            }
            _ => 1,
        };
        self.last = Some((id, now, clicks));
        clicks
    }

    fn clear(&mut self) {
        self.last = None;
    }
}
#[derive(Clone, Copy)]
struct ScrollTarget {
    id: HitId,
    col: u16,
    row: u16,
}

/// Drives an [`App`] without owning a live event stream.
pub struct Pump<A: App, B: Backend> {
    pub app: A,
    terminal: Terminal<B>,
    resolver: ThemeResolver,
    mouse: Option<(u16, u16)>,
    /// Previous frame — used for clicks.
    hits: HitMap,
    anim: AnimationClock,
    scheduled: TickDemand,
    pending_resize: Option<(u16, u16, Instant)>,
    click: ClickTracker,
    scroll: MouseScrollState,
    scroll_cfg: ScrollConfig,
    scroll_target: Option<ScrollTarget>,
    normalizer: KeyboardNormalizer<OsModifierProbe>,
    left_down: Option<HitId>,
    draw_count: u64,
    last_size: (u16, u16),
    modifiers: crossterm::event::KeyModifiers,
    inline: bool,
    hyperlinks: HyperlinkCapabilities,
    #[cfg(feature = "images")]
    images_supported: bool,
}

impl<A: App, B: Backend> Pump<A, B> {
    pub fn new(app: A, terminal: Terminal<B>, theme: Theme, now: Instant) -> Self {
        Self::with_resolver(app, terminal, ThemeResolver::explicit(theme), now)
    }

    pub(crate) fn with_resolver(
        app: A,
        terminal: Terminal<B>,
        resolver: ThemeResolver,
        now: Instant,
    ) -> Self {
        let ctx = terminal_context();
        let size = terminal
            .size()
            .map(|s| (s.width, s.height))
            .unwrap_or((80, 24));
        Self {
            app,
            terminal,
            resolver,
            mouse: None,
            hits: HitMap::new(),
            anim: AnimationClock::new(now),
            scheduled: TickDemand::None,
            pending_resize: None,
            click: ClickTracker { last: None },
            scroll: MouseScrollState::new(now),
            scroll_cfg: ScrollConfig::from_terminal_context(ctx.brand, ctx.multiplexer),
            scroll_target: None,
            normalizer: KeyboardNormalizer::from_terminal_context(),
            left_down: None,
            draw_count: 0,
            last_size: size,
            modifiers: crossterm::event::KeyModifiers::NONE,
            inline: false,
            hyperlinks: hyperlink_capabilities(ctx.brand),
            #[cfg(feature = "images")]
            images_supported: false,
        }
    }

    /// Override detected OSC 8 capabilities for an embedded terminal host.
    ///
    /// Normal applications should use the automatically detected capability
    /// from [`terminal_context`]; this exists for hosts that own probing.
    pub fn with_hyperlink_capabilities(mut self, capabilities: HyperlinkCapabilities) -> Self {
        self.hyperlinks = capabilities;
        self
    }
    /// Override terminal-detected scroll normalization, primarily for hosts
    /// that have already calibrated their mouse-report stream.
    pub fn with_scroll_config(mut self, config: ScrollConfig) -> Self {
        self.scroll_cfg = config;
        self
    }

    pub fn set_inline(&mut self, inline: bool) {
        self.inline = inline;
    }

    pub fn is_inline(&self) -> bool {
        self.inline
    }

    /// Current palette for existing palette-oriented consumers.
    pub fn theme(&self) -> &Theme {
        self.resolver.resolved().palette()
    }

    /// Current resolved semantic theme for hosts that cache semantic styling.
    pub fn resolved_theme(&self) -> &ResolvedTheme {
        self.resolver.resolved()
    }

    /// Monotonic generation for renderer cache invalidation.
    pub fn theme_version(&self) -> u64 {
        self.resolver.resolved().version()
    }

    /// Replaces the palette without terminal adaptation.
    ///
    /// This retains the legacy raw-palette semantics used by [`Self::new`].
    /// Returns whether the effective resolved output changed.
    pub fn set_theme(&mut self, theme: Theme) -> bool {
        self.resolver.set_explicit(theme)
    }

    /// Replaces the semantic definition applied by the resolver.
    ///
    /// Returns whether the effective resolved output changed.
    pub fn set_theme_definition(&mut self, definition: ThemeDefinition) -> bool {
        self.resolver.set_definition(definition)
    }

    /// Updates terminal-dependent resolution facts.
    ///
    /// Returns whether the effective resolved output changed.
    pub fn update_theme_context(&mut self, context: ThemeContext) -> bool {
        self.resolver.set_context(context)
    }

    /// Delivers refreshed terminal facts and re-resolves the active
    /// definition before notifying the application.
    pub(crate) fn handle_terminal_info(
        &mut self,
        info: TerminalInfo,
        context: ThemeContext,
    ) -> Outcome {
        let context_changed = self.update_theme_context(context);
        let outcome = self.dispatch(AppEvent::TerminalInfo(info));
        if context_changed {
            outcome.merge(Outcome::Changed)
        } else {
            outcome
        }
    }

    pub fn draw_count(&self) -> u64 {
        self.draw_count
    }

    pub fn terminal_mut(&mut self) -> &mut Terminal<B> {
        &mut self.terminal
    }

    /// Hit map from the last [`paint`](Self::paint). Clicks resolve against this.
    pub fn hits(&self) -> &HitMap {
        &self.hits
    }

    /// Next animation or scroll wake. `None` means the loop must not spin.
    pub fn tick_deadline(&self, now: Instant) -> Option<Instant> {
        let animation = self
            .scheduled
            .interval(self.anim.fast_fps())
            .and_then(|delay| now.checked_add(delay));
        let scroll = self
            .scroll
            .scroll_clock_deadline(now)
            .and_then(|delay| now.checked_add(delay));
        match (animation, scroll) {
            (Some(animation), Some(scroll)) => Some(animation.min(scroll)),
            (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
            (None, None) => None,
        }
    }

    pub fn resize_deadline(&self) -> Option<Instant> {
        self.pending_resize.map(|(_, _, at)| at + RESIZE_DEBOUNCE)
    }

    pub fn paint(&mut self) -> std::io::Result<()> {
        self.paint_with(|terminal, _, cursor, _| terminal.flush_frame(cursor))
    }

    fn paint_with<Finish>(&mut self, finish: Finish) -> std::io::Result<()>
    where
        Finish: FnOnce(
            &mut Terminal<B>,
            crate::frame::FrameOutput,
            Option<ratatui::layout::Position>,
            bool,
        ) -> std::io::Result<()>,
    {
        let _ = self.apply_theme_request();
        let mut building = HitMap::new();
        let resolved_theme = self.resolver.resolved();
        let theme = resolved_theme.palette();
        let mouse = self.mouse;
        let modifiers = self.modifiers;
        let hyperlinks = self.hyperlinks;
        #[cfg(feature = "images")]
        let images_supported = self.images_supported;
        let app = &mut self.app;
        self.anim.clear_demand();
        let anim = &mut self.anim;
        self.terminal.try_draw_with(
            |f| {
                let mut cx = FrameCtx::new(theme, mouse, anim, &mut building)
                    .with_resolved_theme(resolved_theme)
                    .with_modifiers(modifiers)
                    .with_viewport(f.area())
                    .with_hyperlink_capabilities(hyperlinks);
                #[cfg(feature = "images")]
                {
                    cx = cx.with_image_support(images_supported);
                }
                app.draw(f, &mut cx);
                if let Some(pos) = cx.cursor() {
                    f.set_cursor_position(pos);
                }
                Ok::<_, std::io::Error>(cx.into_output())
            },
            finish,
        )?;
        self.hits = building;
        self.scheduled = self.app.tick_demand().max(self.anim.demand());
        self.draw_count = self.draw_count.saturating_add(1);
        Ok(())
    }

    fn dispatch(&mut self, event: AppEvent<A::Msg>) -> Outcome {
        let outcome = self.app.handle(event);
        if self.apply_theme_request() {
            outcome.merge(Outcome::Changed)
        } else {
            outcome
        }
    }

    fn apply_theme_request(&mut self) -> bool {
        let Some(definition) = self.app.take_theme_request() else {
            return false;
        };
        self.resolver.set_definition(definition)
    }

    pub fn drive(&mut self, now: Instant, event: Event) -> Outcome {
        let rescued = self.normalizer.rescue(&event);
        match rescued.as_ref() {
            Event::Key(key) if key.kind == KeyEventKind::Release => Outcome::Unchanged,
            Event::Key(key) => {
                self.modifiers = key.modifiers;
                self.dispatch(AppEvent::Key(*key))
            }
            Event::Mouse(mouse) => {
                self.modifiers = mouse.modifiers;
                self.drive_mouse(now, *mouse)
            }
            Event::Resize(w, h) => {
                self.cancel_scroll();
                self.pending_resize = Some((*w, *h, now));
                Outcome::Unchanged
            }
            Event::Paste(s) => self.dispatch(AppEvent::Paste(s.clone())),
            Event::FocusGained => self.dispatch(AppEvent::FocusGained),
            Event::FocusLost => self.handle_focus_lost(),
        }
    }

    pub fn drive_msg(&mut self, msg: A::Msg) -> Outcome {
        self.dispatch(AppEvent::Msg(msg))
    }

    pub fn tick(&mut self, now: Instant) -> Outcome {
        self.anim.advance(now);
        let mut out = self.dispatch(AppEvent::Tick);
        if self.scroll.has_active_stream() {
            let Some(target) = self.scroll_target else {
                self.cancel_scroll();
                return out;
            };
            if !self.scroll_target_is_current(target) {
                self.cancel_scroll();
                return out;
            }

            let update = self.scroll.on_tick_at(now);
            if update.lines != 0 {
                out = out.merge(self.dispatch_scroll_to_target(target, update.lines));
            }
            if !self.scroll.has_active_stream() {
                self.scroll_target = None;
            }
        }
        out
    }

    /// Apply a debounced resize if the quiet window has elapsed.
    pub fn flush_resize(&mut self, now: Instant) -> Outcome {
        let Some((w, h, at)) = self.pending_resize else {
            return Outcome::Unchanged;
        };
        if now.duration_since(at) < RESIZE_DEBOUNCE {
            return Outcome::Unchanged;
        }
        self.pending_resize = None;
        self.last_size = (w, h);
        self.dispatch(AppEvent::Resize {
            width: w,
            height: h,
        })
    }

    fn drive_mouse(&mut self, now: Instant, mouse: MouseEvent) -> Outcome {
        self.mouse = Some((mouse.column, mouse.row));
        match mouse.kind {
            MouseEventKind::Moved => {
                let cancelled = self.cancel_pointer_capture();
                let moved = self.dispatch_pointer(
                    now,
                    Pointer::Move {
                        col: mouse.column,
                        row: mouse.row,
                    },
                );
                cancelled.merge(moved)
            }
            MouseEventKind::Down(button) => {
                let cancelled = self.cancel_pointer_capture();
                let pointer_base = |clicks| Pointer::Down {
                    button,
                    col: mouse.column,
                    row: mouse.row,
                    clicks,
                    modifiers: mouse.modifiers,
                };
                let down = match self.hits.hit(mouse.column, mouse.row) {
                    HitResult::Hit(id) => {
                        let clicks = self.click.down(id, now);
                        if button == MouseButton::Left {
                            self.left_down = Some(id);
                        }
                        self.dispatch(AppEvent::Hit(id, pointer_base(clicks)))
                    }
                    HitResult::Occluded | HitResult::Miss => {
                        self.dispatch(AppEvent::Miss(pointer_base(1)))
                    }
                };
                cancelled.merge(down)
            }
            MouseEventKind::Up(button) => {
                let pointer = Pointer::Up {
                    button,
                    col: mouse.column,
                    row: mouse.row,
                };
                let down_id = if button == MouseButton::Left {
                    self.left_down.take()
                } else {
                    None
                };
                if let Some(id) = down_id {
                    self.dispatch(AppEvent::Hit(id, pointer))
                } else {
                    self.dispatch_pointer(now, pointer)
                }
            }
            MouseEventKind::Drag(button) => {
                if let Some(id) = self.left_down {
                    self.dispatch(AppEvent::Hit(
                        id,
                        Pointer::Drag {
                            button,
                            col: mouse.column,
                            row: mouse.row,
                        },
                    ))
                } else {
                    Outcome::Unchanged
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let Some(direction) = ScrollDirection::from_mouse_event(&mouse) else {
                    return Outcome::Unchanged;
                };
                let Some(target) = self.capture_scroll_target(mouse.column, mouse.row) else {
                    return Outcome::Unchanged;
                };
                let config = if self.scroll_cfg.viewport_height == 0 {
                    self.scroll_cfg.with_viewport_height(self.last_size.1)
                } else {
                    self.scroll_cfg
                };
                let update = self.scroll.on_scroll_event_at(now, direction, config);
                if update.lines == 0 {
                    return Outcome::Unchanged;
                }
                self.dispatch_scroll_to_target(target, update.lines)
            }
            _ => Outcome::Unchanged,
        }
    }

    fn capture_scroll_target(&mut self, col: u16, row: u16) -> Option<ScrollTarget> {
        if self.pending_resize.is_some() {
            self.cancel_scroll();
            return None;
        }
        let HitResult::Hit(id) = self.hits.scroll_hit(col, row) else {
            self.cancel_scroll();
            return None;
        };
        if self.scroll_target.is_some_and(|target| target.id != id) {
            self.cancel_scroll();
        }
        let target = ScrollTarget { id, col, row };
        self.scroll_target = Some(target);
        Some(target)
    }

    fn scroll_target_is_current(&self, target: ScrollTarget) -> bool {
        matches!(
            self.hits.scroll_hit(target.col, target.row),
            HitResult::Hit(id) if id == target.id
        )
    }

    fn dispatch_scroll_to_target(&mut self, target: ScrollTarget, delta_lines: i32) -> Outcome {
        if !self.scroll_target_is_current(target) {
            self.cancel_scroll();
            return Outcome::Unchanged;
        }
        self.dispatch(AppEvent::Hit(
            target.id,
            Pointer::Scroll {
                delta_lines,
                col: target.col,
                row: target.row,
            },
        ))
    }

    fn cancel_scroll(&mut self) {
        self.scroll.cancel_stream();
        self.scroll_target = None;
    }

    fn cancel_pointer_capture(&mut self) -> Outcome {
        let Some(id) = self.left_down.take() else {
            return Outcome::Unchanged;
        };
        let (col, row) = self.mouse.unwrap_or_default();
        self.dispatch(AppEvent::Hit(
            id,
            Pointer::Cancel {
                button: MouseButton::Left,
                col,
                row,
            },
        ))
    }

    fn handle_focus_lost(&mut self) -> Outcome {
        let cancelled = self.cancel_pointer_capture();
        self.mouse = None;
        self.modifiers = crossterm::event::KeyModifiers::NONE;
        self.click.clear();
        self.cancel_scroll();
        cancelled
            .merge(self.dispatch(AppEvent::FocusLost))
            .merge(Outcome::Changed)
    }

    fn dispatch_pointer(&mut self, now: Instant, pointer: Pointer) -> Outcome {
        let (col, row) = pointer.position();
        let pointer = match pointer {
            Pointer::Down {
                button,
                col,
                row,
                clicks: _,
                modifiers,
            } => {
                let clicks = match self.hits.hit(col, row) {
                    HitResult::Hit(id) => self.click.down(id, now),
                    _ => 1,
                };
                Pointer::Down {
                    button,
                    col,
                    row,
                    clicks,
                    modifiers,
                }
            }
            other => other,
        };
        match self.hits.hit(col, row) {
            HitResult::Hit(id) => self.dispatch(AppEvent::Hit(id, pointer)),
            HitResult::Occluded | HitResult::Miss => self.dispatch(AppEvent::Miss(pointer)),
        }
    }
}

impl<A: App, B: Backend + std::io::Write> Pump<A, B> {
    /// Paint through the live raw writer so OSC 8 spans participate in the
    /// same synchronized transaction as cell diffing and caret updates.
    pub fn paint_live(&mut self) -> std::io::Result<()> {
        self.paint_with(|terminal, output, cursor, _| {
            terminal.flush_frame_with_links(&output.links, cursor)
        })
    }

    #[cfg(feature = "images")]
    pub(crate) fn paint_live_with_images(
        &mut self,
        scene: &mut gbtui_terminal::image::ImageScene,
        supports_images: bool,
    ) -> std::io::Result<()> {
        self.images_supported = supports_images;
        self.paint_with(|terminal, output, cursor, viewport_resized| {
            terminal.flush_frame_with_output(
                &output.links,
                scene,
                &output.images,
                cursor,
                viewport_resized,
            )
        })
    }
    /// Drain [`App::take_scrollback_emit`] into the native scrollback when
    /// running inline.
    pub fn commit_inline(&mut self) -> std::io::Result<()> {
        let text = self.app.take_scrollback_emit();
        if !self.inline {
            return Ok(());
        }
        let Some(text) = text else {
            return Ok(());
        };
        gbtui_terminal::emit_to_scrollback(&mut self.terminal, &text)
    }

    /// RIS + replay history after a resize (inline viewport only).
    pub fn rerender_inline_history(&mut self) -> std::io::Result<()> {
        if !self.inline {
            return Ok(());
        }
        let Some(history) = self.app.scrollback_history() else {
            return Ok(());
        };
        let history = history.to_owned();
        gbtui_terminal::resize_purge_rerender(&mut self.terminal, &history)
    }
}
