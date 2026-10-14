//! Line helpers used by widgets and wrapping.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/render/line_utils.rs (STRIP)`

use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub fn line_to_static(line: &Line<'_>) -> Line<'static> {
    Line {
        style: line.style,
        alignment: line.alignment,
        spans: line
            .spans
            .iter()
            .map(|s| Span {
                style: s.style,
                content: std::borrow::Cow::Owned(s.content.to_string()),
            })
            .collect(),
    }
}

pub fn push_owned_lines(src: &[Line<'_>], out: &mut Vec<Line<'static>>) {
    for l in src {
        out.push(line_to_static(l));
    }
}

pub fn is_unsafe_display_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
        )
}

/// Byte index of the first grapheme that would exceed `max_width` columns.
pub fn byte_offset_at_width(s: &str, max_width: usize) -> usize {
    let mut used = 0usize;
    for (i, g) in s.grapheme_indices(true) {
        let w = g.width();
        if used + w > max_width {
            return i;
        }
        used += w;
    }
    s.len()
}

pub fn truncate_str(s: &str, max_width: usize) -> String {
    if s.width() <= max_width {
        return s.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    let budget = max_width.saturating_sub(1);
    let end = byte_offset_at_width(s, budget);
    let mut out = s[..end].to_string();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_string_is_unchanged() {
        assert_eq!(truncate_str("hi", 10), "hi");
    }

    #[test]
    fn truncate_long_string_adds_ellipsis() {
        let t = truncate_str("abcdefghij", 4);
        assert!(t.ends_with('…'));
        assert!(t.width() <= 4);
    }
}
