//! Mention chips. Chip-shaped, accent-colored. Host supplies labels — no FS.
//!
//! ```text
//!  @src [✗]  @alice [✗]  +2
//! ```
//!
//! # Mouse
//!
//! Body and close register through [`FrameCtx`]. `@` insertion is the host's
//! job; Prompt stays mention-free.

use gbtui::core::{HitId, SafeBuf};
use gbtui::runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::paint::fill;

pub const NS_MENTION: u32 = 0xA013;
pub const NS_MENTION_CLOSE: u32 = 0xA015;

const CLOSE_LABEL: &str = "[✗]";

pub fn mention_id(index: u32) -> HitId {
    HitId::namespaced(NS_MENTION, index)
}

pub fn mention_close_id(index: u32) -> HitId {
    HitId::namespaced(NS_MENTION_CLOSE, index)
}

pub fn parse_mention_id(id: HitId) -> Option<u32> {
    (id.namespace() == NS_MENTION).then_some(id.index())
}

pub fn parse_mention_close_id(id: HitId) -> Option<u32> {
    (id.namespace() == NS_MENTION_CLOSE).then_some(id.index())
}

fn display_label(label: &str) -> String {
    if label.starts_with('@') {
        label.to_string()
    } else {
        format!("@{label}")
    }
}

/// One mention chip: `@label [✗]`.
pub struct MentionChip<'a> {
    label: &'a str,
    index: u32,
    dismissible: bool,
}

impl<'a> MentionChip<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            index: 0,
            dismissible: true,
        }
    }

    pub fn index(mut self, index: u32) -> Self {
        self.index = index;
        self
    }

    pub fn dismissible(mut self, on: bool) -> Self {
        self.dismissible = on;
        self
    }

    pub fn width(&self) -> u16 {
        let shown = display_label(self.label);
        let mut w = shown.width() as u16 + 2;
        if self.dismissible {
            w = w.saturating_add(1 + CLOSE_LABEL.width() as u16);
        }
        w
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) -> u16 {
        if area.height == 0 || area.width == 0 {
            return 0;
        }
        let shown = display_label(self.label);
        let w = self.width().min(area.width);
        let r = Rect {
            x: area.x,
            y: area.y,
            width: w,
            height: 1,
        };
        cx.register(mention_id(self.index), r);
        let hot = cx.hovered(r);
        let style = if hot {
            Style::default()
                .fg(cx.theme.text_primary)
                .bg(cx.theme.bg_hover)
        } else {
            Style::default()
                .fg(cx.theme.accent)
                .bg(cx.theme.bg_elevated)
        };
        fill(buf, r, style);
        buf.set_string_safe(r.x, r.y, format!(" {shown} "), style);
        if self.dismissible {
            let cw = CLOSE_LABEL.width() as u16;
            if w > cw {
                let close = Rect {
                    x: r.right().saturating_sub(cw),
                    y: r.y,
                    width: cw,
                    height: 1,
                };
                cx.register(mention_close_id(self.index), close);
                let close_style = if cx.hovered(close) {
                    Style::default()
                        .fg(cx.theme.error)
                        .bg(cx.theme.bg_hover)
                        .add_modifier(Modifier::BOLD)
                } else {
                    style
                };
                buf.set_string_safe(close.x, close.y, CLOSE_LABEL, close_style);
            }
        }
        w
    }
}

/// One-row wrap of mention chips. Overflow is a `+N` suffix.
pub fn render_mention_bar(
    labels: &[&str],
    area: Rect,
    buf: &mut Buffer,
    cx: &mut FrameCtx<'_>,
) -> u16 {
    if area.height == 0 || area.width == 0 || labels.is_empty() {
        return 0;
    }
    fill(
        buf,
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
        Style::default()
            .bg(cx.theme.bg_base)
            .fg(cx.theme.text_primary),
    );
    let mut x = area.x;
    for (i, label) in labels.iter().enumerate() {
        let chip = MentionChip::new(label).index(i as u32);
        let w = chip.width();
        let rest = labels.len() - (i + 1);
        let overflow_w = if rest > 0 {
            format!("+{rest}").width() as u16 + 1
        } else {
            0
        };
        if x.saturating_add(w).saturating_add(overflow_w) > area.right() {
            let n = labels.len() - i;
            if n > 0 && x < area.right() {
                let tag = format!("+{n}");
                buf.set_string_safe(
                    x,
                    area.y,
                    &tag,
                    Style::default().fg(cx.theme.gray).bg(cx.theme.bg_base),
                );
            }
            break;
        }
        chip.render(
            Rect {
                x,
                y: area.y,
                width: w,
                height: 1,
            },
            buf,
            cx,
        );
        x = x.saturating_add(w).saturating_add(1);
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui::core::{AnimationClock, Theme};
    use gbtui::runtime::{HitMap, HitResult};
    use std::time::Instant;

    #[test]
    fn close_hit_and_accent() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let chip = MentionChip::new("src").index(2);
        let w = chip.width();
        let mut cx = FrameCtx::new(&theme, Some((w - 1, 0)), &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        chip.render(Rect::new(0, 0, 20, 1), &mut buf, &mut cx);
        assert_eq!(hits.hit(w - 1, 0), HitResult::Hit(mention_close_id(2)));
        assert_eq!(hits.hit(1, 0), HitResult::Hit(mention_id(2)));
        assert_eq!(buf[(w - 1, 0)].fg, theme.error);
        let joined: String = (0..w).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(joined.contains("@src"), "{joined}");
    }

    #[test]
    fn occlude_swallows_close() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        MentionChip::new("src")
            .index(0)
            .render(Rect::new(0, 0, 20, 1), &mut buf, &mut cx);
        cx.occlude(Rect::new(0, 0, 20, 1));
        assert_eq!(hits.hit(1, 0), HitResult::Occluded);
    }

    #[test]
    fn bar_truncates_with_plus_n() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, None, &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 16, 1));
        let labels = ["aaaa", "bbbb", "cccc", "dddd"];
        render_mention_bar(&labels, Rect::new(0, 0, 16, 1), &mut buf, &mut cx);
        let joined: String = (0..16).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(joined.contains('+'), "{joined}");
    }
}
