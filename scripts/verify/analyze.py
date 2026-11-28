#!/usr/bin/python3
"""Write summary.md and minutes.csv for a sampler run folder (#207), from macOS or Windows.

Usage: scripts/verify/analyze.py <run folder>

The folder holds meta.json and samples.csv, as written by sample-macos.py or sample-windows.ps1.
sample-macos.py runs this itself when it stops. sample-windows.ps1 -Analyze writes the same summary
in PowerShell, so the PC needs no Python; this reads a Windows folder too, for a second look on the Mac.

The summary is the #194 format:
- Budgets: per window state, the median (and p95) of the last 3 minutes in that state, against spec §4.
- Memory ceilings (spec §4, #195): the highest 1-minute median in each state over the whole run.
- 1-minute medians from xReveal's launch.
- Minutes 11-20, after the 10-minute warm-up (#192): least-squares slope, the wobble (sigma) of the
  1-minute medians around the line, and max - min. "Settled by minute 10" = slope under 6 MB/h and
  minutes 11 and 20 within 3 MB. The 2-hour soak assumes sigma < ~1.6 MB.
MB are MiB, as Task Manager and Activity Monitor count them. Standard library only, Python 3.6+.
"""

import csv
import json
import math
import os
import sys
from datetime import datetime, timedelta

LAST_MINUTES = 3  # each state's medians come from its last 3 minutes, as in #194
SETTLE_SLOPE = 6.0  # MB/h
SETTLE_STEP = 3.0  # MB, minute 11 vs minute 20
SOAK_SIGMA = 1.6  # MB, what the 2-hour soak's length assumes (#192)

# Per OS: the budget rows, the memory that must settle, the 1-minute table and the ceilings (spec §4).
# A row is (column, label, decimals, unit, budget text, budget limit or None).
OSES = {
    "windows": {
        "name": "Windows",
        "rows": [
            ("cpu_pct", "CPU, share of the machine", 2, "%", "<= 5%", 5.0),
            ("cpu_utility_pct", "CPU, scaled to Processor Utility (Task Manager basis, estimate)", 2, "%", "", None),
            ("gpu_pct", "GPU, process (max engine)", 1, "%", "<= 10%", 10.0),
            ("private_ws_mb", 'Memory: private working set (Task Manager "Memory")', 1, " MB", "ceiling below", None),
            ("private_bytes_mb", 'Private Bytes (Task Manager "Commit size")', 1, " MB", "", None),
            ("working_set_mb", "Working set", 1, " MB", "", None),
            ("gpu_dedicated_mb", "Dedicated GPU memory", 1, " MB", "ceiling below", None),
            ("gpu_shared_mb", "Shared GPU memory", 1, " MB", "", None),
            ("handles", "Handles", 0, "", "", None),
            ("threads", "Threads", 0, "", "", None),
            ("machine_cpu_pct", "Whole machine CPU (% Processor Time)", 1, "%", "", None),
            ("machine_utility_pct", "Whole machine CPU (% Processor Utility)", 1, "%", "", None),
        ],
        "settle": ("private_bytes_mb", "Private Bytes"),
        "fit": [
            ("private_bytes_mb", "Private Bytes"),
            ("private_ws_mb", "Private working set"),
            ("gpu_dedicated_mb", "Dedicated GPU memory"),
        ],
        "minutes": [
            ("private_bytes_mb", "Private Bytes MB", 1),
            ("private_ws_mb", "Private WS MB", 1),
            ("gpu_dedicated_mb", "Dedicated GPU MB", 1),
            ("handles", "Handles", 0),
            ("threads", "Threads", 0),
            ("cpu_pct", "CPU %", 2),
            ("gpu_pct", "GPU %", 1),
        ],
        "ceilings": [
            ("private_ws_mb", "Memory (private working set)", {"windowed": 40, "fullscreen": 40}),
            ("gpu_dedicated_mb", "Dedicated GPU memory", {"windowed": 120, "fullscreen": 290}),
        ],
    },
    "macos": {
        "name": "macOS",
        "rows": [
            ("cpu_pct", "CPU, share of the machine", 2, "%", "<= 5%", 5.0),
            ("gpu_pct", "GPU, process (AGX accumulatedGPUTime; reads ~1.5x a Metal System Trace, #200)", 1, "%", "hint only: judge <= 10% on the driver's gpu_pct (#218)", None),
            ("footprint_mb", 'Memory footprint (Activity Monitor "Memory", phys_footprint)', 1, " MB", "ceiling below", None),
            ("footprint_peak_mb", "Peak footprint since launch", 1, " MB", "", None),
            ("threads", "Threads", 0, "", "", None),
            ("open_files", "Open files (fds)", 0, "", "", None),
        ],
        "settle": ("footprint_mb", "Footprint"),
        "fit": [("footprint_mb", "Footprint")],
        "minutes": [
            ("footprint_mb", "Footprint MB", 1),
            ("threads", "Threads", 0),
            ("open_files", "Open files", 0),
            ("cpu_pct", "CPU %", 2),
            ("gpu_pct", "GPU %", 1),
        ],
        "ceilings": [
            ("footprint_mb", "Memory footprint", {"windowed": 220, "fullscreen": 250}),
        ],
    },
}
STATES = [("windowed", "Windowed"), ("fullscreen", "Fullscreen")]


