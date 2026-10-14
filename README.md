# gbtui

**A general-purpose terminal UI runtime and widget library for Rust.**

**English** | [简体中文](README.zh-CN.md)

[Quick start](#quick-start) · [Examples](#examples) · [Themes](#themes) · [Documentation](#documentation) · [Contributing](CONTRIBUTING.md)

Built on **ratatui 0.29** and **crossterm**, gbtui brings event routing, same-frame hover, on-demand animation, semantic themes, and reusable widgets to immediate-mode terminal applications.

> **Status:** early development, version 0.1.0. APIs may change. Rust 1.94.0 is pinned in `rust-toolchain.toml`.

## Features

- **Runtime:** keyboard and pointer routing, hit testing, focus handling, overlay occlusion, and demand-driven ticks.
- **Widgets:** rich lists, fuzzy pickers, choices, forms, multiline input, prompts, modals, and content viewers.
- **Themes:** named definitions, semantic states, terminal-default backgrounds, automatic appearance, color-depth adaptation, and live switching.
- **Rich content:** optional Markdown, syntax-highlighted code and diffs, native OSC 8 links, and image transport.
- **Inline applications:** keep history in native terminal scrollback while retaining an interactive viewport.

This is a generic framework, **not a Grok Build product replica**. It does not provide LLM integration, tool execution, or session persistence. Extracted code is attributed in [NOTICE](NOTICE) and [THIRD-PARTY-NOTICES](THIRD-PARTY-NOTICES). The optional local `sources/grok-build/` reference checkout is excluded from Git and is not a build dependency.

## Quick start

Use a local checkout dependency. Adjust the path to your checkout; this is not a crates.io installation instruction.

```toml
[dependencies]
gbtui = { path = "../gbtui/crates/gbtui" }
```

```rust
use gbtui::{key, ratatui};
use gbtui::{App, AppEvent, Button, FrameCtx, HitId, Outcome, Pointer, ThemeKind, run_blocking};

const OK: HitId = HitId::named("ok");

struct Hello;

impl App for Hello {
    type Msg = ();

    fn handle(&mut self, ev: AppEvent<()>) -> Outcome {
        match ev {
            AppEvent::Hit(id, Pointer::Down { .. }) if id == OK => Outcome::Quit,
            AppEvent::Key(k) if key!(Esc).matches(&k) || key!('q').matches(&k) => Outcome::Quit,
            AppEvent::Hit(_, Pointer::Move { .. }) | AppEvent::Miss(Pointer::Move { .. }) => {
                Outcome::Changed
            }
            _ => Outcome::Unchanged,
        }
    }

    fn draw(&mut self, f: &mut ratatui::Frame, cx: &mut FrameCtx) {
        Button::new("OK").id(OK).focused(true).render(f.area(), f.buffer_mut(), cx);
    }
}

fn main() -> std::io::Result<()> {
    run_blocking(Hello, ThemeKind::Auto)
}
```

The facade reexports matching versions of `ratatui` and `crossterm`. Route clicks through `AppEvent::Hit` rather than matching raw mouse events; `FrameCtx` computes hover in the current frame.

## Examples

Run these commands from the repository root in an interactive terminal:

| Example | Command | Demonstrates |
|---|---|---|
| Workbench | `cargo run -p studio` | Lists, overlays, forms, and demand-driven animation |
| Settings | `cargo run -p settings` | Choices, validation, pinned actions, and live theme selection |
| Chat shell | `cargo run -p chat` | Inline prompts, completion, history, paste, and attachment metadata |
| Picker | `cargo run -p picker` | Rich rows, groups, fuzzy/regex/substring search |
| Agent shell | `cargo run -p agent` | Local fake session, transcript, code and Markdown viewers |
| Media | `cargo run -p gbtui --example media --features images` | Images and native links |
| Theme laboratory | `cargo run -p gbtui --example themes --features theme-config` | Theme states, transparency, reload, and contrast estimates |

In `themes`, use **1–4** to select a theme, **b** for background policy, **r** to reload, **i** to edit, **j/k** to scroll, and **s** to cycle states in narrow windows. Append a TOML path to load your own theme; see [aurora.toml](crates/gbtui/examples/aurora.toml).

In `agent`, **Ctrl+O** opens the content viewer, **m/c** switches Markdown/code, and **F4** switches Day/Night. Examples do not connect to an LLM or execute tools.

## Themes

`ThemeDefinition` describes policy and overrides; `ThemeResolver` produces a `ResolvedTheme`. Widgets consume `cx.style(ThemeRole, ThemeState)` through allocation-free table lookups. `Theme` remains a lightweight raw palette.

```rust
use gbtui::{BackgroundPolicy, ThemeColor, ThemeDefinition, ThemeKind};
use gbtui::ratatui::style::Color;

let theme = ThemeDefinition::from(ThemeKind::Auto)
    .with_background(BackgroundPolicy::TerminalDefault)
    .with_color(ThemeColor::Accent, Color::Rgb(74, 158, 170));
```

Pass the definition to `run_blocking` or another runtime entry point.

| Background policy | Behavior |
|---|---|
| `Solid` | Uses theme backgrounds |
| `TerminalDefault` | Uses terminal defaults for canvas, input, code, and scrollbar tracks; retains elevated and interaction surfaces |
| `Transparent` | Also resets ordinary elevated and button surfaces; retains hover/selection feedback |

Transparency uses **`Color::Reset`, not alpha compositing**. The emulator controls window opacity. Explicit style overrides can restore opaque backgrounds.

- `ThemeRegistry` includes Night, Day, and Auto and accepts custom named definitions.
- Override order: semantic defaults → Normal role override → current-state override → component-local `Style::patch`.
- State precedence: Disabled → Invalid → Selected → Focused → Hovered → Normal. Monochrome output retains non-color feedback.
- Auto appearance uses environment override → OSC 11 → OS appearance → Night. Explicit Day/Night also controls syntax highlighting.
- Definitions adapt to truecolor, ANSI256, ANSI16, and `NO_COLOR`. Explicit raw `Theme` values remain host-owned.
- Return pending definitions through `App::take_theme_request()` to switch live. Cache generations from `cx.theme_version()` are resolver-scoped; compare complete resolved themes across owners.
- Optional `theme-config` enables strict `ThemeDefinition::from_toml`. The host owns file IO and reload triggers; invalid configuration is returned as an error.
- `ResolvedTheme::contrast` estimates contrast and returns `None` when default RGB values are unknown. ANSI palette estimates are not accessibility certification.

## Architecture and optional features

Applications normally depend on the `gbtui` facade. Layered crates remain available for specialized hosts.

| Crate | Responsibility |
|---|---|
| `gbtui` | Public facade |
| `gbtui-core` | Themes, semantic styles, rendering primitives, shortcuts, and tick demand |
| `gbtui-terminal` | Terminal probes, protocols, native scrollback, links, and optional images |
| `gbtui-input` | Keyboard normalization and scroll behavior |
| `gbtui-runtime` | Event loop, frame context, hit maps, and scheduling |
| `gbtui-widgets` | Reusable interactive widgets |
| `gbtui-actions` | Generic action and keymap registry |
| `gbtui-agent` | Optional transcript/tool chrome and rich document adapters |
| `gbtui-markdown` | Streaming Markdown and syntax highlighting |
| `gbtui-markdown-core` | Headless parsing and structural analysis |

The facade does not depend on the agent or Markdown stack. Drawing remains immediate-mode ratatui; `gbtui-terminal::Terminal` is the adapted terminal implementation used for inline viewport behavior.

Facade features: `clipboard` is enabled by default; `images` and `theme-config` are opt-in. Image bytes, completion candidates, attachments, and history are supplied by the host. The framework does not fetch images or persist application data.

## Compatibility

Native interaction acceptance currently centers on **Kitty on macOS**, complemented by controlled PTY checks. Windows, Apple Terminal, VS Code, and the full multiplexer matrix have not been individually accepted. See the [terminal matrix](docs/terminal-matrix.md) for evidence and limitations.

Appearance refresh consumes validated terminal replies or host context updates. There is no OS-theme subscription or periodic probing loop. This project does not claim pixel parity with Grok Build or universal terminal compatibility.

## Documentation

- [Architecture](docs/architecture.md) — runtime, themes, frame context, and rendering contracts
- [Terminal matrix](docs/terminal-matrix.md) — compatibility and verification boundaries
- [Inline viewport](docs/inline-viewport.md) — native scrollback behavior
- [Implementation plan](docs/plan.md) — scope and constraints, primarily in Chinese
- [Source map](docs/source-map.md) — extraction provenance

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) before changing architecture or adding widgets. Include reproduction steps, target terminals, and verification results with changes.

```bash
cargo fmt --all -- --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --workspace --all-features --bins --examples
cargo check -p gbtui --no-default-features
bash scripts/check-no-grok-deps.sh
```

## License

Licensed under [Apache-2.0](LICENSE). Derived code retains its attribution in [NOTICE](NOTICE), [THIRD-PARTY-NOTICES](THIRD-PARTY-NOTICES), and crate-local notices. Default themes contain no Grok branding.
