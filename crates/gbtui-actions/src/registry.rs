//! Generic action registry: keymap + shortcut hints + command-palette list.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/actions/ (structure only)`
//!
//! Applications own `Id` and `Ctx`. This crate never defines product
//! actions (`SendPrompt`, …). Input bubbling is **explicit at the call
//! site**: each layer does an exact [`ActionRegistry::lookup`], then the
//! caller decides whether to try the next context.

use crossterm::event::KeyEvent;
use gbtui_core::KeyShortcut;

/// One bindable action. `Id` and `Ctx` are application types.
#[derive(Debug, Clone)]
pub struct ActionDef<Id, Ctx> {
    pub id: Id,
    /// Short label for the shortcuts bar (`"quit"`, `"overlay"`).
    pub label: &'static str,
    /// Longer text for a command palette / cheatsheet.
    pub description: &'static str,
    pub default_key: KeyShortcut,
    pub alt_keys: Vec<KeyShortcut>,
    pub context: Ctx,
    /// `None` hides the action from [`ActionRegistry::hints`]. `Some(0)` is first.
    pub hint_priority: Option<u8>,
    /// Optional override for the bar's key text (e.g. `"j/k"`).
    pub hint_key_display: Option<&'static str>,
    /// Caller should arm a double-press confirm (see `PendingConfirm`).
    pub requires_confirmation: bool,
    /// Survive compact-mode truncation on the shortcuts bar.
    pub pinned: bool,
}

impl<Id, Ctx> ActionDef<Id, Ctx> {
    pub fn new(id: Id, label: &'static str, default_key: KeyShortcut, context: Ctx) -> Self {
        Self {
            id,
            label,
            description: label,
            default_key,
            alt_keys: Vec::new(),
            context,
            hint_priority: None,
            hint_key_display: None,
            requires_confirmation: false,
            pinned: false,
        }
    }

    pub fn describe(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    pub fn alts(mut self, keys: impl Into<Vec<KeyShortcut>>) -> Self {
        self.alt_keys = keys.into();
        self
    }

    pub fn hint(mut self, priority: u8) -> Self {
        self.hint_priority = Some(priority);
        self
    }

    pub fn custom_display(mut self, display: &'static str) -> Self {
        self.hint_key_display = Some(display);
        self
    }

    pub fn confirm(mut self) -> Self {
        self.requires_confirmation = true;
        self
    }

    pub fn pinned(mut self) -> Self {
        self.pinned = true;
        self
    }

    pub fn matches_key(&self, event: &KeyEvent) -> bool {
        self.default_key.matches(event) || self.alt_keys.iter().any(|k| k.matches(event))
    }

    pub fn keys(&self) -> impl Iterator<Item = KeyShortcut> + '_ {
        std::iter::once(self.default_key).chain(self.alt_keys.iter().copied())
    }
}

/// Returns `true` when `key` should not be advertised as the primary hint
/// (the terminal often drops it). The registry then prefers the first alt.
pub type UnreliableKey = fn(KeyShortcut) -> bool;

/// Single source of truth for key dispatch, shortcut hints, and palettes.
pub struct ActionRegistry<Id, Ctx> {
    actions: Vec<ActionDef<Id, Ctx>>,
    unreliable: Option<UnreliableKey>,
}

impl<Id, Ctx> ActionRegistry<Id, Ctx> {
    pub fn new(actions: Vec<ActionDef<Id, Ctx>>) -> Self {
        Self {
            actions,
            unreliable: None,
        }
    }

    pub fn unreliable_keys(mut self, hook: UnreliableKey) -> Self {
        self.unreliable = Some(hook);
        self
    }

    pub fn all(&self) -> &[ActionDef<Id, Ctx>] {
        &self.actions
    }

    pub fn advertised_key(&self, def: &ActionDef<Id, Ctx>) -> KeyShortcut {
        if self.unreliable.is_some_and(|f| f(def.default_key)) {
            def.alt_keys.first().copied().unwrap_or(def.default_key)
        } else {
            def.default_key
        }
    }
}

