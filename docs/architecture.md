# Architecture

gbtui is a **ratatui application runtime**, not a retained widget tree and not a fork of ratatui as a whole.

```text
application (examples / your crate)
        │
        ├─ gbtui-agent            optional session chrome (P11)
        │       │
        │       ▼
        └─ gbtui  (facade)
                │
                ├─ gbtui-actions          optional ActionRegistry<Id, Ctx>
                │
                ▼
            gbtui-widgets
                │
                ▼
            gbtui-runtime                 run(), FrameCtx, HitMap, tick scheduler
                │
                ├─ gbtui-input            KeyboardNormalizer, scroll stream
                └─ gbtui-terminal         probe, Kitty, inline Terminal fork
                        │
                        ▼
                    gbtui-core            Theme, KeyShortcut, TickDemand, Renderable
```

`gbtui` does not depend on `gbtui-agent` or either Markdown crate. The optional
agent layer depends on `gbtui-markdown`, which depends on the headless
`gbtui-markdown-core`. Non-chat applications can use either Markdown crate directly.

## Frame

1. crossterm event → KeyboardNormalizer / MouseScrollState
2. pointer hits `HitMap` in reverse registration order (later = on top)
3. `occlude(rect)` swallows hits underneath (modals / dropdowns)
4. keyboard is delivered as `AppEvent::Key` (focus is the app's job)
5. `App::draw` paints with `FrameCtx`; hover is **same-frame** (current mouse vs area)
6. clicks use the **previous** frame’s HitMap
7. widgets `request_tick(Fast|Slow)`; idle is `TickDemand::None` (no 30fps spin)
8. Drive tests with `Pump` + `TestBackend`; live TTY with `run` / `run_blocking` (alt-screen) or `run_inline` / `run_inline_blocking` (history → native scrollback, RIS on resize)

The facade re-exports the matching `ratatui` and `crossterm` versions. Use
`gbtui::ratatui::Frame` in `App::draw`; `key!` resolves its event types through
the defining crate, including when the application renames its `gbtui` dependency.

`run` and `run_inline` own a terminal session guard. They enable bracketed paste,
focus reporting, mouse capture, and the Kitty flags selected by terminal policy.
The guard unwinds acquired modes after setup errors, normal exit, future cancellation,
or a Rust panic that unwinds. Inline viewports clear before protocol restoration.
This cannot recover from process abort/SIGKILL or guarantee cleanup when terminal I/O fails.
A drained, closed message receiver is detached; the input loop remains usable without
polling the closed channel continuously.

Unix capability probing and live input share one incremental decoder. The reader
reopens the stdin TTY with an independent nonblocking descriptor, so output remains
blocking even when inherited stdin/stdout share an open-file description. Requested
OSC11/DA2/XTVERSION replies are removed across read boundaries; bracketed paste
payloads bypass protocol interpretation. Inline DSR uses this same reader, then
`Terminal::with_options_at_cursor` avoids competing backend cursor reads on resize.
Unprobed/fallback paths retain crossterm's event stream.

The DSR filter stops waiting as soon as a numeric CSI is not a cursor reply, and
disarms after its single reply. Kitty functional keys (`CSI … A`, `~`, etc.)
decode press/repeat/release directly, like CSI-u; unsupported legacy-parser
buffering must not consume subsequent ordinary text.

The run functions accept an explicit `Theme`, a `ThemeKind`, or a `ThemeDefinition`.
Only `Auto` resolves environment override → probed background → OS appearance →
Night. `AppEvent::TerminalInfo` delivers initial facts and later changed probe
replies; it contains no user-input bytes. Explicit raw palettes stay host-owned.

Definitions resolve once per configuration/context change into a `ResolvedTheme`.
Built-in/custom definition palettes and style overrides adapt to terminal color
depth; ANSI16 keeps chrome contrast and `None` resets color channels. Background
policy is applied after palette adaptation, so ANSI16 never reinstates an opaque
canvas over a terminal-default request.

`Pointer::Down` carries the synchronous mouse modifiers, not the previous frame's
modifier snapshot. A captured target receives `Pointer::Cancel` on focus loss,
an inconsistent new press, or ordinary motion revealing a lost button release.
Cancellation is not activation: end drag/autoscroll/hover and pending clicks without
copying or submitting. Prompt and Transcript preserve valid selection text.

## Theme resolution

`ThemeDefinition` owns a named palette source plus sparse color/style overrides.
`ThemeResolver` owns the current definition, terminal context and precomputed
role/state style table. `Theme` remains a cheap Copy palette for custom rendering;
`FrameCtx::style` is the semantic seam used by framework widgets.

`BackgroundPolicy::Solid` keeps palette backgrounds. `TerminalDefault` resets
base/dark/input/code/scrollbar-track surfaces. `Transparent` also resets ordinary
elevated and button surfaces. Hover/selection retain contrast; explicit style
overrides can set opaque backgrounds. Reset means terminal default, not alpha;
the emulator owns window opacity. Transparent overlays still clear stale symbols.

Resolution order is palette → color capability → background policy → semantic
defaults → Normal role override → current-state override. Component-local styles
patch last. `ThemeState::from_flags` chooses Disabled > Invalid > Selected >
Focused > Hovered > Normal. Monochrome states use non-color feedback.

`ResolvedTheme::context` retains terminal facts; `appearance` is the effective
theme appearance before transparency/quantization. Explicit Day/Night wins over
terminal appearance, including syntax highlighting. Reset alone is not evidence
of a dark terminal. Auto without appearance evidence falls back to Night.

`App::take_theme_request` is drained after events and before painting, even when
the app returns `Outcome::Unchanged`. `Pump::set_theme_definition` and
`update_theme_context` provide the same seam to custom hosts. Late terminal
updates re-resolve the active definition rather than restoring the startup one.
There is no OS-theme watcher or background polling loop; hosts may supply context
updates, and live input processes validated terminal replies.

Versions advance when resolved output/metadata changes; repeated equivalent
requests do not invalidate caches. Generations are resolver-scoped: use full
resolved equality or output stamps across owners. Rendering borrows the resolved
theme and performs no definition clone or allocation per style lookup.

`ThemeRegistry` includes night/day/auto and accepts host-named definitions.
The opt-in `theme-config` feature parses strict TOML with `name`, `kind`,
`background`, `[colors]`, and `[styles.<role>.<state>]`; core does no file IO.
`ResolvedTheme::contrast` estimates WCAG ratios and returns `None` for unknown
terminal-default RGB. ANSI palette estimates cannot certify emulator contrast.

## FrameCtx

`FrameCtx` is the only mouse/hover/animation handle widgets may use.

```text
FrameCtx
  theme     &'a Theme          // resolved palette; prefer style(role, state) for chrome
  mouse     Option<(col,row)>  // current pointer; hover is same-frame
  modifiers KeyModifiers       // last key or mouse modifiers (LinkActivation::Modifier)
  anim      AnimationClock     // request_tick / phase / wave
  hits      &mut HitMap        // register / occlude
  cursor    Option<Position>   // hardware caret; last set_cursor wins
  links     Vec<LinkSpan>      // per-frame native hyperlink output
  images    Vec<ImagePlacement> // optional `images` feature
```

- `cx.hovered(area)` — true if the current mouse is inside `area` (this draw)
- `cx.register(id, area)` — later registrations sit on top
- `cx.register_scroll(id, area)` — stable scroll-only target; register after the plane's row hits and before later overlays. It does not change click or hover hits.
- `cx.occlude(area)` — swallows hits, trims native links, and removes intersecting image placements below this rect; register overlay targets **after**
- `cx.request_tick(TickDemand::Fast | Slow)` — raise this frame’s animation demand
- `cx.set_cursor(x, y)` — place the blinking terminal caret; Pump calls `Frame::set_cursor_position` after `App::draw`. If nobody sets it, the terminal **hides** the cursor. Ghost text is painted *at* the caret (the caret sits on the first remainder cell).
- `cx.register_link(area, Arc<str>, id)` — register clipped native hyperlink cells; capability/scheme/control-character checks are centralized here

Clicks resolve against the **previous** paint’s HitMap (the press happens between draws). Hover does not.

`resolved_theme()` exposes the borrowed optional resolved theme; `theme_version()`
supports application caches. Standalone `FrameCtx::new` falls back to raw palette
semantic styles. `StatusBar::render` and `render_progress` now take `&FrameCtx`
rather than `&Theme`, so role overrides reach these otherwise noninteractive widgets.

## TickDemand

```text
None  <  Slow (~12 fps)  <  Fast (default 30 fps)
```

The loop sleeps until input, resize debounce (16ms), an animation deadline, or a pending scroll/isolated-Escape deadline. Scroll acceleration uses bounded inter-event timing, caps each flush by viewport size, and coasts for a finite interval. Reversal, target disappearance, resize or occlusion cancels the stream. Once it settles, an idle UI at `TickDemand::None` has no recurring wake.

`AnimationClock::phase(period)` is wall-clock, not frame-counted. Spinners request `Fast` only while running.

## z-order and occlusion

- HitMap is a stack: **later = on top**.
- A modal Popup `occlude`s the full frame, then registers `BODY` so inside clicks are not treated as outside-close (`Miss`).
- A dropdown must occlude (or skip registering) chrome underneath. `Dropdown` occludes its own rect, then registers `BODY` and rows.
- Occluded and miss both arrive as `AppEvent::Miss` in the pump. Overlay chrome that should keep clicks must register its own hit.

Tests that must stay green: later-wins, occlude-under-modal, dropdown click-through, same-frame hover after Move+paint.

## Hard rules

- No `xai-grok-*` in the crate graph
- `gbtui` must not depend on `gbtui-agent` (optional stack-on-top crate)
- Theme core fields are a whitelist (no Grok domain colors)
- Terminal special cases belong in `gbtui-terminal` / `gbtui-input`, never in a widget
- `sources/grok-build/` is reference-only
- Applications must not `match MouseEventKind`

See `docs/plan.md` for the full API sketch and phase gates.

## Actions (P6)

`gbtui-actions` is optional and depends only on `gbtui-core`. Applications define `Id` and `Ctx`; the crate never ships `SendPrompt`-style enums.

- `ActionRegistry::lookup(event, ctx)` is an **exact** context match
- bubbling is written at the call site (`lookup_in` takes an ordered list)
- `hints(contexts)` / `all()` feed the shortcuts bar and a command palette
- `unreliable_keys` swaps the advertised primary when the terminal drops it (default hook: Ctrl+.)
- facade `hint_item` / `hint_items` convert defs to `HintItem` so actions stay widget-free

## Runtime primitives (P2)

- `gbtui_core::{Theme, KeyShortcut, TickDemand, AnimationClock, SafeBuf, Renderable}`
- `gbtui_terminal::{TerminalName, detect_terminal_brand_from_env, keyboard_capabilities, negotiated_kitty_flags}`
- `gbtui_input::{KeyboardNormalizer, MouseScrollState, filter_startup_typeahead}`
- Theme field whitelist only. Color force: `GBTUI_FORCE_COLOR_LEVEL`. Appearance force: `GBTUI_APPEARANCE`.
- Idle animation stays `TickDemand::None`.

## Widgets (P3–P5)

Hover/click always go through `FrameCtx`.

- **Button** — hover uses `theme.button_hover_bg`
- **Modal** — `ModalKind::{Popup, Embedded}`; Popup occludes the full frame then registers `BODY`
- **OverlayState** — Hidden / Visible+Focused / Visible+Unfocused; `Unfocus` / `UnfocusAlt`
- **ListPane** — borrowed labels or stable-ID `ListItem`/`Renderable` rows, physical layout cache and a shared `SearchQuery` projection
- **Picker** — `PickerEntry::group(PickerGroup)` / `PickerEntry::row(PickerRow)`, `LineEditor` query, rich metadata, multiline `Text` descriptions, expansion and nested groups
- **ChoiceList** — single / multi / optional free-form row
- **Prompt** — TextArea, Enter routing, snapshot-validated completions, host-provided history, atomic paste and attachment metadata; no slash parser or resource loading
- **LineEditor** — single-line `EditBuffer`
- **Progress** — 1/8-cell blocks, ConHost shades
- **Spinner / WaveBar** — braille cycle / traveling wave; `cx.request_tick(Fast)` only while running/painted
- **ShortcutsBar** — `HintItem`, compact+pinned, `PendingConfirm` (1000ms)
- **Link** — hover underline + click; `LinkActivation::{Always, Modifier}`

## Lifted pieces (P1)

- **TextArea** — `gbtui_widgets::{TextArea, TextAreaState, EditBuffer, …}`. `apply_theme` / `apply_resolved_theme` provide semantic defaults; public Style fields remain component-local patches. Prompt applies the frame's resolved theme. Hover is element-local (chips).
- **Inline viewport** — `gbtui_terminal::{Terminal, emit_to_scrollback, …}`. Forked ratatui Terminal. Notes: [`inline-viewport.md`](inline-viewport.md). `run_inline` is the app-level loop.

## Transcript scrolling

`gbtui_agent::TranscriptState::offset` is a **physical row offset**, not an item index.
`TranscriptState` owns the rich-text cache and is no longer `Copy`. Construct it
with `TranscriptState::new()` or `default()`. Before pre-draw measurement, call
`state.prepare(items, width, theme)`; either render function also prepares the cache.
Then use `state.transcript_height(items, width)` for total physical rows, including
inter-item gaps, and pass that count and the viewport's `u16` height to
`handle_key(key, content_rows, viewport_rows)` or
`handle_scroll(delta_lines, content_rows, viewport_rows)`.

Home/End, arrow keys, paging and wheel navigation can enter the middle of a single
long item. Follow-tail pins the final physical row across streaming updates and width
changes. Rendering clips item content and hits to the visible slice, including expanded
tool output; it does not allocate a content-height terminal buffer. Assistant and
expanded Thinking bodies use the extracted Markdown renderer, not markdown-lite.
Unchanged source/width/style reuses the prepared layout. Appended text feeds the
streaming renderer incrementally; edits, style changes and width changes invalidate
the appropriate renderer/layout state. Measuring, painting and selection all use
that same output.

Search uses full prepared text, including collapsed/truncated Thinking and Tool
details. `set_search_query`, `next_search_match` and `previous_search_match` retain
matches across folds; navigation expands hidden targets and scrolls to the actual
matching physical row. `navigate` jumps between user turns or assistant responses.
`TranscriptDisplay::{Collapsed, Truncated(rows), Expanded}` applies per item or
to groups via `TranscriptDisplayTarget`. Selection/copy remains visible-only.
Call `invalidate_search()` after same-length, in-place source replacement.

## Wrapped lists

`ListPaneState::with_wrap_mode(WrapMode::Wrap)` uses physical-row offsets and a
`ListLayoutCache` with heights and prefix sums. Plain one-line menus can retain
`NoWrap`; rich Picker entries supply their own variable-height `Renderable`.

`render_list` / `render_list_ns` and `render_list_items` / `render_list_items_ns`
share the same engine. Arrow keys select; wheel/page movement traverses physical
rows without snapping back to the selected row. Continuation hits keep the source
identity (string index or rich stable ID). `SearchQuery` supports smart-case
substring, fuzzy and regex, with visible regex errors and Unicode-safe highlights.
Filter mode removes nonmatches and can fuzzy-sort; Search mode preserves order.

Picker state owns query/filter preparation. Confirm with `selected_id(&items)`,
which returns `None` for empty results; never fall back to source index zero.
Pass the full Picker area to its hit/scroll methods. Row expansion is keyed by
stable ID; `.always_expanded()` is for persistent descriptions such as Slash rows.

The wrapped layout invalidates on width, filter, item count or revision changes.
After changing or reordering same-length label data, call `invalidate_layout()` or
`set_layout_revision(new_revision)` before rendering. Revision values are host-owned
change tokens, not content hashes. Unchanged renders reuse measured heights and the
final scrollbar geometry. The fixed-row filtered path still recomputes matches so
existing callers do not need revision bookkeeping. `studio` demonstrates the wrapped
path with long issue labels, filtering, scrollbar dragging and resize.

Range selection is opt-in through `set_visual_selection_enabled(true)`. `v` starts
or ends a visual range, Shift+Up/Down extends it, and Esc clears it. Endpoints use
stable item IDs across reordering; filtering or removing an endpoint clears the
range. Supply `ListItem::with_copy_text` and use `selected_text(&items)` for host-owned
extraction without visual prefixes. Pointer hosts use `select_item_with_modifier`;
the ContentViewer already maps captured drag coordinates back to physical rows.

## Transcript text selection

Keep a `TranscriptSelection` beside the host's `TranscriptState`, then draw with
`render_transcript_with_selection(state, selection, items, area, buf, cx)`.
Forward left-button `Pointer::Down`, `Drag`, `Up`, and `Cancel` to
`selection.handle_pointer(pointer, state, items, area)` using the same current
items and viewport. Keep overlay/focus routing in the host, as with other widgets.

`selection.selected_text(state, items)` returns selected display bodies without
arrows, timestamps, tool/Thinking headers or Markdown display markers. It preserves
code indentation and raw tool `+/-` text without introducing newlines for terminal
soft wraps. Markdown table copying uses tabs between cells and newlines between
logical table rows, rather than copying drawn borders.
Positions use UTF-8 grapheme boundaries; selection highlights only visible cells.
Scrolling and resizing preserve valid selections; changed/replaced endpoint content
or cleared data invalidates them. Copying itself remains host-owned:
pass the returned text to `ClipboardProvider::set`.

The `agent` example supports drag selection, Ctrl+Shift+C copy and Esc clear.
Tool/Thinking folds happen on pointer-up only when the gesture was a click, not a drag.

## Standalone rich text and diffs

`gbtui-markdown` exposes `StreamingMarkdownRenderer`, `MarkdownStyle`, `Syntect`,
and one-shot `render_markdown_ratatui_full`. `MarkdownStyle::default()` is the
monochrome `PLAIN` style with semantic effects; `pretty=false` retains source syntax.
Create a `Syntect` once and reuse it. `Syntect::default()` loads the bundled,
attributed Tokyo Night syntax theme. Resolved adapters select light/dark syntax
from effective theme appearance, quantize token colors, and preserve explicit
terminal-default code backgrounds instead of applying syntax-theme backgrounds.

`MarkdownRenderOutput` / `MarkdownRenderView` include rendered lines, source-line
mapping, hyperlinks, table-copy metadata and closed fenced-code spans.
`code_lines` also identifies unfinished fenced and indented code body lines, so
streaming copy does not accidentally strip code indentation as prose decoration.
Mermaid fences remain ordinary code. The Markdown engine does not fetch or render
image URLs; hosts can use the explicit optional `Image` widget described below.
The transcript projects hyperlink metadata into wrapped physical rows and registers
visible spans through `FrameCtx`. Supported terminals receive OSC 8 automatically;
unsupported/hostile parsers keep the styled text without native link sequences.

`gbtui_agent::parse_unified_diff` exposes a borrowed-source diff model with files,
metadata, hunks, and old/new line counters. Edit tools use it for dual line-number
gutters, hunk boundaries, themed additions/deletions, and wrapped continuation rows.
Malformed/incomplete diff input remains faithful plain text instead of receiving
invented line numbers. `ToolRow` measures, paints, clips, and supplies selection
ranges from one detail layout; copying excludes gutters while preserving raw detail.

Standalone `ToolRow` callers keep a `ToolRowState` and pass it to `measure`/`render`;
Transcript owns the corresponding per-item caches. Old and new diff sides maintain
separate multiline syntax state, advancing both for context lines. File extensions
select syntax unless `.language(token)` overrides it. The cached source layout
retains gutters/backgrounds without reparsing or copying all detail text on scroll.

## Prompt addon ownership

Hosts obtain `completion_request()` and return candidates with that same snapshot
to `set_completions`; stale draft/cursor generations and invalid UTF-8/atomic ranges
are rejected. Route active addons before global keybindings and forward registered
hits to `handle_hit`. `EnterOutcome::Handled` means completion/history consumed
Enter, not submission; `PromptEvent::is_edited()` tells hosts when to refresh data.

History is supplied through `set_history` / `record_history`. Up/Down shows a live
browse popup; Esc restores the exact draft snapshot, including atomic attachments.
Ctrl+R uses smart-case fuzzy ranking with grapheme highlights and newest-first ties.
Printable `j/k` belong to the query/editor, not the dropdown's standalone navigation.
Ctrl+R may replace an open completion popup.
`PastePolicy::Collapse` creates undoable atomic elements that retain original text.
`insert_attachment` stores only host metadata; use
`try_submission()` to obtain the current text and live attachments, excluding
deleted elements. Preview, loading, persistence and execution stay with the host.

`render_dropdown` accepts an `ExactSizeIterator<Item = DropdownItem>` so addon rows
can borrow existing data without allocating a second display list.

`DropdownState::with_visible_rows` / `set_visible_rows` configures the row cap;
`DropdownScrollPolicy::{Nearest, Centered}` configures selected-row positioning.
Popup placement takes `&DropdownState`, clamps stale anchors into the frame, and
reserves bounded label/detail columns plus an overflow scrollbar. Route scrollbar
hits and captured pointers through `handle_scrollbar_pointer`, including Cancel.

## Generic content reading

`ContentDocument` owns source plus optional `ContentRow` projections. Hosts provide
stable document/row IDs, revisions, source-line mappings, copy text and metadata.
`ContentViewer` adds search, physical scrolling, wrapping and range selection using
the same List engine; the base widgets crate imports neither Markdown nor clipboard.
`ContentView::{Source, Rendered}` switches projections while retaining mapped cursor
and top source locations. A different document/revision clears stale ranges.
`extract(document, ContentCopyTarget)` returns Selection, Source, Rendered or Metadata;
the host decides whether and where to copy it.

Optional `gbtui-agent::{MarkdownDocument, CodeDocument, CodeView}` adapt the existing
streaming Markdown renderer and Syntect output. Prepare via `document_resolved(...)`
before passing data to the viewer; `document(...)` serves raw-palette hosts.
Parsing/highlighting is cached by source revision, resolved styles/version,
effective syntax appearance, color level and relevant width. Code gutters are
display-only and never copied. Transcript and ToolRow consume resolved frame
themes automatically. The `agent` Ctrl+O overlay demonstrates these adapters;
F4 switches Day/Night without replacing the document or input state.

## Rich choices and forms

Construct choices with `Choice::new(id, label).description(...).preview(...)`.
Labels/details may be borrowed or owned; focus expands details through List physical
rows. Picked values use stable IDs, not source positions. `set_other` enables the
freeform row; Enter starts/commits its `LineEditor`, Esc discards the draft, and
navigation is suspended during editing. Pass current choices to `handle_key` and
`click`; modal hosts must route editing keys before their own close/tab shortcuts.

`FormRow::with_error` displays host validation, and `FormRow::multi_select` borrows
options plus selected flags. `FormAction::ToggleMulti` asks the host to mutate a
value; it does not own validation or persistence. `render_form_with_actions` pins
`FormActionRow`s below the physically scrolled body. Use `handle_key_with_actions`,
`click_row`, `click_multi_option`, `click_action`, and the matching hit parsers.
Expanded options and field editors consume Escape before the containing modal.
The body registers a stable `scroll_id(NS_FORM)` target, including errors and blank
space. Forward it to `FormState::handle_scroll`; wheel scrolling detaches from the
field cursor until keyboard or row/option interaction resumes focus following.

## Native output and optional images

`run` / `run_inline` use a live raw-writer path: collect frame output, diff cells and
links, update optional images, restore the caret, then end synchronized output.
Link metadata participates in the diff even if text did not change, so URL replacement
and removal cannot leave stale native targets. `Link` keeps the host's click callback
contract; the framework does not launch a browser. Unknown and hostile OSC parsers
remain quiet, and unsupported schemes or control-bearing URLs are rejected.

`Pump::paint` remains the backend-only/headless path. Custom raw-writer hosts can
use `Pump::paint_live` for links or the `Terminal::try_draw_with` flush strategy APIs.
Standalone `FrameCtx::new` starts with conservative capabilities; use the explicit
viewport/capability builders when integrating another host.

Enable `gbtui`'s `images` feature for `PreparedImage` and `Image`. Prepare once with
`PreparedImage::from_encoded(bytes)` or `from_rgba8(width, height, rgba)`, keep an
`Arc<PreparedImage>`, and render `Image::new(image, alt).id(nonzero_id)`. Decoding,
PNG conversion and base64 preparation happen at construction, not during draw.
The fit helper assumes conventional cells approximately twice as tall as wide.

The widget always paints fallback text and occludes previous output in its area.
On a supported live frame it also registers an aspect-fit image placement. ID `0`
is explicitly fallback-only; hosts assign stable nonzero placement IDs.
Occlusion, hiding and resize remove obsolete placements. Moving an existing image
does not retransmit its payload. Image numbers are process-unique and request
terminal-assigned resource IDs; deletion targets the exact resource/placement pair
and frees retired image data without a delete-all command.

Automatic retained images are limited to Kitty/Ghostty/WezTerm outside multiplexers
and Windows. iTerm2's explicit one-shot OSC 1337 encoder is available separately,
but retained scenes use text fallback because that protocol lacks safe per-image
removal. No filesystem, download, external converter or Grok overlay state is used.
Live-loop teardown clears its image scene before restoring terminal modes, including
panic unwinding and future cancellation; process abort/SIGKILL remains unrecoverable.
