#!/usr/bin/env bash
# Builds the macOS release assets (docs/spec/v1.md §6): xReveal.app, ad-hoc signed, as a .dmg and a
# .zip, in dist/. Usage: scripts/package-macos.sh [version]   (default: Cargo.toml's version)
#
# Signing: ad-hoc unless XREVEAL_SIGN_IDENTITY names a Developer ID, so paid signing and
# notarisation can slot in here later without changing anything else.
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)}"
version="${version#v}"
cargo build --release --locked

out=dist
app="$out/xReveal.app"
rm -rf "$out"
mkdir -p "$app/Contents/MacOS"
cp target/release/xreveal "$app/Contents/MacOS/xreveal"
mkdir -p "$app/Contents/Resources"
cp packaging/macos/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
sed "s/__VERSION__/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
codesign --force --sign "${XREVEAL_SIGN_IDENTITY:--}" "$app"
codesign --verify "$app"

base="xReveal-$version-macos-arm64"
ditto -c -k --keepParent "$app" "$out/$base.zip"
staging="$(mktemp -d)"
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
hdiutil create -volname "xReveal $version" -srcfolder "$staging" -ov -format UDZO "$out/$base.dmg" >/dev/null
rm -rf "$staging"
echo "package-macos: $out/$base.dmg $out/$base.zip"
