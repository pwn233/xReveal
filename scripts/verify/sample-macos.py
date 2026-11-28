#!/usr/bin/python3
"""Sample xReveal on macOS every 5 s into a CSV (#207, #191). No root, no TCC prompt.

Usage:
  scripts/verify/sample-macos.py                  wait for xreveal to start, then sample it
  scripts/verify/sample-macos.py --pid 1234       sample that process
  scripts/verify/sample-macos.py --minutes 25     stop after 25 minutes (default: Ctrl+C)
  scripts/verify/sample-macos.py --analyze DIR    write DIR/summary.md again (analyze.py)

Start it before xReveal. It keeps going after xReveal exits (the rows say so) and, when it is waiting
by name, picks up the next xreveal that starts. On Ctrl+C or --minutes it writes summary.md.

Each row (samples.csv):
- cpu_pct: (ri_user_time + ri_system_time) delta from proc_pid_rusage, Mach ticks -> ns via
  mach_timebase_info, / wall time / hw.logicalcpu. The share of the whole machine, as spec §4 counts it.
- footprint_mb, footprint_peak_mb: ri_phys_footprint and ri_lifetime_max_phys_footprint (MiB).
- threads: proc_pidinfo(PROC_PIDTASKINFO).pti_threadnum; open_files: proc_pidinfo(PROC_PIDLISTFDS).
- gpu_pct: the accumulatedGPUTime delta of the IORegistry's AGXDeviceUserClient entries created by
  the pid, taken as ns / wall time. Undocumented and uncalibrated (#200): treat it as a hint.
- mode, window, display: xReveal's largest on-screen window (CGWindowList, no Screen Recording
  needed): fullscreen when it covers its display (below the notch counts), windowed otherwise,
  none when no window is on screen (minimised, hidden, another Space).
`footprint <pid>` is saved at the start and at the end (when xReveal is still running).
"""

import argparse
import csv
import ctypes as C
import json
import os
import plistlib
import re
import signal
import subprocess
import sys
import time
from datetime import datetime

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # no __pycache__ next to the scripts
import analyze  # noqa: E402

MIB = 1024.0 * 1024.0
COLUMNS = ["time", "pid", "alive", "mode", "window", "display",
           "cpu_pct", "gpu_pct", "footprint_mb", "footprint_peak_mb", "threads", "open_files"]

# --- libproc and Mach, from libSystem
libc = C.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)


class RusageInfoV4(C.Structure):
    _fields_ = [("ri_uuid", C.c_uint8 * 16)] + [(n, C.c_uint64) for n in (
        "ri_user_time", "ri_system_time", "ri_pkg_idle_wkups", "ri_interrupt_wkups", "ri_pageins",
        "ri_wired_size", "ri_resident_size", "ri_phys_footprint", "ri_proc_start_abstime",
        "ri_proc_exit_abstime", "ri_child_user_time", "ri_child_system_time", "ri_child_pkg_idle_wkups",
        "ri_child_interrupt_wkups", "ri_child_pageins", "ri_child_elapsed_abstime", "ri_diskio_bytesread",
        "ri_diskio_byteswritten", "ri_cpu_time_qos_default", "ri_cpu_time_qos_maintenance",
        "ri_cpu_time_qos_background", "ri_cpu_time_qos_utility", "ri_cpu_time_qos_legacy",
        "ri_cpu_time_qos_user_initiated", "ri_cpu_time_qos_user_interactive", "ri_billed_system_time",
        "ri_serviced_system_time", "ri_logical_writes", "ri_lifetime_max_phys_footprint", "ri_instructions",
        "ri_cycles", "ri_billed_energy", "ri_serviced_energy", "ri_interval_max_phys_footprint",
        "ri_runnable_time")]


class ProcTaskInfo(C.Structure):
    _fields_ = [(n, C.c_uint64) for n in (
        "pti_virtual_size", "pti_resident_size", "pti_total_user", "pti_total_system",
        "pti_threads_user", "pti_threads_system")] + [(n, C.c_int32) for n in (
        "pti_policy", "pti_faults", "pti_pageins", "pti_cow_faults", "pti_messages_sent",
        "pti_messages_received", "pti_syscalls_mach", "pti_syscalls_unix", "pti_csw", "pti_threadnum",
        "pti_numrunning", "pti_priority")]


