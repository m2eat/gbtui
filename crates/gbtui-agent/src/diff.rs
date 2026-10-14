//! Strict, source-preserving unified-diff parsing for expanded edit tools.
//!
//! The parser borrows the original tool detail and records byte ranges for every
//! rendered source line. It deliberately returns `None` unless at least one
//! complete hunk is structurally valid, so callers can fall back to a faithful
//! plain-text view instead of inventing line numbers for arbitrary tool output.

use std::ops::Range;

/// A parsed unified diff, grouped by `diff --git` file boundaries when present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedDiff<'a> {
    pub files: Vec<DiffFile<'a>>,
}

/// One file section of a unified diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile<'a> {
    /// Source-order metadata and hunks for this file.
    pub items: Vec<DiffFileItem<'a>>,
}

/// A source-order item in a [`DiffFile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffFileItem<'a> {
    Metadata(DiffMetadata<'a>),
    Hunk(DiffHunk<'a>),
}

/// File-level metadata that is not a hunk body line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffMetadata<'a> {
    pub kind: DiffMetadataKind,
    /// The original line, without its line terminator.
    pub text: &'a str,
    /// Byte range in the detail passed to [`parse_unified_diff`].
    pub range: Range<usize>,
}

/// Display category for file metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffMetadataKind {
    /// `diff --git …`
    FileHeader,
    /// `--- …`
    OldFile,
    /// `+++ …`
    NewFile,
    /// Extended git metadata, binary notices, or an unclassified prelude.
    Extended,
}

/// One parsed hunk, including its header and body lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk<'a> {
    pub header: DiffHunkHeader<'a>,
    pub lines: Vec<DiffLine<'a>>,
}

/// The range and counters declared by a unified-diff hunk header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunkHeader<'a> {
    /// The complete original `@@ … @@` header line.
    pub text: &'a str,
    pub range: Range<usize>,
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
}

/// A source line in a parsed hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine<'a> {
    pub kind: DiffLineKind,
    /// One-based old-file line number, when this line consumes old input.
    pub old_line: Option<u32>,
    /// One-based new-file line number, when this line consumes new input.
    pub new_line: Option<u32>,
    /// The complete original line, including the unified-diff prefix.
    pub text: &'a str,
    /// Byte range in the detail passed to [`parse_unified_diff`].
    pub range: Range<usize>,
}

/// The semantic kind of a unified-diff body line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Addition,
    Deletion,
    /// `\ No newline at end of file`; it consumes neither line counter.
    NoNewlineMarker,
}

#[derive(Debug, Clone)]
struct SourceLine<'a> {
    text: &'a str,
    range: Range<usize>,
}

/// Parse a complete unified diff while borrowing, rather than copying, its source.
///
/// A diff is accepted only if it has at least one syntactically valid hunk whose
/// declared old/new counts agree with its body. Metadata outside a valid hunk is
/// retained in source order. This lets a caller render full `git diff` output,
/// including empty-file headers and EOF markers, while keeping malformed tool
/// output in its original plain-text form.
pub fn parse_unified_diff(input: &str) -> Option<UnifiedDiff<'_>> {
    let source = source_lines(input);
    let mut files = Vec::new();
    let mut current = DiffFile { items: Vec::new() };
    let mut index = 0usize;
    let mut saw_hunk = false;

    while let Some(line) = source.get(index).cloned() {
        if let Some(header) = parse_hunk_header(&line) {
            let (hunk, next) = parse_hunk(&source, index, header)?;
            current.items.push(DiffFileItem::Hunk(hunk));
            saw_hunk = true;
            index = next;
            continue;
        }

        if line.text.starts_with("diff --git ") && !current.items.is_empty() {
            files.push(current);
            current = DiffFile { items: Vec::new() };
        }
        current.items.push(DiffFileItem::Metadata(DiffMetadata {
            kind: metadata_kind(line.text),
            text: line.text,
            range: line.range,
        }));
        index = index.saturating_add(1);
    }

    if !saw_hunk {
        return None;
    }
    if !current.items.is_empty() {
        files.push(current);
    }
    Some(UnifiedDiff { files })
}

