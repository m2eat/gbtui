//! Agent shell layout. Geometry from grok `views/agent.rs` `AgentViewLayout`
//! (REWRITE). No ACP panes (tasks/catalog/btw/voice).
//!
//! ```text
//!  status
//!  todo (optional)
//!  scrollback          ← Min
//!  turn status
//!  banner
//!  follow-ups
//!  permission overlay
//!  prompt
//!  shortcuts
//! ```

use ratatui::layout::{Constraint, Layout, Rect};

/// Scrollback floor, matching grok `SCROLLBACK_MIN_ROWS`.
pub const SCROLLBACK_MIN: u16 = 5;

#[derive(Debug, Clone, Copy, Default)]
pub struct AgentLayoutParams {
    pub todo_h: u16,
    pub turn_h: u16,
    pub banner_h: u16,
    pub follow_h: u16,
    pub perm_h: u16,
    pub prompt_h: u16,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AgentLayout {
    pub status: Rect,
    pub todo: Rect,
    pub scrollback: Rect,
    pub turn_status: Rect,
    pub banner: Rect,
    pub follow_ups: Rect,
    pub perm: Rect,
    pub prompt: Rect,
    pub shortcuts: Rect,
}

impl AgentLayout {
    pub fn compute(area: Rect, p: AgentLayoutParams) -> Self {
        if area.height == 0 || area.width == 0 {
            return Self::default();
        }
        let pad = 1u16;
        let area = Rect {
            x: area.x.saturating_add(pad),
            y: area.y,
            width: area.width.saturating_sub(pad.saturating_mul(2)),
            height: area.height,
        };
        if area.width == 0 {
            return Self::default();
        }
        let prompt_h = p.prompt_h.max(2);
        let mut kinds: Vec<Slot> = vec![Slot::Status];
        if p.todo_h > 0 {
            kinds.push(Slot::Todo);
        }
        kinds.push(Slot::Scroll);
        if p.turn_h > 0 {
            kinds.push(Slot::Turn);
        }
        if p.banner_h > 0 {
            kinds.push(Slot::Banner);
        }
        if p.follow_h > 0 {
            kinds.push(Slot::Follow);
        }
        if p.perm_h > 0 {
            kinds.push(Slot::Perm);
        }
        kinds.push(Slot::Prompt);
        kinds.push(Slot::Shortcuts);

        let constraints: Vec<Constraint> = kinds
            .iter()
            .map(|k| match k {
                Slot::Status | Slot::Shortcuts => Constraint::Length(1),
                Slot::Todo => Constraint::Length(p.todo_h),
                Slot::Scroll => Constraint::Min(SCROLLBACK_MIN),
                Slot::Turn => Constraint::Length(p.turn_h),
                Slot::Banner => Constraint::Length(p.banner_h),
                Slot::Follow => Constraint::Length(p.follow_h),
                Slot::Perm => Constraint::Length(p.perm_h),
                Slot::Prompt => Constraint::Length(prompt_h),
            })
            .collect();

        let rects = Layout::vertical(constraints).split(area);
        let mut layout = Self::default();
        for (kind, rect) in kinds.iter().zip(rects.iter()) {
            match kind {
                Slot::Status => layout.status = *rect,
                Slot::Todo => layout.todo = *rect,
                Slot::Scroll => layout.scrollback = *rect,
                Slot::Turn => layout.turn_status = *rect,
                Slot::Banner => layout.banner = *rect,
                Slot::Follow => layout.follow_ups = *rect,
                Slot::Perm => layout.perm = *rect,
                Slot::Prompt => layout.prompt = *rect,
                Slot::Shortcuts => layout.shortcuts = *rect,
            }
        }
        layout
    }
}

#[derive(Clone, Copy)]
enum Slot {
    Status,
    Todo,
    Scroll,
    Turn,
    Banner,
    Follow,
    Perm,
    Prompt,
    Shortcuts,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_and_shortcuts_survive_short_frame() {
        let l = AgentLayout::compute(
            Rect::new(0, 0, 80, 12),
            AgentLayoutParams {
                todo_h: 3,
                turn_h: 1,
                prompt_h: 3,
                ..AgentLayoutParams::default()
            },
        );
        assert_eq!(l.status.height, 1);
        assert_eq!(l.shortcuts.height, 1);
        assert!(l.prompt.height >= 1);
        assert!(l.scrollback.height >= 1);
        assert_eq!(l.todo.height, 3);
        assert_eq!(l.turn_status.height, 1);
    }
}