impl<Id, Ctx> ActionRegistry<Id, Ctx>
where
    Id: Copy + PartialEq,
    Ctx: PartialEq,
{
    /// Exact context match. The caller bubbles by calling this again with
    /// the next layer (or [`Self::lookup_in`] with an explicit order).
    pub fn lookup(&self, event: &KeyEvent, context: Ctx) -> Option<Id> {
        for def in &self.actions {
            if def.context != context {
                continue;
            }
            if def.matches_key(event) {
                return Some(def.id);
            }
        }
        None
    }

    /// Try `contexts` in the given order. Order **is** the bubble path.
    pub fn lookup_in(
        &self,
        event: &KeyEvent,
        contexts: impl IntoIterator<Item = Ctx>,
    ) -> Option<Id> {
        for context in contexts {
            if let Some(id) = self.lookup(event, context) {
                return Some(id);
            }
        }
        None
    }

    pub fn find(&self, id: Id) -> Option<&ActionDef<Id, Ctx>> {
        self.actions.iter().find(|d| d.id == id)
    }

    pub fn matches_id(&self, id: Id, event: &KeyEvent) -> bool {
        self.find(id).is_some_and(|def| def.matches_key(event))
    }

    /// Hints for the shortcuts bar: `hint_priority` is set and `context` is
    /// in `contexts`, sorted by priority.
    pub fn hints(&self, contexts: &[Ctx]) -> Vec<&ActionDef<Id, Ctx>> {
        let mut hints: Vec<&ActionDef<Id, Ctx>> = self
            .actions
            .iter()
            .filter(|def| def.hint_priority.is_some() && contexts.contains(&def.context))
            .collect();
        hints.sort_by_key(|def| def.hint_priority.unwrap_or(u8::MAX));
        hints
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use gbtui_core::key;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum DemoAction {
        Quit,
        Confirm,
        Overlay,
        Help,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum DemoCtx {
        Home,
        Overlay,
        Always,
    }

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn registry() -> ActionRegistry<DemoAction, DemoCtx> {
        ActionRegistry::new(vec![
            ActionDef::new(DemoAction::Overlay, "overlay", key!('o'), DemoCtx::Home)
                .alts(vec![key!('o', CONTROL)])
                .hint(0)
                .pinned(),
            ActionDef::new(DemoAction::Confirm, "ok", key!(Enter), DemoCtx::Overlay).hint(1),
            ActionDef::new(DemoAction::Quit, "quit", key!('q'), DemoCtx::Always)
                .hint(10)
                .confirm(),
            ActionDef::new(DemoAction::Help, "help", key!('.', CONTROL), DemoCtx::Home)
                .alts(vec![key!('?')])
                .hint(5),
        ])
    }

    #[test]
    fn lookup_is_exact_context() {
        let r = registry();
        assert_eq!(
            r.lookup(&ev(KeyCode::Char('o'), KeyModifiers::NONE), DemoCtx::Home),
            Some(DemoAction::Overlay)
        );
        assert_eq!(
            r.lookup(
                &ev(KeyCode::Char('o'), KeyModifiers::NONE),
                DemoCtx::Overlay
            ),
            None
        );
        assert_eq!(
            r.lookup(&ev(KeyCode::Enter, KeyModifiers::NONE), DemoCtx::Home),
            None
        );
        assert_eq!(
            r.lookup(&ev(KeyCode::Enter, KeyModifiers::NONE), DemoCtx::Overlay),
            Some(DemoAction::Confirm)
        );
    }

    #[test]
    fn alt_key_matches() {
        let r = registry();
        assert_eq!(
            r.lookup(
                &ev(KeyCode::Char('o'), KeyModifiers::CONTROL),
                DemoCtx::Home
            ),
            Some(DemoAction::Overlay)
        );
    }

    #[test]
    fn bubbling_is_explicit_order() {
        let r = registry();
        let enter = ev(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(r.lookup_in(&enter, [DemoCtx::Home, DemoCtx::Always]), None);
        assert_eq!(
            r.lookup_in(&enter, [DemoCtx::Overlay, DemoCtx::Home, DemoCtx::Always]),
            Some(DemoAction::Confirm)
        );
        let q = ev(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(
            r.lookup_in(&q, [DemoCtx::Overlay, DemoCtx::Home, DemoCtx::Always]),
            Some(DemoAction::Quit)
        );
    }

    #[test]
    fn hints_filter_and_sort() {
        let r = registry();
        let home = r.hints(&[DemoCtx::Home]);
        assert_eq!(home[0].id, DemoAction::Overlay);
        assert_eq!(home[1].id, DemoAction::Help);
        assert!(!home.iter().any(|d| d.id == DemoAction::Quit));
        let stacked = r.hints(&[DemoCtx::Home, DemoCtx::Always]);
        assert_eq!(stacked.last().map(|d| d.id), Some(DemoAction::Quit));
        assert!(stacked.iter().any(|d| d.pinned));
        assert!(r.find(DemoAction::Quit).unwrap().requires_confirmation);
    }

    #[test]
    fn all_lists_palette_entries() {
        let r = registry();
        assert_eq!(r.all().len(), 4);
    }

    fn ctrl_dot_is_unreliable(key: KeyShortcut) -> bool {
        key.code == KeyCode::Char('.') && key.modifiers.contains(KeyModifiers::CONTROL)
    }

    #[test]
    fn unreliable_hook_prefers_alt_for_hints() {
        let r = registry().unreliable_keys(ctrl_dot_is_unreliable);
        let help = r.find(DemoAction::Help).unwrap();
        assert_eq!(r.advertised_key(help), key!('?'));
        let overlay = r.find(DemoAction::Overlay).unwrap();
        assert_eq!(r.advertised_key(overlay), key!('o'));
    }
}
