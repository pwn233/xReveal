# Samplers for the verification runs

External samplers for xReveal's CPU, GPU, memory, handles and threads (#207, decided in #191). Each
writes one CSV row every 5 s, starts before xReveal, keeps going after it exits, and needs no admin.
When stopped, each writes `summary.md` in the format of the first Windows baseline (#194).

| File | What it does |
|---|---|
| `sample-windows.ps1` | Windows sampler: `typeperf` on `Process V2`, `GPU Engine` and `GPU Process Memory`. `-Analyze <folder>` writes the summary again. |
| `sample-macos.py` | macOS sampler: `proc_pid_rusage`, `proc_pidinfo`, the IORegistry GPU counter, `footprint` at the start and end. `--analyze <folder>` writes the summary again. |
| `analyze.py` | The summary for a folder from either OS. `sample-macos.py` uses it; on Windows, `sample-windows.ps1` has the same rules in PowerShell so the PC needs no Python. Change both together. |
| `card-macos.py` | The macOS verification card (#220): runs its steps in order, judges its 14 rows and writes the run folder (#222). |

## Before a run, on either OS

- Quit xReveal. Set the run's window mode, window and stats overlay in `setup.toml`, not with key
  presses: F3 and F11 are remembered, so a "press F3" step can hide an overlay that is already on (#194).
- The memory ceilings in spec §4 hold only for the reference setup: the Windows PC's 3440×1440
  monitor with a 1711×974 window, the MacBook's built-in screen with a 1280×887 pt window, overlay on.
  The summary lists the window sizes it saw.

## Windows (the Windows PC)

In a PowerShell window, from the folder holding the script:

```powershell
powershell -ExecutionPolicy Bypass -File .\sample-windows.ps1              # waits for xreveal.exe; Ctrl+C stops
powershell -ExecutionPolicy Bypass -File .\sample-windows.ps1 -Minutes 25  # stops by itself
powershell -ExecutionPolicy Bypass -File .\sample-windows.ps1 -ProcessId 1234
powershell -ExecutionPolicy Bypass -File .\sample-windows.ps1 -Analyze .\xreveal-samples-20261128-120122
```

Then start xReveal. The script starts `typeperf` 10 s after xReveal launches (its GPU engine counters
exist once it has drawn), checks once a minute for new GPU engines, and restarts `typeperf` if any
appear. Windows 11 is needed for `Process V2`.

The folder `xreveal-samples-<time>` holds:
- `samples.csv`: one row per sample (columns listed at the top of the script), MB = MiB as in Task Manager;
- `typeperf-<pid>-<n>.csv` and `counters-<pid>-<n>.txt`: typeperf's raw output, which `-Analyze` rebuilds `samples.csv` from;
- `windows.csv`: the window state every 5 s; `meta.json`: the machine, the monitors and `setup.toml` at the start and end;
- `summary.md` and `minutes.csv`.

## macOS (the MacBook)

In Terminal, from the repository:

```sh
scripts/verify/sample-macos.py                 # waits for xreveal; Ctrl+C stops
scripts/verify/sample-macos.py --minutes 25
scripts/verify/sample-macos.py --pid 1234
scripts/verify/sample-macos.py --analyze xreveal-samples-20261128-120122
```

It needs only the system Python (`/usr/bin/python3`, from the Xcode command-line tools) and asks for
no permission: window sizes come from `CGWindowListCopyWindowInfo` without titles, which needs no
Screen Recording access.

The folder holds `samples.csv`, `meta.json`, `footprint-start-<pid>.txt` and `footprint-end-<pid>.txt`
(the breakdown, for when memory grew), `summary.md` and `minutes.csv`.

**GPU on macOS is a hint.** The sampler's `gpu_pct` is the rate of the undocumented
`AGXDeviceUserClient` `accumulatedGPUTime` counter, taken as nanoseconds. Against a Metal System
Trace of the same 10 s it reads 1.50–1.55× high (#200), so the summary gives it no verdict. The 10%
budget is judged on xReveal's own figure instead: `gpu_pct` in the driver's `state` (#218), the
video pass's `MTLCommandBuffer` GPU time over the last 5 s, within 0.15 points of the trace.

## What the summary says

- **Machine:** OS build, CPU, GPU, RAM, monitors and refresh rates, xReveal's version, `setup.toml`.
- **Budgets:** for Windowed and for Fullscreen, the median and p95 of the last 3 minutes in that state,
  against spec §4: CPU ≤ 5% of the machine, GPU ≤ 10%.
- **Memory ceilings** (spec §4, #195): the highest 1-minute median in each state over the whole run.
  Windows: private working set ≤ 40 MB, dedicated GPU memory ≤ 120 MB Windowed / 290 MB Fullscreen.
  macOS: footprint ≤ 220 MB Windowed / 250 MB Fullscreen (confirmed by #201).
- **1-minute medians from launch**, with the window state of each minute.
- **Minutes 11–20** (#192): the least-squares slope, the wobble (σ) of the 1-minute medians around
  the line, and max − min. *Settled by minute 10* means a slope under 6 MB/h and minutes 11 and 20
  within 3 MB. The 2-hour soak assumes σ < ~1.6 MB. Windows judges Private Bytes, macOS the footprint.

The state comes from xReveal's own windows: **fullscreen** when one covers its monitor (on a notched
MacBook, full width down to the bottom with no menu bar), **windowed** when one is on screen, **none**
when it is minimised, hidden or in another Space.

When waiting by name, a sampler picks up the next xReveal after one exits; the summary covers the
first one and lists the rest. A process that isn't the sampler's child has no readable exit code on
macOS; on Windows the script keeps a handle so it can record it.

## The macOS card runner

`card-macos.py` runs the macOS verification card (#220) on the MacBook with the PS5 idle on the
Reference card: build the app first with `scripts/dev-app.sh --release` (it launches it: quit it),
quit xReveal, then in Terminal, from the repository:

```sh
/usr/bin/python3 -I scripts/verify/card-macos.py               # the whole card, about 4½ hours
/usr/bin/python3 -I scripts/verify/card-macos.py --quick       # every step, shortened (~20 min)
/usr/bin/python3 -I scripts/verify/card-macos.py --no-manual   # skip the unplug/replug by hand
```

Other options: `--out DIR` (default `~/Documents/xreveal-card-macos-<date>-<time>/`), `--app PATH`
(default `target/dev-app/xReveal.app`), `--no-sleep`, `--audio-output NAME` (the second wired
output, when `system_profiler` doesn't find it).

- **Before and after:** it backs up `setup.toml` and sets the card's values (Windowed, a 1280×855
  window, the overlay on, sound on). The backup is put back byte for byte at the end, even after a
  failure or Ctrl+C; `run.log` says so. It refuses to start while xReveal is running.
- **Keep away from the Mac.** xReveal comes to the front before Fullscreen keys (#200). Only the
  unplug/replug steps ask for a person, on the terminal; `--no-manual` marks them not run.
- **Sleep/wake** needs the card's one-time sudoers entry for `pmset sleepnow` and
  `pmset schedule wake`; without it the row is not run. **Audio output** needs a second wired
  output (a USB audio adapter; the card's own sound device doesn't count); without one the batch
  is not run.
- **The drawer is driven by its keys** (`tab`, `up`, `down`, `left`, `right`): Video format and
  Source › Input step one entry per press, each waiting for its picture. On the Reference card
  720p60 is six presses from 1080p50, so a Video format cycle passes the 1080p rates between.
- **`--quick`** tests the runner, not xReveal: 2-minute soak, 1 + 5 cycles per batch, 3 launches.
  Its row 6 comes out inconclusive (too few minutes for a slope).

The run folder holds `run.log` (each step, and every driver command and answer, timestamped), the
sampler's `samples.csv`, `minutes.csv` and `summary.md`, `states.csv` (the driver's `state` every
5 s: `gpu_pct`, xReveal's share, frames), `batches.json` (per batch: footprint before and after,
kB per cycle, the slowest recovery, frames after, crashes) and `card.md`, the card's table with
the measured values and ✅/❌/not run/inconclusive. Post `card.md` on #220.

What `state` can't show is said in `card.md`'s notes: the update tip, the crash note bar and What's
new aren't in `state` (What's new counts as closed when `whats-new-seen` is saved again after
Got it), and an Audio output switch is judged on `audio_output`, not on hearing the sound.
