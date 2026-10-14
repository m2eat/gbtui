//! Pointer events delivered to hit targets.

use crossterm::event::{KeyModifiers, MouseButton};

/// Normalized pointer event. Scroll deltas are already in lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointer {
    Move {
        col: u16,
        row: u16,
    },
    Down {
        button: MouseButton,
        col: u16,
        row: u16,
        clicks: u8,
        /// Modifier snapshot from the initiating mouse event.
        ///
        /// Selection widgets should capture this intent at press time rather
        /// than sampling keyboard state during a later drag.
        modifiers: KeyModifiers,
    },
    Up {
        button: MouseButton,
        col: u16,
        row: u16,
    },
    Drag {
        button: MouseButton,
        col: u16,
        row: u16,
    },
    /// A captured pointer interaction ended without a matching mouse-up.
    ///
    /// `col` and `row` are the last observed pointer position. They preserve
    /// the normalized pointer shape, but cancellation is terminal: receivers
    /// must clear transient drag state without invoking click or release
    /// actions.
    Cancel {
        button: MouseButton,
        col: u16,
        row: u16,
    },

    Scroll {
        delta_lines: i32,
        col: u16,
        row: u16,
    },
}

impl Pointer {
    pub fn position(self) -> (u16, u16) {
        match self {
            Self::Move { col, row }
            | Self::Down { col, row, .. }
            | Self::Up { col, row, .. }
            | Self::Drag { col, row, .. }
            | Self::Cancel { col, row, .. }
            | Self::Scroll { col, row, .. } => (col, row),
        }
    }
}

pub const MULTI_CLICK_TIMEOUT_MS: u128 = 300;
