//! Slash command palette as a host-owned, rich picker overlay.
//!
//! Prompt stays slash-free: the host supplies source tags, argument hints, and
//! descriptions as presentation data only. Registry, filesystem, and command
//! execution behavior remain outside this module.
//!
//! The host detects a leading `/` on Prompt text (see [`slash_query`]) and
//! routes navigation keys here first. Character keys remain unhandled so the
//! Prompt owns editing its slash token.

use crossterm::event::{KeyCode, KeyEvent};
use gbtui::PickerState;
use gbtui::core::HitId;
use gbtui::key;
use gbtui::runtime::FrameCtx;
use gbtui::widgets::{
    PickerEntry, PickerRow, is_picker_scroll, parse_picker_row, render_picker_content,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::paint::fill;

/// Borrowed slash row. `name` has no leading `/`; every other field is
/// optional host-provided presentation data.
#[derive(Debug, Clone, Copy)]
pub struct SlashItem<'a> {
    pub name: &'a str,
    pub source_tag: Option<&'a str>,
    pub argument_hint: Option<&'a str>,
    pub description: Option<&'a str>,
}

impl<'a> SlashItem<'a> {
    pub fn new(name: &'a str) -> Self {
        Self {
            name,
            source_tag: None,
            argument_hint: None,
            description: None,
        }
    }

    pub fn source_tag(mut self, source_tag: &'a str) -> Self {
        self.source_tag = Some(source_tag);
        self
    }

    pub fn argument_hint(mut self, argument_hint: &'a str) -> Self {
        self.argument_hint = Some(argument_hint);
        self
    }

    pub fn description(mut self, description: &'a str) -> Self {
        self.description = Some(description);
        self
    }
}

/// Result of [`SlashPalette::handle_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashAction<'a> {
    Closed,
    Picked { index: usize, name: &'a str },
    Changed,
    Unhandled,
}

/// Overlay picker keyed by the host's slash token.
#[derive(Debug, Clone, Default)]
pub struct SlashPalette {
    pub open: bool,
    picker: PickerState,
    area: Rect,
}

impl SlashPalette {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open_with(&mut self, query: &str) {
        self.open = true;
        self.picker.set_query(query);
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Keep the picker filter in sync with the Prompt's `/token`.
    pub fn sync_query(&mut self, query: &str) {
        self.picker.set_query(query);
    }

    /// First-line `/token` with no whitespace after the slash. `None` if the
    /// prompt is not in slash-entry mode.
    pub fn query(&self) -> &str {
        self.picker.query()
    }

