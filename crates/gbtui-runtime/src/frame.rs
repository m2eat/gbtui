//! Per-frame drawing context: theme, mouse, hit registration, tick demand.

use std::sync::Arc;

use crossterm::event::KeyModifiers;
use gbtui_core::{AnimationClock, HitId, ResolvedTheme, Theme, ThemeRole, ThemeState, TickDemand};
use gbtui_terminal::{HyperlinkCapabilities, LinkSpan};
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;

#[cfg(feature = "images")]
use gbtui_terminal::image::ImagePlacement;

use crate::hitmap::HitMap;

/// Drawing output accumulated while an application paints one frame.
pub(crate) struct FrameOutput {
    pub links: Vec<LinkSpan>,
    #[cfg(feature = "images")]
    pub images: Vec<ImagePlacement>,
}

/// Immediate-mode frame context. Hover is **same-frame** (current mouse vs
/// `area`). Clicks use the **previous** frame's [`HitMap`].
pub struct FrameCtx<'a> {
    pub theme: &'a Theme,
    resolved_theme: Option<&'a ResolvedTheme>,
    pub mouse: Option<(u16, u16)>,
    /// Last known modifier keys (from the most recent key or mouse event).
    pub modifiers: KeyModifiers,
    anim: &'a mut AnimationClock,
    hits: &'a mut HitMap,
    cursor: Option<Position>,
    viewport: Rect,
    hyperlinks: HyperlinkCapabilities,
    links: Vec<LinkSpan>,
    #[cfg(feature = "images")]
    images_supported: bool,
    #[cfg(feature = "images")]
    images: Vec<ImagePlacement>,
}

impl<'a> FrameCtx<'a> {
    pub fn new(
        theme: &'a Theme,
        mouse: Option<(u16, u16)>,
        anim: &'a mut AnimationClock,
        hits: &'a mut HitMap,
    ) -> Self {
        Self {
            theme,
            mouse,
            resolved_theme: None,
            modifiers: KeyModifiers::NONE,
            anim,
            hits,
            cursor: None,
            // Standalone construction is deliberately conservative: it has no
            // confirmed terminal capability and therefore never emits OSC 8.
            viewport: Rect::new(0, 0, u16::MAX, u16::MAX),
            hyperlinks: HyperlinkCapabilities::default(),
            links: Vec::new(),
            #[cfg(feature = "images")]
            images_supported: false,
            #[cfg(feature = "images")]
            images: Vec::new(),
        }
    }

    /// Attaches the runtime's resolved semantic theme to this frame.
    ///
    /// Standalone contexts remain palette-only and use [`Theme::style`] as a
    /// fallback for [`Self::style`].
    pub fn with_resolved_theme(mut self, resolved_theme: &'a ResolvedTheme) -> Self {
        self.resolved_theme = Some(resolved_theme);
        self
    }

    /// The optional resolved semantic theme supplied by a runtime pump.
    pub fn resolved_theme(&self) -> Option<&ResolvedTheme> {
        self.resolved_theme
    }

    /// Resolves a semantic role and state without requiring each consumer to
    /// branch for standalone palette-only contexts.
    pub fn style(&self, role: ThemeRole, state: ThemeState) -> Style {
        self.resolved_theme.map_or_else(
            || self.theme.style(role, state),
            |theme| theme.style(role, state),
        )
    }

    /// Monotonic resolved-theme generation for renderer cache invalidation.
    ///
    /// Standalone palette-only contexts have no resolver and use generation
    /// zero.
    pub fn theme_version(&self) -> u64 {
        self.resolved_theme.map_or(0, ResolvedTheme::version)
    }

