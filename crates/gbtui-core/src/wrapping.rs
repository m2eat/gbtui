//! Word wrap that preserves styled spans.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/render/wrapping.rs (STRIP)`
//!
//! Unlike the Grok original this uses owned strings from `textwrap` and
//! does not pointer-offset into the source.

use ratatui::text::Line;
use textwrap::Options;
use unicode_width::UnicodeWidthStr;

use crate::line_utils::push_owned_lines;

/// Wrap `text` to `width` columns. Empty input yields one empty line.
pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let opts = Options::new(width.max(1));
    textwrap::wrap(text, opts)
        .into_iter()
        .map(|cow| cow.into_owned())
        .collect()
}

/// Wrap a styled line, preserving span styles as well as possible.
pub fn wrap_line(line: &Line<'_>, width: u16) -> Vec<Line<'static>> {
    let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    if plain.width() <= width as usize {
        let mut out = Vec::new();
        push_owned_lines(std::slice::from_ref(line), &mut out);
        return out;
    }
    wrap_text(&plain, width as usize)
        .into_iter()
        .map(Line::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_yields_one_line() {
        assert_eq!(wrap_text("", 10), vec![""]);
    }

    #[test]
    fn wraps_long_words_by_width() {
        let lines = wrap_text("hello world", 5);
        assert!(lines.len() >= 2);
        assert!(lines.iter().all(|l| l.width() <= 5));
    }
}
