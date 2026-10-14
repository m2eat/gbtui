# Source map (Grok Build → gbtui)

`sources/grok-build/` is read-only. Every lift updates the **Status** column.

Legend: **COPY** almost as-is · **STRIP** copy then delete domain deps · **REWRITE** FrameCtx rewrite · **NEVER** must not enter the framework

Paths under `sources/grok-build/crates/codegen/` unless noted.

## COPY

| Source | Target | Status |
|--------|--------|--------|
| `xai-ratatui-inline/**` | `crates/gbtui-terminal/src/inline/` | **Copied (P1b)** |
| `xai-ratatui-textarea/**` | `crates/gbtui-widgets/src/textarea/` | **Copied (P1a)** |
| `xai-grok-pager-render/src/input/key.rs` | `gbtui-core` | **Copied (P2a)** |
| `xai-grok-pager-render/src/render/safe_buf.rs` | `gbtui-core` (bidi off by default) | **Copied (P2a)** |
| `xai-grok-pager-render/src/render/renderable.rs` | `gbtui-core` | **Copied (P2a)** |
| `xai-grok-pager-render/src/render/wrapping.rs` | `gbtui-core` | **Stripped (P2a)** |
| `xai-grok-pager-render/src/render/line_utils.rs` | `gbtui-core` / widgets | **Stripped (P2a)** |
| `xai-grok-pager/src/views/overlay.rs` | `gbtui-widgets` overlay | **Copied (P4b)** — `FocusScrollback`/`FocusPrompt` → `Unfocus`/`UnfocusAlt` |
| `xai-grok-pager-render/src/input/line_editor.rs` | `gbtui-widgets` line_editor | **Copied (P5)** |

## STRIP

| Source | How | Status |
|--------|-----|--------|
| `xai-grok-pager-render/src/theme/**` | Keep quantize, system appearance, OSC11, two de-domained themes | **Stripped (P2a/P2b/P13/P14)** — env → OSC11 → OS appearance; built-in color-depth adaptation and ANSI16 contrast, explicit palettes host-owned |
| `xai-grok-pager-render/src/input/keyboard_normalizer.rs` | Drop AppView binding | **Stripped (P2c)** |
| `xai-grok-pager-render/src/input/macos_modifiers.rs` | macOS probe | **Copied (P2c)** |
| `xai-grok-pager-render/src/input/mouse.rs` | Rename `GROK_SCROLL_*` → framework config | **Stripped (P2c/P13)** — speed acceleration, finite coasting, viewport cap and independent deadlines; no HUD log |
| `xai-grok-pager-render/src/input/terminal_support.rs` | `is_mod_enter` | **Stripped (P5)** → `gbtui-input` enter.rs |
| `xai-grok-pager-render/src/terminal/**` | Probe + Kitty + tmux; independent optional graphics | **Stripped (P2b/P13)** — bounded OSC11/DA2/XTVERSION, persistent Unix input decoder and safe Alacritty gate; `images` remains optional |
| `xai-grok-pager-render/src/terminal/image.rs` | Strip global overlay IDs, FS/sips conversion and product state | **Stripped** — immutable prepared PNG, retained Kitty scene, explicit iTerm2 encoder, Image widget fallback |
| `xai-grok-pager-render/src/glyphs.rs` | Drop voice record_dot; keep chrome glyphs | **Stripped (P2b)** — spinner/accent frames in P5 |
| `xai-grok-pager/src/views/modal_window.rs` | `ModalKind::{Popup, Embedded}` instead of global AtomicBool | **Stripped (P4a)** |
| `xai-grok-pager-render/src/modal_window_state.rs` | With modal | **Stripped (P4a)** — folded into `ModalState` |
| `xai-grok-pager/src/views/list_pane/**` | Generic list; Theme-derived style | **Stripped (P4c/P13/P14)** — shared physical-row engine, stable-ID range selection/extraction, cached fuzzy/regex/substring matching and highlighting |
| `xai-grok-pager/src/views/picker.rs` | Drop session/palette copy | **Stripped (P4d/P13)** — rich labels, metadata, multiline descriptions, expanded fields, nested collapsible groups and Filter/Search modes |
| `xai-grok-pager/src/views/progress_bar.rs` | 1/8-cell + ConHost shades | **Stripped (P5b)** |
| `xai-grok-pager/src/views/shortcuts_bar.rs` | Keep PendingConfirm API, drop product copy | **Stripped (P5c)** |
| `xai-grok-pager/src/views/prompt_widget/**` | Replace domain parsing and loading with host-driven addons | **Stripped (P5a/P13/P14)** — snapshot-validated completion, visible fuzzy history with draft restoration, atomic paste, attachment metadata and live submissions |
| `xai-grok-pager/src/actions/{mod,defaults}.rs` | Learn structure only | **Rewritten (P6)** — generic `ActionDef<Id, Ctx>`; do not copy `ActionId` |
| `xai-grok-pager-render/src/theme/color_support.rs` | Quantize | **Stripped (P2a)** |
| `xai-grok-pager-render/src/theme/system_appearance.rs` | dark/light | **Stripped (P2b)** |
| `xai-grok-pager-render/src/terminal/hyperlinks.rs` | Preserve per-terminal policy and safe URL schemes | **Stripped (P5d)** — FrameCtx → Terminal OSC 8 diff; Link and wrapped Markdown register native spans |
| `xai-grok-pager/src/views/status_bar.rs` | Unicode width; Theme gray/bg_base; no hits | **Stripped (P9a)** |
| `xai-grok-pager/src/views/completion_dropdown.rs` + `overlay_list.rs` | Geometry only; no slash/@/FS | **Stripped (P9e/P14)** — bounded label/detail columns, highlighted matches, configurable row cap/scroll policy and captured scrollbar |
| `xai-grok-pager/src/views/shortcuts_help.rs` | Structure; caller supplies Header/Hint | **Stripped (P9f)** — no grok `ActionId`/`Category` |
| `xai-grok-pager-render/src/clipboard/**` | arboard + OSC 52; no GROK_COPY_FILE / grok wrap / remote probe | **Stripped (P9g)** — feature `clipboard` |
| `xai-grok-pager/src/views/settings_modal` | Generic host-owned row kinds/actions | **Stripped (P10b/P14)** — errors, expanded multi-select, physical detail scrolling and pinned actions; no validation engine or `SettingKey` |
| `xai-grok-pager-render/src/render/preview_overlay.rs` | Theme colors; occlude+BODY | **Stripped (P10c)** |
| `xai-grok-pager/src/views/context_bar.rs` | Compact + hover bar | **Stripped (P10e)** — no `accent_user` |
| `xai-grok-pager-render/src/search/` | Keep core matcher light; richer search belongs in widgets | **Stripped (P10f/P13)** — smart-case substring in core; `SearchQuery` fuzzy/regex/substring in widgets |

