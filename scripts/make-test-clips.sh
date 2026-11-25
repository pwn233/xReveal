#!/usr/bin/env bash
# Regenerates the synthetic test clips in tests/clips/ (docs/spec/v1.md §5).
#
# Each clip shows five vertical colour bars of equal width, left to right:
#   white, red, green, blue, black   (sRGB 255/0 values)
# encoded with BT.709 and the colour range in the file name. Tests decode them through the same
# Media Foundation path as a capture card and check the bars' colours.
set -euo pipefail
cd "$(dirname "$0")/.."
out=tests/clips
mkdir -p "$out"

bars() { # width height fps frames output-filter
  local h=$2 r=$3 n=$4 bw=$(( $1 / 5 ))
  local d; d=$(awk "BEGIN { printf \"%.4f\", $n / $r }")
  local inputs=""
  for c in 0xFFFFFF 0xFF0000 0x00FF00 0x0000FF 0x000000; do
    inputs+=" -f lavfi -i color=c=$c:s=${bw}x$h:r=$r:d=$d"
  done
  echo "$inputs -filter_complex [0][1][2][3][4]hstack=inputs=5,format=rgb24,$5"
}

common=(-y -loglevel error -fflags +bitexact -flags:v +bitexact -map_metadata -1
        -colorspace bt709 -color_primaries bt709 -color_trc bt709)

# H.264, limited range → Media Foundation's H.264 decoder → NV12.
# shellcheck disable=SC2046
ffmpeg $(bars 1280 720 60 6 "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p") "${common[@]}" \
  -color_range tv \
  -c:v libx264 -preset veryslow -crf 4 -g 6 -bf 0 -x264-params threads=1 \
  "$out/bars-1280x720p60-h264-limited.mp4"

# MJPEG, full range (JPEG is full range) → Media Foundation's MJPEG decoder.
# shellcheck disable=SC2046
ffmpeg $(bars 1280 720 60 6 "scale=out_color_matrix=bt709:out_range=pc,format=yuvj422p") "${common[@]}" \
  -color_range pc \
  -c:v mjpeg -q:v 2 \
  "$out/bars-1280x720p60-mjpeg-full.avi"

# Raw YUY2, limited range: no decoder at all, like an uncompressed USB capture.
# shellcheck disable=SC2046
ffmpeg $(bars 640 360 60 2 "scale=out_color_matrix=bt709:out_range=tv,format=yuyv422") "${common[@]}" \
  -color_range tv \
  -c:v rawvideo -pix_fmt yuyv422 -vtag YUY2 \
  "$out/bars-640x360p60-yuy2-limited.avi"

ls -l "$out"
