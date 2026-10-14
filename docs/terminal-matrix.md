# Terminal support matrix

v1 does **not** claim Grok-level coverage of every emulator. Special cases go in `gbtui-terminal` / `gbtui-input`, never in widgets.

| Environment | v1 | Notes |
|-------------|----|--------|
| Kitty / Ghostty | **supported** | Kitty keyboard protocol |
| iTerm2 / WezTerm | **supported** | |
| tmux (`extended-keys` on) | **supported** | OSC may need passthrough; Ctrl+. often unreliable |
| macOS Terminal.app | best-effort | no truecolor; Enter modifiers via OS rescue; OSC 8 hostile |
| VS Code / Cursor / Windsurf / Zed integrated terminal | best-effort | host steals some Ctrl chords; native link hover |
| Windows Terminal | best-effort | |
| Legacy ConHost | best-effort | glyph fallbacks (shades, ASCII spinner) |
| screen / unknown | unsupported | |

When a new quirk is found: document it here first, then decide whether it is code.

Brand/mux/keyboard/hyperlink probes live in `gbtui-terminal`. Force color: `GBTUI_FORCE_COLOR_LEVEL`. Force appearance: `GBTUI_APPEARANCE`.

Startup probes are bounded and conservative: OSC 11 is queried only on a
known-safe direct Unix terminal; DA2 is queried for direct Alacritty, and
XTVERSION for the explicit supported-brand allowlist. Unknown terminals and
multiplexers skip capability queries. Inline DSR remains required for cursor
placement. A persistent decoder preserves type-ahead and fragmented UTF-8/CSI/paste
across startup and live input; valid late replies to issued queries are filtered.

- Alacritty DA2 unknown or packed version `<= 2401`: Kitty disambiguation only.
- Alacritty DA2 `>= 2402`: event-type reporting may also be enabled.
- `ThemeKind::Auto`: environment override → startup OSC11 background → OS → Night.
  All built-in kinds adapt to the detected color level after palette selection.
  ANSI16 has distinct chrome states; `NO_COLOR` wins over forced depth.
  Explicit `Theme` palettes are never replaced or automatically quantized.
- `AppEvent::TerminalInfo`: initial facts and changed late replies, including
  sanitized XTVERSION text. A version string is not treated as a capability proof.
- Bracketed paste contents are opaque: embedded terminal replies, Kitty keys or
  mouse sequences stay pasted text rather than triggering UI actions.
- Cursor replies accept only the DSR grammar and disarm the one-shot query after
  its reply, rather than holding unrelated numeric CSI keyboard frames.
- Kitty press/repeat/release decoding covers both CSI-u and functional-key
  suffixes (`A`–`D`, `H`, `F`, `~`, etc.), as defined by the
  [keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/).
  Native Kitty's `CSI 1;1:3 A` release must not stall subsequent ordinary text.

Unix stdin and stdout may share one open-file description (notably PTY launchers).
The asynchronous reader must reopen the same TTY independently: setting
`O_NONBLOCK` on inherited stdin can otherwise make terminal painting fail with
`WouldBlock` on stdout.

## P13 verification boundary

The macOS controlled-PTY runs exercised `settings`, `chat`, `picker`, `agent`,
`studio` and `media`, including normal exits and inline resize. Injected responses
verified OSC11 light/dark precedence, Alacritty 2401/unknown/2402 keyboard flags,
late fragmented XTVERSION, UTF-8 split across startup, and an 80-byte paste
containing reply/key/mouse sequences with no accidental submission. Reopening
the input descriptor removed the observed shared-fd `WouldBlock` paint failure.
Settled Picker/Transcript views emitted no bytes over a 500ms idle observation.

These are real program/PTY and protocol-byte checks, not headful acceptance of
the named terminal emulators. Windows, VS Code, Apple Terminal and the full
multiplexer matrix still require environment-specific acceptance.

## P14 verification boundary

Kitty 0.46.1 on macOS ran the actual `chat`, `settings`, and `agent` binaries.
Remote keyboard input plus the emulator's rendered text/style state verified:

- Prompt completion with printable `j`, Ctrl+R takeover, `j/k` query input,
  fuzzy `hst` history recall, visible browse, attachment-draft restoration,
  Escape handling and normal exit.
- Rich Choice descriptions/previews; freeform spaces, `j/k` and Unicode;
  commit/cancel without the outer modal stealing Escape.
