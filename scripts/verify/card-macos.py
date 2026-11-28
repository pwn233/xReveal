#!/usr/bin/python3 -I
"""Run the macOS verification card (#220) and write its run folder (#222).

    /usr/bin/python3 -I scripts/verify/card-macos.py                # the whole card, about 4½ hours
    /usr/bin/python3 -I scripts/verify/card-macos.py --quick        # every step, shortened (~20 min)
    /usr/bin/python3 -I scripts/verify/card-macos.py --no-manual    # skip the unplug/replug by hand
    /usr/bin/python3 -I scripts/verify/card-macos.py --out DIR      # run folder (default below)

It runs the card's steps in order on the signed .app (default `target/dev-app/xReveal.app`, built by
`scripts/dev-app.sh --release`), drives it through the driver (#204) and samples it with
`sample-macos.py` (#207). The person only does the unplug/replug, when asked.

- Before: backs up the person's `setup.toml` and sets the card's values (#194). It puts the backup
  back at the end, byte for byte, even after a failure or Ctrl+C.
- The driver: `XREVEAL_DRIVE=1`, commands through a FIFO the runner holds read-write (`O_RDWR`,
  written with `os.write`: a buffered writer stalls, and opening the writer after `open` deadlocks).
  Answers are read from xReveal's stdout file. Each step waits for `state` to settle; recoveries are
  timed from `state`. xReveal is brought to the front before keys (#200).
- The drawer's Audio output, Video format and Source are picked with its own keys (`tab`, `up`,
  `down`, `left`, `right`; src/drawer/nav.rs), never click positions.
- Sleep/wake needs the card's one-time sudoers entry for `pmset`; without it that step is "not run".
  Without a second wired output (a USB audio adapter), the Audio output batch is "not run".

The run folder (`~/Documents/xreveal-card-macos-<date>/`) holds `run.log` (every driver command and
answer, timestamped), the sampler's `samples.csv`, `minutes.csv` and `summary.md`, `states.csv`
(the driver's `state` every 5 s), `batches.json` and `card.md` (the card's table, filled in).
"""

import argparse
import csv
import json
import math
import os
import re
import signal
import subprocess
import sys
import time
from datetime import datetime

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # no __pycache__ next to the scripts
import analyze  # noqa: E402  (median, fit, parse_time: the sampler's own rules)

SETTINGS = os.path.expanduser("~/Library/Application Support/xReveal")
SETUP = os.path.join(SETTINGS, "setup.toml")
MARKER = os.path.join(SETTINGS, "running.toml")
PANIC = os.path.join(SETTINGS, "last-panic.txt")
XLOG = os.path.join(SETTINGS, "xreveal.log")
REPORTS = os.path.expanduser("~/Library/Logs/DiagnosticReports")

# The card's setup.toml values (#220 Setup, #194): (table, key, TOML value). "" is the top level.
CARD_VALUES = [
    ("", "window-mode", '"windowed"'),
    ("", "sound-skipped", "false"),
    ("window", "width", "1280.0"),
    ("window", "height", "855.0"),
    ("stats", "shown", "true"),
]
# What every 10th launch sets `whats-new-seen` back to, so What's new shows (#197).
OLD_VERSION = '"0.0.0"'

# Minutes, cycles and counts: the card (#220), and --quick to test the runner itself.
FULL = {"warm_up": 10, "soak": 120, "fs_settle": 5, "fs_measure": 10, "warm_cycles": 5, "cycles": 200,
        "window_s": 60, "sleeps": 10, "unplugs": 10, "launches": 50, "whats_new_every": 10}
QUICK = {"warm_up": 0.5, "soak": 2, "fs_settle": 0.5, "fs_measure": 1, "warm_cycles": 1, "cycles": 5,
         "window_s": 20, "sleeps": 2, "unplugs": 2, "launches": 3, "whats_new_every": 3}

PAUSE = 2.0          # each step waits for `state` to settle, then 2 s more (#197)
BURST, BURST_GAP = 20, 0.1   # rapid bursts: 20 actions 100 ms apart (#197)
POLL = 0.03          # how often `state` is asked while waiting for a step
EVERY = 5.0          # the 5 s cadence of states.csv (gpu_pct is a 5 s figure, #218)

# Budgets and limits, with where they come from.
CPU_MAX, GPU_MAX, SHARE_MAX = 5.0, 10.0, 8.3              # §4
LAUNCH_MEDIAN_MAX, LAUNCH_ANY_MAX = 1000.0, 1500.0         # §4, #220 row 4 (ms)
CEILINGS = {"windowed": 290.0, "fullscreen": 250.0}        # §4, #195, #201, #220 (MB): Windowed covers the UI animating
LEAK_MB_H, COUNT_PER_H = 1.0, 10.0                         # #192
LEAK_KB_CYCLE = 10.0                                       # #192, #197
FRAMES_MIN = 0.95                                          # #192: of the source rate
WAKE_LIMIT, REPLUG_LIMIT = 10.0, 5.0                       # #197 (s)

PASS, FAIL, NOT_RUN, INCONCLUSIVE, REPORTED = "✅", "❌", "not run", "inconclusive", "reported"

# The capture card's own sound device: a wired output, but not the second one the card means.
CARD_AUDIO = ("USB2 Digital Audio", "MACROSILICON")


def now_stamp():
    t = time.time()
    return time.strftime("%H:%M:%S", time.localtime(t)) + ".%03d" % int((t % 1) * 1000)


class Log:
    """run.log: what the runner does, and every driver command and answer, timestamped."""

    def __init__(self, path):
        self.f = open(path, "a", buffering=1, encoding="utf-8")

    def __call__(self, *parts):
        line = "{} {}".format(now_stamp(), " ".join(str(p) for p in parts))
        self.f.write(line + "\n")
        print(line, flush=True)

    def quiet(self, *parts):
        self.f.write("{} {}\n".format(now_stamp(), " ".join(str(p) for p in parts)))


class NoAnswer(Exception):
    """xReveal didn't answer a driver command: it has crashed, hung or quit."""


class CardStop(Exception):
    """The card can't go on (no picture, xReveal gone): what's left is not run."""


# --- setup.toml, edited line by line so every other key (and the person's formatting) stays

def set_toml(text, table, key, value):
    """`text` with `key = value` in `[table]` ("" for the top level), added when missing."""
    lines = text.split("\n")
    current, start, end = "", 0 if table == "" else None, None
    for i, line in enumerate(lines):
        head = re.match(r"\s*\[\[?([^\]]+)\]\]?\s*$", line)
        if head:
            if current == table and start is not None and end is None:
                end = i
            current = head.group(1).strip()
            if current == table and start is None:
                start = i + 1
            continue
        if current == table and re.match(r"\s*{}\s*=".format(re.escape(key)), line):
            lines[i] = "{} = {}".format(key, value)
            return "\n".join(lines)
    if start is None:  # no such table: add it at the end
        return text.rstrip("\n") + "\n\n[{}]\n{} = {}\n".format(table, key, value)
    if end is None:
        end = len(lines)
    # After the table's last key, before the blank lines that end it.
    at = end
    while at > start and lines[at - 1].strip() == "":
        at -= 1
    lines.insert(at, "{} = {}".format(key, value))
    return "\n".join(lines)


def get_toml(text, table, key):
    current = ""
    for line in text.split("\n"):
        head = re.match(r"\s*\[\[?([^\]]+)\]\]?\s*$", line)
        if head:
            current = head.group(1).strip()
        elif current == table:
            m = re.match(r"\s*{}\s*=\s*(.*?)\s*$".format(re.escape(key)), line)
            if m:
                return m.group(1)
    return None


def read_bytes(path):
    try:
        with open(path, "rb") as f:
            return f.read()
    except FileNotFoundError:
        return None


def write_bytes(path, data):
    with open(path, "wb") as f:
        f.write(data)


def changed_lines(before, after):
    """The lines of `after` that differ from `before` (setup.toml is small)."""
    a, b = before.decode("utf-8", "replace").split("\n"), after.decode("utf-8", "replace").split("\n")
    return sorted(set(b) - set(a)) + ["(gone) " + line for line in sorted(set(a) - set(b))]


# --- processes

def xreveal_pids():
    r = subprocess.run(["pgrep", "-x", "xreveal"], stdout=subprocess.PIPE, universal_newlines=True)
    return {int(p) for p in r.stdout.split()}


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def wait_gone(pid, timeout):
    end = time.time() + timeout
    while time.time() < end:
        if not alive(pid):
            return True
        time.sleep(0.05)
    return not alive(pid)


def stop(pid):
    """SIGTERM, then SIGKILL: only for an xReveal the runner launched and can't quit."""
    for sig, wait in ((signal.SIGTERM, 5), (signal.SIGKILL, 5)):
        if not alive(pid):
            return
        try:
            os.kill(pid, sig)
        except ProcessLookupError:
            return
        wait_gone(pid, wait)


# --- the driver