class ProcBsdInfo(C.Structure):
    _fields_ = [(n, C.c_uint32) for n in (
        "pbi_flags", "pbi_status", "pbi_xstatus", "pbi_pid", "pbi_ppid", "pbi_uid", "pbi_gid", "pbi_ruid",
        "pbi_rgid", "pbi_svuid", "pbi_svgid", "rfu_1")] + [
        ("pbi_comm", C.c_char * 16), ("pbi_name", C.c_char * 32)] + [(n, C.c_uint32) for n in (
        "pbi_nfiles", "pbi_pgid", "pbi_pjobc", "e_tdev", "e_tpgid")] + [
        ("pbi_nice", C.c_int32), ("pbi_start_tvsec", C.c_uint64), ("pbi_start_tvusec", C.c_uint64)]


class Timebase(C.Structure):
    _fields_ = [("numer", C.c_uint32), ("denom", C.c_uint32)]


RUSAGE_INFO_V4 = 4
PROC_PIDLISTFDS, PROC_PIDTBSDINFO, PROC_PIDTASKINFO = 1, 3, 4
PROC_FDINFO_SIZE = 8  # struct proc_fdinfo { int32_t proc_fd; uint32_t proc_fdtype; }

libc.proc_pid_rusage.argtypes = [C.c_int, C.c_int, C.c_void_p]
libc.proc_pidinfo.argtypes = [C.c_int, C.c_int, C.c_uint64, C.c_void_p, C.c_int]
libc.proc_listallpids.argtypes = [C.c_void_p, C.c_int]
libc.proc_name.argtypes = [C.c_int, C.c_void_p, C.c_uint32]
libc.proc_pidpath.argtypes = [C.c_int, C.c_void_p, C.c_uint32]
libc.mach_timebase_info.argtypes = [C.POINTER(Timebase)]
libc.sysctlbyname.argtypes = [C.c_char_p, C.c_void_p, C.POINTER(C.c_size_t), C.c_void_p, C.c_size_t]

_tb = Timebase()
libc.mach_timebase_info(C.byref(_tb))
TICK_NS = _tb.numer / _tb.denom  # 125/3 on Apple Silicon, 1 on Intel


def sysctl(name, kind=C.c_int64):
    if kind is str:
        size = C.c_size_t(0)
        libc.sysctlbyname(name.encode(), None, C.byref(size), None, 0)
        buf = C.create_string_buffer(size.value)
        libc.sysctlbyname(name.encode(), buf, C.byref(size), None, 0)
        return buf.value.decode()
    v = kind(0)
    size = C.c_size_t(C.sizeof(v))
    libc.sysctlbyname(name.encode(), C.byref(v), C.byref(size), None, 0)
    return v.value


LOGICAL_CPUS = sysctl("hw.logicalcpu", C.c_int32)


def rusage(pid):
    ri = RusageInfoV4()
    return ri if libc.proc_pid_rusage(pid, RUSAGE_INFO_V4, C.byref(ri)) == 0 else None


def taskinfo(pid):
    ti = ProcTaskInfo()
    n = libc.proc_pidinfo(pid, PROC_PIDTASKINFO, 0, C.byref(ti), C.sizeof(ti))
    return ti if n == C.sizeof(ti) else None


def bsdinfo(pid):
    bi = ProcBsdInfo()
    n = libc.proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, C.byref(bi), C.sizeof(bi))
    return bi if n == C.sizeof(bi) else None


def open_files(pid):
    need = libc.proc_pidinfo(pid, PROC_PIDLISTFDS, 0, None, 0)  # a size with room to spare
    if need <= 0:
        return None
    buf = C.create_string_buffer(need)
    got = libc.proc_pidinfo(pid, PROC_PIDLISTFDS, 0, buf, need)
    return got // PROC_FDINFO_SIZE if got >= 0 else None


