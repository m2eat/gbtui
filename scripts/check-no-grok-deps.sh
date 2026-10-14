#!/usr/bin/env bash
# Fail if any gbtui crate depends on xai-grok-* or path-depends on sources/grok-build.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

fail=0

die() {
    echo "error: $*" >&2
    fail=1
}

# Manifests under crates/ (and future examples/) must not name grok packages.
while IFS= read -r file; do
    hits="$(grep -nE 'xai-grok-|xai_grok_|xai-ratatui-' "$file" 2>/dev/null | grep -vE '^[^:]+:[[:space:]]*#' || true)"
    if [[ -n "$hits" ]]; then
        echo "forbidden grok/xai crate reference in $file:" >&2
        echo "$hits" >&2
        fail=1
    fi
    path_hits="$(grep -n 'sources/grok-build' "$file" 2>/dev/null | grep -vE '^[^:]+:[[:space:]]*#' || true)"
    if [[ -n "$path_hits" ]]; then
        echo "forbidden path into sources/grok-build in $file:" >&2
        echo "$path_hits" >&2
        fail=1
    fi
done < <(find crates examples -name Cargo.toml -print 2>/dev/null; printf '%s\n' Cargo.toml)

if ! tree="$(cargo tree --workspace --format '{p}' --prefix none 2>&1)"; then
    echo "error: cargo tree failed:" >&2
    echo "$tree" >&2
    exit 1
fi

if echo "$tree" | grep -E '^(xai-grok-|xai_grok_|xai-ratatui-)'; then
    echo "error: cargo tree contains a forbidden grok package:" >&2
    echo "$tree" | grep -E '^(xai-grok-|xai_grok_|xai-ratatui-)' >&2 || true
    fail=1
fi

# D11: the base facade must not pull in optional agent or rich-text layers.
if gbtui_tree="$(cargo tree -p gbtui --format '{p}' --prefix none 2>&1)"; then
    if echo "$gbtui_tree" | grep -E '^(gbtui-agent|gbtui-markdown(-core)?)($| )'; then
        echo "error: gbtui must not depend on optional agent/markdown layers:" >&2
        echo "$gbtui_tree" | grep -E '^(gbtui-agent|gbtui-markdown(-core)?)($| )' >&2 || true
        fail=1
    fi
else
    echo "error: cargo tree -p gbtui failed:" >&2
    echo "$gbtui_tree" >&2
    fail=1
fi

if [[ "$fail" -ne 0 ]]; then
    die "gbtui crates must not depend on xai-grok-* / xai-ratatui-* / sources/grok-build"
    exit 1
fi

echo "check-no-grok-deps: ok"
