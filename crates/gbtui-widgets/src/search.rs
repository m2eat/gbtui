//! Search policy shared by rich lists and pickers.
//!
//! Matching produces grapheme-safe UTF-8 byte ranges, never terminal columns.
//! Substring mode reuses core's ASCII smart-case matcher; fuzzy mode uses skim's
//! smart-case scoring; regex mode respects regex syntax and inline flags.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};
use gbtui_core::TextMatcher;
use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchKind {
    #[default]
    Substring,
    Fuzzy,
    Regex,
}

/// Whether the view hides unmatched rows or keeps them while navigating matches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchMode {
    #[default]
    Filter,
    Search,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    /// Higher fuzzy scores rank before lower scores. Other modes use zero.
    pub score: i64,
    /// Sorted, non-overlapping byte ranges expanded to complete graphemes.
    pub ranges: Vec<Range<usize>>,
}

/// A prepared query with no corpus or filesystem ownership.
#[derive(Clone)]
pub struct SearchQuery {
    substring: TextMatcher,
    kind: SearchKind,
    mode: SearchMode,
    regex: Option<Regex>,
    error: Option<String>,
    fuzzy: Arc<SkimMatcherV2>,
    revision: u64,
}

impl fmt::Debug for SearchQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchQuery")
            .field("query", &self.query())
            .field("kind", &self.kind)
            .field("mode", &self.mode)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            substring: TextMatcher::new(""),
            kind: SearchKind::Substring,
            mode: SearchMode::Filter,
            regex: None,
            error: None,
            fuzzy: Arc::new(SkimMatcherV2::default()),
            revision: 0,
        }
    }
}

impl SearchQuery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn query(&self) -> &str {
        self.substring.query()
    }
    pub fn kind(&self) -> SearchKind {
        self.kind
    }
    pub fn mode(&self) -> SearchMode {
        self.mode
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        let query = query.into();
        if self.query() != query {
            self.substring = TextMatcher::new(query);
            self.recompile();
        }
    }

    pub fn set_kind(&mut self, kind: SearchKind) {
        if self.kind != kind {
            self.kind = kind;
            self.recompile();
        }
    }

    pub fn set_mode(&mut self, mode: SearchMode) {
        if self.mode != mode {
            self.mode = mode;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    fn recompile(&mut self) {
        self.regex = None;
        self.error = None;
        if self.kind == SearchKind::Regex {
            match Regex::new(self.query()) {
                Ok(regex) => self.regex = Some(regex),
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        self.revision = self.revision.wrapping_add(1);
    }

    /// An empty query matches every row without highlighting it. Invalid regex
    /// queries match no rows and expose their compilation error through `error`.
    /// Zero-width regex matches remain matches but produce no highlighted cells.
    pub fn find(&self, text: &str) -> Option<SearchMatch> {
        if self.error.is_some() {
            return None;
        }
        if self.query().is_empty() {
            return Some(SearchMatch {
                score: 0,
                ranges: Vec::new(),
            });
        }
        let (score, ranges) = match self.kind {
            SearchKind::Substring => {
                let starts = self.substring.find_all(text);
                if starts.is_empty() {
                    return None;
                }
                (
                    0,
                    starts
                        .into_iter()
                        .map(|start| start..start + self.query().len())
                        .collect(),
                )
            }
            SearchKind::Regex => {
                let regex = self.regex.as_ref()?;
                let matches: Vec<_> = regex.find_iter(text).collect();
                if matches.is_empty() {
                    return None;
                }
                (0, matches.into_iter().map(|m| m.range()).collect())
            }
            SearchKind::Fuzzy => {
                let (score, positions) = self.fuzzy.fuzzy_indices(text, self.query())?;
                let mut positions = positions.into_iter().peekable();
                let mut ranges = Vec::new();
                for (index, (byte, ch)) in text.char_indices().enumerate() {
                    if positions.peek() == Some(&index) {
                        ranges.push(byte..byte + ch.len_utf8());
                        positions.next();
                    }
                    if positions.peek().is_none() {
                        break;
                    }
                }
                (score, ranges)
            }
        };
        Some(SearchMatch {
            score,
            ranges: grapheme_ranges(text, ranges),
        })
    }
}

fn grapheme_ranges(text: &str, mut matches: Vec<Range<usize>>) -> Vec<Range<usize>> {
    matches.retain(|range| !range.is_empty());
    matches.sort_unstable_by_key(|range| range.start);
    let mut output: Vec<Range<usize>> = Vec::with_capacity(matches.len());
    let mut current = 0;
    for (start, grapheme) in text.grapheme_indices(true) {
        let end = start + grapheme.len();
        while current < matches.len() && matches[current].end <= start {
            current += 1;
        }
        let Some(matched) = matches.get(current) else {
            break;
        };
        if matched.start < end {
            if let Some(previous) = output.last_mut()
                && previous.end == start
            {
                previous.end = end;
            } else {
                output.push(start..end);
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_regex_recovers_without_losing_the_query() {
        let mut query = SearchQuery::new();
        query.set_kind(SearchKind::Regex);
        query.set_query("(");
        assert!(query.error().is_some());
        assert!(query.find("anything").is_none());
        query.set_kind(SearchKind::Substring);
        assert_eq!(query.find("a(b").unwrap().ranges, vec![1..2]);
        query.set_kind(SearchKind::Regex);
        query.set_query("(?i)^alpha$");
        assert!(query.find("ALPHA").is_some());
        assert!(query.find("xalpha").is_none());
    }

    #[test]
    fn fuzzy_scoring_prefers_consecutive_matches_and_returns_utf8_ranges() {
        let mut query = SearchQuery::new();
        query.set_kind(SearchKind::Fuzzy);
        query.set_query("ab");
        assert!(query.find("ab").unwrap().score > query.find("axxxb").unwrap().score);
        assert_eq!(query.find("甲a😀b").unwrap().ranges, vec![3..4, 8..9]);
    }

    #[test]
    fn highlights_never_split_combining_graphemes() {
        let mut query = SearchQuery::new();
        query.set_query("e");
        assert_eq!(query.find("e\u{301}cho").unwrap().ranges, vec![0..3]);
        query.set_kind(SearchKind::Regex);
        query.set_query(r"\p{M}");
        assert_eq!(query.find("e\u{301}cho").unwrap().ranges, vec![0..3]);
    }

    #[test]
    fn zero_width_and_empty_queries_match_without_highlighting() {
        let mut query = SearchQuery::new();
        query.set_kind(SearchKind::Regex);
        query.set_query("^");
        assert!(query.find("text").unwrap().ranges.is_empty());
        query.set_query("");
        assert!(query.find("").unwrap().ranges.is_empty());
    }
}