class Driver:
    """One xReveal, launched with `open` and driven through a FIFO (#204, spec §5).

    `open` is a must: macOS gives the Camera and Microphone grants to the .app. The FIFO is held
    read-write by the runner before `open` starts, so neither side ever blocks opening it."""

    def __init__(self, run, name, env=()):
        self.run, self.name = run, name
        folder = os.path.join(run.folder, "xreveal-io")
        os.makedirs(folder, exist_ok=True)
        self.fifo = os.path.join(folder, name + ".fifo")
        stdout = os.path.join(folder, name + "-stdout.txt")
        stderr = os.path.join(folder, name + "-stderr.txt")
        if os.path.exists(self.fifo):
            os.remove(self.fifo)
        os.mkfifo(self.fifo)
        open(stdout, "w").close()
        self.wfd = os.open(self.fifo, os.O_RDWR)
        self.reader = open(stdout, encoding="utf-8", errors="replace")
        self.partial, self.queue, self.owed = "", [], 0
        self.last, self.last_kept, self.kept = None, 0.0, []
        before = xreveal_pids()
        command = ["open", "-n", "--env", "XREVEAL_DRIVE=1"]
        for e in env:
            command += ["--env", e]
        command += ["--stdin", self.fifo, "--stdout", stdout, "--stderr", stderr, run.app]
        self.launched = time.time()
        subprocess.check_call(command, timeout=30)
        self.pid = None
        end = time.time() + 15
        while time.time() < end and self.pid is None:
            new = xreveal_pids() - before
            self.pid = max(new) if new else None
            time.sleep(0.02)
        run.log(name, "launched", "pid", self.pid, "with " + " ".join(env) if env else "")
        if self.pid is None:
            raise CardStop("xReveal didn't start")

    def _answers(self):
        data = self.reader.read()
        if data:
            lines = (self.partial + data).split("\n")
            self.partial = lines.pop()
            self.queue += [line[len("drive: "):] for line in lines if line.startswith("drive: ")]
        while self.queue and self.owed:  # answers to commands that timed out
            self.queue.pop(0)
            self.owed -= 1

    def _next(self, timeout):
        end = time.time() + timeout
        while True:
            self._answers()
            if self.queue:
                return self.queue.pop(0)
            if time.time() > end or not alive(self.pid):
                return None
            time.sleep(0.005)

    def ready(self, timeout=30):
        answer = self._next(timeout)
        self.run.log.quiet("<", self.name, answer)
        if answer != "ready":
            raise CardStop("{}: no `drive: ready` (got {!r})".format(self.name, answer))
        return time.time() - self.launched

    def send(self, command, timeout=10.0):
        os.write(self.wfd, (command + "\n").encode())
        self.run.log.quiet(">", self.name, command)
        answer = self._next(timeout)
        self.run.log.quiet("<", self.name, answer if answer is not None else "(no answer)")
        if answer is None:
            self.owed += 1
            raise NoAnswer("{}: no answer to {!r}{}".format(self.name, command, "" if alive(self.pid) else " (xReveal is gone)"))
        return answer

    def state(self):
        s = json.loads(self.send("state"))
        self.last = s
        if time.time() - self.last_kept >= EVERY - 0.05:
            self.last_kept = time.time()
            self.run.keep(self, s)
        return s

    def until(self, check, limit, since=None):
        """Asks `state` until `check` holds: the seconds since `since` (default now), or None."""
        t0 = since or time.time()
        while True:
            s = self.state()
            if check(s):
                return time.time() - t0
            if time.time() - t0 > limit:
                return None
            time.sleep(POLL)

    def idle(self, seconds):
        """Leaves xReveal alone for `seconds`, asking `state` every 5 s."""
        end = time.time() + seconds
        while time.time() < end:
            self.state()
            time.sleep(max(0.0, min(EVERY - (time.time() - self.last_kept), end - time.time())))

    def quit(self, timeout=15):
        """Quits as Cmd+Q does: the seconds until the process was gone, or None."""
        t0 = time.time()
        try:
            self.send("quit")
        except NoAnswer:
            pass
        return time.time() - t0 if wait_gone(self.pid, timeout) else None

    def close(self):
        if self.pid and alive(self.pid):
            self.run.log(self.name, "still running: stopping pid", self.pid)
            stop(self.pid)
        os.close(self.wfd)
        self.reader.close()
        os.remove(self.fifo)


# --- what `state` says

FORMAT = re.compile(r"(\d+)\s*[×x]\s*(\d+)\s*@\s*([\d.]+)")


def source_rate(s):
    m = FORMAT.search(s.get("status") or "")
    return float(m.group(3)) if m else math.nan


def showing(fmt=None, name=None):
    """The live picture, of `name` (in the status) at `fmt` ("1920×1080@50")."""
    def check(s):
        if s["screen"] != "Picture" or (s.get("status") or "").startswith("Restarting"):
            return False
        status = s.get("status") or ""
        if name and name not in status:
            return False
        if fmt:
            m = FORMAT.search(status)
            if not m or "{}×{}@{:g}".format(m.group(1), m.group(2), round(float(m.group(3)))) != fmt:
                return False
        return True
    return check


def settled(fullscreen):
    return lambda s: s["fullscreen"] == fullscreen and not s["transition_in_progress"]


TABS = ["Source", "Display", "Colour", "Audio", "Interface"]  # Tab::ALL, src/drawer.rs


# --- the paced batches (#197): each cycle goes there and back, every step waits to settle + 2 s

class Batch:
    key = title = ""
    limit = 1.0

    def __init__(self, run):
        self.run = run
        self.d = run.main

    def prepare(self):
        """None, or why this batch can't run here."""
        return None

    def cycle(self):
        """One cycle: the recovery of each step (seconds, or None when it missed)."""
        raise NotImplementedError

    def finish(self):
        pass

    def burst_actions(self):
        """The burst's 20 commands and the check for the state the last one asked for."""
        raise NotImplementedError

    def step(self, command, check, pause=PAUSE):
        """Sends `command`, waits for `check`, then `pause`: the recovery in seconds, or None."""
        t0 = time.time()
        self.d.send(command)
        took = self.d.until(check, self.limit + 5, since=t0)
        if took is None or took > self.limit:
            self.run.log("  {}: {} -> {} (limit {:g} s)".format(self.key, command, "missed" if took is None else "%.2f s" % took, self.limit))
        time.sleep(pause)
        return took


class Overlay(Batch):
    key, title, limit = "overlay", "Overlay (F3)", 1.0

    def toggle(self):
        shown = self.d.state()["stats_shown"]
        return self.step("key f3", lambda s: s["stats_shown"] != shown)

    def cycle(self):
        return [self.toggle(), self.toggle()]

    def burst_actions(self):
        shown = self.d.state()["stats_shown"]  # 20 presses: back where it started
        return ["key f3"] * BURST, lambda s: s["stats_shown"] == shown


