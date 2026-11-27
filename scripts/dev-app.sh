#!/usr/bin/env bash
# Build xReveal, wrap it in a minimal .app and launch it with `open` (docs/spec/v1.md §3.8).
#
# macOS gives camera/microphone permission to the app that launches a binary, so `cargo run` from a
# terminal without camera usage strings is killed on first capture. Launching a real .app avoids that.
# Sign with a stable identity so macOS keeps the permission: xReveal's self-signed certificate
# (scripts/make-signing-cert.sh, #162), else an Apple Development identity (free Personal Team in Xcode)
# across rebuilds; without one the app is ad-hoc signed and macOS may ask again after every build.
#
# Usage: scripts/dev-app.sh [--release] [-- app args...]
# Env:   XREVEAL_SIGN_IDENTITY  codesign identity to use (default: "xReveal Self-Signed", else the
#                               first "Apple Development" identity)
set -euo pipefail

cd "$(dirname "$0")/.."

profile=debug
cargo_flags=()
if [[ "${1:-}" == "--release" ]]; then
  profile=release
  cargo_flags+=(--release)
  shift
fi
[[ "${1:-}" == "--" ]] && shift

cargo build "${cargo_flags[@]}"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
app="target/dev-app/xReveal.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "target/$profile/xreveal" "$app/Contents/MacOS/xreveal"
mkdir -p "$app/Contents/Resources"
cp packaging/macos/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
sed "s/__VERSION__/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"

identity="${XREVEAL_SIGN_IDENTITY:-}"
if [[ -z "$identity" ]] && security find-certificate -c "xReveal Self-Signed" >/dev/null 2>&1; then
  identity="xReveal Self-Signed"
fi
if [[ -z "$identity" ]]; then
  identity=$(security find-identity -v -p codesigning 2>/dev/null \
    | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -n 1)
fi
if [[ -z "$identity" ]]; then
  echo "dev-app: no signing identity found; ad-hoc signing." >&2
  echo "dev-app: run scripts/make-signing-cert.sh once so permissions survive rebuilds." >&2
  echo "dev-app: macOS may ask for camera/microphone access again after each build." >&2
  echo "dev-app: fix once: Xcode > Settings > Accounts > add your Apple ID (Personal Team), then" >&2
  echo "dev-app:           Manage Certificates > + > Apple Development." >&2
  identity="-"
fi
codesign --force --sign "$identity" "$app"

log="target/dev-app/xreveal.log"
: > "$log"
echo "dev-app: launching $app (log: $log)"
tail -f "$log" &
tail_pid=$!
trap 'kill $tail_pid 2>/dev/null' EXIT
open -n -W --stdout "$log" --stderr "$log" "$app" --args "$@"
