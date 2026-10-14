//! Bottom shortcuts bar: `HintItem`, compact truncation, pinned, pending confirm.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/views/shortcuts_bar.rs (STRIP)`
//!
//! ```text
//!  o:overlay  │  Enter:modal  │  q:quit
//! ```
//!
//! Hover/click go through `FrameCtx`. Double-press confirm is [`PendingConfirm`]
//! (default TTL 1000ms), not a product `ActionId`.

use std::borrow::Cow;
use std::time::{Duration, Instant};

use crossterm::event::KeyEvent;
use gbtui_core::{HitId, KeyShortcut, SafeBuf, ThemeRole, ThemeState};
use gbtui_runtime::FrameCtx;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use unicode_width::UnicodeWidthStr;

use crate::paint::fill;

pub const NS_HINT: u32 = 0x4849;

/// One shortcuts-bar entry. Keys are structured; the bar decides how they render.
#[derive(Debug, Clone)]
pub struct HintItem {
    pub keys: Vec<KeyShortcut>,
    pub label: Cow<'static, str>,
    pub custom_display: Option<&'static str>,
    pub description: Option<Cow<'static, str>>,
    pub pinned: bool,
}

impl HintItem {
    pub fn new(key: KeyShortcut, label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            keys: vec![key],
            label: label.into(),
            custom_display: None,
            description: None,
            pinned: false,
        }
    }

    pub fn paired(a: KeyShortcut, b: KeyShortcut, label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            keys: vec![a, b],
            label: label.into(),
            custom_display: None,
            description: None,
            pinned: false,
        }
    }

    pub fn pinned(mut self) -> Self {
        self.pinned = true;
        self
    }

    fn bar_key_display(&self) -> String {
        bar_key_segments(self)
            .into_iter()
            .map(|seg| seg.text)
            .collect()
    }
}

struct BarKeySeg {
    text: String,
    #[allow(dead_code)]
    is_join: bool,
}

fn bar_key_segments(hint: &HintItem) -> Vec<BarKeySeg> {
    if hint.keys.len() <= 1 {
        let text = if let Some(d) = hint.custom_display {
            d.to_string()
        } else if let Some(k) = hint.keys.first() {
            k.display()
        } else {
            String::new()
        };
        if text.is_empty() {
            return Vec::new();
        }
        return vec![BarKeySeg {
            text,
            is_join: false,
        }];
    }

    let shared = hint
        .keys
        .iter()
        .all(|k| k.modifiers == hint.keys[0].modifiers);
    let mut segs = Vec::new();
    if shared {
        let prefix = hint.keys[0].modifiers_prefix();
        if !prefix.is_empty() {
            segs.push(BarKeySeg {
                text: prefix,
                is_join: false,
            });
        }
        for (i, k) in hint.keys.iter().enumerate() {
            if i > 0 {
                segs.push(BarKeySeg {
                    text: "/".into(),
                    is_join: true,
                });
            }
            segs.push(BarKeySeg {
                text: k.code_display(),
                is_join: false,
            });
        }
    } else {
        for (i, k) in hint.keys.iter().enumerate() {
            if i > 0 {
                segs.push(BarKeySeg {
                    text: "/".into(),
                    is_join: true,
                });
            }
            segs.push(BarKeySeg {
                text: k.display(),
                is_join: false,
            });
        }
    }
    segs
}

/// Truncate to `max_visible` unpinned slots, then always append `help_hint`.
pub struct CompactConfig {
    pub max_visible: usize,
    pub help_hint: Option<HintItem>,
}

/// Info needed to render the "press again" hint.
/// “press again to {label}” payload for the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingHint {
    pub shortcut: KeyShortcut,
    pub label: &'static str,
}

/// Generic double-press confirmation. Default TTL matches Grok (1000ms).
#[derive(Debug, Clone)]
pub struct PendingConfirm {
    pub shortcut: KeyShortcut,
    pub label: &'static str,
    expires_at: Instant,
    ttl: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingOutcome {
    /// First press: armed.
    Armed,
    /// Same key within TTL: confirmed.
    Confirmed,
    /// Expired or other key: cleared without confirming.
    Cleared,
}

impl PendingConfirm {
    pub const DEFAULT_TTL: Duration = Duration::from_millis(1000);

    pub fn arm(now: Instant, shortcut: KeyShortcut, label: &'static str) -> Self {
        Self::arm_with_ttl(now, shortcut, label, Self::DEFAULT_TTL)
    }

    pub fn arm_with_ttl(
        now: Instant,
        shortcut: KeyShortcut,
        label: &'static str,
        ttl: Duration,
    ) -> Self {
        Self {
            shortcut,
            label,
            expires_at: now + ttl,
            ttl,
        }
    }