def parse_time(s):
    s = s.strip()
    # Python 3.6 has no fromisoformat; the samplers write yyyy-mm-ddThh:mm:ss.fff+hh:mm.
    if len(s) > 6 and s[-6] in "+-" and s[-3] == ":":
        s = s[:-3] + s[-2:]
    return datetime.strptime(s, "%Y-%m-%dT%H:%M:%S.%f%z")


def num(s):
    try:
        v = float(s)
    except (TypeError, ValueError):
        return math.nan
    return v


def clean(values):
    return sorted(v for v in values if not math.isnan(v))


def median(values):
    v = clean(values)
    if not v:
        return math.nan
    m = len(v) // 2
    return v[m] if len(v) % 2 else (v[m - 1] + v[m]) / 2


def p95(values):
    v = clean(values)
    if not v:
        return math.nan
    return v[min(len(v) - 1, math.ceil(0.95 * len(v)) - 1)]


def fit(points):
    """Least-squares line through (minute, value): slope per hour, sigma around the line, max - min."""
    pts = [(x, y) for x, y in points if not math.isnan(y)]
    if len(pts) < 3:
        return math.nan, math.nan, math.nan
    mx = sum(x for x, _ in pts) / len(pts)
    my = sum(y for _, y in pts) / len(pts)
    sxx = sum((x - mx) ** 2 for x, _ in pts)
    sxy = sum((x - mx) * (y - my) for x, y in pts)
    b = sxy / sxx
    a = my - b * mx
    ss = sum((y - (a + b * x)) ** 2 for x, y in pts)
    sigma = math.sqrt(ss / max(1, len(pts) - 2))
    ys = [y for _, y in pts]
    return b * 60, sigma, max(ys) - min(ys)


def fmt(v, decimals, unit=""):
    if v is None or math.isnan(v):
        return "n/a"
    return "{:,.{}f}{}".format(v, decimals, unit)


def load(folder):
    with open(os.path.join(folder, "meta.json"), encoding="utf-8-sig") as f:
        meta = json.load(f)
    with open(os.path.join(folder, "samples.csv"), newline="", encoding="utf-8-sig") as f:
        rows = list(csv.DictReader(f))
    return meta, rows