def proc_name(pid):
    buf = C.create_string_buffer(256)
    return buf.value.decode(errors="replace") if libc.proc_name(pid, buf, 256) > 0 else ""


def proc_path(pid):
    buf = C.create_string_buffer(4096)
    return buf.value.decode(errors="replace") if libc.proc_pidpath(pid, buf, 4096) > 0 else ""


def start_time(pid):
    bi = bsdinfo(pid)
    return None if bi is None else bi.pbi_start_tvsec + bi.pbi_start_tvusec / 1e6


def find(name):
    """The newest same-user process called `name` (case-insensitive), or None."""
    n = libc.proc_listallpids(None, 0)
    pids = (C.c_int * (n + 64))()
    n = libc.proc_listallpids(pids, C.sizeof(pids))
    best = None
    for pid in pids[:max(0, n)]:
        if pid <= 0 or pid == os.getpid() or proc_name(pid).lower() != name.lower():
            continue
        st = start_time(pid)
        if st is not None and rusage(pid) is not None and (best is None or st > best[1]):
            best = (pid, st)
    return best[0] if best else None


# --- GPU: IORegistry AGXDeviceUserClient (undocumented, #200)
def gpu_time_ns(pid):
    try:
        out = subprocess.run(["ioreg", "-r", "-c", "AGXDeviceUserClient", "-l", "-a"],
                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=10).stdout
        entries = plistlib.loads(out) if out.strip() else []
    except Exception:
        return None
    prefix = "pid {},".format(pid)
    total, found = 0, False
    for e in entries:
        if str(e.get("IOUserClientCreator", "")).startswith(prefix):
            found = True
            for use in e.get("AppUsage") or []:
                total += int(use.get("accumulatedGPUTime", 0))
    return total if found else None


