//! Clickable hyperlink. Hover underline + HitMap click. OSC 8 emission is
//! the Terminal's job (inline `set_frame_links`); this widget is hover/click.
//!
//! ```text
//!  docs →
//! ```
//!
//! Default activation is always-on hover. [`LinkActivation::Modifier`] only
//! highlights when Ctrl or Super is held (not a macOS-only Cmd default).

use std::sync::Arc;

use crossterm::event::KeyModifiers;
use gbtui_core::{HitId, SafeBuf, ThemeRole, ThemeState};
use gbtui_runtime::FrameCtx;
use gbtui_runtime::terminal::HyperlinkCapabilities;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkActivation {
    #[default]
    Always,
    /// Highlight only while Ctrl or Super is held.
    Modifier,
}

/// Underlined label. Clicks arrive as `AppEvent::Hit(id, …)`.
pub struct Link {
    id: HitId,
    label: String,
    url: Arc<str>,
    activation: LinkActivation,
    native_hover: bool,
    style: Style,
}

impl Link {
    pub fn new(label: impl Into<String>, url: impl Into<Arc<str>>) -> Self {
        Self {
            id: HitId::from_raw(0),
            label: label.into(),
            url: url.into(),
            activation: LinkActivation::Always,
            native_hover: false,
            style: Style::default(),
        }
    }

    pub fn id(mut self, id: HitId) -> Self {
        self.id = id;
        self
    }

    pub fn activation(mut self, activation: LinkActivation) -> Self {
        self.activation = activation;
        self
    }

    /// Patches the semantic link style after the active theme role and state.
    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Skip app hover paint when the terminal styles links natively.
    pub fn capabilities(mut self, caps: HyperlinkCapabilities) -> Self {
        self.native_hover = caps.native_link_hover;
        self
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn hit_id(&self) -> HitId {
        self.id
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let label = truncate_to_width(&self.label, area.width);
        let width = label.width() as u16;
        if width == 0 {
            return;
        }
        let r = Rect::new(area.x, area.y, width, 1);
        cx.register(self.id, r);
        cx.register_link(r, Arc::clone(&self.url), None);
        let hovered = cx.hovered(r);
        let primed = match self.activation {
            LinkActivation::Always => hovered,
            LinkActivation::Modifier => hovered && activation_held(cx.modifiers),
        };
        let state =
            ThemeState::from_flags(false, false, false, false, primed && !self.native_hover);
        let style = cx.style(ThemeRole::Link, state).patch(self.style);
        buf.set_string_safe(r.x, r.y, label, style);
    }
}

fn truncate_to_width(text: &str, max_width: u16) -> &str {
    let mut width = 0u16;
    let mut end = 0;
    for (offset, ch) in text.char_indices() {
        let next = u16::try_from(ch.width().unwrap_or(0)).unwrap_or(u16::MAX);
        if width.saturating_add(next) > max_width {
            break;
        }
        width = width.saturating_add(next);
        end = offset + ch.len_utf8();
    }
    &text[..end]
}

fn activation_held(modifiers: KeyModifiers) -> bool {
    modifiers.contains(KeyModifiers::CONTROL) || modifiers.contains(KeyModifiers::SUPER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme};
    use gbtui_runtime::HitMap;
    use gbtui_runtime::terminal::HyperlinkCapabilities;
    use ratatui::style::Color;
    use std::time::Instant;

    #[test]
    fn hover_uses_semantic_link_style() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, Some((1, 0)), &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        Link::new("docs", "https://example.com")
            .id(HitId::named("docs"))
            .render(Rect::new(0, 0, 10, 1), &mut buf, &mut cx);
        let expected = theme.style(ThemeRole::Link, ThemeState::Hovered);
        assert_eq!(buf[(1, 0)].fg, expected.fg.unwrap_or(Color::Reset));
        assert_eq!(buf[(1, 0)].bg, expected.bg.unwrap_or(Color::Reset));
    }

    #[test]
    fn modifier_activation_needs_ctrl() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, Some((1, 0)), &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        Link::new("docs", "https://example.com")
            .id(HitId::named("docs"))
            .activation(LinkActivation::Modifier)
            .render(Rect::new(0, 0, 10, 1), &mut buf, &mut cx);
        let expected = theme.style(ThemeRole::Link, ThemeState::Normal);
        assert_eq!(buf[(1, 0)].fg, expected.fg.unwrap_or(Color::Reset));
        assert_eq!(buf[(1, 0)].bg, expected.bg.unwrap_or(Color::Reset));
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, Some((1, 0)), &mut anim, &mut hits)
            .with_modifiers(KeyModifiers::CONTROL);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        Link::new("docs", "https://example.com")
            .id(HitId::named("docs"))
            .activation(LinkActivation::Modifier)
            .render(Rect::new(0, 0, 10, 1), &mut buf, &mut cx);
        let expected = theme.style(ThemeRole::Link, ThemeState::Hovered);
        assert_eq!(buf[(1, 0)].fg, expected.fg.unwrap_or(Color::Reset));
        assert_eq!(buf[(1, 0)].bg, expected.bg.unwrap_or(Color::Reset));
    }

    #[test]
    fn link_span_matches_unicode_truncated_paint() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits)
            .with_viewport(Rect::new(0, 0, 3, 1))
            .with_hyperlink_capabilities(HyperlinkCapabilities::native());
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));

        Link::new("世A", "https://example.com").render(Rect::new(0, 0, 2, 1), &mut buf, &mut cx);

        assert_eq!(buf[(0, 0)].symbol(), "世");
        assert_eq!(cx.frame_links().len(), 1);
        assert_eq!(
            (cx.frame_links()[0].col_start, cx.frame_links()[0].col_end),
            (0, 2)
        );
    }
}
