"""PROTOTYPE for #196, never merged: drives xReveal through its local control channel.

Launches a `--features drive` build with XREVEAL_DRIVE=1, sends commands on stdin and reads the
`drive:` lines from stdout. Each step is an action plus the state it must reach, with a time limit
(#197's recovery limits). The same script runs on Windows and macOS.

    python3 scripts/driver/drive.py target/release/xreveal [cycles]
"""
import json
import subprocess
import sys
import threading
import time
import queue

exe = sys.argv[1]
cycles = int(sys.argv[2]) if len(sys.argv) > 2 else 5
SETTLE = float(sys.argv[3]) if len(sys.argv) > 3 else 1.5
fullscreen_key = "ctrl-cmd-f" if sys.platform == "darwin" else "f11"

started = time.monotonic()
proc = subprocess.Popen([exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=dict(__import__("os").environ, XREVEAL_DRIVE="1"), universal_newlines=True, bufsize=1)
lines = queue.Queue()


def reader():
    for line in proc.stdout:
        if line.startswith("drive: "):
            lines.put(line[len("drive: "):].rstrip("\n"))


threading.Thread(target=reader, daemon=True).start()


def answer(timeout=5.0):
    return lines.get(timeout=timeout)


def send(command):
    proc.stdin.write(command + "\n")
    proc.stdin.flush()
    return answer()


def state():
    return json.loads(send("state"))


def until(check, limit):
    """Polls `state` until `check` holds; returns the seconds it took, or None past `limit`."""
    t0 = time.monotonic()
    while time.monotonic() - t0 < limit:
        s = state()
        if check(s):
            return time.monotonic() - t0
        time.sleep(0.02)
    return None


ready = answer(timeout=20.0)
print("launch -> channel ready: %.2f s (%s)" % (time.monotonic() - started, ready))
s0 = state()
print("start state:", json.dumps(s0))

results = {}


def step(name, command, check, limit):
    reply = send(command)
    took = until(check, limit)
    results.setdefault(name, []).append(took)
    if took is None:
        print("FAIL %s: %s, not reached within %.1f s; state %s" % (name, reply, limit, json.dumps(state())))


for i in range(cycles):
    shown = state()["stats_shown"]
    step("overlay F3", "key f3", lambda s, shown=shown: s["stats_shown"] != shown, 1.0)
    step("drawer open (Tab)", "key tab", lambda s: s["drawer_open"], 1.0)
    step("drawer next tab (right)", "key right", lambda s: True, 1.0)
    step("drawer close (Esc)", "key escape", lambda s: not s["drawer_open"], 1.0)
    step("Fullscreen on", "key " + fullscreen_key, lambda s: s["fullscreen"], 2.0)
    time.sleep(SETTLE)
    step("Fullscreen off", "key " + fullscreen_key, lambda s: not s["fullscreen"], 2.0)
    time.sleep(SETTLE)

# Hover: the pointer reaches GPUI's hit-testing, but nothing in `state` says a tip is showing.
w, h = s0["bounds"][2], s0["bounds"][3]
print("hover:", send("move %d %d" % (20, h - 40)), "->", state()["mouse"])

for name, times in results.items():
    ok = [t for t in times if t is not None]
    print("%-26s %d/%d reached, worst %.3f s" % (name, len(ok), len(times), max(ok) if ok else float("nan")))

print("quit:", send("quit"))
try:
    proc.wait(timeout=10)
    print("exit code", proc.returncode)
except subprocess.TimeoutExpired:
    proc.kill()
    print("did not quit; killed")
