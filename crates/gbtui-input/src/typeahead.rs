//! Startup type-ahead filter. Pure: no TTY.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/app/event_loop.rs (STRIP)`

use crossterm::event::{Event, KeyCode, KeyEventKind};

use gbtui_core::is_text_input_key;

/// Whether an event is genuine typing for a composer.
pub fn is_typeahead_event(event: &Event) -> bool {
    match event {
        Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            is_text_input_key(key) || key.code == KeyCode::Backspace
        }
        Event::Paste(_) => true,
        _ => false,
    }
}

/// Keep typing, drop at the first Esc (query-reply residue).
pub fn filter_startup_typeahead(batch: Vec<Event>) -> Vec<Event> {
    let cutoff = batch
        .iter()
        .position(|e| matches!(e, Event::Key(key) if key.code == KeyCode::Esc))
        .unwrap_or(batch.len());
    batch
        .into_iter()
        .take(cutoff)
        .filter(is_typeahead_event)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

    fn key(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    #[test]
    fn keeps_typing_drops_mouse() {
        let mouse = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        let out = filter_startup_typeahead(vec![mouse, key('h'), key('i')]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn truncates_at_esc() {
        let esc = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let out = filter_startup_typeahead(vec![key('a'), esc, key('b')]);
        assert_eq!(out.len(), 1);
    }
}