# --- Windows and displays: CoreGraphics through ctypes
CF = C.CDLL("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
CG = C.CDLL("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics")


class CGRect(C.Structure):
    _fields_ = [("x", C.c_double), ("y", C.c_double), ("w", C.c_double), ("h", C.c_double)]


CG.CGWindowListCopyWindowInfo.restype = C.c_void_p
CG.CGWindowListCopyWindowInfo.argtypes = [C.c_uint32, C.c_uint32]
CG.CGRectMakeWithDictionaryRepresentation.restype = C.c_bool
CG.CGRectMakeWithDictionaryRepresentation.argtypes = [C.c_void_p, C.POINTER(CGRect)]
CG.CGGetActiveDisplayList.argtypes = [C.c_uint32, C.POINTER(C.c_uint32), C.POINTER(C.c_uint32)]
CG.CGDisplayBounds.restype = CGRect
CG.CGDisplayBounds.argtypes = [C.c_uint32]
CG.CGDisplayIsBuiltin.restype = C.c_bool
CG.CGDisplayIsBuiltin.argtypes = [C.c_uint32]
CF.CFArrayGetCount.restype = C.c_long
CF.CFArrayGetCount.argtypes = [C.c_void_p]
CF.CFArrayGetValueAtIndex.restype = C.c_void_p
CF.CFArrayGetValueAtIndex.argtypes = [C.c_void_p, C.c_long]
CF.CFStringCreateWithCString.restype = C.c_void_p
CF.CFStringCreateWithCString.argtypes = [C.c_void_p, C.c_char_p, C.c_uint32]
CF.CFDictionaryGetValue.restype = C.c_void_p
CF.CFDictionaryGetValue.argtypes = [C.c_void_p, C.c_void_p]
CF.CFNumberGetValue.restype = C.c_bool
CF.CFNumberGetValue.argtypes = [C.c_void_p, C.c_int, C.c_void_p]
CF.CFRelease.argtypes = [C.c_void_p]

_key = {k: CF.CFStringCreateWithCString(None, k.encode(), 0x08000100)  # kCFStringEncodingUTF8
        for k in ("kCGWindowOwnerPID", "kCGWindowLayer", "kCGWindowBounds")}
ON_SCREEN_ONLY, EXCLUDE_DESKTOP = 1 << 0, 1 << 4
# kCGStatusWindowLevel: the menu bar's status items (clock, Control Center), absent in a fullscreen
# Space. The menu bar itself (kCGMainMenuWindowLevel, 24) stays listed there on macOS 26, so it
# can't tell a fullscreen window below the notch from a window zoomed to fill the screen.
STATUS_LAYER = 25


def _cfnum(d, key):
    v = CF.CFDictionaryGetValue(d, _key[key])
    out = C.c_int64()
    return out.value if v and CF.CFNumberGetValue(v, 4, C.byref(out)) else None  # kCFNumberSInt64Type


def displays():
    ids = (C.c_uint32 * 16)()
    n = C.c_uint32()
    CG.CGGetActiveDisplayList(16, ids, C.byref(n))
    return [(ids[i], CG.CGDisplayBounds(ids[i]), CG.CGDisplayIsBuiltin(ids[i])) for i in range(n.value)]


def window_state(pid):
    """(mode, "WxH pt", display) for the pid's largest on-screen window."""
    arr = CG.CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP, 0)
    if not arr:
        return "none", "", ""
    mine, menubars = [], []
    try:
        for i in range(CF.CFArrayGetCount(arr)):
            d = CF.CFArrayGetValueAtIndex(arr, i)
            r = CGRect()
            if not CG.CGRectMakeWithDictionaryRepresentation(CF.CFDictionaryGetValue(d, _key["kCGWindowBounds"]), C.byref(r)):
                continue
            layer = _cfnum(d, "kCGWindowLayer")
            if layer == STATUS_LAYER:
                menubars.append(r)
            elif layer == 0 and _cfnum(d, "kCGWindowOwnerPID") == pid:
                mine.append(r)
    finally:
        CF.CFRelease(arr)
    if not mine:
        return "none", "", ""
    w = max(mine, key=lambda r: r.w * r.h)
    cx, cy = w.x + w.w / 2, w.y + w.h / 2
    disp = None
    for did, b, builtin in displays():
        if b.x <= cx < b.x + b.w and b.y <= cy < b.y + b.h:
            disp = (did, b, builtin)
    if disp is None:
        return "windowed", "{:.0f}x{:.0f} pt".format(w.w, w.h), ""
    did, b, builtin = disp
    dname = "display {} {:.0f}x{:.0f} pt{}".format(did, b.w, b.h, " (built-in)" if builtin else "")
    near = lambda a, z: abs(a - z) < 1.5  # noqa: E731
    covers = near(w.x, b.x) and near(w.y, b.y) and near(w.w, b.w) and near(w.h, b.h)
    # Fullscreen on a notched screen sits below the camera housing: full width, down to the bottom,
    # a little short at the top, and the display shows no menu bar items.
    menubar_here = any(b.x <= m.x < b.x + b.w and near(m.y, b.y) for m in menubars)
    below_notch = (near(w.x, b.x) and near(w.w, b.w) and near(w.y + w.h, b.y + b.h)
                   and w.y - b.y <= 40 and not menubar_here)
    return ("fullscreen" if covers or below_notch else "windowed"), "{:.0f}x{:.0f} pt".format(w.w, w.h), dname


# --- The machine and xReveal
def run(cmd):
    try:
        return subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=60).stdout.decode(errors="replace")
    except Exception:
        return ""


def machine():
    sv = {k: run(["sw_vers", "-" + k]).strip() for k in ("productName", "productVersion", "buildVersion")}
    gpus, monitors = [], []
    try:
        sp = json.loads(run(["system_profiler", "SPDisplaysDataType", "-json"]) or "{}").get("SPDisplaysDataType", [])
    except ValueError:
        sp = []
    for g in sp:
        gpus.append("{}{}".format(g.get("sppci_model", g.get("_name", "?")), ", {} cores".format(g["sppci_cores"]) if g.get("sppci_cores") else ""))
        for d in g.get("spdisplays_ndrvs", []):
            monitors.append("{}: {} ({} px){}".format(
                d.get("_name", "?"), d.get("_spdisplays_resolution", "?"), d.get("_spdisplays_pixels", "?"),
                " (main)" if d.get("spdisplays_main") == "spdisplays_yes" else ""))
    return {
        "os_version": "{} {}, build {}".format(sv["productName"], sv["productVersion"], sv["buildVersion"]),
        "cpu": sysctl("machdep.cpu.brand_string", str),
        "logical_cpus": LOGICAL_CPUS,
        "ram_gb": round(sysctl("hw.memsize") / 1024 ** 3, 1),
        "gpus": gpus,
        "monitors": monitors,
    }


