//! Application contract the event loop drives.

use crossterm::event::KeyEvent;
use gbtui_core::{HitId, ThemeDefinition, TickDemand};
use gbtui_terminal::TerminalInfo;

use ratatui::Frame;

use crate::frame::FrameCtx;
use crate::pointer::Pointer;

/// What the event loop should do after `handle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Unchanged,
    Changed,
    Quit,
}

impl Outcome {
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Quit, _) | (_, Self::Quit) => Self::Quit,
            (Self::Changed, _) | (_, Self::Changed) => Self::Changed,
            (Self::Unchanged, Self::Unchanged) => Self::Unchanged,
        }
    }
}

/// Input the runtime delivers to [`App::handle`].
#[derive(Debug, Clone)]
pub enum AppEvent<M> {
    Key(KeyEvent),
    /// Pointer over a registered hit from the **previous** frame.
    Hit(HitId, Pointer),
    /// Pointer over empty space or an occlude region.
    Miss(Pointer),
    Resize {
        width: u16,
        height: u16,
    },
    Tick,
    Paste(String),
    FocusGained,
    /// Focus ended. If a pointer was captured, its target first receives
    /// [`Pointer::Cancel`](crate::Pointer::Cancel), never a synthetic release.
    FocusLost,
    Msg(M),
    /// Terminal facts from the startup probe or a later validated reply.
    TerminalInfo(TerminalInfo),
}

/// Application the event loop drives. Focus and domain state stay here.
pub trait App {
    type Msg;

    /// Consume one event. Return [`Outcome::Changed`] to repaint.
    fn handle(&mut self, ev: AppEvent<Self::Msg>) -> Outcome;
    /// Paint one frame. Register hits on `cx`; do not match `MouseEventKind`.
    fn draw(&mut self, f: &mut Frame, cx: &mut FrameCtx);
    fn tick_demand(&self) -> TickDemand {
        TickDemand::None
    }

    /// Takes a requested runtime theme change after an event is handled.
    ///
    /// The pump applies the definition before the next paint and repaints only
    /// when it changes the effective resolved theme.
    fn take_theme_request(&mut self) -> Option<ThemeDefinition> {
        None
    }

    /// Text to print into the native scrollback (inline viewport). Taken
    /// once after `handle`/`tick` when the loop is in inline mode.
    fn take_scrollback_emit(&mut self) -> Option<String> {
        None
    }

    /// Full history (CRLF lines) for RIS re-render on resize.
    fn scrollback_history(&self) -> Option<&str> {
        None
    }
}