    pub fn handle_key<'a>(
        &mut self,
        key: &KeyEvent,
        items: &'a [SlashItem<'a>],
    ) -> SlashAction<'a> {
        if !self.open {
            return SlashAction::Unhandled;
        }
        if key!(Esc).matches(key) {
            self.open = false;
            return SlashAction::Closed;
        }
        let entries = entries(items);
        if key!(Enter).matches(key) {
            if let Some(index) = self.picker.selected_id(&entries)
                && let Some(item) = items.get(index)
            {
                self.open = false;
                return SlashAction::Picked {
                    index,
                    name: item.name,
                };
            }
            self.open = false;
            return SlashAction::Closed;
        }
        let navigation = key!(Up).matches(key)
            || key!(Down).matches(key)
            || key!(PageUp).matches(key)
            || key!(PageDown).matches(key)
            || key!(Home).matches(key)
            || key!(End).matches(key)
            || key!('j').matches(key)
            || key!('k').matches(key)
            || key!('g').matches(key)
            || key!('G').matches(key)
            || key!('n', CONTROL).matches(key)
            || key!('p', CONTROL).matches(key);
        if navigation {
            let mut picker_key = *key;
            match key.code {
                KeyCode::Char('j') if key.modifiers.is_empty() => {
                    picker_key.code = KeyCode::Down;
                }
                KeyCode::Char('k') if key.modifiers.is_empty() => {
                    picker_key.code = KeyCode::Up;
                }
                KeyCode::Char('g') if key.modifiers.is_empty() => {
                    picker_key.code = KeyCode::Home;
                }
                KeyCode::Char('G') if key.modifiers.is_empty() => {
                    picker_key.code = KeyCode::End;
                }
                _ => {}
            }
            let _ = self
                .picker
                .handle_key(&picker_key, &entries, self.area.height.max(1) as usize);
            return SlashAction::Changed;
        }
        SlashAction::Unhandled
    }

    pub fn handle_scroll(&mut self, delta_lines: i32, items: &[SlashItem<'_>]) {
        if !self.open {
            return;
        }
        let entries = entries(items);
        let _ = self.picker.handle_scroll(delta_lines, self.area, &entries);
    }

    /// Occlude `area`, then render rich rows without an internal query bar.
    /// The host prompt remains the sole slash query editor.
    pub fn render(
        &mut self,
        items: &[SlashItem<'_>],
        area: Rect,
        buf: &mut Buffer,
        cx: &mut FrameCtx<'_>,
    ) {
        if !self.open || area.height == 0 || area.width < 8 {
            return;
        }
        self.area = area;
        cx.occlude(area);
        fill(
            buf,
            area,
            Style::default()
                .bg(cx.theme.bg_highlight)
                .fg(cx.theme.text_primary),
        );
        let entries = entries(items);
        let _ = render_picker_content(&mut self.picker, &entries, area, buf, cx);
    }

    /// Whether `id` belongs to a slash row or its rich-picker scrollbar.
    pub fn handles_hit(id: HitId) -> bool {
        parse_picker_row(id).is_some() || is_picker_scroll(id)
    }

    /// Stable source index for a rich picker row hit.
    pub fn parse_row(id: HitId) -> Option<usize> {
        parse_picker_row(id)
    }

    /// Map a row hit to the source item after the current rich-picker filter.
    pub fn pick_hit<'a>(
        &mut self,
        id: HitId,
        pointer_row: u16,
        items: &'a [SlashItem<'a>],
    ) -> Option<(usize, &'a str)> {
        if !self.open {
            return None;
        }
        let source = Self::parse_row(id)?;
        let entries = entries(items);
        if !self.picker.handle_hit(id, pointer_row, self.area, &entries) {
            return None;
        }
        let index = self.picker.selected_id(&entries)?;
        (index == source)
            .then(|| items.get(index).map(|item| (index, item.name)))
            .flatten()
    }
}

fn entries<'a>(items: &[SlashItem<'a>]) -> Vec<PickerEntry<'a>> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let mut row = PickerRow::new(index, item.name)
                .label(Line::from(vec![Span::raw("/"), Span::raw(item.name)]));
            if let Some(source_tag) = item.source_tag {
                row = row.badge(source_tag).search_field(source_tag);
            }
            if let Some(argument_hint) = item.argument_hint {
                row = row.right(argument_hint).search_field(argument_hint);
            }
            if let Some(description) = item.description {
                row = row.description(description).search_field(description);
            }
            PickerEntry::row(row.always_expanded())
        })
        .collect()
}