fn source_lines(input: &str) -> Vec<SourceLine<'_>> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    for raw in input.split_inclusive('\n') {
        let full_end = start.saturating_add(raw.len());
        let line_end = raw.strip_suffix('\n').map_or(full_end, |_| full_end - 1);
        let content_end = if input.as_bytes().get(line_end.saturating_sub(1)) == Some(&b'\r') {
            line_end.saturating_sub(1)
        } else {
            line_end
        };
        lines.push(SourceLine {
            text: &input[start..content_end],
            range: start..content_end,
        });
        start = full_end;
    }
    lines
}

fn metadata_kind(text: &str) -> DiffMetadataKind {
    if text.starts_with("diff --git ") {
        DiffMetadataKind::FileHeader
    } else if text.starts_with("--- ") {
        DiffMetadataKind::OldFile
    } else if text.starts_with("+++ ") {
        DiffMetadataKind::NewFile
    } else {
        DiffMetadataKind::Extended
    }
}

fn parse_hunk<'a>(
    source: &[SourceLine<'a>],
    header_index: usize,
    header: DiffHunkHeader<'a>,
) -> Option<(DiffHunk<'a>, usize)> {
    let mut index = header_index.saturating_add(1);
    let mut old_remaining = header.old_count;
    let mut new_remaining = header.new_count;
    let mut old_line = header.old_start;
    let mut new_line = header.new_start;
    let mut lines = Vec::new();
    let mut previous_was_body = false;

    while old_remaining != 0 || new_remaining != 0 {
        let line = source.get(index)?.clone();
        let (kind, consumes_old, consumes_new) = match line.text.as_bytes().first().copied() {
            Some(b' ') if old_remaining != 0 && new_remaining != 0 => {
                (DiffLineKind::Context, true, true)
            }
            Some(b'-') if old_remaining != 0 => (DiffLineKind::Deletion, true, false),
            Some(b'+') if new_remaining != 0 => (DiffLineKind::Addition, false, true),
            Some(b'\\') if line.text == "\\ No newline at end of file" && previous_was_body => {
                lines.push(DiffLine {
                    kind: DiffLineKind::NoNewlineMarker,
                    old_line: None,
                    new_line: None,
                    text: line.text,
                    range: line.range,
                });
                index = index.saturating_add(1);
                previous_was_body = false;
                continue;
            }
            _ => return None,
        };

        let old_number = consumes_old.then_some(old_line);
        let new_number = consumes_new.then_some(new_line);
        lines.push(DiffLine {
            kind,
            old_line: old_number,
            new_line: new_number,
            text: line.text,
            range: line.range,
        });
        if consumes_old {
            old_remaining -= 1;
            if old_remaining != 0 {
                old_line = old_line.checked_add(1)?;
            }
        }
        if consumes_new {
            new_remaining -= 1;
            if new_remaining != 0 {
                new_line = new_line.checked_add(1)?;
            }
        }
        previous_was_body = true;
        index = index.saturating_add(1);
    }

    while let Some(line) = source.get(index).cloned() {
        if line.text != "\\ No newline at end of file" || !previous_was_body {
            break;
        }
        lines.push(DiffLine {
            kind: DiffLineKind::NoNewlineMarker,
            old_line: None,
            new_line: None,
            text: line.text,
            range: line.range,
        });
        index = index.saturating_add(1);
        previous_was_body = false;
    }

    Some((DiffHunk { header, lines }, index))
}

fn parse_hunk_header<'a>(line: &SourceLine<'a>) -> Option<DiffHunkHeader<'a>> {
    let rest = line.text.strip_prefix("@@ -")?;
    let (old_start, old_count, rest) = parse_hunk_range(rest)?;
    let rest = rest.strip_prefix(" +")?;
    let (new_start, new_count, rest) = parse_hunk_range(rest)?;
    rest.strip_prefix(" @@")?;
    if old_count != 0 && old_start == 0 || new_count != 0 && new_start == 0 {
        return None;
    }
    Some(DiffHunkHeader {
        text: line.text,
        range: line.range.clone(),
        old_start,
        old_count,
        new_start,
        new_count,
    })
}

