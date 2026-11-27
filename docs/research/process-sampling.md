# Sampling xReveal's memory, handles, CPU and GPU over time

Research for [#191](https://github.com/pwn233/xReveal/issues/191) (map [#190](https://github.com/pwn233/xReveal/issues/190)). The question is how a script, or xReveal itself, can sample xReveal's own process every few seconds for hours, on Windows and macOS, without admin rights, in the units spec §4 uses:

- CPU ≤ 5% of the **whole machine**, as Task Manager shows it;
- GPU ≤ 10%, the **process's share**;
- memory ≤ 100 MB **OS footprint**.

Facts below are from Microsoft Learn, Apple developer docs and man pages, and Apple's published XNU kernel source. Anything that isn't documented by the OS vendor says so. A few Windows facts were also checked by hand on a Windows 11 machine (build 26300, 16 logical processors, **not elevated**); those are marked *checked here*.

---

## Summary

| | Windows | macOS |
|---|---|---|
| Memory (footprint) | PDH `\Process V2(<name>:<pid>)\Working Set - Private` = Task Manager's **Memory** column | `proc_pid_rusage(…).ri_phys_footprint` = `footprint`'s `phys_footprint` |
| Memory (private / committed) | `Private Bytes` = `PROCESS_MEMORY_COUNTERS_EX.PrivateUsage` = Task Manager's **Commit size** | No commit-charge equivalent. `ri_phys_footprint` is the kernel ledger of dirty memory the process owns; `footprint` breaks it down |
| Handles | `Handle Count` / `GetProcessHandleCount` = Task Manager's **Handles** | No handle table. Open file descriptors via `proc_pidinfo(PROC_PIDLISTFDS)`; Mach ports only in-process or via `top` |
| Threads | `Thread Count` = Task Manager's **Threads** | `proc_pidinfo(PROC_PIDTASKINFO).pti_threadnum` |
| CPU, whole machine | `% Processor Time` ÷ logical processors (or `GetProcessTimes` deltas ÷ wall time ÷ logical processors) | (`ri_user_time` + `ri_system_time`) delta, in Mach ticks → ns, ÷ wall time ÷ logical CPUs |
| GPU, process share | PDH `\GPU Engine(pid_<pid>_*)\Utilization Percentage`, max over engines = Task Manager's **GPU** column | Not documented by Apple. In-process: Metal command-buffer GPU times. External: IORegistry `AGXDeviceUserClient` `accumulatedGPUTime` (undocumented) |
| Admin needed? | No (same user) | No (same user), except `powermetrics` (root) |

**Recommendation: an external sampler script per OS**, not an in-process sampler. It is closest to what Task Manager and Activity Monitor show, it adds nothing to xReveal's own footprint or threads, and it keeps sampling if xReveal hangs or crashes (which the crash-capture check on #190 needs). The one thing that needs xReveal's help is macOS GPU time if the undocumented IORegistry counter proves unreliable; see [Recommendation](#recommendation).

---

## Windows

### Memory

| Quantity | API | PDH counter (`Process V2`) | Task Manager column |
|---|---|---|---|
| Private working set (RAM only) | none | `Working Set - Private` | **Memory** (Processes tab) / **Memory (private working set)** (Details) |
| Private committed memory | `PROCESS_MEMORY_COUNTERS_EX.PrivateUsage` (= `PagefileUsage`) | `Private Bytes` (= `Page File Bytes`) | **Commit size** |
| Working set (private + shared) | `WorkingSetSize` | `Working Set` | **Working set (memory)** |
| GPU memory | none (Win32) | `\GPU Process Memory(pid_<pid>_*)\Dedicated Usage`, `Shared Usage` | **Dedicated / Shared GPU memory** (Details) |

- Microsoft's *Memory Performance Information* page maps each Process counter to the API field and to the Task Manager column: `Private Bytes` ↔ `PrivateUsage` ↔ **Commit Size**; `Working Set - Private` ↔ (no API) ↔ **Private Working Set**; `Handle Count` ↔ **Handles**; `Thread Count` ↔ **Threads**. [MS-MEMPERF]
- `PrivateUsage` / `PagefileUsage` is "the Commit Charge value in bytes for this process … the total amount of private memory that the memory manager has committed". `PROCESS_MEMORY_COUNTERS_EX2` adds `PrivateWorkingSetSize`, so the private working set *is* available from the API, but only on Windows 10 22H2 / Windows 11 22H2 with the September 2023 cumulative update or later. [MS-PMC-EX2]
- `GetProcessMemoryInfo` needs only `PROCESS_QUERY_LIMITED_INFORMATION` on the process handle. [MS-GPMI]
- Per-process GPU memory: Task Manager's Details-tab counters "represent the total amount of dedicated and shared video memory currently in use by that process"; per-process values can include memory shared across processes. [MS-GPUTM]
- **Which is "the OS footprint" for §4?** Task Manager's default **Memory** column is the private working set. It drops when Windows trims the working set, so for leak detection log **`Private Bytes`** too: commit only grows if something keeps allocating.

### Handles and threads

- `\Process V2(…)\Handle Count` and `Thread Count`, or `GetProcessHandleCount` (needs `PROCESS_QUERY_INFORMATION` or `PROCESS_QUERY_LIMITED_INFORMATION`). The count covers the process's executive (kernel) objects. [MS-GPHC], [MS-MEMPERF]
- GDI and USER object counts (Task Manager's **GDI objects** / **USER objects** columns) are separate: `GetGuiResources`. Not researched further here.

### CPU as a share of the whole machine

- `\Process(X)\% Processor Time` "is the sum of the processor usage by all threads of process X … The range for this counter is from 0 through 100 * ProcessorCount." `\Processor(_Total)\% Processor Time` is 0–100. [MS-COLLECT]
  - *Checked here:* a single-thread busy loop read 99.2–99.9 on `\Process V2(…)\% Processor Time` on a 16-thread machine. So **whole-machine share = `% Processor Time` ÷ logical processor count**.
- `GetProcessTimes` gives summed kernel and user time over all threads in 100 ns units, which "can exceed the amount of real time elapsed … if the process executes across multiple CPU cores". Needs `PROCESS_QUERY_LIMITED_INFORMATION`. [MS-GPT]
  - *Checked here:* Δ(kernel+user) ÷ Δwall = 0.9996 for the same busy loop, matching the counter.
- Rate counters need two samples; PDH keeps the previous sample and computes the value from the pair, with at least one second between them. [MS-COLLECT], [MS-CALC]
- **Task Manager's CPU number changed in 2025.** Windows 11 now "use[s] the standard metrics to display CPU workload consistently across all pages", and the old Processes-page value moved to an optional **CPU Utility** column on the Details tab. [WIN-26120] The old value was frequency-scaled (`% Processor Utility`), so on a turbo-boosting CPU it reads higher than `% Processor Time` ÷ N. The `Process` counter set has no `% Processor Utility`, so a script matches the *current* Task Manager, not the old one. Record the Windows build alongside the samples.

### GPU as the process's share

- Task Manager's per-process **GPU** column shows "the utilization of the busiest engine, across all GPUs, for that process", and a second column names that engine. The data "is gathered directly from VidSch and VidMm", the WDDM scheduler and memory manager, and needs a WDDM 2.0+ driver. [MS-GPUTM]
- The same data is exposed as PDH counters `\GPU Engine(*)\Utilization Percentage` and `\GPU Engine(*)\Running Time`, with one instance per process × adapter × engine, for example `pid_10848_luid_0x00000000_0x0000D2B2_phys_0_eng_0_engtype_3D` (*checked here*). Microsoft Learn has no reference page for this counter set; the instance format comes from `typeperf -qx "GPU Engine"`.
- **To match Task Manager:** for xReveal's PID, take the **maximum** over its `GPU Engine` instances (busiest engine), not the sum. Log the `engtype_3D` value too, since that's where xReveal's video pass runs.
- *Checked here:* `typeperf "\GPU Engine(pid_7996*)\Utilization Percentage"` works unelevated and accepts a `pid_…*` wildcard.

### Tools

| Tool | Gives | Unattended? | Admin? |
|---|---|---|---|
| `typeperf` | Any PDH counter at `-si` interval, `-sc` count, CSV via `-f CSV -o file`, counter list via `-cf file` [MS-TYPEPERF] | Yes | No (*checked here*) |
| PowerShell `Get-Counter` | Same counters, as objects | Yes | No |
| Win32 API (`OpenProcess` + `GetProcessMemoryInfo` / `GetProcessTimes` / `GetProcessHandleCount`, PDH for GPU) | All of the above | Yes | No, same user. `PROCESS_QUERY_LIMITED_INFORMATION` is even allowed against protected processes [MS-PSAR] |
| ETW / WPR | Full traces (CPU sampling, GPU, heap) | Yes, but traces grow (`-filemode` "can grow in size until it fills the disk") [MS-WPR] | **Yes**: only admins, the *Performance Log Users* group and services can control trace sessions, and only admins/LocalSystem can run the NT Kernel Logger [MS-STARTTRACE] |

- **Use `Process V2`, not `Process`.** The old counter set names instances by EXE name only, so PDH can pair samples from different processes and give out-of-range CPU values when a same-named process starts or stops. `Process V2` (Windows 11+) puts the PID in the instance name. [MS-COLLECT] *Checked here:* instance names look like `explorer:7996` (no `.exe`).
- ETW/WPR is for diagnosing a regression after the fact (#118 used the macOS equivalents), not for hours-long unattended sampling.

### Overhead on xReveal

- PDH and the `GetProcess*` calls read kernel bookkeeping from **another** process. Nothing is injected into xReveal, and it does no extra work. The cost is the sampler's own CPU.
- *Checked here:* `typeperf` with 5 counters (CPU, handles, private working set, GPU Engine wildcard, GPU memory) at 1 s for 20 samples used 0.31 s of CPU in total, including start-up: ~1.5% of one core, ~0.1% of the 16-thread machine, billed to `typeperf.exe`, not xReveal. At a 5 s interval it is lower still.

---

## macOS

### Memory

- **`phys_footprint`** is "the current value of the kernel ledger tracking the amount of dirty memory owned by a process"; `footprint`'s default breakdown is built to match it. [MAN-FOOTPRINT] This is the "memory footprint" #118 and §4 already use.
- In code: `proc_pid_rusage(pid, RUSAGE_INFO_V4…, &ri)` → `ri.ri_phys_footprint` (and `ri_lifetime_max_phys_footprint` for the peak). Apple's WWDC22 memory session gives this call as the way to read the footprint on any Apple platform. [WWDC22-MEM], [XNU-RESOURCE], [XNU-LIBPROC]
- In-process alternative: `task_info(mach_task_self(), TASK_VM_INFO, …)` → `phys_footprint` (plus `ledger_tag_graphics_footprint` etc. for a GPU split). [XNU-TASKINFO]
- `footprint` also has a sampling mode: `--sample <secs>` (fractional allowed) with `--sample-duration`, and JSON output (`-j`) with a `samples` array. "`footprint` must be run as root when inspecting processes that are not owned by the current user", so no root is needed for our own app. [MAN-FOOTPRINT] It's heavier than one syscall; use it for the breakdown at the start and end of a run, not every few seconds.
- `leaks` / `heap` / `vmmap` scan the target's memory. [MAN-LEAKS] They're for the leak *diagnosis* card, not for periodic sampling.
- **Activity Monitor's Memory column:** Apple's Activity Monitor guide doesn't define the per-process columns. That it equals `phys_footprint` is a widely repeated third-party finding, consistent with #118's numbers, but not an Apple statement.

### Handles and threads

- **Threads:** `proc_pidinfo(pid, PROC_PIDTASKINFO, …)` → `pti_threadnum` ("number of threads in the task"). [XNU-PROCINFO-H]
- **Handles:** macOS has no single handle count. The nearest equivalents are:
  - open file descriptors, from `proc_pidinfo(pid, PROC_PIDLISTFDS, …)` (same-user check, below) or `lsof -p`;
  - Mach port names (Activity Monitor's **Ports** column, `top -stats ports`). Reading another process's port names needs its task port, which a normal unsigned script can't get, so log ports either through `top` or from inside xReveal (`mach_port_names(mach_task_self(), …)`).

### CPU as a share of the whole machine

- `proc_pid_rusage` → `ri_user_time` + `ri_system_time`, or `PROC_PIDTASKINFO` → `pti_total_user` + `pti_total_system`. Both are filled from the kernel's `recount` **Mach-time** totals (`rtm_user`, `rm_time_mach`), not nanoseconds. [XNU-BSDKERN], [XNU-TASK] Convert with `mach_timebase_info` before dividing by wall time; on Apple Silicon one tick isn't 1 ns.
- `top` and Activity Monitor count **100% = one core** (as #118 found). Divide by the logical CPU count (`sysctl hw.logicalcpu`) for §4's whole-machine share.
- `top -l <n> -s <secs> -pid <pid> -stats pid,cpu,mem,th,ports` logs samples. In the default mode CPU is computed "since the previous sample" [MAN-TOP], so drop the first sample.

### GPU as the process's share

Apple documents **no** per-process GPU-utilisation API for a script. Options:

1. **IORegistry, `AGXDeviceUserClient` → `AppUsage` → `accumulatedGPUTime`** (Apple Silicon). Each Metal client of the GPU is a registry entry whose `IOUserClientCreator` names the owning PID. Sum `accumulatedGPUTime` per PID and divide the delta by wall time. It's readable without root via `ioreg -r -c AGXDeviceUserClient -l -a` or IOKit. This is **undocumented**: Apple doesn't define the key or its unit, and open-source tools that use it treat it as nanoseconds. That's very likely the "process's `accumulatedGPUTime`" measured in #118. Calibrate it once against a Metal System Trace.
2. **In-process, documented:** `MTLCommandBuffer.gpuStartTime` / `gpuEndTime` on each completed command buffer. That gives xReveal's own GPU busy time per frame (#125's `shader_gpu_time` is of this kind), but only for command buffers xReveal can see. GPUI's own buffers are out of reach unless GPUI exposes them.
3. **`powermetrics --samplers tasks --show-process-gpu`**: needs root ("must be invoked as the superuser"), so it fails the no-admin rule. GPU clock and residency (`gpu_power`) are only available here.
4. **Instruments / Metal System Trace** (`xctrace`): authoritative, but a trace, not hours of sampling. Use it for calibration.

Spec §4 notes the GPU share depends on GPU clock state (#125: ~90% of the time at the lowest clock). A utilisation sample is "busy time ÷ wall time" at whatever clock the GPU picked, so the card should report it as Activity Monitor would, not normalise it.

### Permissions

- `proc_pid_rusage` and `proc_pidinfo(PROC_PIDTASKINFO / PROC_PIDLISTFDS)` go through `proc_security_policy(…, CHECK_SAME_USER)`: if the caller's UID differs from the target's, the call fails with `EPERM` unless the caller holds a privilege that root has by default. **Same user → no root, no entitlement.** [XNU-PROCINFO]
- `footprint`: root only for other users' processes. [MAN-FOOTPRINT]
- `powermetrics`: root always.

### Overhead on xReveal

- `proc_pid_rusage` / `proc_pidinfo` are single syscalls that read the target's kernel accounting. The target does no work.
- `ioreg -r -c AGXDeviceUserClient` walks part of the IORegistry in the sampler's process. Open-source tools report on the order of 10 ms per refresh. Cheap at a 5 s interval.
- `footprint`, `vmmap`, `leaks` and `heap` inspect the target's address space. `footprint` notes some modes are "expensive and cannot be done in real time" [MAN-FOOTPRINT]. Keep these to start-of-run and end-of-run snapshots.

---

## How the numbers line up with §4

| §4 budget | Windows sample | macOS sample |
|---|---|---|
| CPU ≤ 5% of the machine | `Process V2 % Processor Time` ÷ logical CPUs = Task Manager's Processes-tab **CPU** (current Windows 11) | Δ(`ri_user_time`+`ri_system_time`) in ns ÷ Δwall ÷ `hw.logicalcpu` = Activity Monitor's **% CPU** ÷ cores |
| GPU ≤ 10% | max over xReveal's `GPU Engine` instances = Task Manager's **GPU** | Δ`accumulatedGPUTime` ÷ Δwall (undocumented; ≈ Activity Monitor's **% GPU**) |
| Memory ≤ 100 MB | `Working Set - Private` = Task Manager's **Memory**; also log `Private Bytes` (**Commit size**) | `ri_phys_footprint` = `footprint`'s `phys_footprint` (≈ Activity Monitor's **Memory**) |

---

## Recommendation

**An external sampler script per OS, writing one CSV row per sample (every 5 s), started by the run driver before xReveal and kept running after it exits.**

Why external rather than a `--soak` mode inside xReveal:

- It reads the same kernel bookkeeping Task Manager and Activity Monitor read, so its numbers are the §4 units with no translation.
- It adds no thread, allocation or wake-up to the process being measured. An in-process sampler would show up in xReveal's own thread count and footprint, which a leak check is trying to watch.
- It survives xReveal hanging or crashing, and can record that the PID went away and when. The crash-capture check needs that.
- Nothing needs admin on either OS.

**Windows counters** (one `typeperf -cf counters.txt -si 5 -f CSV -o samples.csv`, or `Get-Counter`, after resolving xReveal's PID):

```
\Process V2(xreveal:<pid>)\% Processor Time      ÷ logical CPUs → CPU % of machine
\Process V2(xreveal:<pid>)\Working Set - Private → Task Manager "Memory"
\Process V2(xreveal:<pid>)\Private Bytes         → "Commit size" (leak trend)
\Process V2(xreveal:<pid>)\Handle Count
\Process V2(xreveal:<pid>)\Thread Count
\GPU Engine(pid_<pid>_*)\Utilization Percentage   → max over instances = Task Manager "GPU"
\GPU Process Memory(pid_<pid>_*)\Dedicated Usage
\GPU Process Memory(pid_<pid>_*)\Shared Usage
```

Check the exact instance name once with `typeperf -qx "Process V2"`; it is the image name without `.exe`. A `typeperf` CSV keeps a fixed column set, so a `GPU Engine` instance that appears later (a new engine used) won't be picked up. A small PowerShell loop around `Get-Counter` that re-expands the wildcard each sample avoids that.

**macOS counters** (a small script or compiled helper calling libproc, plus `ioreg`):

```
proc_pid_rusage(pid, RUSAGE_INFO_V4)  → ri_phys_footprint, ri_lifetime_max_phys_footprint,
                                        ri_user_time + ri_system_time (Mach ticks → ns via mach_timebase_info)
proc_pidinfo(pid, PROC_PIDTASKINFO)   → pti_threadnum
proc_pidinfo(pid, PROC_PIDLISTFDS)    → open file descriptors (size of the returned list)
ioreg -r -c AGXDeviceUserClient -l -a → Σ accumulatedGPUTime for IOUserClientCreator "pid <pid>, …"
top -l 0 -s 5 -pid <pid> -stats pid,cpu,mem,th,ports   (optional cross-check and Mach port count)
```

Plus one `footprint -p <pid> -j start.json` at the start and end of the run, for the breakdown if memory grew.

**Where xReveal itself should help (small, optional):** on macOS, if the IORegistry GPU counter disagrees with a Metal System Trace, have xReveal log its own command buffers' `gpuEndTime − gpuStartTime` to the same run log. That's the only §4 number without a documented external source.

---

## Sources

- **[MS-MEMPERF]** Microsoft Learn, *Memory Performance Information*: https://learn.microsoft.com/en-us/windows/win32/memory/memory-performance-information
- **[MS-PMC-EX2]** Microsoft Learn, *PROCESS_MEMORY_COUNTERS_EX2*: https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex2
- **[MS-GPMI]** Microsoft Learn, *GetProcessMemoryInfo*: https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getprocessmemoryinfo
- **[MS-GPHC]** Microsoft Learn, *GetProcessHandleCount*: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesshandlecount
- **[MS-GPT]** Microsoft Learn, *GetProcessTimes*: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes
- **[MS-PSAR]** Microsoft Learn, *Process Security and Access Rights*: https://learn.microsoft.com/en-us/windows/win32/procthread/process-security-and-access-rights
- **[MS-COLLECT]** Microsoft Learn, *Collecting Performance Data* (multi-processor counters, `Process V2`): https://learn.microsoft.com/en-us/windows/win32/perfctrs/collecting-performance-data
- **[MS-CALC]** Microsoft Learn, *Calculating Counter Values*: https://learn.microsoft.com/en-us/windows/win32/perfctrs/calculating-counter-values
- **[MS-TYPEPERF]** Microsoft Learn, *typeperf*: https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/typeperf
- **[MS-WPR]** Microsoft Learn, *WPR Command-Line Options*: https://learn.microsoft.com/en-us/windows-hardware/test/wpt/wpr-command-line-options
- **[MS-STARTTRACE]** Microsoft Learn, *StartTraceW* (who may control trace sessions): https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-starttracew
- **[MS-GPUTM]** Microsoft DirectX Developer Blog, *GPUs in the Task Manager*: https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/
- **[WIN-26120]** Windows Insider Blog, *Announcing Windows 11 Insider Preview Build 26120.3360* (Task Manager CPU change, CPU Utility column): https://blogs.windows.com/windows-insider/2025/02/28/announcing-windows-11-insider-preview-build-26120-3360-dev-and-beta-channels/
- **[WWDC22-MEM]** Apple, WWDC22 *Profile and optimize your game's memory* (footprint via `proc_pid_rusage`): https://developer.apple.com/videos/play/wwdc2022/10106/
- **[MAN-FOOTPRINT]** `footprint(1)` man page (Xcode command-line tools): https://keith.github.io/xcode-man-pages/footprint.1.html (also `man footprint` on the Mac)
- **[MAN-LEAKS]** `leaks(1)` man page: https://keith.github.io/xcode-man-pages/leaks.1.html
- **[MAN-TOP]** `top(1)` man page (`man top` on the Mac; mirror: https://ss64.com/mac/top.html)
- **[XNU-LIBPROC]** Apple XNU source, `libsyscall/wrappers/libproc/libproc.h` (`proc_pid_rusage`, `proc_pidinfo`): https://github.com/apple-oss-distributions/xnu/blob/main/libsyscall/wrappers/libproc/libproc.h
- **[XNU-RESOURCE]** Apple XNU source, `bsd/sys/resource.h` (`rusage_info_v0…v6`, `ri_phys_footprint`): https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/resource.h
- **[XNU-PROCINFO-H]** Apple XNU source, `bsd/sys/proc_info.h` (`proc_taskinfo.pti_threadnum`, `PROC_PIDLISTFDS`): https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info.h
- **[XNU-PROCINFO]** Apple XNU source, `bsd/kern/proc_info.c` (`proc_security_policy`, `CHECK_SAME_USER`): https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/proc_info.c
- **[XNU-BSDKERN]** Apple XNU source, `osfmk/kern/bsd_kern.c` (`fill_task_rusage`, `pti_total_user` from Mach-time totals): https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/kern/bsd_kern.c
- **[XNU-TASK]** Apple XNU source, `osfmk/kern/task.c` (`task_power_info_locked`, `rm_time_mach`): https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/kern/task.c
- **[XNU-TASKINFO]** Apple XNU source, `osfmk/mach/task_info.h` (`TASK_VM_INFO.phys_footprint`, graphics ledgers): https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/mach/task_info.h
- Apple, *MTLCommandBuffer.gpuStartTime*: https://developer.apple.com/documentation/metal/mtlcommandbuffer/gpustarttime
- Apple, *Gathering information about memory use*: https://developer.apple.com/documentation/xcode/gathering-information-about-memory-use