/// `/token` on the first line, with no whitespace after `/`.
///
/// `"/help"` → `Some("help")` · `"/"` → `Some("")` · `"/help "` → `None`.
pub fn slash_query(text: &str) -> Option<&str> {
    let line = text.lines().next().unwrap_or(text);
    let rest = line.strip_prefix('/')?;
    if rest.contains(|c: char| c.is_whitespace()) {
        None
    } else {
        Some(rest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui::core::{AnimationClock, Theme};
    use gbtui::runtime::{HitMap, HitResult};
    use std::time::Instant;

    fn items() -> Vec<SlashItem<'static>> {
        vec![
            SlashItem::new("help")
                .source_tag("builtin")
                .argument_hint("[topic]")
                .description("show commands\nwith examples"),
            SlashItem::new("permit")
                .source_tag("workspace")
                .argument_hint("<rule>")
                .description("change a permission rule"),
            SlashItem::new("quit")
                .source_tag("session")
                .description("exit"),
        ]
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(
            crossterm::event::KeyCode::Char(c),
            crossterm::event::KeyModifiers::NONE,
        )
    }

    #[test]
    fn slash_query_detects_token() {
        assert_eq!(slash_query("/help"), Some("help"));
        assert_eq!(slash_query("/"), Some(""));
        assert_eq!(slash_query("/help extra"), None);
        assert_eq!(slash_query("help"), None);
        assert_eq!(slash_query(""), None);
    }

    #[test]
    fn filter_and_pick() {
        let mut pal = SlashPalette::new();
        pal.open_with("he");
        let items = items();
        let enter = KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        match pal.handle_key(&enter, &items) {
            SlashAction::Picked { index, name } => {
                assert_eq!(index, 0);
                assert_eq!(name, "help");
            }
            other => panic!("expected pick, got {other:?}"),
        }
        assert!(!pal.open);
    }

    #[test]
    fn esc_closes_without_pick() {
        let mut pal = SlashPalette::new();
        pal.open_with("");
        let items = items();
        let esc = KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        assert_eq!(pal.handle_key(&esc, &items), SlashAction::Closed);
        assert!(!pal.open);
    }

    #[test]
    fn j_navigates_not_types() {
        let mut pal = SlashPalette::new();
        pal.open_with("q");
        let items = items();
        assert_eq!(pal.handle_key(&key('j'), &items), SlashAction::Changed);
        assert_eq!(pal.query(), "q");
        assert!(pal.open);
    }

    #[test]
    fn rich_rows_render_host_metadata_and_occlude() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 48, 10));
        let behind = HitId::named("behind");
        cx.register(behind, Rect::new(0, 0, 48, 10));
        let mut pal = SlashPalette::new();
        pal.open_with("");
        let items = items();
        pal.render(&items, Rect::new(0, 2, 48, 8), &mut buf, &mut cx);
        assert_eq!(hits.hit(0, 0), HitResult::Hit(behind));
        let row_id = match hits.hit(5, 2) {
            HitResult::Hit(id) => id,
            other => panic!("expected rich slash row hit, got {other:?}"),
        };
        assert!(SlashPalette::handles_hit(row_id));
        let row: String = (0..48).map(|x| buf[(x, 2)].symbol().to_string()).collect();
        let description: String = (0..48).map(|x| buf[(x, 3)].symbol().to_string()).collect();
        assert!(row.contains("/help"), "{row}");
        assert!(row.contains("builtin"), "{row}");
        assert!(row.contains("[topic]"), "{row}");
        assert!(description.contains("show commands"), "{description}");
        assert_eq!(pal.pick_hit(row_id, 2, &items), Some((0, "help")));
    }

    #[test]
    fn filtering_description_scrolls_variable_rows_without_fallback_selection() {
        let items = vec![
            SlashItem::new("first").description("one"),
            SlashItem::new("second").description("findable description"),
            SlashItem::new("third").description("three"),
        ];
        let mut pal = SlashPalette::new();
        pal.open_with("findable");
        let enter = KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        assert_eq!(
            pal.handle_key(&enter, &items),
            SlashAction::Picked {
                index: 1,
                name: "second"
            }
        );

        let many: Vec<_> = (0..12)
            .map(|index| {
                SlashItem::new(if index == 0 { "first" } else { "later" })
                    .description("two\nphysical rows")
            })
            .collect();
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut pal = SlashPalette::new();
        pal.open_with("");
        let area = Rect::new(0, 0, 36, 4);
        let mut before = Buffer::empty(area);
        pal.render(&many, area, &mut before, &mut cx);
        let physical_line = |y| {
            (0..35)
                .map(|x| before[(x, y)].symbol())
                .collect::<String>()
                .trim()
                .to_owned()
        };
        assert_eq!(physical_line(1), "two");
        assert_eq!(physical_line(2), "physical rows");
        assert!(physical_line(3).contains("/later"));
        pal.handle_scroll(5, &many);
        let mut after = Buffer::empty(area);
        pal.render(&many, area, &mut after, &mut cx);
        let before_text: String = (0..36)
            .map(|x| before[(x, 0)].symbol().to_string())
            .collect();
        let after_text: String = (0..36)
            .map(|x| after[(x, 0)].symbol().to_string())
            .collect();
        assert!(before_text.contains("/first"), "{before_text}");
        assert!(!after_text.contains("/first"), "{after_text}");

        let mut empty = SlashPalette::new();
        empty.open_with("missing");
        assert_eq!(empty.handle_key(&enter, &many), SlashAction::Closed);
    }
}
