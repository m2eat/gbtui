//! Substring text search with smart-case. No regex crate.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/search/ (STRIP)` — P10f.
//! Regex `QueryKind` is intentionally omitted this phase.

/// Compiled substring query. Owns no corpus and no UI state.
#[derive(Debug, Clone)]
pub struct TextMatcher {
    query: String,
    /// Case-insensitive when the query has no uppercase (Vim smartcase).
    smart_ci: bool,
}

impl TextMatcher {
    pub fn new(query: impl Into<String>) -> Self {
        let query = query.into();
        let smart_ci = !query.chars().any(|c| c.is_uppercase());
        Self { query, smart_ci }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn is_match(&self, haystack: &str) -> bool {
        if self.query.is_empty() {
            return false;
        }
        if self.smart_ci {
            contains_ignore_ascii_case(haystack, &self.query)
        } else {
            haystack.contains(&self.query)
        }
    }

    /// Byte offsets of each match start, ascending.
    pub fn find_all(&self, haystack: &str) -> Vec<usize> {
        if self.query.is_empty() {
            return Vec::new();
        }
        if self.smart_ci {
            find_all_ignore_ascii_case(haystack, &self.query)
        } else {
            haystack
                .match_indices(&self.query)
                .map(|(i, _)| i)
                .collect()
        }
    }
}

/// First index in `sorted` strictly after `current`, wrapping to the front.
pub fn next_index_after(sorted: &[usize], current: usize) -> Option<usize> {
    if sorted.is_empty() {
        return None;
    }
    let pos = sorted.partition_point(|&i| i <= current);
    Some(if pos < sorted.len() { pos } else { 0 })
}

/// Last index in `sorted` strictly before `current`, wrapping to the back.
pub fn prev_index_before(sorted: &[usize], current: usize) -> Option<usize> {
    if sorted.is_empty() {
        return None;
    }
    let pos = sorted.partition_point(|&i| i < current);
    Some(if pos > 0 { pos - 1 } else { sorted.len() - 1 })
}

fn contains_ignore_ascii_case(haystack: &str, query: &str) -> bool {
    !find_all_ignore_ascii_case(haystack, query).is_empty()
}

fn find_all_ignore_ascii_case(haystack: &str, query: &str) -> Vec<usize> {
    let q: Vec<u8> = query.bytes().map(|b| b.to_ascii_lowercase()).collect();
    if q.is_empty() {
        return Vec::new();
    }
    let h = haystack.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + q.len() <= h.len() {
        if h[i..i + q.len()]
            .iter()
            .zip(q.iter())
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
            && haystack.is_char_boundary(i)
        {
            out.push(i);
            i += q.len().max(1);
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_case_insensitive_without_uppercase() {
        let m = TextMatcher::new("foo");
        assert!(m.is_match("FOO bar"));
        assert!(m.is_match("a foo b"));
        assert!(!m.is_match("bar"));
    }

    #[test]
    fn uppercase_query_is_sensitive() {
        let m = TextMatcher::new("Foo");
        assert!(m.is_match("Foo"));
        assert!(!m.is_match("foo"));
        assert!(!m.is_match("FOO"));
    }

    #[test]
    fn empty_query_matches_nothing() {
        let m = TextMatcher::new("");
        assert!(!m.is_match("abc"));
        assert!(m.find_all("abc").is_empty());
    }

    #[test]
    fn find_all_reports_byte_offsets() {
        let m = TextMatcher::new("ab");
        assert_eq!(m.find_all("ab x ab"), vec![0, 5]);
    }

    #[test]
    fn next_index_after_wraps() {
        let sorted = [0usize, 2, 4];
        assert_eq!(next_index_after(&sorted, 0), Some(1));
        assert_eq!(next_index_after(&sorted, 2), Some(2));
        assert_eq!(next_index_after(&sorted, 4), Some(0));
        assert_eq!(next_index_after(&[], 3), None);
        let one = [5usize];
        assert_eq!(next_index_after(&one, 5), Some(0));
        assert_eq!(next_index_after(&one, 1), Some(0));
    }

    #[test]
    fn prev_index_before_wraps() {
        let sorted = [0usize, 2, 4];
        assert_eq!(prev_index_before(&sorted, 4), Some(1));
        assert_eq!(prev_index_before(&sorted, 2), Some(0));
        assert_eq!(prev_index_before(&sorted, 0), Some(2));
        assert_eq!(prev_index_before(&[], 3), None);
    }
}
