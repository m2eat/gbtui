//! Hit testing: later registrations sit on top; occlude swallows everything below.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/app/agent_view HitArea (REWRITE)`

use gbtui_core::HitId;
use ratatui::layout::{Position, Rect};

#[derive(Debug, Clone, Copy)]
enum Entry {
    Target {
        id: HitId,
        rect: Rect,
    },
    /// Stable scroll recipient. It participates in scroll z-order only.
    Scroll {
        id: HitId,
        rect: Rect,
    },
    Occlude {
        rect: Rect,
    },
}

/// Hit-test map built during draw. Clicks use the **previous** frame.
/// Later [`register`](Self::register) calls sit on top; [`occlude`](Self::occlude)
/// swallows everything below that rect.
#[derive(Debug, Clone, Default)]
pub struct HitMap {
    entries: Vec<Entry>,
}

/// Result of [`HitMap::hit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitResult {
    Hit(HitId),
    /// Point is inside an occlude region (modal backdrop, dropdown mask).
    Occluded,
    Miss,
}

impl HitMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn register(&mut self, id: HitId, rect: Rect) {
        self.entries.push(Entry::Target { id, rect });
    }

    /// Register the stable recipient for scroll input in this plane.
    ///
    /// Register it after ordinary targets in the same scrollable plane so it
    /// stays stable while rows recycle, but before any later overlay paint.
    /// It is deliberately invisible to [`Self::hit`], so clicks and hover
    /// keep resolving only ordinary targets.
    pub fn register_scroll(&mut self, id: HitId, rect: Rect) {
        self.entries.push(Entry::Scroll { id, rect });
    }

    /// Swallow hits underneath this rect. Register *before* the overlay's
    /// own targets so those targets still sit on top.
    pub fn occlude(&mut self, rect: Rect) {
        self.entries.push(Entry::Occlude { rect });
    }

    pub fn hit(&self, col: u16, row: u16) -> HitResult {
        let pos = Position { x: col, y: row };
        for entry in self.entries.iter().rev() {
            match *entry {
                Entry::Target { id, rect } if rect.contains(pos) => return HitResult::Hit(id),
                Entry::Occlude { rect } if rect.contains(pos) => return HitResult::Occluded,
                _ => {}
            }
        }
        HitResult::Miss
    }

    /// Resolve a scroll recipient with the same z-order and occlusion rules
    /// as a normal hit. A stable scroll registration wins over row targets
    /// beneath it; without one, ordinary targets remain the fallback.
    pub fn scroll_hit(&self, col: u16, row: u16) -> HitResult {
        let pos = Position { x: col, y: row };
        for entry in self.entries.iter().rev() {
            match *entry {
                Entry::Target { id, rect } | Entry::Scroll { id, rect } if rect.contains(pos) => {
                    return HitResult::Hit(id);
                }
                Entry::Occlude { rect } if rect.contains(pos) => return HitResult::Occluded,
                _ => {}
            }
        }
        HitResult::Miss
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: HitId = HitId::named("a");
    const B: HitId = HitId::named("b");
    const C: HitId = HitId::named("c");

    fn r(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn later_registration_wins() {
        let mut map = HitMap::new();
        map.register(A, r(0, 0, 10, 3));
        map.register(B, r(5, 0, 10, 3));
        assert_eq!(map.hit(6, 1), HitResult::Hit(B));
        assert_eq!(map.hit(1, 1), HitResult::Hit(A));
    }

    #[test]
    fn occlude_swallows_below_but_not_later_targets() {
        let mut map = HitMap::new();
        map.register(A, r(0, 0, 20, 5));
        map.occlude(r(0, 0, 20, 5));
        map.register(B, r(2, 1, 4, 1));
        assert_eq!(map.hit(3, 1), HitResult::Hit(B));
        assert_eq!(map.hit(10, 2), HitResult::Occluded);
    }

    #[test]
    fn empty_is_miss() {
        let map = HitMap::new();
        assert_eq!(map.hit(0, 0), HitResult::Miss);
    }

    #[test]
    fn scroll_regions_are_stable_without_changing_click_hits() {
        let mut map = HitMap::new();
        map.register(A, r(0, 0, 20, 5));
        map.register_scroll(B, r(0, 0, 20, 5));

        assert_eq!(map.hit(3, 2), HitResult::Hit(A));
        assert_eq!(map.scroll_hit(3, 2), HitResult::Hit(B));
        map.register(C, r(2, 1, 4, 1));
        assert_eq!(map.scroll_hit(3, 1), HitResult::Hit(C));

        map.occlude(r(0, 0, 20, 5));
        assert_eq!(map.scroll_hit(3, 2), HitResult::Occluded);

        map.register_scroll(A, r(2, 1, 4, 1));
        assert_eq!(map.scroll_hit(3, 1), HitResult::Hit(A));
    }
}
