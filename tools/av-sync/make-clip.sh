#!/bin/sh
# Make a 60 fps flash + beep A/V sync clip that a PS5 Media Gallery can play (H.264 High L4.2 + AAC-LC in MP4).
# One full-white frame and a 1 kHz tone burst of the same length (1/60 s) start together at t = k + 0.5 s.
#
# Audio uses Apple's AAC encoder (aac_at, macOS ffmpeg builds) on purpose: its 2112-frame priming is both written
# to the MP4 edit list AND equal to the priming Apple tells players to assume (TN2258), so a player that honours
# either convention plays the clip in sync. ffmpeg's own `aac` primes 1024 frames, which only edit-list-aware
# players trim. No B-frames, so the video track needs no edit list at all.
set -e
out="${1:-avsync-1080p60.mp4}"
acodec=aac_at
ffmpeg -hide_banner -encoders 2>/dev/null | grep -q ' aac_at ' || { acodec=aac; echo "aac_at missing: using ffmpeg aac (1024-frame priming)" >&2; }
beep="if(between(mod(t,1),0.5,0.5+1/60),0.8*sin(2*PI*1000*t),0)"
ffmpeg -hide_banner -loglevel warning -y \
  -f lavfi -i "color=black:s=1920x1080:r=60,drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='eq(mod(n,60),30)'" \
  -f lavfi -i "aevalsrc=exprs='${beep}|${beep}':s=48000" \
  -t 120 -c:v libx264 -profile:v high -level 4.2 -pix_fmt yuv420p -g 60 -bf 0 \
  -c:a "$acodec" -b:a 192k -ar 48000 -movflags +faststart "$out"
