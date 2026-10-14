# Contributing

`docs/plan.md` is the constraint document. Change the plan before changing architecture.

## Rules

1. Do not edit `sources/grok-build/`. It is a read-only reference clone.
2. Do not depend on `xai-grok-*`, `xai-ratatui-*`, or path-deps into `sources/`. `bash scripts/check-no-grok-deps.sh` must stay green.
3. Crate deps only point downward:

   ```text
   gbtui-agent → gbtui → widgets → runtime → {input, terminal} → core
   gbtui-actions → core only
   ```

   `runtime` must not depend on `widgets`. `gbtui` must not depend on `gbtui-agent`.
   Examples depend on the `gbtui` facade, not on layered crates (unless they are crate-local demos).
   `examples/agent` may also depend on `gbtui-agent`.
   `examples/studio` is the workbench showcase (facade only).
4. After P3: widgets talk to the mouse through `FrameCtx`. A new `match MouseEventKind` in application or widget code is a regression.
5. Theme colors live in theme definitions. No ad-hoc `Color::Rgb` in widgets (except the Theme files).
6. Idle UI must not request `TickDemand::Fast`.
7. When copying Grok files, the commit message includes `SOURCE: crates/codegen/<path> (COPY|STRIP|REWRITE)`.
8. Update `docs/source-map.md` when a Grok file is lifted. Tick the matching box in `docs/plan.md`.
9. Terminal quirks belong in `gbtui-terminal` / `gbtui-input`, never `if brand == …` inside a widget. Record the quirk in `docs/terminal-matrix.md` first.

## Checks before a PR

```bash
cargo fmt --all -- --check
cargo test --workspace --all-features --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --workspace --all-features --bins --examples
cargo check -p gbtui --no-default-features
bash scripts/check-no-grok-deps.sh
```

## New widget exit checklist

1. Draw using `Theme` only
2. Hover and focus styles
3. Documented key bindings
4. Mouse only via `FrameCtx`
5. No response when occluded
6. Unit test and visible in an example
7. rustdoc with a tiny layout sketch