def app_version(path):
    m = re.match(r"(.*?\.app)/Contents/MacOS/", path)
    if not m:
        return "unknown (not in an .app: {})".format(path or "?")
    try:
        with open(os.path.join(m.group(1), "Contents", "Info.plist"), "rb") as f:
            return "{} ({})".format(plistlib.load(f).get("CFBundleShortVersionString", "?"), m.group(1))
    except Exception:
        return "unknown ({})".format(m.group(1))


SETUP_TOML = os.path.expanduser("~/Library/Application Support/xReveal/setup.toml")


def read_setup():
    """window-mode, stats.shown and the video format from setup.toml, as flat "section.key" values."""
    flat, section = {}, ""
    try:
        with open(SETUP_TOML, encoding="utf-8") as f:
            for line in f:
                m = re.match(r"\s*\[+([^\]]+)\]+\s*$", line)
                if m:
                    section = m.group(1) + "."
                    continue
                m = re.match(r'\s*([\w-]+)\s*=\s*"?([^"]*?)"?\s*$', line)
                if m:
                    flat.setdefault(section + m.group(1), m.group(2))
    except OSError:
        return None
    vf = "{}x{} @ {}".format(flat.get("video-format.width", "?"), flat.get("video-format.height", "?"), flat.get("video-format.fps", "?"))
    return {"window_mode": flat.get("window-mode", "windowed (default)"), "stats_shown": flat.get("stats.shown", "false (default)"), "video_format": vf}


def iso(t):
    return datetime.fromtimestamp(t).astimezone().isoformat(timespec="milliseconds")


# --- The sampler
class Stop(Exception):
    pass


def footprint(pid, path):
    with open(path, "w") as f:
        f.write(run(["footprint", str(pid)]) or "footprint {} gave no output\n".format(pid))