    pub fn with_modifiers(mut self, modifiers: KeyModifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// Sets the absolute terminal viewport used to clip output registrations.
    pub fn with_viewport(mut self, viewport: Rect) -> Self {
        self.viewport = viewport;
        self
    }

    /// Enables OSC 8 output only for a detected native terminal.
    pub fn with_hyperlink_capabilities(mut self, capabilities: HyperlinkCapabilities) -> Self {
        self.hyperlinks = capabilities;
        self
    }

    #[cfg(feature = "images")]
    /// Enables image output only when the live terminal session has a protocol.
    pub fn with_image_support(mut self, supported: bool) -> Self {
        self.images_supported = supported;
        self
    }

    pub fn hovered(&self, area: Rect) -> bool {
        self.mouse
            .is_some_and(|(col, row)| area.contains(Position { x: col, y: row }))
    }

    pub fn register(&mut self, id: HitId, area: Rect) {
        self.hits.register(id, area);
    }

    /// Register a stable scroll recipient without changing click or hover
    /// targets. Register after row targets belonging to this scrollable plane
    /// and before later overlay paint.
    pub fn register_scroll(&mut self, id: HitId, area: Rect) {
        self.hits.register_scroll(id, area);
    }

    /// Register an OSC 8 target over an absolute viewport rectangle.
    ///
    /// The target is clipped to the current viewport and split into physical
    /// rows for the terminal link layer. Later paint wins where rectangles
    /// overlap, mirroring immediate-mode widget z-order.
    pub fn register_link(&mut self, area: Rect, url: Arc<str>, id: Option<u32>) {
        let area = intersect_rect(area, self.viewport);
        if area.width == 0 || area.height == 0 {
            return;
        }

        self.occlude_links(area);
        if !self.hyperlinks.allows_url(&url) {
            return;
        }
        let id = self.hyperlinks.id_param.then_some(id).flatten();
        for row in area.top()..area.bottom() {
            self.links.push(LinkSpan {
                row,
                col_start: area.left(),
                col_end: area.right(),
                url: Arc::clone(&url),
                id,
            });
        }
    }

    /// Links collected for this frame, primarily for adapter tests and custom
    /// terminal hosts. The live pump consumes the same slice after drawing.
    pub fn frame_links(&self) -> &[LinkSpan] {
        &self.links
    }

    #[cfg(feature = "images")]
    pub fn supports_images(&self) -> bool {
        self.images_supported
    }

    #[cfg(feature = "images")]
    pub fn register_image(&mut self, mut placement: ImagePlacement) {
        if !self.images_supported {
            return;
        }
        placement.area = intersect_rect(placement.area, self.viewport);
        if placement.area.width == 0 || placement.area.height == 0 {
            return;
        }
        self.images.retain(|existing| {
            intersect_rect(existing.area, placement.area).width == 0
                || intersect_rect(existing.area, placement.area).height == 0
        });
        self.images.push(placement);
    }

    pub fn occlude(&mut self, area: Rect) {
        self.hits.occlude(area);
        self.occlude_links(intersect_rect(area, self.viewport));
        #[cfg(feature = "images")]
        self.images.retain(|placement| {
            intersect_rect(placement.area, area).width == 0
                || intersect_rect(placement.area, area).height == 0
        });
    }

    pub fn request_tick(&mut self, demand: TickDemand) {
        self.anim.request(demand);
    }

    pub fn anim(&self) -> &AnimationClock {
        self.anim
    }

    /// Hardware caret for this frame. Later calls win. The pump applies this
    /// with `Frame::set_cursor_position` so the terminal shows a blinking
    /// cursor (ratatui hides it when none is set).
    pub fn set_cursor(&mut self, x: u16, y: u16) {
        self.cursor = Some(Position { x, y });
    }

    pub fn cursor(&self) -> Option<Position> {
        self.cursor
    }

    pub(crate) fn into_output(self) -> FrameOutput {
        FrameOutput {
            links: self.links,
            #[cfg(feature = "images")]
            images: self.images,
        }
    }

    fn occlude_links(&mut self, area: Rect) {
        if area.width == 0 || area.height == 0 || self.links.is_empty() {
            return;
        }

        let mut index = 0;
        while index < self.links.len() {
            let span = &self.links[index];
            if span.row < area.top()
                || span.row >= area.bottom()
                || span.col_end <= area.left()
                || span.col_start >= area.right()
            {
                index += 1;
                continue;
            }

            let keep_left = span.col_start < area.left();
            let keep_right = span.col_end > area.right();
            match (keep_left, keep_right) {
                (false, false) => {
                    self.links.remove(index);
                }
                (true, false) => {
                    self.links[index].col_end = area.left();
                    index += 1;
                }
                (false, true) => {
                    self.links[index].col_start = area.right();
                    index += 1;
                }
                (true, true) => {
                    let mut right = self.links[index].clone();
                    self.links[index].col_end = area.left();
                    right.col_start = area.right();
                    self.links.insert(index + 1, right);
                    index += 2;
                }
            }
        }
    }
}

fn intersect_rect(a: Rect, b: Rect) -> Rect {
    let x = a.left().max(b.left());
    let y = a.top().max(b.top());
    Rect::new(
        x,
        y,
        a.right().min(b.right()).saturating_sub(x),
        a.bottom().min(b.bottom()).saturating_sub(y),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_terminal::HyperlinkCapabilities;
    use std::time::Instant;

    fn native() -> HyperlinkCapabilities {
        HyperlinkCapabilities::native()
    }

    fn context<'a>(
        theme: &'a Theme,
        anim: &'a mut AnimationClock,
        hits: &'a mut HitMap,
    ) -> FrameCtx<'a> {
        FrameCtx::new(theme, None, anim, hits)
            .with_viewport(Rect::new(2, 3, 6, 3))
            .with_hyperlink_capabilities(native())
    }