fn parse_hunk_range(input: &str) -> Option<(u32, u32, &str)> {
    let digits = input.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let start = input[..digits].parse().ok()?;
    let rest = &input[digits..];
    if let Some(rest) = rest.strip_prefix(',') {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let count = rest[..digits].parse().ok()?;
        Some((start, count, &rest[digits..]))
    } else {
        Some((start, 1, rest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_hunks_and_advances_both_counters() {
        let diff = parse_unified_diff(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -8,2 +8,3 @@ fn f() {\n keep\n-old\n+new\n+tail\n@@ -20 +21 @@\n-again\n+later",
        )
        .expect("valid diff");
        let DiffFileItem::Hunk(first) = &diff.files[0].items[3] else {
            panic!("first hunk");
        };
        assert_eq!(first.lines[0].old_line, Some(8));
        assert_eq!(first.lines[0].new_line, Some(8));
        assert_eq!(first.lines[1].old_line, Some(9));
        assert_eq!(first.lines[1].new_line, None);
        assert_eq!(first.lines[2].old_line, None);
        assert_eq!(first.lines[2].new_line, Some(9));
        assert_eq!(first.lines[3].new_line, Some(10));
        let DiffFileItem::Hunk(second) = &diff.files[0].items[4] else {
            panic!("second hunk");
        };
        assert_eq!(second.header.old_start, 20);
        assert_eq!(second.header.new_start, 21);
    }

    #[test]
    fn keeps_file_headers_and_eof_markers_out_of_content() {
        let diff = parse_unified_diff(
            "--- /dev/null\n+++ b/new.rs\n@@ -0,0 +1 @@\n+hello\n\\ No newline at end of file",
        )
        .expect("valid new file");
        assert!(matches!(
            diff.files[0].items[0],
            DiffFileItem::Metadata(DiffMetadata {
                kind: DiffMetadataKind::OldFile,
                ..
            })
        ));
        assert!(matches!(
            diff.files[0].items[1],
            DiffFileItem::Metadata(DiffMetadata {
                kind: DiffMetadataKind::NewFile,
                ..
            })
        ));
        let DiffFileItem::Hunk(hunk) = &diff.files[0].items[2] else {
            panic!("hunk");
        };
        assert_eq!(hunk.lines[0].kind, DiffLineKind::Addition);
        assert_eq!(hunk.lines[0].new_line, Some(1));
        assert_eq!(hunk.lines[1].kind, DiffLineKind::NoNewlineMarker);
    }

    #[test]
    fn parses_empty_file_deletion() {
        let diff =
            parse_unified_diff("--- a/old.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-old file contents")
                .expect("valid deleted file");
        let DiffFileItem::Hunk(hunk) = &diff.files[0].items[2] else {
            panic!("hunk");
        };
        assert_eq!(hunk.lines[0].kind, DiffLineKind::Deletion);
        assert_eq!(hunk.lines[0].old_line, Some(1));
        assert_eq!(hunk.lines[0].new_line, None);
    }

    #[test]
    fn rejects_incomplete_or_non_diff_text() {
        assert!(parse_unified_diff("+ output\n- output").is_none());
        assert!(parse_unified_diff("@@ -1,2 +1 @@\n-old\n+new").is_none());
        assert!(parse_unified_diff("@@ not a hunk @@\nplain text").is_none());
    }

    #[test]
    fn treats_prefix_looking_lines_as_content_inside_a_hunk() {
        let diff = parse_unified_diff("@@ -1 +1 @@\n--- old content\n+++ new content")
            .expect("valid replacement");
        let DiffFileItem::Hunk(hunk) = &diff.files[0].items[0] else {
            panic!("hunk");
        };
        assert_eq!(hunk.lines[0].kind, DiffLineKind::Deletion);
        assert_eq!(hunk.lines[1].kind, DiffLineKind::Addition);
    }
}
