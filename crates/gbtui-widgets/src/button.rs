//! Clickable button. Hover/click go through [`FrameCtx`], never a mouse match.
//!
//! ```text
//!  ┌──────────┐
//!  │  Cancel  │   hover → semantic Button/Hovered style
//!  └──────────┘
//! ```
//!
//! Clicks arrive as `AppEvent::Hit(id, Pointer::Down { .. })`. Enter/Space
//! while focused is the application's job (see the settings example).

use gbtui_core::{HitId, SafeBuf, ThemeRole, ThemeState};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

/// Clickable label styled through [`ThemeRole::Button`].
pub struct Button {
    id: HitId,
    label: String,
    focused: bool,
    style: Style,
}

impl Button {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            id: HitId::from_raw(0),
            label: label.into(),
            focused: false,
            style: Style::default(),
        }
    }

    pub fn id(mut self, id: HitId) -> Self {
        self.id = id;
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Patches the semantic button style after the active theme role and state.
    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn hit_id(&self) -> HitId {
        self.id
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) {
        cx.register(self.id, area);
        let state = ThemeState::from_flags(false, false, false, self.focused, cx.hovered(area));
        let style = cx.style(ThemeRole::Button, state).patch(self.style);
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                buf.set_string_safe(x, y, " ", style);
            }
        }
        let label_w = self.label.width() as u16;
        let x = area.x + area.width.saturating_sub(label_w) / 2;
        let y = area.y + area.height.saturating_sub(1) / 2;
        buf.set_string_safe(x, y, &self.label, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme, ThemeContext, ThemeDefinition, ThemeResolver};
    use gbtui_runtime::HitMap;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Style};
    use std::time::Instant;

    #[test]
    fn hover_uses_theme_hover_background() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = gbtui_runtime::FrameCtx::new(&theme, Some((2, 0)), &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        Button::new("Cancel").id(HitId::named("cancel")).render(
            Rect::new(0, 0, 10, 1),
            &mut buf,
            &mut cx,
        );
        assert_eq!(buf[(2, 0)].bg, theme.button_hover_bg);
    }

    #[test]
    fn idle_uses_plain_background() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = gbtui_runtime::FrameCtx::new(&theme, Some((20, 20)), &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        Button::new("Cancel").id(HitId::named("cancel")).render(
            Rect::new(0, 0, 10, 1),
            &mut buf,
            &mut cx,
        );
        assert_eq!(buf[(2, 0)].bg, theme.button_bg);
    }

    #[test]
    fn resolved_button_role_override_reaches_render() {
        let definition = ThemeDefinition::from_palette("test", Theme::night()).with_style(
            ThemeRole::Button,
            ThemeState::Normal,
            Style::default().fg(Color::Yellow).bg(Color::Blue),
        );
        let resolver = ThemeResolver::new(definition, ThemeContext::default());
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx =
            gbtui_runtime::FrameCtx::new(resolver.resolved().palette(), None, &mut anim, &mut hits)
                .with_resolved_theme(resolver.resolved());
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));

        Button::new("Save").render(Rect::new(0, 0, 10, 1), &mut buf, &mut cx);

        assert_eq!(buf[(2, 0)].fg, Color::Yellow);
        assert_eq!(buf[(2, 0)].bg, Color::Blue);
    }

    #[test]
    fn local_button_style_patches_resolved_role_override_last() {
        let definition = ThemeDefinition::from_palette("test", Theme::night()).with_style(
            ThemeRole::Button,
            ThemeState::Normal,
            Style::default().fg(Color::Yellow).bg(Color::Blue),
        );
        let resolver = ThemeResolver::new(definition, ThemeContext::default());
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx =
            gbtui_runtime::FrameCtx::new(resolver.resolved().palette(), None, &mut anim, &mut hits)
                .with_resolved_theme(resolver.resolved());
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));

        Button::new("Save")
            .style(Style::default().fg(Color::Red))
            .render(Rect::new(0, 0, 10, 1), &mut buf, &mut cx);

        assert_eq!(buf[(2, 0)].fg, Color::Red);
        assert_eq!(buf[(2, 0)].bg, Color::Blue);
    }
}