def analyze(folder):
    meta, rows = load(folder)
    cfg = OSES[meta["os"]]
    interval = float(meta.get("interval_s", 5))
    per_minute = 60.0 / interval

    procs = meta.get("processes") or []
    if not procs:
        sys.exit("analyze: meta.json lists no xReveal process; nothing was sampled.")
    main = procs[0]
    pid = str(main["pid"])
    launch = parse_time(main["start"])
    cols = [c for c, *_ in cfg["rows"]]
    samples = []
    for r in rows:
        if r.get("pid") != pid or r.get("alive") != "1":
            continue
        s = {c: num(r.get(c)) for c in cols}
        s["t"] = parse_time(r["time"])
        s["mode"] = r.get("mode", "")
        s["window"] = r.get("window", "")
        s["display"] = r.get("display", "")
        s["min"] = int((s["t"] - launch).total_seconds() // 60) + 1
        samples.append(s)
    if not samples:
        sys.exit("analyze: no samples of pid {} in samples.csv.".format(pid))

    # --- Each state's last 3 minutes, and its 1-minute medians for the ceilings
    phases, ceil_minutes = {}, {}
    for state, _ in STATES:
        ss = [s for s in samples if s["mode"] == state]
        if not ss:
            continue
        end = ss[-1]["t"]
        last = [s for s in ss if s["t"] > end - timedelta(minutes=LAST_MINUTES)]
        phases[state] = {
            "n": len(last),
            "from": last[0]["min"],
            "to": last[-1]["min"],
            "med": {c: median(s[c] for s in last) for c in cols},
            "p95": {c: p95(s[c] for s in last) for c in cols},
        }
        by_min = {}
        for s in ss:
            by_min.setdefault(s["min"], []).append(s)
        # Only minutes spent mostly in this state count, so a switch doesn't make a half-minute bin.
        ceil_minutes[state] = {m: v for m, v in by_min.items() if len(v) >= per_minute / 2}

    # --- 1-minute medians from launch
    by_min = {}
    for s in samples:
        by_min.setdefault(s["min"], []).append(s)
    minutes = []
    for m in sorted(by_min):
        ss = by_min[m]
        modes = [s["mode"] for s in ss]
        row = {"min": m, "n": len(ss), "mode": max(set(modes), key=modes.count) or "-"}
        for c in cols:
            row[c] = median(s[c] for s in ss)
        minutes.append(row)
    with open(os.path.join(folder, "minutes.csv"), "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["min", "n", "mode"] + cols)
        for row in minutes:
            w.writerow([row["min"], row["n"], row["mode"]] + ["" if math.isnan(row[c]) else round(row[c], 3) for c in cols])

    # --- Minutes 11-20
    tail = [row for row in minutes if 11 <= row["min"] <= 20]
    fits = {c: fit([(row["min"], row[c]) for row in tail]) for c, _ in cfg["fit"]}
    settle_col, settle_label = cfg["settle"]
    slope, sigma, _ = fits[settle_col]
    m11 = next((row[settle_col] for row in tail if row["min"] == 11), math.nan)
    m20 = next((row[settle_col] for row in tail if row["min"] == 20), math.nan)
    tail_modes = sorted({row["mode"] for row in tail})

    # --- summary.md
    L = []
    stamp = meta.get("stamp", os.path.basename(os.path.normpath(folder)))
    L.append("# xReveal {} samples, {}".format(cfg["name"], stamp))
    L.append("")
    L.append("- **{}:** {}  ".format(cfg["name"], meta.get("os_version", "?")))
    L.append("- **CPU:** {}, {} logical; **RAM:** {} GB  ".format(meta.get("cpu", "?"), meta.get("logical_cpus", "?"), meta.get("ram_gb", "?")))
    L.append("- **GPU:** {}  ".format("; ".join(meta.get("gpus") or ["?"])))
    L.append("- **Monitors:** {}  ".format("; ".join(meta.get("monitors") or ["?"])))
    L.append("- **xReveal:** {}; pid {}, launched {}  ".format(meta.get("xreveal", "?"), pid, launch.strftime("%H:%M:%S")))
    L.append("- **Sampler:** {}, every {:g} s, {} samples of pid {} (min {}-{})  ".format(
        meta.get("sampler", "?"), interval, len(samples), pid, samples[0]["min"], samples[-1]["min"]))
    for label, key in (("at the start", "setup_start"), ("at the end", "setup_end")):
        st = meta.get(key)
        if st:
            L.append("- **setup.toml {}:** window mode {}, stats overlay {}, video format {}  ".format(
                label, st.get("window_mode") or "?", st.get("stats_shown") or "?", st.get("video_format") or "?"))
    for state, title in STATES:
        seen = {}
        for s in samples:
            if s["mode"] == state:
                k = "{} on {}".format(s["window"], s["display"]) if s["display"] else s["window"]
                seen[k] = seen.get(k, 0) + 1
        if seen:
            L.append("- **{} window:** {}  ".format(title, "; ".join("{} ({} samples)".format(k, n) for k, n in sorted(seen.items(), key=lambda kv: -kv[1]))))
    other = sum(1 for s in samples if s["mode"] not in ("windowed", "fullscreen"))
    if other:
        L.append("- **No window on screen** (minimised, hidden or another Space) in {} samples.  ".format(other))
    if main.get("exited"):
        code = main.get("exit_code")
        L.append("- **xReveal exited** at {}{}.  ".format(parse_time(main["exited"]).strftime("%H:%M:%S"), "" if code in (None, "") else " (exit code {})".format(code)))
    for p in procs[1:]:
        L.append("- Note: another xreveal process, pid {}, launched {}, was sampled too; it isn't in this summary.  ".format(p["pid"], parse_time(p["start"]).strftime("%H:%M:%S")))
    for note in meta.get("notes") or []:
        L.append("- Note: {}  ".format(note))
    if L[-1].endswith("  "):
        L[-1] = L[-1][:-2]

    shown = [(st, t) for st, t in STATES if st in phases]
    L += ["", "## Budgets (medians; p95 in brackets)", ""]
    if shown:
        L.append("| | " + " | ".join("{} (last {} min: min {}-{})".format(t, LAST_MINUTES, phases[st]["from"], phases[st]["to"]) for st, t in shown) + " | Budget |")
        L.append("|---|" + "---|" * len(shown) + "---|")
        for c, label, dec, unit, budget, limit in cfg["rows"]:
            cells = []
            verdicts = []
            for st, _ in shown:
                med, hi = phases[st]["med"][c], phases[st]["p95"][c]
                cells.append("{} ({})".format(fmt(med, dec, unit), fmt(hi, dec)))
                if limit is not None and not math.isnan(med):
                    verdicts.append(med <= limit)
            if verdicts:
                budget += ": ok" if all(verdicts) else ": OVER"
            L.append("| {} | {} | {} |".format(label, " | ".join(cells), budget))
    else:
        L.append("No samples with xReveal's window on screen, so no Windowed or Fullscreen medians.")

    L += ["", "## Memory ceilings (spec §4: the highest 1-minute median in each state, whole run)", ""]
    L.append("| | " + " | ".join("{} highest (minute)".format(t) for _, t in STATES) + " | Ceiling | Verdict |")
    L.append("|---|---|---|---|---|")
    for c, label, limits in cfg["ceilings"]:
        cells, verdict = [], []
        for st, _ in STATES:
            mins = ceil_minutes.get(st) or {}
            meds = [(median(s[c] for s in v), m) for m, v in mins.items()]
            meds = [(v, m) for v, m in meds if not math.isnan(v)]
            if not meds:
                cells.append("not seen")
                continue
            v, m = max(meds)
            cells.append("{} ({})".format(fmt(v, 1, " MB"), m))
            verdict.append("{} {}".format(st, "ok" if v <= limits[st] else "OVER"))
        L.append("| {} | {} | <= {} / {} MB | {} |".format(label, " | ".join(cells), limits["windowed"], limits["fullscreen"], ", ".join(verdict) or "n/a"))
    L.append("")
    L.append("The ceilings hold for the reference setup only (spec §4: the Windows PC's 3440x1440 monitor with a 1711x974 window; the MacBook's built-in screen with a 1280x887 pt window; overlay on). Check the window lines above.")

    L += ["", "## 1-minute medians from launch", ""]
    L.append("| min | mode | " + " | ".join(t for _, t, _ in cfg["minutes"]) + " |")
    L.append("|---|---|" + "---|" * len(cfg["minutes"]))
    for row in minutes:
        L.append("| {} | {} | {} |".format(row["min"], row["mode"], " | ".join(fmt(row[c], d) for c, _, d in cfg["minutes"])))

    L += ["", "## Minutes 11-20 (after the warm-up)", ""]
    L.append("| | Slope | Wobble (sigma) of 1-min medians around the line | Max - min |")
    L.append("|---|---|---|---|")
    for c, label in cfg["fit"]:
        sl, sg, rg = fits[c]
        L.append("| {} | {} MB/h | {} MB | {} MB |".format(label, fmt(sl, 2), fmt(sg, 2), fmt(rg, 2)))
    L.append("")
    if math.isnan(slope) or math.isnan(m11) or math.isnan(m20):
        settled = "n/a (no minutes 11 and 20 in the run)"
    elif abs(slope) < SETTLE_SLOPE and abs(m20 - m11) < SETTLE_STEP:
        settled = "yes"
    else:
        settled = "NO: look at the table"
    L.append("- **Settled by minute 10 ({}):** {} (rule: slope under {:g} MB/h and minutes 11 and 20 within {:g} MB).".format(settle_label, settled, SETTLE_SLOPE, SETTLE_STEP))
    holds = "n/a" if math.isnan(sigma) else ("holds" if sigma < SOAK_SIGMA else "DOES NOT HOLD")
    L.append("- **2-hour soak assumption (sigma < ~{:g} MB):** {} sigma = {} MB -> {}.".format(SOAK_SIGMA, settle_label, fmt(sigma, 2), holds))
    if len(tail_modes) > 1:
        L.append("- Note: the window state changed within minutes 11-20 ({}), so the line mixes states.".format(", ".join(tail_modes)))
    L.append("- Ten points is a rough sigma; it is a check of the assumption, not a leak verdict.")

    with open(os.path.join(folder, "summary.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(L) + "\n")
    return os.path.join(folder, "summary.md")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    print("analyze: wrote " + analyze(sys.argv[1]))