class UpdateTip(Batch):
    """The update button's tip shows on hover (spec §2.10). It sits at the foot of the drawer's rail."""
    key, title, limit = "tip", "Update tip (hover)", 1.0

    def prepare(self):
        self.run.open_drawer()
        s = self.d.state()
        width, height = s["bounds"][2], s["bounds"][3] - self.run.title_bar
        rail = 64.0  # compact density (theme.rs); the button is centred 11 px right of the rail's middle
        left = (self.run.setup_value("", "drawer-side") or '"right"').strip('"') == "left"
        x = rail / 2 + 11 if left else width - rail / 2 + 11
        self.button = (x, height - 21.5)          # 22 px button, 10.5 px from the bottom
        self.away = (width / 2, height / 2)
        return None

    def move(self, p, tip_open):
        """Moves the pointer and waits for the tip to open or close (`update_tip_open`); it closes
        300 ms after the pointer leaves (CLOSE_AFTER in update_button.rs)."""
        return self.step("move {:g} {:g}".format(*p), lambda s: abs(s["mouse"][0] - p[0]) < 1 and abs(s["mouse"][1] - p[1]) < 1
                         and s.get("update_tip_open") == tip_open)

    def cycle(self):
        self.run.open_drawer()   # closed for the before window
        return [self.move(self.button, True), self.move(self.away, False)]

    def finish(self):
        self.d.send("move {:g} {:g}".format(*self.away))
        self.run.close_drawer()

    def burst_actions(self):
        moves = [self.button, self.away] * (BURST // 2)
        last = moves[-1]
        return ["move {:g} {:g}".format(*p) for p in moves], lambda s: abs(s["mouse"][0] - last[0]) < 1 and s.get("update_tip_open") is False


class Drawer(Batch):
    key, title, limit = "drawer", "Drawer open/close, next tab", 1.0

    def prepare(self):
        self.run.home_cursor()  # → on the tab's title, where → switches tabs
        return None

    def cycle(self):
        opened = self.step("key tab", lambda s: s["drawer_open"])
        tab = self.d.state()["drawer_tab"]
        moved = self.step("key right", lambda s: s["drawer_tab"] != tab)
        closed = self.step("key escape", lambda s: not s["drawer_open"])
        return [opened, moved, closed]

    def burst_actions(self):
        tab = self.d.state()["drawer_tab"]
        actions = (["key tab", "key right", "key escape"] * BURST)[:BURST]
        rights = actions.count("key right")
        want = TABS[(TABS.index(tab) + rights) % len(TABS)] if tab in TABS else None
        open_at_end = actions[-1] != "key escape"
        return actions, lambda s: s["drawer_open"] == open_at_end and (want is None or s["drawer_tab"] == want)

    def finish(self):
        self.run.close_drawer()


class Fullscreen(Batch):
    key, title, limit = "fullscreen", "Fullscreen ↔ Windowed", 2.0   # 2 s on macOS: the slide

    def prepare(self):
        self.run.front()
        return None

    def cycle(self):
        return [self.step("key f11", settled(True)), self.step("key f11", settled(False))]

    def burst_actions(self):
        self.run.front()
        return ["key f11"] * BURST, settled(False)   # an even number: ends Windowed (#203)


class Picker(Batch):
    """A drawer selector (Output, Video format, Input), stepped with ←/→ on its row (nav.rs).

    ←/→ choose the next entry at once, so reaching one `steps` entries away passes the ones
    between: each press waits for its own picture (each is a change in its own right, judged by the
    same limit), and only the last one waits the 2 s more."""
    tab, row = "Source", 1
    there_key, back_key, steps = "key right", "key left", 1

    def to_row(self):
        self.run.go_to_row(self.tab, self.row)

    def there(self, s):
        raise NotImplementedError

    def back(self, s):
        raise NotImplementedError

    def moved(self, before):
        """An entry between: something else on screen, live."""
        status = before.get("status")
        return lambda s: s.get("status") != status and showing()(s)

    def walk(self, key, final):
        times = []
        for i in range(self.steps):
            if i == self.steps - 1:
                times.append(self.step(key, final))
            else:
                times.append(self.step(key, self.moved(self.d.state()), pause=0.0))
        return times

    def cycle(self):
        if not self.d.state()["drawer_open"]:   # closed for the before window
            self.to_row()
        return self.walk(self.there_key, self.there) + self.walk(self.back_key, self.back)

    def burst_actions(self):
        self.to_row()
        # Alternating ←/→ one step at a time: 20 actions end where they started.
        return [self.there_key, self.back_key] * (BURST // 2), self.back

    def finish(self):
        self.run.close_drawer()


class AudioOutput(Picker):
    key, title, limit = "audio", "Audio output, default ↔ second wired output", 1.0
    tab, row = "Audio", 2   # the tab's title, Input, Output (Microphone allowed: the card sets sound on)

    def prepare(self):
        name = self.run.adapter
        if not name:
            return "no second wired output (a USB audio adapter) found"
        self.name = name.lower()
        self.to_row()
        if self.d.state()["audio_output"] is not None:
            return "Audio › Output isn't the default at the start"
        if "audio" in self.run.found:
            self.steps = self.run.found["audio"]
            return None if self.steps else "{} isn't in Audio › Output".format(name)
        self.steps = None
        for n in range(1, 9):   # step through the outputs until the adapter is chosen
            output = self.d.state()["audio_output"]
            self.d.send("key right")
            self.d.until(lambda s: s["audio_output"] != output, 2)
            if self.there(self.d.last):
                self.steps = n
                break
            if self.d.last["audio_output"] == output:
                break   # the end of the list
        for _ in range(10):   # back to the default
            if self.d.state()["audio_output"] is None:
                break
            self.d.send("key left")
            time.sleep(0.5)
        self.run.found["audio"] = self.steps
        if self.steps is None:
            return "{} isn't in Audio › Output".format(name)
        self.run.log("  audio: {} is {} step(s) right of the default".format(name, self.steps))
        return None

    def there(self, s):
        return self.name in (s["audio_output"] or "").lower() and s["screen"] == "Picture"

    def moved(self, before):
        output = before.get("audio_output")
        return lambda s: s["audio_output"] != output and s["screen"] == "Picture"

    def back(self, s):
        return s["audio_output"] is None and s["screen"] == "Picture"


class VideoFormat(Picker):
    key, title, limit = "format", "Video format 1080p50 ↔ 720p60", 3.0
    tab, row = "Source", 2   # the tab's title, Input, Video format (Camera allowed)
    start, other = "1920×1080@50", "1280×720@60"

    def prepare(self):
        self.to_row()
        if not showing(self.start)(self.d.state()):
            return "the card isn't showing {} at the start".format(self.start)
        if self.run.format_steps is None:
            self.run.format_steps = self.run.find_steps(self.there_key, self.there, self.back_key, self.back) or 0
        self.steps = self.run.format_steps
        if not self.steps:
            return "{} isn't reachable with → from {}".format(self.other, self.start)
        return None

    def there(self, s):
        return showing(self.other, self.run.card)(s)

    def back(self, s):
        return showing(self.start, self.run.card)(s)


class Source(Picker):
    key, title, limit = "source", "Source, the card ↔ the FaceTime camera", 3.0
    tab, row = "Source", 1   # the tab's title, Input
    other = "FaceTime"

    def prepare(self):
        self.to_row()
        if "source" not in self.run.found:
            self.run.found["source"] = None
            for keys in (("key right", "key left"), ("key left", "key right")):
                if self.run.find_steps(keys[0], self.there, keys[1], self.back, most=1):
                    self.run.found["source"] = keys
                    break
        if self.run.found["source"]:
            self.there_key, self.back_key = self.run.found["source"]
            return None
        return "the {} camera isn't next to the card in Source › Input".format(self.other)

    def there(self, s):
        return showing(name=self.other)(s)

    def back(self, s):
        # Back on the card, at its default format: the Input list picks a card's default (spec §2.4).
        return showing(name=self.run.card)(s)

    def finish(self):
        self.run.restore_source()
        self.run.restore_format()
        self.run.close_drawer()


BATCHES = [Overlay, UpdateTip, Drawer, Fullscreen, AudioOutput, VideoFormat, Source]


# --- the run

class Run:
    def __init__(self, args):
        self.args = args
        self.plan = QUICK if args.quick else FULL
        self.app = os.path.abspath(args.app)
        self.folder = os.path.abspath(os.path.expanduser(args.out))
        os.makedirs(self.folder, exist_ok=True)
        self.log = Log(os.path.join(self.folder, "run.log"))
        self.main = None
        self.drivers = []
        self.sampler = None
        self.phases = []          # [name, start, end]
        self.phase_name = ""
        self.states = []          # state every 5 s, with its phase
        self.prev = {}            # pid → the last kept state, for the on-screen frame rate
        self.results = {}         # what each step found, for card.md and batches.json
        self.card = "USB2 Video"
        self.title_bar = 32.0
        self.format_steps = None   # → presses from 1080p50 to 720p60 in Video format
        self.found = {}            # what the drawer's lists look like, found once
        self.adapter = None
        self.started = time.time()

    # -- bookkeeping

    def phase(self, name):
        t = time.time()
        if self.phases and self.phases[-1][2] is None:
            self.phases[-1][2] = t
        if name:
            self.phases.append([name, t, None])
            self.log("phase:", name)
        self.phase_name = name

    def keep(self, driver, s):
        """One row of states.csv (every 5 s): what judges GPU, xReveal's share and frames."""
        t = time.time()
        share = s.get("share_ms") or [None, None]
        row = {"t": t, "who": driver.name, "phase": self.phase_name, "screen": s["screen"],
               "fullscreen": s["fullscreen"], "status": s.get("status"), "fps": s.get("fps"),
               "dropped": s.get("dropped"), "gpu_pct": s.get("gpu_pct"),
               "share_p50": share[0], "share_p99": share[1], "rate": source_rate(s), "on_screen": None}
        before = self.prev.get(driver.pid)
        if before and row["fps"] is not None and row["dropped"] is not None and before["dropped"] is not None:
            dropped = max(0, row["dropped"] - before["dropped"])
            row["on_screen"] = row["fps"] - dropped / max(0.001, t - before["t"])
        self.prev[driver.pid] = row
        self.states.append(row)

    def during(self, name):
        """The states.csv rows of the main session kept during phase `name`."""
        return [r for r in self.states if r["phase"] == name and r["who"] == "main"]

    def phase_at(self, t):
        for n, a, b in self.phases:
            if a <= t <= (b or time.time()):
                return n
        return None

    def span(self, name):
        for n, a, b in self.phases:
            if n == name:
                return a, b or time.time()
        return None

    # -- xReveal

    def front(self):
        """macOS ignores Fullscreen for an app in the background (#200)."""
        subprocess.call(["open", self.app])
        time.sleep(2)

    def setup_value(self, table, key):
        data = read_bytes(SETUP)
        return get_toml(data.decode("utf-8", "replace"), table, key) if data else None

    def open_drawer(self):
        if not self.main.state()["drawer_open"]:
            self.main.send("key tab")
            self.main.until(lambda s: s["drawer_open"], 2)
            time.sleep(0.5)  # the slide (320 ms)

    def close_drawer(self):
        if self.main.state()["drawer_open"]:
            self.main.send("key escape")
            self.main.until(lambda s: not s["drawer_open"], 2)
            time.sleep(0.3)

    def home_cursor(self):
        """The drawer's ↑/↓ cursor onto the tab's title (it stays where it was between opens)."""
        self.open_drawer()
        for _ in range(15):
            self.main.send("key up")
        self.close_drawer()

    def go_to_row(self, tab, row):
        """Opens the drawer on `tab` with the cursor on its `row`-th setting (0: the tab's title)."""
        d = self.main
        self.open_drawer()
        for _ in range(15):
            d.send("key up")
        for _ in range(len(TABS)):
            current = d.state()["drawer_tab"]
            if current == tab:
                break
            d.send("key right")   # on the title, → switches tabs; the cursor stays on the title
            d.until(lambda s: s["drawer_tab"] != current, 2)
        if d.state()["drawer_tab"] != tab:
            raise CardStop("couldn't reach the drawer's {} tab".format(tab))
        for _ in range(row):
            d.send("key down")

    def find_steps(self, there_key, there, back_key, back, most=12):
        """How many presses of `there_key` from `back` reach `there` (one at a time, each settled);
        then back again. None when it isn't reached within `most`."""
        d = self.main
        steps = None
        for n in range(1, most + 1):
            before = d.state().get("status")
            d.send(there_key)
            d.until(lambda s: s.get("status") != before and showing()(s), 8)
            self.log("  step {}: {}".format(n, d.last.get("status")))
            if there(d.last):
                steps = n
                break
        for _ in range(n):
            if back(d.state()):
                break
            before = d.last.get("status")
            d.send(back_key)
            d.until(lambda s: s.get("status") != before and showing()(s), 8)
        if not back(d.state()):
            raise CardStop("couldn't step back: {}".format(d.last.get("status")))
        return steps

    def restore_source(self):
        """The card back in Source › Input, after the Source batch or its burst."""
        d = self.main
        if showing(name=self.card)(d.state()):
            return
        self.go_to_row("Source", 1)
        back = (self.found.get("source") or ("key right", "key left"))[1]
        for _ in range(3):
            s = d.state()
            if showing(name=self.card)(s):
                return
            # From the camera, back; from "None" (the first entry), →.
            d.send(back if Source.other in (s.get("status") or "") else "key right")
            d.until(showing(name=self.card), 8)

    def restore_format(self):
        """The card back at 1080p50 (choosing the card in Source › Input picks its default, 720p60),
        one press at a time: each waits for its picture."""
        d = self.main
        if showing(VideoFormat.start, self.card)(d.state()) or not self.format_steps:
            return
        self.go_to_row("Source", 2)
        for _ in range(self.format_steps + 2):
            s = d.state()
            if showing(VideoFormat.start, self.card)(s):
                return
            m = FORMAT.search(s.get("status") or "")
            # The list runs from the biggest and fastest down (source.rs): 720p is after 1080p50.
            key = "key left" if m and (int(m.group(2)) < 1080 or float(m.group(3)) < 50) else "key right"
            d.send(key)
            d.until(lambda x: x.get("status") != s.get("status") and showing()(x), 8)
        if not showing(VideoFormat.start, self.card)(d.state()):
            self.log("  couldn't set 1080p50 back:", d.last.get("status"))

    # -- the steps, in the card's order

    def run(self):
        self.log("card-macos: {} run, folder {}".format("quick" if self.args.quick else "full", self.folder))
        if xreveal_pids():
            sys.exit("card-macos: quit xReveal first (it rewrites setup.toml).")
        self.facts()
        self.adapter = self.find_adapter()
        backup = read_bytes(SETUP)
        panic_before = os.path.exists(PANIC)
        if backup is not None:
            write_bytes(os.path.join(self.folder, "setup.toml.backup"), backup)
        try:
            self.set_card_setup(backup)
            try:
                self.main_session()
            except (CardStop, NoAnswer) as e:
                self.log("main session stopped:", e)
                self.results.setdefault("stopped", str(e))
            finally:
                self.end_main()
            self.launch_quit()
            self.crash_capture()
            self.last_clean_launch()
        except KeyboardInterrupt:
            self.log("interrupted (Ctrl+C)")
            self.results.setdefault("stopped", "interrupted")
        finally:
            for d in self.drivers:
                try:
                    d.close()
                except OSError:
                    pass
            if self.sampler and self.sampler.poll() is None:
                self.sampler.send_signal(signal.SIGTERM)
                self.sampler.wait(60)
            if not xreveal_pids() and os.path.exists(MARKER):
                os.remove(MARKER)   # left by an xReveal the runner had to stop
            if not panic_before and os.path.exists(PANIC):
                os.rename(PANIC, os.path.join(self.folder, "last-panic.txt"))   # the crash test's, kept here
            if backup is None:
                if os.path.exists(SETUP):
                    os.remove(SETUP)
            else:
                write_bytes(SETUP, backup)
            restored = read_bytes(SETUP) == backup
            self.log("setup.toml put back:", "byte-identical" if restored else "DIFFERENT")
            self.results["setup_restored"] = restored
        self.write_results()

    def facts(self):
        def out(cmd):
            r = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, universal_newlines=True)
            return r.stdout.strip()
        plist = os.path.join(self.app, "Contents", "Info.plist")
        self.results["facts"] = {
            "macos": "{} ({})".format(out(["sw_vers", "-productVersion"]), out(["sw_vers", "-buildVersion"])),
            "model": out(["sysctl", "-n", "hw.model"]),
            "chip": out(["sysctl", "-n", "machdep.cpu.brand_string"]),
            "version": out(["/usr/libexec/PlistBuddy", "-c", "Print :CFBundleShortVersionString", plist]),
            "commit": out(["git", "-C", REPO, "rev-parse", "--short", "HEAD"]),
            "built": time.strftime("%Y-%m-%d %H:%M", time.localtime(os.path.getmtime(os.path.join(self.app, "Contents", "MacOS", "xreveal")))),
            "power": out(["pmset", "-g", "batt"]).split("\n")[0].replace("Now drawing from ", "").strip("'"),
        }
        r = subprocess.run(["codesign", "-dvv", self.app], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, universal_newlines=True)
        self.results["facts"]["signed"] = next((line.split("=", 1)[1] for line in r.stdout.splitlines() if line.startswith("Authority=")), "not signed")
        self.log("facts:", json.dumps(self.results["facts"]))

    def find_adapter(self):
        """A second wired output: not built-in, not Bluetooth, AirPlay or virtual, not the card's own."""
        if self.args.audio_output:
            return self.args.audio_output
        r = subprocess.run(["system_profiler", "SPAudioDataType", "-json"], stdout=subprocess.PIPE, universal_newlines=True)
        try:
            devices = json.loads(r.stdout)["SPAudioDataType"][0]["_items"]
        except (ValueError, KeyError, IndexError):
            devices = []
        for dev in devices:
            transport = dev.get("coreaudio_device_transport", "")
            if not dev.get("coreaudio_device_output"):
                continue
            if not any(t in transport for t in ("usb", "thunderbolt", "pci", "firewire", "hdmi", "displayport")):
                continue
            if dev.get("_name") in CARD_AUDIO or dev.get("coreaudio_device_manufacturer") in CARD_AUDIO:
                continue
            self.log("second wired output:", dev.get("_name"), transport)
            return dev.get("_name")
        self.log("second wired output: none (outputs: {})".format(", ".join(d.get("_name", "?") for d in devices if d.get("coreaudio_device_output"))))
        return None

    def set_card_setup(self, backup):
        text = backup.decode("utf-8") if backup else "version = 1\n"
        for table, key, value in CARD_VALUES:
            text = set_toml(text, table, key, value)
        os.makedirs(SETTINGS, exist_ok=True)
        write_bytes(SETUP, text.encode("utf-8"))
        self.log("setup.toml: the card's values set ({})".format(", ".join("{}{} = {}".format(t + "." if t else "", k, v) for t, k, v in CARD_VALUES)))

    def main_session(self):
        """Steps 1 to 7 in one xReveal, sampled the whole time."""
        d = self.main = Driver(self, "main")
        self.drivers.append(d)
        d.ready()
        self.sampler = subprocess.Popen(
            ["/usr/bin/python3", "-I", os.path.join(HERE, "sample-macos.py"), "--pid", str(d.pid), "--out", self.folder, "--interval", "5"],
            stdout=open(os.path.join(self.folder, "sampler.log"), "w"), stderr=subprocess.STDOUT)
        self.front()
        if d.until(showing(), 20) is None:
            raise CardStop("no picture 20 s after launch: {} / {}".format(d.last["screen"], d.last["status"]))
        s = d.state()
        self.card = s["status"].split(" · ")[0].strip() or self.card   # "USB2 Video · 1920x1080 @ 50 · 420v"
        self.title_bar = 32.0 if not s["fullscreen"] else 0.0
        self.results["start"] = {k: s.get(k) for k in ("status", "window_mode", "fullscreen", "bounds", "stats_shown", "last_run", "audio_output")}
        self.log("picture:", json.dumps(self.results["start"]))
        if s["fullscreen"]:
            d.send("key f11")
            d.until(settled(False), 5)
        p = self.plan

        self.phase("warm-up")
        d.idle(p["warm_up"] * 60)
        self.phase("soak")
        d.idle(p["soak"] * 60 / 2)
        self.results["assertions"] = self.display_assertion(d.pid)
        d.idle(p["soak"] * 60 / 2)
        self.phase("")

        self.front()
        d.send("key f11")
        took = d.until(settled(True), 10)
        self.log("Fullscreen:", "settled in %.2f s" % took if took is not None else "not reached")
        self.phase("fullscreen-settle")
        d.idle(p["fs_settle"] * 60)
        self.phase("fullscreen")
        d.idle(p["fs_measure"] * 60)
        self.phase("")
        d.send("key f11")
        d.until(settled(False), 10)

        self.paced_batches()
        self.rapid_bursts()
        self.results["display_off"] = self.display_off_events(self.started, time.time())
        self.sleep_wake()
        self.unplug_replug()

    def paced_batches(self):
        p = self.plan
        self.results["batches"] = []
        for kind in BATCHES:
            batch = kind(self)
            out = {"key": batch.key, "title": batch.title, "limit_s": batch.limit}
            self.results["batches"].append(out)
            self.log("batch:", batch.title)
            reason = batch.prepare()
            if reason:
                out["not_run"] = reason
                self.log("  not run:", reason)
                batch.finish()
                continue
            try:
                self.phase("batch:{}:warm-up".format(batch.key))
                for _ in range(p["warm_cycles"]):
                    batch.cycle()
                self.close_drawer()   # the before and after windows are alike: drawer closed
                self.phase("batch:{}:before".format(batch.key))
                self.main.idle(p["window_s"])
                self.phase("batch:{}:cycles".format(batch.key))
                out["recoveries"] = []
                for _ in range(p["cycles"]):
                    out["recoveries"].append(batch.cycle())
                self.phase("")
                batch.finish()
                self.phase("batch:{}:after".format(batch.key))
                self.main.idle(p["window_s"])
                self.phase("")
            except NoAnswer as e:
                self.phase("")
                out["crashed"] = str(e)
                raise CardStop("xReveal stopped answering in the {} batch".format(batch.key))

    def rapid_bursts(self):
        self.results["bursts"] = []
        self.phase("bursts")
        for kind in BATCHES:
            batch = kind(self)
            out = {"key": batch.key, "title": batch.title}
            self.results["bursts"].append(out)
            reason = batch.prepare()
            if reason:
                out["not_run"] = reason
                batch.finish()
                continue
            try:
                actions, check = batch.burst_actions()
                for command in actions:
                    t = time.time()
                    self.main.send(command)
                    time.sleep(max(0.0, BURST_GAP - (time.time() - t)))
                took = self.main.until(check, 15)
                out["ended_right"] = took is not None
                out["settled_s"] = took
                out["state"] = {k: self.main.last.get(k) for k in ("screen", "status", "fullscreen", "drawer_open", "drawer_tab", "stats_shown", "audio_output", "mouse")}
                self.log("  burst {}: {}".format(batch.key, "ended in the state asked for, %.2f s after" % took if took is not None else "NOT in the state asked for: " + json.dumps(out["state"])))
                time.sleep(PAUSE)
                batch.finish()
            except NoAnswer as e:
                out["crashed"] = str(e)
                self.phase("")
                raise CardStop("xReveal stopped answering in the {} burst".format(batch.key))
        self.phase("")

    def display_assertion(self, pid):
        """xReveal holds PreventUserIdleDisplaySleep while the picture shows (#215)."""
        r = subprocess.run(["pmset", "-g", "assertions"], stdout=subprocess.PIPE, universal_newlines=True)
        lines = [line.strip() for line in r.stdout.splitlines() if "pid {}(".format(pid) in line]
        held = any("PreventUserIdleDisplaySleep" in line for line in lines)
        self.log("assertions of pid {}: {}".format(pid, lines or "none"))
        return {"held": held, "lines": lines}

    def display_off_events(self, start, end):
        """'Display is turned off' in `pmset -g log` between `start` and `end`."""
        r = subprocess.run(["pmset", "-g", "log"], stdout=subprocess.PIPE, universal_newlines=True, errors="replace")
        found = []
        for line in r.stdout.splitlines():
            if "Display is turned off" not in line:
                continue
            try:
                t = datetime.strptime(line[:25], "%Y-%m-%d %H:%M:%S %z").timestamp()
            except ValueError:
                continue
            if start <= t <= end:
                found.append(line[:19])
        self.log("display turned off during steps 1-5:", found or "never")
        return found

    def sleep_wake(self):
        n = self.plan["sleeps"]
        out = self.results["sleep"] = {"cycles": []}
        if self.args.no_sleep:
            out["not_run"] = "skipped (--no-sleep)"
            return
        ok = subprocess.run(["sudo", "-n", "-l", "/usr/bin/pmset", "sleepnow"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
        ok = ok and subprocess.run(["sudo", "-n", "-l", "/usr/bin/pmset", "schedule", "wake", "01/01/30 00:00:00"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
        if not ok:
            out["not_run"] = "no sudoers entry for pmset sleepnow / schedule wake"
            self.log("sleep/wake: not run (no sudoers entry)")
            return
        d = self.main
        self.phase("sleep-wake")
        for i in range(n):
            wake = time.time() + 60
            subprocess.run(["sudo", "-n", "/usr/bin/pmset", "schedule", "wake", time.strftime("%m/%d/%y %H:%M:%S", time.localtime(wake))])
            self.log("sleep {}/{}: sleeping, wake at {}".format(i + 1, n, time.strftime("%H:%M:%S", time.localtime(wake))))
            slept = time.time()
            subprocess.run(["sudo", "-n", "/usr/bin/pmset", "sleepnow"])
            while time.time() < wake + 1:
                time.sleep(1)
            # A scheduled wake can leave the display off: declare the user active, as a touch would.
            subprocess.run(["caffeinate", "-u", "-t", "2"])
            on = self.display_on_after(slept) or time.time()
            back = d.until(showing(name=self.card), 60)
            took = None if back is None else max(0.0, time.time() - on)
            out["cycles"].append(took)
            self.log("  picture back {}".format("%.1f s after the screen came on" % took if took is not None else "NOT within 60 s"))
            d.idle(10)
        self.phase("")

    def display_on_after(self, t0):
        r = subprocess.run(["pmset", "-g", "log"], stdout=subprocess.PIPE, universal_newlines=True, errors="replace")
        for line in r.stdout.splitlines():
            if "Display is turned on" in line:
                try:
                    t = datetime.strptime(line[:25], "%Y-%m-%d %H:%M:%S %z").timestamp()
                except ValueError:
                    continue
                if t >= t0:
                    return t
        return None

    def unplug_replug(self):
        n = self.plan["unplugs"]
        out = self.results["unplug"] = {"cycles": []}
        if self.args.no_manual:
            out["not_run"] = "skipped (--no-manual)"
            return
        if not sys.stdin.isatty():
            out["not_run"] = "no terminal to ask on"
            return
        d = self.main
        self.phase("unplug")
        for i in range(n):
            input("\n>>> Unplug {} ({} of {}) and press Enter. ".format(self.card, i + 1, n))
            gone = d.until(lambda s: s["screen"].startswith("Unplugged"), 10)
            self.log("  unplugged: {}".format("Card-unplugged screen" if gone is not None else "no Card-unplugged screen: " + d.last["screen"]))
            input(">>> Plug it back in, then press Enter right away. ")
            back_at = time.time()
            back = d.until(showing(name=self.card), 30, since=back_at)
            out["cycles"].append(back)
            self.log("  picture back {}".format("%.1f s after Enter" % back if back is not None else "NOT within 30 s"))
            d.idle(10)
        self.phase("")

    def end_main(self):
        d = self.main
        if d is None:
            return
        if alive(d.pid):
            if d.last and d.last.get("fullscreen"):
                try:
                    d.send("key f11")
                    d.until(settled(False), 5)
                except NoAnswer:
                    pass
            took = d.quit()
            self.log("main: quit", "in %.1f s" % took if took is not None else "FAILED")
        self.results["main_marker_left"] = os.path.exists(MARKER)
        if self.sampler:
            if self.sampler.poll() is None:
                self.sampler.send_signal(signal.SIGTERM)
            try:
                self.sampler.wait(120)
            except subprocess.TimeoutExpired:
                self.sampler.kill()
        d.close()
        self.drivers.remove(d)

    def launch(self, name, env=(), picture=20):
        """Launches a driven xReveal and waits for its picture: (driver, state, seconds) or raises."""
        d = Driver(self, name, env)
        self.drivers.append(d)
        d.ready()
        took = d.until(lambda s: showing()(s) or s["screen"].startswith("Busy"), picture)
        return d, d.last, took

    def finish(self, d):
        took = d.quit()
        d.close()
        self.drivers.remove(d)
        return took

    def launch_quit(self):
        """Step 8: launch, wait for the picture, quit; every 10th first sets What's new back (#197)."""
        out = self.results["launches"] = []
        n, every = self.plan["launches"], self.plan["whats_new_every"]
        self.phase("launch-quit")
        for i in range(1, n + 1):
            whats_new = i % every == 0
            before = read_bytes(SETUP)
            if whats_new:
                write_bytes(SETUP, set_toml(before.decode("utf-8"), "", "whats-new-seen", OLD_VERSION).encode("utf-8"))
            row = {"cycle": i, "whats_new": whats_new}
            out.append(row)
            try:
                d, s, took = self.launch("launch-%02d" % i)
                row["busy"] = s["screen"].startswith("Busy")
                row["picture"] = took is not None and not row["busy"]
                if row["picture"]:
                    d.until(lambda s: s.get("launch_to_picture_ms") is not None, 5)
                row["launch_to_picture_ms"] = d.last.get("launch_to_picture_ms")
                row["last_run"] = d.last.get("last_run")
                row["whats_new_shown"] = d.state().get("whats_new_shown")
                if whats_new:
                    row["whats_new_closed"] = self.close_whats_new(d)
                row["quit_s"] = self.finish(d)
            except (CardStop, NoAnswer) as e:
                row["error"] = str(e)
            row["marker_removed"] = not os.path.exists(MARKER)
            after = read_bytes(SETUP)
            row["setup_changes"] = [] if after == before else changed_lines(before, after)
            if after != before:
                write_bytes(SETUP, before)   # so one cycle's change can't hide in the next
            if not row["marker_removed"] and not xreveal_pids():
                os.remove(MARKER)
            self.log("launch {}/{}: {}".format(i, n, json.dumps(row)))
        self.phase("")

    def close_whats_new(self, d):
        """Clicks What's new's Got it (only it closes the dialog, spec §2.9). The dialog's height
        depends on the notes, so this clicks up a column through where Got it can be; the dim
        backdrop and the notes take clicks without doing anything. Closed = whats-new-seen is this
        version again in setup.toml (saved within 1 s)."""
        time.sleep(1.0)   # drawn
        s = d.state()
        width, height = s["bounds"][2], s["bounds"][3] - (0 if s["fullscreen"] else self.title_bar)
        # The 420 px panel is centred, with 17.5 px padding; Got it is ~68 px wide at its right.
        x = width / 2 + 210 - 17.5 - 34
        version = '"{}"'.format(self.results["facts"]["version"])
        y = height / 2 + 240
        while y > height / 2 - 120:
            d.send("click {:g} {:g}".format(x, y))
            y -= 6
        end = time.time() + 3
        while time.time() < end:
            if self.setup_value("", "whats-new-seen") == version and d.state().get("whats_new_shown") is False:
                return True
            time.sleep(0.2)
        return False

    def crash_capture(self):
        """Step 9: each crash test, then a normal launch that must say what happened (§2.11)."""
        out = self.results["crashes"] = []
        self.phase("crash")
        for kind in ("panic", "abort", "native"):
            row = {"kind": kind}
            out.append(row)
            reports = set(os.listdir(REPORTS)) if os.path.isdir(REPORTS) else set()
            launched = time.time()
            try:
                d, s, took = self.launch("crash-" + kind, env=("XREVEAL_CRASH_TEST=" + kind,))
                row["picture"] = took is not None
                row["exited"] = wait_gone(d.pid, 40)   # 10 s after the picture
                self.drivers.remove(d)
                d.close()
                # macOS writes the .ips a few seconds after the crash; a panic may leave none.
                end = time.time() + (5 if kind == "panic" else 30)
                new = []
                while time.time() < end and not new:
                    time.sleep(1)
                    new = sorted(f for f in set(os.listdir(REPORTS)) - reports if f.startswith("xreveal") and f.endswith(".ips")) if os.path.isdir(REPORTS) else []
                row["ips"] = new
                row["panic_file"] = os.path.exists(PANIC) and os.path.getmtime(PANIC) >= launched   # this run's, not the last test's
                log_from = os.path.getsize(XLOG) if os.path.exists(XLOG) else 0
                d, s, took = self.launch("after-" + kind)
                d.until(lambda s: s.get("last_run") == "crashed", 20)
                row["crash_note"] = d.last.get("crash_note")
                row["crash_issue_url_len"] = d.last.get("crash_issue_url_len")
                row["last_run"] = d.last.get("last_run")
                row["log"] = self.log_lines_since(log_from, "last run")
                row["quit_s"] = self.finish(d)
            except (CardStop, NoAnswer) as e:
                row["error"] = str(e)
            if not xreveal_pids() and os.path.exists(MARKER):
                os.remove(MARKER)
            self.log("crash test {}: {}".format(kind, json.dumps(row)))
        self.phase("")

    def last_clean_launch(self):
        """One more launch: no crash note is left pending, and the last run was clean."""
        try:
            d, s, took = self.launch("last")
            self.results["last_launch"] = {"last_run": d.last.get("last_run"), "crash_note": d.last.get("crash_note"), "picture": took is not None, "quit_s": self.finish(d)}
        except (CardStop, NoAnswer) as e:
            self.results["last_launch"] = {"error": str(e)}
        self.results["last_launch"]["marker_removed"] = not os.path.exists(MARKER)
        self.log("last launch:", json.dumps(self.results["last_launch"]))

    def log_lines_since(self, offset, needle):
        try:
            with open(XLOG, "rb") as f:
                if os.path.getsize(XLOG) >= offset:
                    f.seek(offset)
                text = f.read().decode("utf-8", "replace")
        except FileNotFoundError:
            return []
        return [line for line in text.splitlines() if needle in line]

    # -- judging and writing

    def write_results(self):
        with open(os.path.join(self.folder, "states.csv"), "w", newline="", encoding="utf-8") as f:
            cols = ["t", "who", "phase", "screen", "fullscreen", "status", "fps", "dropped", "on_screen", "gpu_pct", "share_p50", "share_p99"]
            w = csv.writer(f)
            w.writerow(cols)
            for r in self.states:
                w.writerow([datetime.fromtimestamp(r["t"]).isoformat(timespec="milliseconds") if c == "t" else ("" if r[c] is None else r[c]) for c in cols])
        rows, batches = Judge(self).rows()
        with open(os.path.join(self.folder, "batches.json"), "w", encoding="utf-8") as f:
            json.dump(batches, f, indent=2, ensure_ascii=False)
        path = os.path.join(self.folder, "card.md")
        with open(path, "w", encoding="utf-8") as f:
            f.write(CardWriter(self, rows, batches).text())
        self.log("wrote", path)


def med(values):
    v = [x for x in values if x is not None and not (isinstance(x, float) and math.isnan(x))]
    return analyze.median(v) if v else math.nan


def f1(v, unit="", decimals=1):
    return "n/a" if v is None or (isinstance(v, float) and math.isnan(v)) else "{:.{}f}{}".format(v, decimals, unit)


# One-sided 95% t quantiles by degrees of freedom: the slope's upper bound (#192).
T95 = [(1, 6.314), (2, 2.920), (3, 2.353), (4, 2.132), (5, 2.015), (6, 1.943), (7, 1.895), (8, 1.860), (9, 1.833),
       (10, 1.812), (12, 1.782), (15, 1.753), (20, 1.725), (25, 1.708), (30, 1.697), (40, 1.684), (60, 1.671), (120, 1.658)]


def slope_upper(points):
    """Least-squares slope per hour of (minute, value), and its one-sided 95% upper bound."""
    pts = [(x, y) for x, y in points if y is not None and not math.isnan(y)]
    if len(pts) < 3:
        return math.nan, math.nan
    n = len(pts)
    mx, my = sum(x for x, _ in pts) / n, sum(y for _, y in pts) / n
    sxx = sum((x - mx) ** 2 for x, _ in pts)
    b = sum((x - mx) * (y - my) for x, y in pts) / sxx
    a = my - b * mx
    se = math.sqrt(sum((y - a - b * x) ** 2 for x, y in pts) / (n - 2) / sxx)
    t = next((q for df, q in T95 if df >= n - 2), 1.645)
    return b * 60, (b + t * se) * 60


def quarters(samples):
    """The footprint's median in each quarter of `samples` (#192's four windows)."""
    n = len(samples)
    return [med(s["mb"] for s in samples[k * n // 4:(k + 1) * n // 4]) for k in range(4)]


def rising(values):
    v = [x for x in values if not math.isnan(x)]
    return len(v) >= 2 and len(v) == len(values) and all(b > a for a, b in zip(v, v[1:]))


class Judge:
    """Each of the card's 14 rows, by the rule it cites (#220)."""

    def __init__(self, run):
        self.run = run
        self.r = run.results
        self.samples = self.load_samples()

    def load_samples(self):
        path = os.path.join(self.run.folder, "samples.csv")
        if not os.path.exists(path) or not self.run.main:
            return []
        out = []
        with open(path, newline="") as f:
            for row in csv.DictReader(f):
                if row.get("alive") != "1" or row.get("pid") != str(self.run.main.pid):
                    continue
                out.append({"t": analyze.parse_time(row["time"]).timestamp(), "mode": row["mode"],
                            "cpu": analyze.num(row["cpu_pct"]), "mb": analyze.num(row["footprint_mb"]),
                            "threads": analyze.num(row["threads"]), "fds": analyze.num(row["open_files"])})
        return out

    def in_phase(self, name):
        span = self.run.span(name)
        if not span:
            return []
        return [s for s in self.samples if span[0] <= s["t"] <= span[1]]

    def minutes(self, samples, key):
        """1-minute medians: (minute, median) from the first sample."""
        if not samples:
            return []
        t0 = samples[0]["t"]
        bins = {}
        for s in samples:
            bins.setdefault(int((s["t"] - t0) // 60), []).append(s[key])
        return [(m, med(v)) for m, v in sorted(bins.items())]

    def rows(self):
        batches = self.batches()
        rows = [self.cpu(), self.gpu(), self.share(), self.launch(), self.memory(), self.leak(), self.frames(),
                self.paced(batches), self.bursts(), self.sleep(), self.unplug(), self.launch_quit(), self.crash(), self.display()]
        return rows, batches

    def windows(self, key):
        """`key` (a states.csv column) in the soak and the measured Fullscreen minutes."""
        return [(name, med(r[key] for r in self.run.during(phase))) for name, phase in (("soak", "soak"), ("Fullscreen", "fullscreen"))]

    def two(self, values, limit, unit):
        if any(math.isnan(v) for _, v in values):
            measured = ", ".join("{} {}".format(n, f1(v, unit, 2)) for n, v in values)
            return measured, NOT_RUN if all(math.isnan(v) for _, v in values) else FAIL
        measured = ", ".join("{} median {}".format(n, f1(v, unit, 2)) for n, v in values)
        return measured, PASS if all(v <= limit for _, v in values) else FAIL

    def cpu(self):
        values = [(n, med(s["cpu"] for s in self.in_phase(p))) for n, p in (("soak", "soak"), ("Fullscreen", "fullscreen"))]
        return self.two(values, CPU_MAX, "%")

    def gpu(self):
        """Reported, not judged (#220): macOS lowers the GPU's clock when there's headroom, and a
        busy share then reads high (6% or 14.5% for the same work). Latency (row 3) and frames
        (row 7) catch a GPU that's really too slow."""
        measured, result = self.two(self.windows("gpu_pct"), GPU_MAX, "%")
        return measured + ("" if result == PASS else " (over 10%: the GPU at a low clock?)"), NOT_RUN if result == NOT_RUN else REPORTED

    def share(self):
        return self.two(self.windows("share_p50"), SHARE_MAX, " ms")

    def launch(self):
        ms = [r.get("launch_to_picture_ms") for r in self.r.get("launches", []) if r.get("launch_to_picture_ms") is not None]
        n = len(self.r.get("launches", []))
        if not ms:
            return "no launch reached the picture", NOT_RUN if not n else FAIL
        m, worst = med(ms), max(ms)
        ok = len(ms) == n and m <= LAUNCH_MEDIAN_MAX and worst <= LAUNCH_ANY_MAX
        return "{} of {} launches: median {:.0f} ms, slowest {:.0f} ms".format(len(ms), n, m, worst), PASS if ok else FAIL

    def memory(self):
        """The highest 1-minute median per state over the whole main session (§4, #195)."""
        parts, ok, seen = [], True, False
        for mode, ceiling in CEILINGS.items():
            ss = [s for s in self.samples if s["mode"] == mode]
            if not ss:
                parts.append("{} n/a".format(mode))
                continue
            t0 = self.samples[0]["t"]
            bins = {}
            for s in ss:
                bins.setdefault(int((s["t"] - t0) // 60), []).append(s)
            # Only minutes spent mostly in this state, as analyze.py counts them.
            full = {m: v for m, v in bins.items() if len(v) >= 6} or bins
            minute, top = max(((m, med(x["mb"] for x in v)) for m, v in full.items()), key=lambda mv: mv[1])
            seen = True
            ok = ok and top <= ceiling
            when = self.run.phase_at(full[minute][len(full[minute]) // 2]["t"]) or "between steps"
            parts.append("{} {:.1f} MB (≤ {:.0f}), minute {} ({})".format(mode.capitalize(), top, ceiling, minute + 1, when))
        return ", ".join(parts), (PASS if ok else FAIL) if seen else NOT_RUN

    def leak(self):
        soak = self.in_phase("soak")
        if not soak:
            return "no soak samples", NOT_RUN
        mins = self.minutes(soak, "mb")
        slope, upper = slope_upper(mins)
        windows = quarters(soak)
        up = rising(windows)
        counts = {}
        for key in ("threads", "fds"):
            counts[key] = slope_upper(self.minutes(soak, key))[0]
        counts_ok = all(not math.isnan(v) and v < COUNT_PER_H for v in counts.values())
        measured = "footprint slope {} MB/h (95% upper {}), 4 windows {} ({}); threads {} /h, fds {} /h; {} min".format(
            f1(slope, "", 2), f1(upper, "", 2), " → ".join(f1(w, "", 1) for w in windows), "rising" if up else "not rising",
            f1(counts["threads"], "", 1), f1(counts["fds"], "", 1), len(mins))
        if math.isnan(upper):
            return measured + " (too few minutes for a slope)", INCONCLUSIVE
        if (slope >= LEAK_MB_H and up) or not counts_ok:
            return measured, FAIL
        if upper < LEAK_MB_H and not up:
            return measured, PASS
        return measured, INCONCLUSIVE

    def frames(self):
        rows = [r for r in self.run.during("soak") if r["on_screen"] is not None]
        if not rows:
            return "no frame readings in the soak", NOT_RUN
        rate = med(r["rate"] for r in rows)
        on = sum(r["on_screen"] for r in rows) / len(rows)
        low = min(r["on_screen"] for r in rows)
        ok = on >= FRAMES_MIN * rate
        return "{:.2f} fps on screen of {:g} ({:.1f}%), lowest 5 s {:.1f}".format(on, rate, 100 * on / rate, low), PASS if ok else FAIL

    def batches(self):
        out = []
        for b in self.r.get("batches", []):
            row = {k: b[k] for k in ("key", "title", "limit_s")}
            out.append(row)
            if b.get("not_run"):
                row["not_run"] = b["not_run"]
                continue
            rec = b.get("recoveries") or []
            flat = [t for cycle in rec for t in cycle]
            before = [s["mb"] for s in self.in_phase("batch:{}:before".format(b["key"]))]
            after = [s["mb"] for s in self.in_phase("batch:{}:after".format(b["key"]))]
            cycles = self.in_phase("batch:{}:cycles".format(b["key"]))
            parts = quarters(cycles)
            fb, fa = med(before), med(after)
            per_cycle = (fa - fb) * 1024 / len(rec) if rec else math.nan
            frames = [r for r in self.run.during("batch:{}:after".format(b["key"])) if r["on_screen"] is not None]
            on = sum(r["on_screen"] for r in frames) / len(frames) if frames else math.nan
            rate = med(r["rate"] for r in frames) if frames else math.nan
            missed = sum(1 for t in flat if t is None or t > b["limit_s"])
            row.update({
                "cycles": len(rec), "steps": len(flat),
                "footprint_before_mb": fb, "footprint_after_mb": fa, "kb_per_cycle": per_cycle,
                "cycle_quarters_mb": parts, "rising": rising(parts) if len(cycles) >= 4 else None,
                "slowest_recovery_s": max((t for t in flat if t is not None), default=None),
                "missed": missed, "crashed": b.get("crashed"),
                "frames_after_fps": on, "source_rate": rate,
            })
            row["passed"] = (not b.get("crashed") and not math.isnan(per_cycle) and per_cycle < LEAK_KB_CYCLE
                             and not row["rising"] and missed == 0 and not math.isnan(on) and on >= FRAMES_MIN * rate)
        for b in out:   # JSON has no NaN
            for k, v in b.items():
                if isinstance(v, float) and math.isnan(v):
                    b[k] = None
                elif isinstance(v, list):
                    b[k] = [None if isinstance(x, float) and math.isnan(x) else x for x in v]
        return out

    def paced(self, batches):
        ran = [b for b in batches if not b.get("not_run")]
        if not ran:
            return "no batch ran", NOT_RUN
        bad = [b["key"] for b in ran if not b["passed"]]
        skipped = ["{} ({})".format(b["key"], b["not_run"]) for b in batches if b.get("not_run")]
        worst = max(ran, key=lambda b: b["kb_per_cycle"] if b["kb_per_cycle"] is not None else -1e9)
        measured = "{} of {} batches pass; most per cycle {} kB ({}); see Paced batches".format(
            len(ran) - len(bad), len(ran), f1(worst["kb_per_cycle"], "", 1), worst["key"])
        if skipped:
            measured += "; not run: " + "; ".join(skipped)
        return measured, PASS if not bad else FAIL

    def bursts(self):
        bursts = self.r.get("bursts", [])
        ran = [b for b in bursts if not b.get("not_run")]
        if not ran:
            return "no burst ran", NOT_RUN
        bad = [b["key"] for b in ran if not b.get("ended_right") or b.get("crashed")]
        measured = "{} of {} end in the state asked for, no crash".format(len(ran) - len(bad), len(ran)) if not bad else "wrong end state or crash: " + ", ".join(bad)
        skipped = [b["key"] for b in bursts if b.get("not_run")]
        if skipped:
            measured += "; not run: " + ", ".join(skipped)
        return measured, PASS if not bad else FAIL

    def recoveries(self, key, limit, what):
        out = self.r.get(key) or {}
        if out.get("not_run"):
            return out["not_run"], NOT_RUN
        cycles = out.get("cycles") or []
        if not cycles:
            return "not reached", NOT_RUN
        ok = [t for t in cycles if t is not None]
        measured = "{} of {} back, slowest {} {}".format(len(ok), len(cycles), f1(max(ok), " s") if ok else "n/a", what)
        return measured, PASS if len(ok) == len(cycles) and max(ok) <= limit else FAIL

    def sleep(self):
        return self.recoveries("sleep", WAKE_LIMIT, "after the screen came on")

    def unplug(self):
        return self.recoveries("unplug", REPLUG_LIMIT, "after the card was back")

    def launch_quit(self):
        rows = self.r.get("launches") or []
        if not rows:
            return "not reached", NOT_RUN
        problems = []
        for r in rows:
            if r.get("error"):
                problems.append("#{}: {}".format(r["cycle"], r["error"]))
            if r.get("busy") or not r.get("picture"):
                problems.append("#{}: {}".format(r["cycle"], "Source busy" if r.get("busy") else "no picture"))
            if not r.get("marker_removed"):
                problems.append("#{}: running.toml left".format(r["cycle"]))
            if r.get("setup_changes"):
                problems.append("#{}: setup.toml changed ({})".format(r["cycle"], "; ".join(r["setup_changes"])[:120]))
            if r.get("whats_new") and not r.get("whats_new_shown"):
                problems.append("#{}: What's new didn't show".format(r["cycle"]))
            elif r.get("whats_new") and not r.get("whats_new_closed"):
                problems.append("#{}: What's new not closed".format(r["cycle"]))
            if not r.get("whats_new") and r.get("whats_new_shown"):
                problems.append("#{}: What's new showed unasked".format(r["cycle"]))
        shown = sum(1 for r in rows if r.get("whats_new"))
        closed = sum(1 for r in rows if r.get("whats_new_closed"))
        measured = "{} cycles; What's new shown and closed {} of {}".format(len(rows), closed, shown)
        if problems:
            measured += "; " + "; ".join(problems[:5])
        return measured, PASS if not problems else FAIL

    def crash(self):
        rows = self.r.get("crashes") or []
        if not rows:
            return "not reached", NOT_RUN
        parts, ok = [], True
        for r in rows:
            want = "crashed" if (r["kind"] == "panic" or r.get("ips")) else "not_clean"
            good = r.get("last_run") == "crashed" and r.get("crash_note") == "crashed" and not r.get("error") and bool(r.get("log"))
            ok = ok and good
            evidence = ".ips" if r.get("ips") else ("panic file" if r.get("panic_file") else "no report")
            note = "note bar {}{}".format(r.get("crash_note") or "missing",
                                          ", issue link {} B".format(r["crash_issue_url_len"]) if r.get("crash_issue_url_len") else "")
            parts.append("{}: {} ({}, {}{})".format(r["kind"], r.get("last_run") or r.get("error"), evidence, note,
                                                    "" if r.get("log") else ", log says nothing" if want == "crashed" else ""))
        last = self.r.get("last_launch") or {}
        clean = last.get("last_run") == "clean" and last.get("marker_removed")
        parts.append("next launch {}".format(last.get("last_run") or last.get("error")))
        clean = clean and last.get("crash_note") is None
        return "; ".join(parts), PASS if ok and clean else FAIL

    def display(self):
        a = self.r.get("assertions")
        off = self.r.get("display_off")
        if a is None or off is None:
            return "not reached", NOT_RUN
        measured = "{} PreventUserIdleDisplaySleep during the soak; display turned off {}".format(
            "held" if a["held"] else "NOT held", "never" if not off else "at " + ", ".join(off))
        return measured, PASS if a["held"] and not off else FAIL


ROWS = [
    ("CPU", "≤ 5% of the machine, soak and Fullscreen medians"),
    ("GPU", "`gpu_pct`, the same minutes: reported, not judged (the GPU's clock varies)"),
    ("xReveal's share (latency)", "p50 ≤ 8.3 ms, the same minutes"),
    ("Launch → picture", "median ≤ 1 s, none over 1.5 s"),
    ("Memory ceilings", "highest 1-min median ≤ 290 MB Windowed, ≤ 250 MB Fullscreen"),
    ("Soak leak", "footprint slope's 95% upper bound < 1 MB/h, windows not rising; threads, fds < 10/h"),
    ("Frames, soak", "≥ 95% of the source rate"),
    ("Each paced batch", "< 10 kB/cycle, not rising; every recovery in its limit; no crash; frames ≥ 95% after"),
    ("Rapid bursts", "no crash; ends in the state last asked for"),
    ("Sleep/wake", "picture back within 10 s, every cycle"),
    ("Unplug/replug", "same card's picture back within 5 s, every cycle"),
    ("Launch/quit", "no Source busy; running.toml removed; setup.toml unchanged; What's new on every 10th"),
    ("Crash capture", "`last_run` crashed (panic; abort/native with .ips); log names the evidence"),
    ("Display awake", "xReveal holds PreventUserIdleDisplaySleep; display never turned off in steps 1–5"),
]


class CardWriter:
    def __init__(self, run, rows, batches):
        self.run, self.rows, self.batches = run, rows, batches

    def text(self):
        run, r = self.run, self.run.results
        facts = r.get("facts", {})
        start = r.get("start", {})
        L = ["# xReveal verification card: macOS, {}".format(datetime.fromtimestamp(run.started).strftime("%Y-%m-%d %H:%M")), ""]
        if run.args.quick:
            L += ["> **Quick run** (`--quick`), to test the runner: soak {soak} min, Fullscreen {fs_settle} + {fs_measure} min, "
                  "{warm_cycles} + {cycles} cycles per batch, {launches} launches. Its verdicts don't count for the card.".format(**run.plan), ""]
        verdicts = [v for _, v in self.rows]
        card = FAIL if FAIL in verdicts else (PASS if all(v in (PASS, REPORTED) for v in verdicts) else "incomplete")
        L += [
            "- **Card:** {}{}".format(card, "" if card != "incomplete" else " (no row fails; some are not run or inconclusive)"),
            "- **Machine:** {} ({}), {}".format(facts.get("model", "?"), facts.get("chip", "?"), facts.get("power", "?")),
            "- **macOS:** {}".format(facts.get("macos", "?")),
            "- **xReveal:** {} from commit {}, built {}, signed `{}`".format(facts.get("version", "?"), facts.get("commit", "?"), facts.get("built", "?"), facts.get("signed", "?")),
            "- **Source:** {}".format(start.get("status", "?")),
            "- **Second Audio output:** {}".format(run.adapter or "none found"),
            "- **Run:** {} to {}{}".format(time.strftime("%H:%M", time.localtime(run.started)), time.strftime("%H:%M", time.localtime(time.time())),
                                           "; stopped early: " + r["stopped"] if r.get("stopped") else ""),
            "- **setup.toml:** put back {}".format("byte-identical" if r.get("setup_restored") else "**NOT identical**"),
            "",
            "| # | Check | Passes when | Measured | Result |",
            "|---|---|---|---|---|",
        ]
        for i, ((check, rule), (measured, verdict)) in enumerate(zip(ROWS, self.rows), 1):
            L.append("| {} | {} | {} | {} | {} |".format(i, check, rule, measured.replace("|", "/"), verdict))
        L += ["", "## Paced batches", "",
              "| Batch | Cycles | Footprint before → after | Per cycle | Rising | Slowest (limit) | Missed | Frames after | Result |",
              "|---|---|---|---|---|---|---|---|---|"]
        for b in self.batches:
            if b.get("not_run"):
                L.append("| {} | – | – | – | – | – | – | – | not run: {} |".format(b["title"], b["not_run"]))
                continue
            L.append("| {} | {} | {} → {} MB | {} kB | {} | {} ({:g} s) | {} | {} fps | {} |".format(
                b["title"], b["cycles"], f1(b["footprint_before_mb"]), f1(b["footprint_after_mb"]), f1(b["kb_per_cycle"]),
                {True: "yes", False: "no", None: "n/a"}[b["rising"]], f1(b["slowest_recovery_s"], " s", 2), b["limit_s"],
                b["missed"], f1(b["frames_after_fps"]), PASS if b["passed"] else FAIL))
        L += ["", "## Rapid bursts", "", "| Burst | Ended in the state asked for | Result |", "|---|---|---|"]
        for b in r.get("bursts", []):
            if b.get("not_run"):
                L.append("| {} | – | not run: {} |".format(b["title"], b["not_run"]))
            else:
                L.append("| {} | {} | {} |".format(b["title"], "yes, %.2f s after" % b["settled_s"] if b.get("ended_right") else "no", PASS if b.get("ended_right") and not b.get("crashed") else FAIL))
        launches = r.get("launches", [])
        if launches:
            L += ["", "## Launch/quit", "", "| Cycle | Launch → picture | What's new | Quit | Notes |", "|---|---|---|---|---|"]
            for x in launches:
                notes = "; ".join(filter(None, [x.get("error"), "Source busy" if x.get("busy") else "", "" if x.get("marker_removed") else "running.toml left",
                                                ("setup.toml: " + "; ".join(x["setup_changes"])) if x.get("setup_changes") else ""]))
                L.append("| {} | {} | {} | {} | {} |".format(x["cycle"], f1(x.get("launch_to_picture_ms"), " ms", 0),
                                                             ("closed" if x.get("whats_new_closed") else "NOT closed") if x.get("whats_new") else "–",
                                                             f1(x.get("quit_s"), " s"), notes or "–"))
        crashes = r.get("crashes", [])
        if crashes:
            L += ["", "## Crash capture", "", "| Test | `last_run` next launch | OS report | Log |", "|---|---|---|---|"]
            for x in crashes:
                L.append("| {} | {} | {} | {} |".format(x["kind"], x.get("last_run") or x.get("error"), ", ".join(x.get("ips") or []) or "none",
                                                        "<br>".join(line[21:] for line in x.get("log") or []) or "–"))
        L += ["", "## Notes", "",
              "- The update tip, What's new and the crash note bar are judged on `state` (`update_tip_open`, `whats_new_shown`, `crash_note`).",
              "- Audio output is judged on `state.audio_output`; that sound actually plays there isn't visible to the runner.",
              "- Choosing the card again in Source › Input opens it at its default format (720p60); the runner sets 1080p50 back after the Source batch."]
        if self.run.format_steps and self.run.format_steps > 1:
            L.append("- Video format: 720p60 is {} presses of → from 1080p50, so each switch passes {} other format(s) on the way.".format(
                self.run.format_steps, self.run.format_steps - 1))
        L += ["", "Files: `run.log`, `samples.csv`, `minutes.csv`, `summary.md`, `states.csv`, `batches.json`."]
        return "\n".join(L) + "\n"


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--quick", action="store_true", help="every step, shortened: tests the runner, not xReveal")
    p.add_argument("--no-manual", action="store_true", help="skip the unplug/replug by hand (marked not run)")
    p.add_argument("--no-sleep", action="store_true", help="skip sleep/wake (marked not run)")
    p.add_argument("--audio-output", metavar="NAME", help="the second wired output's name (default: found by system_profiler)")
    p.add_argument("--app", default=os.path.join(REPO, "target", "dev-app", "xReveal.app"), help="the signed xReveal.app")
    p.add_argument("--out", help="run folder (default: ~/Documents/xreveal-card-macos-<date>)")
    args = p.parse_args()
    if not args.out:
        args.out = os.path.expanduser("~/Documents/xreveal-card-macos-" + time.strftime("%Y-%m-%d-%H%M"))
    if not os.path.isdir(args.app):
        sys.exit("card-macos: no app at {} (build it with scripts/dev-app.sh --release)".format(args.app))
    Run(args).run()


if __name__ == "__main__":
    main()
