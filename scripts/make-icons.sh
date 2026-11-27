#!/usr/bin/env bash
# Regenerates xReveal's app icons from assets/icon/xreveal.svg (#130): packaging/macos/AppIcon.icns
# and packaging/windows/xreveal.ico. Run on a Mac (AppKit renders the SVG, iconutil packs the .icns);
# the generated files are committed, so builds and CI never need this.
set -euo pipefail
cd "$(dirname "$0")/.."

work="$(mktemp -d)"
trap 'rm -r "$work"' EXIT
xcrun swiftc -O -module-cache-path "$work/mc" -o "$work/render-svg" scripts/render-svg.swift
render() { "$work/render-svg" assets/icon/xreveal.svg "$2" "$1"; }

# macOS: every size iconutil expects, 1x and 2x.
set_dir="$work/AppIcon.iconset"
mkdir -p "$set_dir"
for size in 16 32 128 256 512; do
  render "$size" "$set_dir/icon_${size}x${size}.png"
  render "$((size * 2))" "$set_dir/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$set_dir" -o packaging/macos/AppIcon.icns

# Windows: one .ico holding PNG images (Windows Vista and later read PNG inside .ico).
sizes=(16 24 32 48 64 128 256)
for size in "${sizes[@]}"; do render "$size" "$work/ico-$size.png"; done
/usr/bin/python3 -I - "$work" packaging/windows/xreveal.ico "${sizes[@]}" <<'PY'
import struct, sys
work, out, sizes = sys.argv[1], sys.argv[2], [int(s) for s in sys.argv[3:]]
images = [open(f"{work}/ico-{s}.png", "rb").read() for s in sizes]
header = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
entries, data = b"", b""
for size, png in zip(sizes, images):
    side = 0 if size >= 256 else size  # 0 means 256 in an .ico directory entry
    entries += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(png), offset + len(data))
    data += png
open(out, "wb").write(header + entries + data)
PY
echo "make-icons: wrote packaging/macos/AppIcon.icns and packaging/windows/xreveal.ico"
