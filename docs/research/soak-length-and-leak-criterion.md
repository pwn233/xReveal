# Research: how long must a soak run be, and what growth counts as a leak?

Ticket: [#192](https://github.com/pwn233/xReveal/issues/192) (parent map [#190](https://github.com/pwn233/xReveal/issues/190)). Researched 2026-11-28.

Related tickets on the same map: sampling ([#191](https://github.com/pwn233/xReveal/issues/191)), the memory ceiling ([#195](https://github.com/pwn233/xReveal/issues/195)), the unattended driver ([#196](https://github.com/pwn233/xReveal/issues/196)), the first Windows baseline ([#194](https://github.com/pwn233/xReveal/issues/194)).

Labels: **[primary]** = OS vendor or tool docs, or a tool's own source and docs. **[inference]** = my reasoning or arithmetic; no source says it outright.

## Answer

**Run length, per OS:**
- 10 min warm-up, thrown away.
- Then **120 min of steady capture**, with the window kept at a fixed size and nobody touching the machine.
- Then **event cycles**: 200 drawer open/close cycles and 200 input switches away and back. Each is preceded by 5 warm-up cycles that aren't counted.
- That's about 2 h 10 min of soak plus roughly 30–40 min of cycles.

**Sampling:**
- Take one sample **every 10 s**.
- Reduce the samples to **1-minute medians** before judging.

**What to record:**
- **Windows:** Private Bytes (`PrivateUsage`), the process's DXGI video-memory usage, the handle count, GDI and USER objects, and the thread count.
- **macOS:** `ri_phys_footprint`, the thread count and the open file descriptors.
- **Both:** the presented-frame counter.

**Pass/fail rule (all automatic):**

| Signal | Leak = FAIL when **both** hold | Otherwise |
|---|---|---|
| Memory (Private Bytes, DXGI usage, `phys_footprint`) | (a) the least-squares slope over the post-warm-up 1-min medians is **≥ 1 MB/h**, and (b) the medians of the four 30-min windows **rise every time** (W1 < W2 < W3 < W4) | **PASS** if the slope's 95% upper bound is < 1 MB/h. **INCONCLUSIVE** (re-run at 4 h) if only one of (a) and (b) holds, or if the slope's 95% interval is wider than ±0.5 MB/h |
| Counts (handles, GDI, USER, threads, fds) | slope ≥ 10 per hour **and** the four window medians rise every time | PASS |
| Per event (drawer, input switch) | the slope against the cycle number is **≥ 10 kB per cycle**, **and** the medians of 5 batches of 40 cycles rise every time | PASS if the 95% upper bound is < 10 kB/cycle |
| Frame health | the presented frames in any 1-min bucket fall below 95% of the source rate | — |

**Why these numbers:**
- **1 MB/h is about 5.6 bytes per frame at 50 fps.**
- One heap allocation leaked per frame is at least 16 bytes once the allocator's overhead is counted, which is ≥ 2.9 MB/h at 50 fps [inference].
- So **any** per-frame heap leak fails. A leak slower than that would need hours to cost a megabyte.
- Microsoft's PAL tool flags 5 and 10 MB/h trends on servers [primary]. xReveal is a small desktop app with a ~100–200 MB footprint, so it gets a tighter line.

## 1. Warm-up: when is memory settled?

**What the vendors say:**
- "When you start monitoring, the counter values change rapidly. It can take time for the memory pools values to reach a steady state. Wait several hours before you decide there's a leak." [primary: [Microsoft, Look for potential memory leaks with PerfMon](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/determining-whether-a-leak-exists)]
- "Newly started processes will initially appear as a memory leak when it is simply normal startup behavior. A memory leak occurs when a process continues to consume memory and does not release memory over a long period of time." [primary: [Microsoft, PAL tool, Memory Leak Detection Analysis](https://learn.microsoft.com/en-us/biztalk/technical-guides/using-the-performance-analysis-of-logs-pal-tool)]
- Apple separates **transient** growth (spikes that are freed again) from **persistent** growth ("memory increasing over time"). Only persistent growth is a leak or abandoned memory [primary: [WWDC24 "Analyze heap memory"](https://developer.apple.com/videos/play/wwdc2024/10173/)].
- Apple's stair-step example has a cache that should grow "only the first time I open the gallery" [primary: same session]. That is the warm-up pattern for events: the first occurrence may allocate for good, and later ones must not.
- LeakCanary's growth detector added an explicit warm-up "to reduce chances of classloading tied to analysis being picked up as false positive heap growth signal" [primary: [LeakCanary changelog, 3.0 Alpha 6](https://github.com/square/leakcanary/blob/main/docs/changelog.md)].

**What xReveal itself allocates per frame** (skimmed from the source) [inference from code]:

| Stage | What's allocated | Steady after |
|---|---|---|
| Capture → video thread | One-frame slot that replaces any undrawn frame (`src/video/slot.rs`, spec §3.3) | First frame |
| MJPEG decode (Windows) | One reused `Vec<u8>`, `resize`d per frame (`src/video/mjpeg.rs`) | First frame at a given size |
| GPU upload (Windows) | Dynamic D3D11 textures, keyed on (layout, width, height) and remade only when that key changes (`src/platform/windows/render.rs`) | First frame per format |
| Metal textures (macOS) | Made once per IOSurface ID, cache capped at `MAX_SURFACES` (`src/platform/macos/render.rs`); a per-frame `autoreleasepool` around the draw | First lap of the card's buffer pool |
| Per-frame OS objects | MF `IMFSample` and `ConvertToContiguousBuffer` (Windows), `CMSampleBuffer`/`CVPixelBuffer` (macOS) | Reference-counted, recycled by OS pools |
| Audio | A ring sized for 1.5 s up front; the output callback's buffers are "allocated up front" (`src/audio/ring.rs`, `src/audio/pipeline.rs`) | Stream start |

**What this means:**
- xReveal's own state settles within seconds.
- The unknowns are the parts we don't own: Media Foundation and CoreMedia pools, the GPU driver, GPUI's atlases, and the allocator's arenas.
- **10 minutes is a conservative fixed warm-up.** It costs little against a 2-hour run.
- The first baseline run (#194, and its macOS twin) should plot the whole curve once to confirm the footprint is flat by minute 10. If it isn't, lengthen the warm-up rather than loosen the rule.
- The rising-windows condition (b) also guards against slow settling. Settling flattens out, so W3 → W4 stops rising.

## 2. Run length vs. the size of leak you can detect

### Leak rate per frame (MB = 10⁶ bytes)

At 50 fps a run sees **180,000 frames an hour**. At 60 fps it sees 216,000.

| Leak per frame | 50 fps | 60 fps | Growth over the 2-h window (50 fps) | Verdict |
|---|---|---|---|---|
| 1 B (e.g. a `Vec<u8>` log grown by a byte a frame) | 0.18 MB/h | 0.22 MB/h | 0.36 MB | Below the threshold on purpose: 4.3 MB a day |
| 5.6 B | **1.0 MB/h** | 1.2 MB/h | 2 MB | The threshold |
| 16 B (one smallest heap block) | 2.9 MB/h | 3.5 MB/h | 5.8 MB | FAIL |
| 64 B (one small `Box`/`String`) | 11.5 MB/h | 13.8 MB/h | 23 MB | FAIL, visible in about 15 min |
| 1 KiB | 184 MB/h | 221 MB/h | 369 MB | FAIL; adds 100 MB in about 33 min |
| One 1080p NV12 frame (3.1 MB) | 156 MB **per second** | 187 MB/s | — | Out of memory in about a minute; no soak needed |

### How much the noise hides

Fit a least-squares line through *n* evenly spaced samples over a span *T*, with sample noise σ. The slope's standard error is [primary for the method: [NIST/SEMATECH e-Handbook §1.4.2.4.3](https://www.itl.nist.gov/div898/handbook/eda/section4/eda4243.htm), which tests drift by fitting a line and testing whether the slope is zero; the arithmetic is mine]:

```
SE(slope) = σ / sqrt(Σ(tᵢ − t̄)²) ≈ σ · sqrt(12 / n) / T
```

A leak stands out when its slope is ≥ about 3 × SE. That leaves a 2·SE confidence interval with room to spare.

**For the steady soak**, with 1-min medians (n = 120) over T = 2 h:

| Noise σ of a 1-min median | SE(slope) | Smallest leak that stands out (3·SE) |
|---|---|---|
| 0.5 MB | 0.08 MB/h | 0.24 MB/h ≈ 1.3 B/frame |
| 1 MB | 0.16 MB/h | 0.47 MB/h ≈ 2.6 B/frame |
| 2 MB | 0.32 MB/h | 0.95 MB/h ≈ 5.3 B/frame |

- So **2 hours is enough to separate a 1 MB/h leak from flat** as long as the medians' noise stays under about 1.6 MB.
- The rule enforces this itself through the "interval wider than ±0.5 MB/h → INCONCLUSIVE" branch. If a machine turns out to be noisier, the length needed grows as T ∝ σ^(2/3) for a fixed sample interval [inference].

**Two caveats** [inference, but the reason for the rule's shape]:
- **Samples are not independent.** Allocators and drivers grow in steps (a new arena, a driver-side pool). A single legitimate step of size S looks like a slope of S / T.
  - That's why the 10-s samples are reduced to 1-min medians, which removes spikes and transient growth.
  - It's also why the slope alone never fails a run. Condition (b) needs growth in **every** 30-min window, as LeakCanary keeps only objects that "grow on *every* iteration" [primary: [LeakCanary, Detecting heap growth](https://github.com/square/leakcanary/blob/main/docs/heap-growth.md)].
  - A one-off step lifts at most one window boundary. Four flat-noise windows rise every time by chance with probability 1/4! ≈ 4%, and the magnitude condition (a) cuts that further.
- **"Statistically significant" isn't "a leak".** NIST's own case study finds a slope with t = 4.4 and concludes "the drift, while statistically significant, is not practically significant" [primary: [NIST e-Handbook §1.4.2.4.3](https://www.itl.nist.gov/div898/handbook/eda/section4/eda4243.htm)]. That's why the rule fixes a practical threshold (1 MB/h) and uses the confidence interval only to decide between PASS and INCONCLUSIVE.

**Why not hours or days, as Microsoft suggests:**
- Microsoft's PerfMon walkthroughs sample every 600 s over 24 h, because "leaks usually develop slowly. It can take hours for a data leak to accumulate to a detectable level" [primary: [Use PerfMon to find a user-mode memory leak](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/using-performance-monitor-to-find-a-user-mode-memory-leak)].
- That advice is for leaks triggered by sporadic requests. xReveal repeats its whole per-frame path 180,000 times an hour, so a per-frame leak builds up far faster than a per-request one [inference].
- The long tail of "slow" leaks is per **event**, and repetition, not wall time, is what surfaces those (§4).
- An optional overnight run (8 h, same rule) before v0.1.0 would tighten the floor to about 0.1 MB/h, but it isn't needed for the gate.

## 3. Standard criteria

**Slope after warm-up:**
- Microsoft's PAL is the closest thing to a vendor-standard automatic check. It "checks for 10 MB's per hour and 5 MB's per hour increasing trends" in Private Bytes, and the same 10 MB/h for Virtual Bytes and Working Set [primary: [PAL tool](https://learn.microsoft.com/en-us/biztalk/technical-guides/using-the-performance-analysis-of-logs-pal-tool)].
- PAL flags threads at "more than 500 threads and … increasing by 50 threads per hour". It treats handles as a leak on "a large number of handles and/or an aggressive upward trend", which it says "typically results in a memory leak" [primary: same page].
- Microsoft's WMI guide uses the same idea in words: memory or handles that "don't decrease or free … over time" are a memory or handle leak [primary: [Troubleshoot WMI performance issues](https://learn.microsoft.com/en-us/troubleshoot/windows-server/system-management-components/scenario-guide-troubleshoot-wmi-performance-issues)].

**Comparing windows or snapshots:**
- Apple's workflow compares two points: memgraphs taken "pre" and "post" an iteration, `vmmap -summary` to see where the footprint grew, then `heap -diffFrom` to list "the objects that exist in the post memgraph heap but not in the pre" [primary: [WWDC21 "Detect and diagnose memory issues"](https://developer.apple.com/videos/play/wwdc2021/10180/)].
- XCTest's `XCTMemoryMetric` averages iterations and fails when the average passes a baseline, which flags "a regression" (the example was 12% worse) [primary: same session].
- Instruments' *Mark Generation* groups the allocations that persisted from each span of time [primary: [WWDC24](https://developer.apple.com/videos/play/wwdc2024/10173/)].

**Which counter is "memory":**
- **Windows: Private Bytes** = `PROCESS_MEMORY_COUNTERS_EX.PrivateUsage` ("Commit Size" in Task Manager). Working Set moves with OS trimming, so it's recorded for information only [primary: [Memory Performance Information](https://learn.microsoft.com/en-us/windows/win32/memory/memory-performance-information); [PerfMon user-mode leak](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/using-performance-monitor-to-find-a-user-mode-memory-leak)].
  - GPU memory isn't in Private Bytes when it lives on the card. `IDXGIAdapter3::QueryVideoMemoryInfo` "informs the process of the current budget and process usage" for the local and non-local segment groups [primary: [QueryVideoMemoryInfo](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_4/nf-dxgi1_4-idxgiadapter3-queryvideomemoryinfo)].
  - It is called from inside the process, which matters for where the sampler sits (#191).
- **macOS: `phys_footprint`** = dirty + compressed memory, read with `proc_pid_rusage(pid, RUSAGE_INFO_CURRENT, …)` → `ri_phys_footprint`. "It includes both CPU and GPU objects on Apple silicon", so Metal textures are covered [primary: [WWDC22 "Profile and optimize your game's memory"](https://developer.apple.com/videos/play/wwdc2022/10106/); [WWDC21](https://developer.apple.com/videos/play/wwdc2021/10180/)]. It's also the number spec §4 budgets against.

**Handles and other counts:**
- **Windows:** the process handle count (`GetProcessHandleCount`, the Task Manager "Handles" column [primary: [Memory Performance Information](https://learn.microsoft.com/en-us/windows/win32/memory/memory-performance-information)]), plus GDI and USER objects via `GetGuiResources(GR_GDIOBJECTS / GR_USEROBJECTS)` [primary: [GetGuiResources](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getguiresources)].
- **macOS:** threads and open file descriptors (`proc_pidinfo`) [inference: no Apple page fetched for this].
- Counts are integers with almost no noise once settled, so a rising trend shows within minutes. 10/h is a tolerance for thread-pool churn, not a measurement floor. PAL's thread threshold is 50/h [primary].

**A counting allocator (optional, Rust):**
- A `#[global_allocator]` wrapper that counts live bytes and live allocations gives an exact, noise-free view of xReveal's own heap (~4 MB, #118) [inference].
- That drops the floor to well under a byte per frame within minutes, and it is cheap if built into a `--soak` mode (#191).
- It can't see OS, driver or GPU memory, so it adds to the process counters and doesn't replace them.

## 4. Event-driven leaks: input switch, drawer open

**Why the steady soak misses them:**
- An idle steady soak never triggers them, and real use triggers them only tens to hundreds of times a session.
- Every source uses the same approach: **repeat a round trip many times and look for growth that scales with the repeat count.**
  - **LeakCanary** runs a scenario `scenarioLoopsPerDump` times (default 2) between heap dumps, for up to `maxHeapDumps` (default 5). It keeps only what grew by at least the loop count on every dump. The scenario "has to be a **round trip**: it should leave the app where it found it", otherwise "everything it legitimately allocates and keeps looks exactly like a leak" [primary: [LeakCanary, Detecting heap growth](https://github.com/square/leakcanary/blob/main/docs/heap-growth.md)].
  - **Apple** repeats the gallery open/close and reads a "stair step pattern" as a bug. After the fix, the check is to repeat and see "no growth. I'll try it one more time just to be sure" [primary: [WWDC24](https://developer.apple.com/videos/play/wwdc2024/10173/)].
  - **Chromium** found a navigation leak by repeating forward and back 7 times and looking for objects that "leaked exactly 14 times" [primary: [Chromium issue 41495645](https://issues.chromium.org/issues/41495645)].

**How to apply it to xReveal** [inference]:
- **Cycles:**
  - Drawer: open → wait for the animation → close.
  - Input switch: switch away from the Reference card → wait for the signal state → switch back → wait for the picture.
  - Each cycle must end in the same state it started in.
- **Warm-up:** run 5 cycles that aren't counted. They absorb first-time caches, the stair's first step.
- **Measure:** 200 counted cycles, in 5 batches of 40. Sample the footprint (and handles) about 2 s after each cycle, once the state has settled.
- **Detectability:** the slope against the cycle number has SE ≈ σ · sqrt(12 / n³).
  - With n = 200 and σ = 1 MB, that's ≈ 1.2 kB/cycle, so the **10 kB/cycle** threshold is about 8 SE.
  - 10 kB × 1,000 drawer opens in a heavy session is 10 MB, which is about the most a long session should give away.
- **Fail rule:** slope ≥ 10 kB/cycle **and** the five batch medians rise every time. The second condition is LeakCanary's "grows on every iteration", adapted to a scalar.
- **Handles per cycle:** with no noise, a leak of even 1 handle per input switch shows as a slope of 1.0 per cycle. Any handle or GDI/USER slope ≥ 0.5 per cycle with rising batches fails.
- **Time:** an input switch takes a few seconds, so this is about 15–20 min. The drawer takes about 7 min. Both have to be driven by the unattended driver (#196). Switching inputs needs a second source present (the macOS camera or any second UVC device). Flag that as a setup step.

## 5. Recommendation in one place

1. **Setup:** the Reference card, the PS5 idle on a menu, the window at a fixed size (drawables are sized to the window, so resizing moves the footprint), and no user input.
2. **Warm-up:** 10 min, discarded. On the first baseline run per OS, confirm on a plot that the curve is flat by then.
3. **Steady soak:** 120 min. Sample every 10 s, reduce to 1-min medians, and apply the memory and count rules in the table above.
4. **Event cycles:** drawer ×(5 + 200) and input switch ×(5 + 200), with the per-event rule.
5. **Frame health:** presented frames per minute ≥ 95% of the source rate for the whole run.
   - A COM or CF reference leak on `IMFSample` or `CVPixelBuffer` would more likely drain the capture pool and stall frames than grow memory [inference].
6. **Verdict:** PASS, FAIL or INCONCLUSIVE. INCONCLUSIVE re-runs the steady soak at 4 h. It never passes by default.
7. **Absolute ceiling:** separate from growth. See #195.

## Sources

**Microsoft:**
- [Look for potential memory leaks with PerfMon](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/determining-whether-a-leak-exists)
- [Use PerfMon to find a user-mode memory leak](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/using-performance-monitor-to-find-a-user-mode-memory-leak)
- [Find and fix memory leaks](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/finding-a-memory-leak)
- [PAL tool (BizTalk technical guide)](https://learn.microsoft.com/en-us/biztalk/technical-guides/using-the-performance-analysis-of-logs-pal-tool)
- [Troubleshoot WMI performance issues](https://learn.microsoft.com/en-us/troubleshoot/windows-server/system-management-components/scenario-guide-troubleshoot-wmi-performance-issues)
- [Memory Performance Information](https://learn.microsoft.com/en-us/windows/win32/memory/memory-performance-information)
- [GetGuiResources](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getguiresources)
- [IDXGIAdapter3::QueryVideoMemoryInfo](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_4/nf-dxgi1_4-idxgiadapter3-queryvideomemoryinfo)

**Apple:**
- [WWDC21 Detect and diagnose memory issues](https://developer.apple.com/videos/play/wwdc2021/10180/)
- [WWDC22 Profile and optimize your game's memory](https://developer.apple.com/videos/play/wwdc2022/10106/)
- [WWDC24 Analyze heap memory](https://developer.apple.com/videos/play/wwdc2024/10173/)

**Statistics:**
- [NIST/SEMATECH e-Handbook §1.4.2.4.3, drift as a fitted slope](https://www.itl.nist.gov/div898/handbook/eda/section4/eda4243.htm)

**Tools:**
- [LeakCanary, Detecting heap growth](https://github.com/square/leakcanary/blob/main/docs/heap-growth.md)
- [LeakCanary changelog](https://github.com/square/leakcanary/blob/main/docs/changelog.md)
- [Chromium issue 41495645](https://issues.chromium.org/issues/41495645)
