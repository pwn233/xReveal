# Driving xReveal unattended

Prototype for [#196](https://github.com/pwn233/xReveal/issues/196) (map [#190](https://github.com/pwn233/xReveal/issues/190)). The verification cards have to run every #197 scenario for hours on both OSes, with no one at the machine. They must:
- toggle the overlay, Fullscreen and the drawer;
- hover the update button;
- move the window between monitors;
- switch Audio › Output, Video format and Source;
- launch and quit;

and know when each step has *finished* (#197's recovery limits). The options were:
1. **OS input automation** (SendInput on Windows, CGEvent / AppleScript on macOS);
2. **a local control channel** into xReveal;
3. **a built-in scenario mode** (`--soak <file>`) that xReveal plays itself.

Branch `prototype/driver-196`, never merged. What was **run** is marked *run here*: MacBook Pro M3, macOS, built-in 3024×1964 screen, no capture card plugged in, so xReveal showed "USB2 Video isn't connected". Everything else is from the code and from the OS vendors' docs.

---

## Summary

| | OS input automation | Local control channel (prototyped) | Built-in scenario mode |
|---|---|---|---|
| **Path the input takes** | The real one: OS → window → GPUI | GPUI's own input path from the keymap down (`Window::dispatch_keystroke`, `dispatch_event`). Skips only the OS's event translation | The same as the channel, but the steps live inside xReveal |
| **One script for both OSes** | No. Different APIs, coordinates and focus rules on each | Yes. stdin/stdout, the same commands and the same JSON state | Yes (one scenario file) |
| **Permissions** | macOS: **Accessibility** for the app that runs the script. Windows: the screen must stay unlocked, with xReveal in front | None | None |
| **Observing "step done"** | Not by itself. `setup.toml` is saved 1 s after a change and doesn't hold the drawer, the screen state or fps. Window frames are OS calls. The overlay needs OCR | `state` gives one JSON line: drawer, overlay, Window mode, fullscreen, bounds, screen, source status, fps, dropped/late | Inside xReveal; it has to write a results file |
| **Added to xReveal** | Nothing | ~100 lines (`src/drive.rs`) + ~30 in `app.rs`, behind a feature and `XREVEAL_DRIVE=1` | More: a step language, timing and judging, in the measured process |
| **Survives an xReveal crash** | Yes | Yes (the judge is outside) | No: the judge dies with it |
| **Every #197 scenario** | Yes, through keys and the mouse | Yes, through the same keys (drawer navigation reaches every selector) | Yes |

**Recommendation: the local control channel**, with two changes before the cards use it:
1. Compile it into **release builds**, active only when `XREVEAL_DRIVE=1` (like `XREVEAL_SMOKE_TEST`), so the cards test the shipped exe. Read commands only from stdin, so there's no socket and no network surface, and only the process that launched xReveal can drive it.
2. Replace the 10 ms poll with an awaited channel, so a driven xReveal is idle during the 2-hour soak.

Keep OS input for a short **realism check**: a few real keypresses per card, not the 200-cycle batches. The OS layer is what the hands-on passes (#160, #118) already cover, and the leaks and races #197 is after live in xReveal's handlers, capture restarts and swap chains, which the channel reaches.

---

## The prototype (branch `prototype/driver-196`)

- `src/drive.rs`, behind `--features drive` and `XREVEAL_DRIVE=1`. It reads commands from stdin and prints one `drive:` answer line per command:
  - `key <k>`: one of xReveal's own bindings (`tab`, `escape`, `f3`, `f11`, `ctrl-cmd-f`, the arrows, `enter`, `space`), sent through `Window::dispatch_keystroke`. Key-binding matching, focus and the action handlers run as for a real key. `Keystroke::parse` needs a `&'static str`, so keys are looked up in a fixed list rather than leaking a string per command, which a per-cycle leak check would see.
  - `move x y` / `click x y`: `PlatformInput::MouseMove` (plus down and up), through `Window::dispatch_event`, so hit-testing, hover and click handlers run.
  - `state`: one JSON line from `XReveal::drive_state`.
  - `quit`.
- Every command is dispatched **outside** any entity update (`AsyncWindowContext::update`), because the action handlers update `XReveal` themselves.
- `scripts/driver/drive.py`: launches the build, then runs overlay, drawer open → next tab → Esc, and Fullscreen on/off for N cycles. Each step polls `state` until it's reached or its #197 limit passes, and the script ends with a mouse move and `quit`. The same script is meant for Windows (it presses `f11` there).
- `scripts/driver/ax-check.swift`: asks macOS whether this process may post input events.

### Results (*run here*)

| Step | Reached | Slowest |
|---|---|---|
| Launch → channel ready | | 0.23–1.34 s (the first run was cold) |
| Overlay (F3) | 5/5 | 16 ms |
| Drawer open (Tab) | 5/5 | 13 ms |
| Next tab (→) | 5/5 | – (`state` doesn't report the tab yet) |
| Drawer close (Esc) | 5/5 | 60 ms |
| Fullscreen on (Ctrl+Cmd+F) | 5/5 | 45 ms |
| Fullscreen off | 5/5 with 1.5 s between toggles; **2/5 with 0.5 s** | 32 ms |
| Mouse move | the pointer reached GPUI (`mouse` = the sent point) | |
| `quit` | exit code 0 | |

**Finding: on macOS, the `fullscreen` flag isn't a "settled" signal.** `window.is_fullscreen()` turns true when macOS *starts* its slide into the Space. A toggle sent 0.5 s later, during the slide, was ignored three times in five, and the window stayed in Fullscreen. A paced cycle has to wait until the switch has finished: `state` should report it, for example from `windowDidEnterFullScreen` / `windowDidExitFullScreen`, or the app's own `FULLSCREEN_SWITCH` timer. #197's rapid burst (20 actions 100 ms apart) will hit exactly this. Whether xReveal should queue or drop a toggle sent mid-slide is a product question for that card, not a driver bug.

`setup.toml` was backed up before the runs and restored afterwards. The runs had moved the window by 20 pt and flipped the overlay.

### To add before the cards use it

- **`state`:** the drawer's tab and highlighted row, so a script can move to Source › Video format, Display › Monitor or Audio › Output with the arrow keys and know where it is. Also "a fullscreen switch is in progress", whether the update tip is showing, the current Source and format, and the audio output in use (not just the saved choice). Plus a count of presented frames, so "the picture is back" doesn't depend on fps averaging.
- **Await, don't poll:** the prototype checks stdin every 10 ms. That wakes the main thread 100 times a second, which would show up in the soak's CPU.
- **Windows:** the drawer and the overlays are separate windows there (§3.5). A `move`/`click` meant for the drawer has to go to the drawer's window, so the command needs a target window or a rule for picking it.

---

## OS input automation

### macOS (*run here*: the permission check only)

- Posting events with `CGEventPost` (or `osascript` / System Events) needs **Accessibility** for the process that posts them, which is the app the script runs in. *Run here:* `AXIsProcessTrusted()` and `CGPreflightPostEventAccess()` are both **false** for this session (launched from T3 Code). Without the permission, events are dropped silently, so the spike went no further. To try it, a person has to open **System Settings › Privacy & Security › Accessibility** and turn on the host app (here **T3 Code (Nightly)**; on the card it would be Terminal or whatever runs the script). `osascript` also asks for **Automation › System Events**.
- Keyboard events go to the **frontmost** app, so the script has to bring xReveal to the front and keep it there for hours. A notification, an update prompt or the screen saver steals it. The display has to stay awake (`caffeinate -d`).
- Mouse events use global screen coordinates in points. The script has to work out where the update button is from the window frame, the drawer side and the Density.

### Windows (from the docs, not run)

- `SendInput` puts events into the input stream of whichever thread has **foreground focus**. It is blocked by UIPI for windows of a higher integrity level (not an issue: same user) and **does nothing while the workstation is locked** or the session is disconnected (a remote desktop that's minimised, or a locked screen). The card would need the lock screen, the screen saver and display sleep turned off, and an interactive session that stays signed in.
- `SetForegroundWindow` only works when the caller is allowed to set the foreground (the foreground lock). A script started from a console usually can't bring xReveal to the front after something else took focus. The usual workarounds (`AttachThreadInput`, a fake Alt press) are fragile.
- xReveal on Windows has three top-level windows (main, drawer, overlay; the #194 baseline even saw two titled ones). Keys go to whichever has focus, and the drawer window has key handling of its own.
- DPI scaling changes what mouse coordinates mean. #194's PC had a 3440×1440 monitor and a 2560×1440 one.

### What OS input can't do on its own

It can press keys but can't see the result. It still needs xReveal to report the drawer state, the screen ("No signal", "Unplugged", "Blocked"), fps and which Source and format are showing. Window frames are available from the OS (`CGWindowListCopyWindowInfo`, `GetWindowRect`), as the #194 script used. So **OS input needs a status channel too**, and once xReveal has one, sending the commands through it as well costs little.

---

## Built-in scenario mode

xReveal would read a file of steps (`--soak scenario.toml`) and play it, timing each step against its limit and writing a results file. It could reuse the same `dispatch_keystroke` / `dispatch_event` calls as the channel, but it adds:
- a step language (waits, loops, conditions), timing, and writing results, all inside the process being measured. Its own allocations and timers show up in the soak's memory and CPU;
- a judge that dies with xReveal, so crash capture (#193) needs an outside watcher anyway;
- a second place to change when a scenario changes (the file format and the app), instead of a script.

The channel covers the same ground with the logic outside xReveal. No reason found to prefer the scenario mode.

---

## Where the channel hooks in

| Piece | Where | Size |
|---|---|---|
| The `drive` feature | `Cargo.toml` | 3 lines |
| The module | `src/main.rs` (`mod drive`), `src/drive.rs` | ~100 lines |
| Start | end of `XReveal::new` (`src/app.rs`) | 4 lines |
| State | `XReveal::drive_state` (`src/app.rs`) | ~25 lines; grows with the additions above |

Every scenario in #197 goes through keys xReveal already has (`src/app.rs:21` actions; `src/drawer/nav.rs`: ↑/↓ between settings, ←/→ to change one, which steps through a selector's entries), so the channel needs no per-scenario code. The one exception is the update tip, which needs the pointer. Launch/quit is the script starting the exe and sending `quit` (or killing it, for crash runs).

### Not checked here

- Selector switching (Video format, Source, Audio output, Monitor) through the channel: no card and one screen. The keys are the same ones the drawer already handles.
- The **Windows** GUI-subsystem release exe with redirected stdin/stdout: a parent that passes pipe handles (Python's `subprocess`, PowerShell's `Start-Process -Redirect…`) should reach a GUI-subsystem process. This has to be confirmed on the PC.
- Whether the update tip shows when the pointer arrives through `dispatch_event` rather than from the OS (hover is GPUI's, so it should).
