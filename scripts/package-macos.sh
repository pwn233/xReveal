#!/usr/bin/env bash
# Builds the macOS release assets (docs/spec/v1.md §6): xReveal.app, ad-hoc signed, as a .dmg and a
# .zip, in dist/. Usage: scripts/package-macos.sh [version]   (default: Cargo.toml's version)
#
# Signing (#162): XREVEAL_SIGN_IDENTITY, else xReveal's stable self-signed certificate when it's in
# the keychain (scripts/make-signing-cert.sh), else ad hoc. A stable identity keeps macOS's Camera and
# Microphone access across updates. Paid signing and notarisation can slot in here later.
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
identity="${XREVEAL_SIGN_IDENTITY:-}"
if [[ -z "$identity" ]] && security find-certificate -c "xReveal Self-Signed" >/dev/null 2>&1; then
  identity="xReveal Self-Signed"
fi
if [[ -z "$identity" ]]; then
  echo "package-macos: no signing identity; ad-hoc signing (macOS will ask for camera/microphone again after updates)." >&2
  identity="-"
fi
codesign --force --sign "$identity" "$app"
codesign --verify "$app"

base="xReveal-$version-macos-arm64"
ditto -c -k --keepParent "$app" "$out/$base.zip"
staging="$(mktemp -d)"
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
hdiutil create -volname "xReveal $version" -srcfolder "$staging" -ov -format UDZO "$out/$base.dmg" >/dev/null
rm -rf "$staging"
echo "package-macos: $out/$base.dmg $out/$base.zip"