    pub fn expired(&self, now: Instant) -> bool {
        now >= self.expires_at
    }

    pub fn hint(&self) -> PendingHint {
        PendingHint {
            shortcut: self.shortcut,
            label: self.label,
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}

/// Feed a key into an optional pending confirm.
pub fn pending_on_key(
    pending: &mut Option<PendingConfirm>,
    now: Instant,
    key: &KeyEvent,
    shortcut: KeyShortcut,
    label: &'static str,
) -> Option<PendingOutcome> {
    match pending {
        Some(p) if p.expired(now) => {
            *pending = None;
            if shortcut.matches(key) {
                *pending = Some(PendingConfirm::arm(now, shortcut, label));
                Some(PendingOutcome::Armed)
            } else {
                Some(PendingOutcome::Cleared)
            }
        }
        Some(p) if p.shortcut.matches(key) => {
            *pending = None;
            Some(PendingOutcome::Confirmed)
        }
        Some(_) => {
            *pending = None;
            Some(PendingOutcome::Cleared)
        }
        None if shortcut.matches(key) => {
            *pending = Some(PendingConfirm::arm(now, shortcut, label));
            Some(PendingOutcome::Armed)
        }
        None => None,
    }
}

pub fn hint_id(i: usize) -> HitId {
    HitId::namespaced(NS_HINT, i as u32)
}

pub fn parse_hint_id(id: HitId) -> Option<usize> {
    (id.namespace() == NS_HINT).then_some(id.index() as usize)
}

/// Bottom hint strip. Hover/click go through `FrameCtx`.
pub struct ShortcutsBar<'a> {
    hints: &'a [HintItem],
    pending_confirmation: Option<PendingHint>,
    right_text: Option<&'a str>,
    compact: Option<CompactConfig>,
}

impl<'a> ShortcutsBar<'a> {
    pub fn new(hints: &'a [HintItem]) -> Self {
        Self {
            hints,
            pending_confirmation: None,
            right_text: None,
            compact: None,
        }
    }

    pub fn compact(mut self, max_visible: usize, help_hint: Option<HintItem>) -> Self {
        self.compact = Some(CompactConfig {
            max_visible,
            help_hint,
        });
        self
    }

    pub fn with_pending(mut self, pending: Option<PendingHint>) -> Self {
        self.pending_confirmation = pending;
        self
    }

    pub fn with_right_text(mut self, text: Option<&'a str>) -> Self {
        self.right_text = text;
        self
    }

