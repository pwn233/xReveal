# Samplers for the verification runs

External samplers for xReveal's CPU, GPU, memory, handles and threads (#207, decided in #191). Each
writes one CSV row every 5 s, starts before xReveal, keeps going after it exits, and needs no admin.
When stopped, each writes `summary.md` in the format of the first Windows baseline (#194).

| File | What it does |
|---|---|
| `sample-windows.ps1` | Windows sampler: `typeperf` on `Process V2`, `GPU Engine` and `GPU Process Memory`. `-Analyze <folder>` writes the summary again. |
| `sample-macos.py` | macOS sampler: `proc_pid_rusage`, `proc_pidinfo`, the IORegistry GPU counter, `footprint` at the start and end. `--analyze <folder>` writes the summary again. |
| `analyze.py` | The summary for a folder from either OS. `sample-macos.py` uses it; on Windows, `sample-windows.ps1` has the same rules in PowerShell so the PC needs no Python. Change both together. |

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

**GPU on macOS is a hint, not a measurement yet.** `gpu_pct` is the rate of the undocumented
`AGXDeviceUserClient` `accumulatedGPUTime` counter, taken as nanoseconds. Until #200 calibrates it
against a Metal System Trace, don't judge the 10% budget on it.

## What the summary says

- **Machine:** OS build, CPU, GPU, RAM, monitors and refresh rates, xReveal's version, `setup.toml`.
- **Budgets:** for Windowed and for Fullscreen, the median and p95 of the last 3 minutes in that state,
  against spec §4: CPU ≤ 5% of the machine, GPU ≤ 10%.
- **Memory ceilings** (spec §4, #195): the highest 1-minute median in each state over the whole run.
  Windows: private working set ≤ 40 MB, dedicated GPU memory ≤ 120 MB Windowed / 290 MB Fullscreen.
  macOS: footprint ≤ 220 MB Windowed / 250 MB Fullscreen (provisional until #201).
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
