#!/usr/bin/env python3
"""Measure sound-vs-picture skew in a recording of the flash + beep clip.

Usage: python3 -I av_skew.py RECORDING [--crop W:H:X:Y]

Needs Python 3.7+ and ffmpeg/ffprobe on PATH. Use --crop to look only at the xReveal screen in a phone video. Reads the file's own timestamps (not a slowed-down playback), finds each flash
(mean luma jumps up) and each beep onset (audio envelope crosses a threshold), pairs each flash with the nearest
beep, and prints skew = beep - flash in ms. Positive = sound late (picture first); negative = sound early.
xReveal's A/V sync offset that cancels it is -median(skew) (positive offset delays the sound).
"""
import array, json, statistics, subprocess, sys

def video_flashes(path, crop):
    vf = (f"crop={crop}," if crop else "") + "signalstats"
    out = subprocess.run(["ffprobe", "-v", "error", "-f", "lavfi", "-i", f"movie={path},{vf}",
                          "-show_entries", "frame=pts_time:frame_tags=lavfi.signalstats.YAVG", "-of", "json"],
                         capture_output=True, text=True, check=True).stdout
    frames = [(float(f["pts_time"]), float(f["tags"]["lavfi.signalstats.YAVG"])) for f in json.loads(out)["frames"]]
    lo, hi = min(y for _, y in frames), max(y for _, y in frames)
    thr = lo + (hi - lo) / 2
    flashes, prev = [], lo
    for t, y in frames:
        if prev < thr <= y:
            flashes.append(t)
        prev = y
    return flashes, statistics.median(b[0] - a[0] for a, b in zip(frames, frames[1:]))

def audio_onsets(path, rate=48000):
    raw = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-vn", "-ac", "1", "-ar", str(rate),
                          "-f", "s16le", "-"], capture_output=True, check=True).stdout
    start = float(subprocess.run(["ffprobe", "-v", "error", "-select_streams", "a:0", "-show_entries",
                                  "stream=start_time", "-of", "csv=p=0", path],
                                 capture_output=True, text=True, check=True).stdout.strip() or 0)
    s = array.array("h", raw)
    win = rate // 1000  # 1 ms envelope
    env = [max(abs(v) for v in s[i:i + win]) for i in range(0, len(s) - win, win)]
    peak = max(env)
    thr, onsets, armed = peak * 0.3, [], True
    for i, e in enumerate(env):
        if armed and e >= thr:
            onsets.append(start + i / 1000)
            armed = False
        elif not armed and e < peak * 0.05:
            armed = True
    return onsets

def main():
    path = sys.argv[1]
    crop = sys.argv[sys.argv.index("--crop") + 1] if "--crop" in sys.argv else None
    flashes, frame_dur = video_flashes(path, crop)
    beeps = audio_onsets(path)
    skews = []
    for f in flashes:
        b = min(beeps, key=lambda b: abs(b - f), default=None)
        if b is not None and abs(b - f) < 0.4:
            skews.append((b - f) * 1000)
    if not skews:
        sys.exit("no flash/beep pairs found")
    print(f"pairs={len(skews)} frame={frame_dur*1000:.2f} ms")
    print(f"skew ms (beep - flash): median={statistics.median(skews):.1f} "
          f"min={min(skews):.1f} max={max(skews):.1f} stdev={statistics.pstdev(skews):.1f}")
    print(f"offset that cancels it: {-statistics.median(skews):+.0f} ms")

if __name__ == "__main__":
    main()