    pub fn render(self, area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let bg_style = cx
            .style(ThemeRole::Surface, ThemeState::Normal)
            .patch(cx.style(ThemeRole::Muted, ThemeState::Normal));
        fill(buf, area, bg_style);

        let key_style = cx
            .style(ThemeRole::Surface, ThemeState::Normal)
            .patch(cx.style(ThemeRole::Text, ThemeState::Normal))
            .add_modifier(Modifier::BOLD);
        let action_style = bg_style;

        if let Some(pending) = &self.pending_confirmation {
            cx.request_tick(gbtui_core::TickDemand::Slow);
            let key_text = pending.shortcut.display();
            let label = format!("press again to {}", pending.label);
            let mut x = area.x;
            buf.set_string_safe(x, area.y, &key_text, key_style);
            x = x.saturating_add(key_text.width() as u16);
            buf.set_string_safe(x, area.y, ":", action_style);
            x = x.saturating_add(1);
            buf.set_string_safe(x, area.y, &label, action_style);
            return;
        }

        let sep_style = bg_style.add_modifier(Modifier::DIM);
        let mut x = area.x;
        let effective = compute_effective_hints(self.hints, self.compact.as_ref());

        for (i, hint) in effective.iter().enumerate() {
            if i > 0 {
                let sep = "  │  ";
                let sep_width = sep.width() as u16;
                if x + sep_width > area.right() {
                    break;
                }
                buf.set_string_safe(x, area.y, sep, sep_style);
                x = x.saturating_add(sep_width);
            }

            let key_text = hint.bar_key_display();
            let colon_label = format!(":{}", hint.label);
            let total = key_text.width() as u16 + colon_label.width() as u16;
            if x + total > area.right() {
                break;
            }
            let r = Rect {
                x,
                y: area.y,
                width: total,
                height: 1,
            };
            cx.register(hint_id(i), r);
            let hot_state = ThemeState::from_flags(false, false, false, false, cx.hovered(r));
            let ks = if hot_state == ThemeState::Hovered {
                cx.style(ThemeRole::Surface, ThemeState::Normal)
                    .patch(cx.style(ThemeRole::Hover, hot_state))
                    .patch(cx.style(ThemeRole::Accent, hot_state))
                    .add_modifier(Modifier::BOLD)
            } else {
                key_style
            };
            let as_ = if hot_state == ThemeState::Hovered {
                cx.style(ThemeRole::Surface, ThemeState::Normal)
                    .patch(cx.style(ThemeRole::Hover, hot_state))
                    .patch(cx.style(ThemeRole::Text, hot_state))
            } else {
                action_style
            };
            buf.set_string_safe(x, area.y, &key_text, ks);
            x = x.saturating_add(key_text.width() as u16);
            buf.set_string_safe(x, area.y, &colon_label, as_);
            x = x.saturating_add(colon_label.width() as u16);
        }

        if let Some(text) = self.right_text {
            let display = format!("{text} ");
            let rw = display.width() as u16;
            if rw > 0 && rw < area.width {
                let rx = area.x + area.width.saturating_sub(rw);
                if rx > x + 1 {
                    buf.set_string_safe(rx, area.y, &display, action_style);
                }
            }
        }
    }
}

pub fn compute_effective_hints<'a>(
    hints: &'a [HintItem],
    compact: Option<&'a CompactConfig>,
) -> Vec<&'a HintItem> {
    if let Some(cfg) = compact {
        let pinned_count = hints.iter().filter(|h| h.pinned).count();
        let unpinned_budget = cfg.max_visible.saturating_sub(pinned_count);
        let mut unpinned_used = 0;
        let mut v: Vec<&HintItem> = hints
            .iter()
            .filter(|h| {
                if h.pinned {
                    true
                } else if unpinned_used < unpinned_budget {
                    unpinned_used += 1;
                    true
                } else {
                    false
                }
            })
            .collect();
        if let Some(ref h) = cfg.help_hint {
            v.push(h);
        }
        v
    } else {
        hints.iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbtui_core::{AnimationClock, Theme, key};
    use gbtui_runtime::HitMap;
    use std::time::Instant;

    fn h(label: &'static str, k: KeyShortcut) -> HintItem {
        HintItem::new(k, label)
    }

    #[test]
    fn full_mode_returns_all_hints() {
        let hints = vec![h("a", key!('a')), h("b", key!('b')), h("c", key!('c'))];
        let out = compute_effective_hints(&hints, None);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn compact_takes_first_n_then_appends_help() {
        let hints = vec![h("a", key!('a')), h("b", key!('b')), h("c", key!('c'))];
        let help = h("shortcuts", key!('/', CONTROL));
        let cfg = CompactConfig {
            max_visible: 2,
            help_hint: Some(help),
        };
        let out = compute_effective_hints(&hints, Some(&cfg));
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].label, "a");
        assert_eq!(out[2].label, "shortcuts");
    }

    #[test]
    fn compact_pinned_hints_always_included() {
        let hints = vec![
            h("a", key!('a')),
            h("b", key!('b')),
            h("c", key!('c')),
            h("d", key!('d')).pinned(),
            h("e", key!('e')).pinned(),
        ];
        let cfg = CompactConfig {
            max_visible: 3,
            help_hint: None,
        };
        let out = compute_effective_hints(&hints, Some(&cfg));
        let labels: Vec<_> = out.iter().map(|h| h.label.as_ref()).collect();
        assert!(labels.contains(&"d"));
        assert!(labels.contains(&"e"));
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn pending_double_press() {
        let now = Instant::now();
        let mut p = None;
        let k = key!('q').to_key_event();
        assert_eq!(
            pending_on_key(&mut p, now, &k, key!('q'), "quit"),
            Some(PendingOutcome::Armed)
        );
        assert_eq!(
            pending_on_key(
                &mut p,
                now + Duration::from_millis(100),
                &k,
                key!('q'),
                "quit"
            ),
            Some(PendingOutcome::Confirmed)
        );
    }

    #[test]
    fn pending_expires() {
        let now = Instant::now();
        let mut p = Some(PendingConfirm::arm(now, key!('q'), "quit"));
        let k = key!('q').to_key_event();
        let later = now + PendingConfirm::DEFAULT_TTL + Duration::from_millis(1);
        assert_eq!(
            pending_on_key(&mut p, later, &k, key!('q'), "quit"),
            Some(PendingOutcome::Armed)
        );
    }

    #[test]
    fn hover_registers_hint_hits() {
        let theme = Theme::night();
        let mut hits = HitMap::new();
        let mut anim = AnimationClock::new(Instant::now());
        let mut cx = FrameCtx::new(&theme, Some((0, 0)), &mut anim, &mut hits);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        let hints = [h("overlay", key!('o'))];
        ShortcutsBar::new(&hints).render(Rect::new(0, 0, 40, 1), &mut buf, &mut cx);
        assert_eq!(hits.hit(0, 0), gbtui_runtime::HitResult::Hit(hint_id(0)));
    }
}