- Host-owned Form errors, multi-select changes, and a 60-column/20-row window
  keeping the selected expanded option above pinned Apply/Reset actions.
- Code/Markdown and raw/rendered switching, narrow wrapping on/off, search,
  gutter-free clipboard text and Shift-click ranges. Injected focus loss followed
  by late drag motion preserved the range without extending it. The prior system
  clipboard was restored after the check.
- ANSI16 native frames and a `NO_COLOR` frame without color SGRs. Controlled PTY
  runs separately checked dark/light ANSI16 palette indices within 0–15,
  ANSI256 without RGB, and truecolor RGB output. The backend may encode ANSI16
  colors as indexed SGR values 0–15 rather than short 30–37/90–97 forms.

Native acceptance uncovered and reproduced DSR buffering of CSI-u input, Kitty
functional-key release buffering, rich Choice detail rows not painted, and modal
Escape precedence. Regression tests plus reruns of the real interactions cover
the fixes. A settled ContentViewer emitted no bytes over a 600ms PTY observation.
A subsequent controlled-PTY Form check scrolled past the focused field to later
rows without moving selection, then restored focus following with a direction key;
Apply/Reset stayed pinned throughout.
The migrated Studio Form wheel route also reached its final field in an eight-row
PTY viewport and exited normally via Ctrl+Q.

This is native emulator text/style and protocol acceptance, not pixel screenshots
or a full physical-input/accessibility review. Windows, VS Code, Apple Terminal,
other emulators and multiplexers remain outside this run's acceptance coverage.

## P15 theme verification boundary

Kitty 0.46.1 on macOS ran the actual `themes` and `agent` programs. The emulator's
rendered text and ANSI style state verified:

- Night/Day/Auto/custom theme changes without losing a Unicode prompt draft.
- Terminal-default canvas and transparent normal buttons, while an explicit custom
  focused-button background remained `39/93/104`; native OSC8 links stayed registered.
- A 60-column/18-row preview retaining the draft and showing the selected state
  column instead of overflowing the wide state matrix.
- Native `NO_COLOR` without color SGR: hover underline, focus bold, selection
  reverse, invalid bold/underline, disabled dim.
- Agent CodeView switching dark/light syntax colors and Markdown recoloring in
  place through F4, without replacing the document.

Controlled PTY runs separately verified ANSI16 indices at most 15, ANSI256 with no
RGB output, truecolor output, and `NO_COLOR` taking precedence over forced truecolor.
They also exercised external TOML loading, failed reload retaining the theme/version,
successful reload changing the focused style, no-op theme versions, late OSC11
switching Auto to light, and explicit Night surviving subsequent terminal updates.
Monochrome state collision, explicit-theme syntax precedence, and a custom
terminal-default palette retaining Reset with light ANSI16 chrome have permanent
behavior regressions. A final 60×18 light-ANSI16 PTY run observed black foreground
with reset background, then ANSI15 canvas after switching to Solid.

Transparency is terminal-default background output, not alpha compositing.
Appearance refresh consumes validated terminal facts or host context updates;
there is no OS-theme subscription or periodic terminal polling. Contrast estimates
cannot know an unprobed default foreground or customized ANSI palette.

This is actual emulator text/style and PTY protocol acceptance, not a pixel
screenshot, accessibility certification, or a full terminal/multiplexer matrix.
Windows, VS Code, Apple Terminal and other emulators remain unverified for P15.

## Native output policy

- OSC 8: enabled for a detected native-capable terminal; unknown, unsupported and
  hostile parsers get ordinary text. Native links do not invoke host-side URL opening.
- `images` (opt-in): retained Kitty graphics for Kitty, Ghostty and WezTerm.
  Windows, multiplexers, iTerm2 and unknown terminals use text fallback.
  An explicit one-shot iTerm2 encoder exists, but is not used for dynamic scenes.
- Image input is decoded once and prepared as PNG. Scene move/hide/occlusion/cleanup
  uses exact image-number and placement selectors, never global delete-all.

Verification in this extraction used Kitty 0.46.1 native hyperlink/resource state
and PTY byte capture. Image resources were observed to transition `1 → 0 → 1` under
occlusion and show/hide; transmitted PNG dimensions/pixels were checked after decode.
Apple Terminal/unknown/tmux fallback policies were exercised with controlled PTY
environments, not those emulators themselves. Pixel screenshots were unavailable
because screen capture was not authorized; the other emulator matrix remains unverified.