## REWRITE

| Source | Why | Status |
|--------|-----|--------|
| `xai-grok-pager/src/app/agent_view/mod.rs` `HitArea` | Become HitMap | **Rewritten (P3)** |
| `xai-grok-pager/src/app/mouse.rs` | App-level match; the thing the framework removes | **Rewritten (P3/P14)** — HitMap routing, synchronous press modifiers, cancellation on focus/lost release and no synthetic activation |
| `xai-grok-pager/src/app/event_loop.rs` | Loop skeleton only, no ACP | **Rewritten (P3)** — `run` / `run_inline`; synchronized link/image output, RAII terminal/image cleanup, detached closed channels |
| `xai-grok-pager/src/app/app_view.rs` `tick` / `needs_animation` | Scheduler idea, not 200 lines of product | **Rewritten (P3)** |
| `xai-grok-pager/src/views/question_view.rs` | `ChoiceList`, no ACP | **Rewritten (P4e/P14)** — stable IDs, rich focus details, explicit freeform edit/commit/cancel |
| Button / Link / Chip | Do not exist as types in Grok | **Button rewritten (P3)**; **Link rewritten (P5d)**; **Chip rewritten (P9d)** |
| `xai-grok-pager/src/views/welcome/toast.rs` | Layout only; no welcome flow | **Rewritten (P9b)** — `Toast`, `accent`/`bg_elevated` |
| Spinner / WaveBar | Product-tied in Grok | **Rewritten (P5b)** — Theme + `request_tick` |
| `xai-grok-pager/src/views/prompt_suggestion.rs` | Ghost remainder only | **Rewritten (P10a)** — no ACP / env |
| `xai-grok-pager/src/views/announcements.rs` | Banner layout | **Rewritten (P10d)** — no announcements crate |
| `xai-grok-pager/src/views/todo_pane.rs` | Status icons | **Rewritten (P10g)** — no `TodoItem` |
| `xai-grok-pager/src/views/turn_status.rs` | Activity row layout | **Rewritten (P10h)** — no ACP |
| (no product state copied) | Data-only viewer + optional format adapters | **Rewritten (P14)** — `gbtui-widgets::ContentViewer` uses List; `gbtui-agent::{MarkdownDocument, CodeDocument, CodeView}` reuse Markdown/Syntect and source maps |
| (framework-owned theme policy) | `gbtui-core::theme_system` + runtime + widget/rich adapters | **Rewritten (P15)** — named definitions, terminal-default backgrounds, semantic states, strict opt-in TOML, effective appearance, hot switching/versioned caches and contrast estimates |

