"""Drives xReveal through its control channel (#204; docs/spec/v1.md §5): an example for the
unattended cards (#197).

Launches xReveal with XREVEAL_DRIVE=1, sends commands on its stdin and reads the `drive:` answers
from its stdout. Each step is a command plus the state it must reach within a time limit, checked
by polling `state`. The same script runs on macOS and Windows (any build: the channel is in release
builds too).

    python3 scripts/driver/drive.py target/release/xreveal [cycles]

It changes the Remembered setup like a person would (overlay, Window mode): back up setup.toml
first if that matters. Exits 1 if any step missed its limit.
"""
import json
import os
import queue
import subprocess
import sys
import threading
import time

exe = sys.argv[1]
cycles = int(sys.argv[2]) if len(sys.argv) > 2 else 3

started = time.monotonic()
# stderr is left alone: xReveal says there once that it's driven.
proc = subprocess.Popen([exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=dict(os.environ, XREVEAL_DRIVE="1"), universal_newlines=True, bufsize=1)
answers = queue.Queue()


def reader():
    for line in proc.stdout:
        if line.startswith("drive: "):
            answers.put(line[len("drive: "):].rstrip("\n"))


threading.Thread(target=reader, daemon=True).start()


def send(command):
    proc.stdin.write(command + "\n")
    proc.stdin.flush()
    return answers.get(timeout=5.0)


def state():
    return json.loads(send("state"))


def until(check, limit):
    """Polls `state` until `check` holds: the seconds it took, or None past `limit`."""
    t0 = time.monotonic()
    while True:
        s = state()
        if check(s):
            return time.monotonic() - t0
        if time.monotonic() - t0 > limit:
            return None
        time.sleep(0.02)


results = {}


def step(name, commands, check, limit, gap=0.0):
    """Sends `commands` (`gap` seconds apart), then waits for `check`."""
    for i, command in enumerate(commands):
        if i:
            time.sleep(gap)
        reply = send(command)
        if not reply.startswith("ok"):
            print("FAIL %s: %s -> %s" % (name, command, reply))
    took = until(check, limit)
    results.setdefault(name, []).append(took)
    if took is None:
        print("FAIL %s: not reached within %.1f s; state %s" % (name, limit, json.dumps(state())))


def settled(fullscreen):
    """Fullscreen on or off, and macOS's slide (if any) over (#203)."""
    return lambda s: s["fullscreen"] == fullscreen and not s["transition_in_progress"]


ready = answers.get(timeout=30.0)
print("launch -> channel ready: %.2f s (%s)" % (time.monotonic() - started, ready))
s0 = state()
print("start state:", json.dumps(s0))
if s0["fullscreen"]:
    step("leave the remembered Fullscreen", ["key f11"], settled(False), 3.0)

for i in range(cycles):
    shown = state()["stats_shown"]
    step("overlay (F3)", ["key f3"], lambda s, shown=shown: s["stats_shown"] != shown, 1.0)
    step("drawer open (Tab)", ["key tab"], lambda s: s["drawer_open"], 1.0)
    tab = state()["drawer_tab"]
    step("drawer next tab (right)", ["key right"], lambda s, tab=tab: s["drawer_tab"] != tab, 1.0)
    step("drawer close (Esc)", ["key escape"], lambda s: not s["drawer_open"], 1.0)
    step("Fullscreen on (F11)", ["key f11"], settled(True), 3.0)
    step("Fullscreen off (F11)", ["key f11"], settled(False), 3.0)
    # Twice in quick succession: the second lands during macOS's slide and must not be lost.
    step("F11 twice 100 ms apart, ends Windowed", ["key f11", "key f11"], settled(False), 5.0, gap=0.1)
    step("F11 three times 100 ms apart, ends Fullscreen", ["key f11"] * 3, settled(True), 5.0, gap=0.1)
    step("F11 twice from Fullscreen, ends Fullscreen", ["key f11", "key f11"], settled(True), 5.0, gap=0.1)
    step("back to Windowed", ["key f11"], settled(False), 3.0)

# The pointer reaches GPUI's hit-testing (the update button's tip is hover).
w, h = s0["bounds"][2], s0["bounds"][3]
print("hover:", send("move %d %d" % (20, h - 40)), "->", state()["mouse"])

failed = 0
for name, times in results.items():
    ok = [t for t in times if t is not None]
    failed += len(times) - len(ok)
    print("%-46s %d/%d reached, slowest %s" % (name, len(ok), len(times), "%.3f s" % max(ok) if ok else "-"))

print("quit:", send("quit"))
try:
    proc.wait(timeout=10)
    print("exit code", proc.returncode)
except subprocess.TimeoutExpired:
    proc.kill()
    print("did not quit; killed")
    failed += 1
sys.exit(1 if failed else 0)