def sample(args):
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    out = os.path.abspath(args.out or "xreveal-samples-" + stamp)
    os.makedirs(out, exist_ok=True)
    meta = {"os": "macos", "stamp": stamp, "interval_s": args.interval,
            "sampler": "sample-macos.py (proc_pid_rusage, proc_pidinfo, ioreg AGXDeviceUserClient)",
            "gpu_note": "accumulatedGPUTime, uncalibrated, see #200", "tick_ns": TICK_NS}
    meta.update(machine())
    meta["setup_start"] = read_setup()
    meta["processes"] = []
    meta["sampler_start"] = iso(time.time())
    meta["notes"] = []

    def save_meta():
        with open(os.path.join(out, "meta.json"), "w") as f:
            json.dump(meta, f, indent=2)

    save_meta()
    print("sample-macos: writing {} every {:g} s. Ctrl+C stops.".format(out, args.interval), flush=True)

    def on_term(*_):
        raise Stop()
    signal.signal(signal.SIGTERM, on_term)

    f = open(os.path.join(out, "samples.csv"), "w", newline="")
    w = csv.writer(f)
    w.writerow(COLUMNS)
    f.flush()

    cur = None  # {"pid", "start", "entry", "cpu", "gpu", "mono"}
    used_pid = False
    t0 = time.monotonic()
    k = 0
    waiting_said = False
    try:
        while True:
            if args.minutes and time.monotonic() - t0 >= args.minutes * 60:
                break
            now_wall, now_mono = time.time(), time.monotonic()

            # Attach: the given pid once, or the newest xreveal by name.
            if cur is None:
                pid = None
                if args.pid and not used_pid:
                    pid, used_pid = args.pid, True
                    if rusage(pid) is None:
                        sys.exit("sample-macos: pid {} isn't running, or belongs to another user.".format(pid))
                elif not args.pid:
                    pid = find(args.name)
                    if pid is None and not waiting_said:
                        print("sample-macos: waiting for {} to start...".format(args.name), flush=True)
                        waiting_said = True
                if pid is not None:
                    st = start_time(pid)
                    entry = {"pid": pid, "start": iso(st), "attached": iso(now_wall), "exited": None,
                             "path": proc_path(pid)}
                    meta["processes"].append(entry)
                    meta.setdefault("xreveal", app_version(entry["path"]))
                    footprint(pid, os.path.join(out, "footprint-start-{}.txt".format(pid)))
                    save_meta()
                    cur = {"pid": pid, "start": st, "entry": entry, "cpu": None, "gpu": None, "mono": None}
                    waiting_said = False
                    print("sample-macos: sampling {} (pid {}), launched {}".format(proc_name(pid), pid, entry["start"]), flush=True)
                    now_wall, now_mono = time.time(), time.monotonic()

            row = {"time": iso(now_wall), "pid": "", "alive": "0"}
            if cur is not None:
                pid = cur["pid"]
                ri, ti, fds = rusage(pid), taskinfo(pid), open_files(pid)
                if ri is None or start_time(pid) != cur["start"]:
                    cur["entry"]["exited"] = iso(now_wall)
                    meta["notes"].append("pid {} was gone at {} (exit code unknown: it isn't the sampler's child).".format(pid, cur["entry"]["exited"][11:19]))
                    save_meta()
                    print("sample-macos: pid {} exited; still sampling.".format(pid), flush=True)
                    row["pid"] = str(pid)
                    cur = None
                else:
                    cpu_ns = (ri.ri_user_time + ri.ri_system_time) * TICK_NS
                    gpu = gpu_time_ns(pid)
                    mode, window, display = window_state(pid)
                    row.update(pid=str(pid), alive="1", mode=mode, window=window, display=display,
                               footprint_mb="{:.3f}".format(ri.ri_phys_footprint / MIB),
                               footprint_peak_mb="{:.3f}".format(ri.ri_lifetime_max_phys_footprint / MIB),
                               threads=ti.pti_threadnum if ti else "", open_files="" if fds is None else fds)
                    if cur["mono"] is not None:
                        dt = now_mono - cur["mono"]
                        row["cpu_pct"] = "{:.4f}".format((cpu_ns - cur["cpu"]) / (dt * 1e9) / LOGICAL_CPUS * 100)
                        if gpu is not None and cur["gpu"] is not None and gpu >= cur["gpu"]:
                            row["gpu_pct"] = "{:.4f}".format((gpu - cur["gpu"]) / (dt * 1e9) * 100)
                    cur.update(cpu=cpu_ns, gpu=gpu, mono=now_mono)
            w.writerow([row.get(c, "") for c in COLUMNS])
            f.flush()

            k += 1
            delay = t0 + k * args.interval - time.monotonic()
            if delay > 0:
                time.sleep(delay)
    except (KeyboardInterrupt, Stop):
        print("", flush=True)
    finally:
        f.close()
        if cur is not None and rusage(cur["pid"]) is not None:
            footprint(cur["pid"], os.path.join(out, "footprint-end-{}.txt".format(cur["pid"])))
        meta["sampler_end"] = iso(time.time())
        meta["setup_end"] = read_setup()
        save_meta()
    if meta["processes"]:
        print("sample-macos: wrote " + analyze.analyze(out))
    else:
        print("sample-macos: xReveal never ran; no summary. Samples in " + out)


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--pid", type=int, help="sample this process (default: wait for --name)")
    p.add_argument("--name", default="xreveal", help="process name to wait for (default: xreveal)")
    p.add_argument("--interval", type=float, default=5.0, help="seconds between samples (default: 5)")
    p.add_argument("--minutes", type=float, help="stop after this many minutes (default: Ctrl+C)")
    p.add_argument("--out", help="output folder (default: ./xreveal-samples-<time>)")
    p.add_argument("--analyze", metavar="DIR", help="only write DIR/summary.md again")
    args = p.parse_args()
    if args.analyze:
        print("sample-macos: wrote " + analyze.analyze(args.analyze))
    else:
        sample(args)


if __name__ == "__main__":
    main()