    #[test]
    fn scroll_registration_keeps_normal_hit_resolution_unchanged() {
        use crate::hitmap::HitResult;

        let theme = Theme::night();
        let mut anim = AnimationClock::new(Instant::now());
        let mut hits = HitMap::new();
        let row = HitId::named("row");
        let scroll = HitId::named("scroll");
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        cx.register(row, Rect::new(0, 0, 8, 1));
        cx.register_scroll(scroll, Rect::new(0, 0, 8, 1));
        drop(cx);

        assert_eq!(hits.hit(3, 0), HitResult::Hit(row));
        assert_eq!(hits.scroll_hit(3, 0), HitResult::Hit(scroll));
    }

    #[test]
    fn links_are_clipped_and_split_by_physical_row() {
        let theme = Theme::night();
        let mut anim = AnimationClock::new(Instant::now());
        let mut hits = HitMap::new();
        let mut cx = context(&theme, &mut anim, &mut hits);

        cx.register_link(Rect::new(1, 2, 4, 3), "https://example.com".into(), Some(9));

        assert_eq!(cx.frame_links().len(), 2);
        assert_eq!(cx.frame_links()[0].row, 3);
        assert_eq!(cx.frame_links()[0].col_start, 2);
        assert_eq!(cx.frame_links()[0].col_end, 5);
        assert_eq!(cx.frame_links()[1].row, 4);
        assert_eq!(cx.frame_links()[0].id, Some(9));
    }

    #[test]
    fn later_links_and_occlusion_hide_previously_painted_targets() {
        let theme = Theme::night();
        let mut anim = AnimationClock::new(Instant::now());
        let mut hits = HitMap::new();
        let mut cx = context(&theme, &mut anim, &mut hits);

        cx.register_link(
            Rect::new(2, 3, 6, 1),
            "https://first.example".into(),
            Some(1),
        );
        cx.occlude(Rect::new(4, 3, 2, 1));
        cx.register_link(Rect::new(5, 3, 2, 1), "https://top.example".into(), Some(2));

        let links = cx.frame_links();
        assert_eq!(links.len(), 3);
        assert_eq!((links[0].col_start, links[0].col_end), (2, 4));
        assert_eq!((links[1].col_start, links[1].col_end), (7, 8));
        assert_eq!(links[2].url.as_ref(), "https://top.example");
        assert_eq!((links[2].col_start, links[2].col_end), (5, 7));
    }

    #[test]
    fn unsupported_hostile_and_control_urls_never_enter_link_output() {
        let theme = Theme::night();
        let mut anim = AnimationClock::new(Instant::now());
        let mut hits = HitMap::new();
        let mut cx =
            FrameCtx::new(&theme, None, &mut anim, &mut hits).with_viewport(Rect::new(0, 0, 10, 1));

        cx.register_link(Rect::new(0, 0, 2, 1), "https://example.com".into(), None);
        assert!(cx.frame_links().is_empty(), "unknown support must be quiet");

        cx = cx.with_hyperlink_capabilities(HyperlinkCapabilities::hostile_parser());
        cx.register_link(Rect::new(0, 0, 2, 1), "https://example.com".into(), None);
        cx.register_link(
            Rect::new(0, 0, 2, 1),
            "https://example.com/\x1b]8;;evil".into(),
            None,
        );
        assert!(cx.frame_links().is_empty());
    }

    #[test]
    fn rejected_top_link_does_not_retain_a_hidden_target() {
        let theme = Theme::night();
        let mut anim = AnimationClock::new(Instant::now());
        let mut hits = HitMap::new();
        let mut cx = context(&theme, &mut anim, &mut hits);
        cx.register_link(Rect::new(2, 3, 6, 1), "https://example.com".into(), None);
        cx.register_link(Rect::new(4, 3, 2, 1), "javascript:alert(1)".into(), None);
        let spans: Vec<_> = cx
            .frame_links()
            .iter()
            .map(|span| (span.col_start, span.col_end))
            .collect();
        assert_eq!(spans, [(2, 4), (6, 8)]);
    }

    #[cfg(feature = "images")]
    #[test]
    fn occluded_images_are_removed_from_the_frame_output() {
        use std::sync::Arc;

        use gbtui_terminal::image::{ImagePlacement, PreparedImage};

        let theme = Theme::night();
        let mut anim = AnimationClock::new(Instant::now());
        let mut hits = HitMap::new();
        let mut cx = context(&theme, &mut anim, &mut hits).with_image_support(true);
        let image = Arc::new(PreparedImage::from_rgba8(1, 1, &[255, 0, 0, 255]).unwrap());

        cx.register_image(ImagePlacement {
            id: 42,
            image,
            area: Rect::new(3, 3, 2, 1),
        });
        cx.occlude(Rect::new(4, 3, 1, 1));

        assert!(cx.into_output().images.is_empty());
    }
}