## NEVER

Do not copy these into gbtui crates:

- `xai-grok-shell/**`
- `xai-grok-pager/src/app/dispatch/**`
- `xai-grok-pager/src/scrollback/blocks/**` 的 ACP/`BlockContent` 类型（绘制可 STRIP 进 `gbtui-agent`）
- `xai-grok-pager/src/slash/**` 的 registry/tags/@ FS（下拉绘制可 STRIP 进 `gbtui-agent`）
- `xai-grok-pager/src/acp/**`
- `xai-grok-mermaid/**`
- `xai-grok-gboom/**`
- `xai-grok-voice/**`
- `xai-grok-telemetry/**`
- `xai-grok-pager/src/views/privacy_banner.rs`
- `xai-grok-pager/src/views/credit_bar.rs`
- `xai-grok-pager/src/views/welcome/**` (animation may be *referenced*, the welcome flow must not be copied)
- Theme fields `accent_skill` / `accent_remember` / `accent_verify` / `accent_plan` / `accent_user` / `accent_assistant` / `accent_tool`
- `ActionId` product enum

## P11 layout references (STRIP paint into `gbtui-agent`, do not COPY types)

These grok **types** stay on the NEVER list. Visual chrome (user band, Thought, Read/Edit/`$`, slash rows, permission overlay) is STRIP'd 1:1 into `gbtui-agent`:

| Source | Target | Status |
|--------|--------|--------|
| `xai-grok-pager/src/scrollback/blocks/{user,agent,system,thinking,session_event}.rs` | `gbtui-agent` Transcript | **STRIP paint (P11b/P13)** — cached Markdown, physical scrolling, full-text search with hidden-hit reveal, turn/response navigation, three-level folding and visible-body copy; no `BlockContent` / diagram renderer |
| `xai-grok-pager/src/scrollback/blocks/tool/**` | `gbtui-agent` ToolRow | **STRIP paint (P11c/P13)** — generic read/edit/run/search/web/memory/message/hook chrome; file/language-aware diff syntax, dual gutters and source ranges shared via owned `ToolRowState`; no execution |
| `xai-grok-pager/src/views/permission_view.rs` | `gbtui-agent` PermissionCard | **STRIP layout (P11d)** — title/body/Allow/Deny; no ACP/MCP/bash pattern |
| `xai-grok-pager/src/views/slash_dropdown.rs` | `gbtui-agent` SlashPalette | **STRIP layout (P11e/P13)** — names, tags, argument hints and multiline descriptions through the shared rich Picker engine; no registry or dispatch |
| (no grok type) | `gbtui-agent` MentionChip | **Rewritten (P11f)** — Chip-shaped; host labels; no FS |

## Rich Markdown extraction

| Source | Target | Status |
|--------|--------|--------|
| `xai-grok-markdown-core/src/lib.rs` | `gbtui-markdown-core` | **Stripped** — shared parser configuration and headless structure analysis |
| `xai-grok-markdown/src/**` (excluding diagram renderer) | `gbtui-markdown` | **Stripped** — parser/render/checkpoint/streaming, tables/copy, links, LaTeX and syntax highlighting; no playground/product dependencies |
| `xai-grok-markdown/assets/tokyo-night.tmTheme` | `gbtui-markdown/assets/` | **Copied** — attribution in `crates/gbtui-markdown/NOTICE` |

The extracted renderer adds usable monochrome defaults and explicit code-line
metadata, including unfinished fences. The base `gbtui` dependency graph excludes
both Markdown crates.

## Notices already placed (P0)

| Source NOTICE | Target |
|---------------|--------|
| `xai-ratatui-inline/NOTICE` | `crates/gbtui-terminal/NOTICE` (rewritten names) |
| `xai-ratatui-textarea/NOTICE` | `crates/gbtui-widgets/NOTICE` (rewritten names) |
