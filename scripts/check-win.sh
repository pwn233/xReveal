#!/usr/bin/env bash
# Windows dry lab (docs/spec/v1.md §5): type-check and lint the Windows build from a Mac.
# Linking and running happen on Windows CI. Extra args go to clippy, e.g. `scripts/check-win.sh -- -D warnings`.
set -euo pipefail
cd "$(dirname "$0")/.."

target=x86_64-pc-windows-msvc
rustup target list --installed | grep -qx "$target" || rustup target add "$target"

# GPUI embeds its Windows manifest with embed-resource, which needs llvm-rc (Homebrew's llvm is keg-only).
if [[ -z "${RC_x86_64_pc_windows_msvc:-}" ]]; then
  rc="$(brew --prefix llvm 2>/dev/null)/bin/llvm-rc"
  if [[ ! -x "$rc" ]]; then
    echo "check-win: llvm-rc not found. Install it with: brew install llvm" >&2
    exit 1
  fi
  export RC_x86_64_pc_windows_msvc="$rc"
fi

exec cargo clippy --target "$target" --all-targets --color "${CARGO_TERM_COLOR:-always}" "$@"
