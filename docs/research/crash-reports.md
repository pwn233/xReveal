# Crash reports for an unsigned app, and finding them on the next launch

Research for [#193](https://github.com/pwn233/xReveal/issues/193) (map [#190](https://github.com/pwn233/xReveal/issues/190), feeds [#198](https://github.com/pwn233/xReveal/issues/198)).

**Question.** Where do Windows and macOS keep crash reports for xReveal as it ships (Windows: unsigned, per-user NSIS install in `%LOCALAPPDATA%\Programs\xReveal`; macOS: signed with the stable self-signed "xReveal Self-Signed" certificate, not notarised, not sandboxed)? Can the app read them, or turn on dumps for itself? How does it tell, at the next launch, that it crashed last time? What can a Rust panic hook record, and what happens when the crash is in native code (Media Foundation, Direct3D, CoreMediaIO, Metal)?

**Constraint from the spec.** Nothing is sent except the update check (`docs/spec/v1.md` §2.10, §1 non-goals). The map adds: crash data stays local; the next-launch note may offer a pre-filled GitHub issue the person submits themselves.

Source trust: **[MS]** Microsoft Learn, **[Apple]** Apple developer documentation or Apple DTS staff on the Apple Developer Forums, **[Rust]** doc.rust-lang.org / rust-lang/rust source / the Rust blog, **[obs]** observed on the Windows dev machine on 2026-11-28 (a real xReveal crash from 2026-11-26 was on disk), **[crate]** third-party crate docs (notes only).

---

## 1. Windows

### Where WER puts things

- WER handles crashes of every desktop app by default, with no code in the app: "Beginning with Windows Vista, Windows provides crash, no response, and kernel fault error reporting by default without requiring changes to your application." [MS, About WER]
- **Reports** (a folder per crash holding `Report.wer`) go to `ReportArchive` (handled) or `ReportQueue` (waiting), under either the per-user `%LOCALAPPDATA%\Microsoft\Windows\WER\` or the machine-wide `%ProgramData%\Microsoft\Windows\WER\`. [MS archived blog "WER for developers"; MS Q&A] The official settings page lists `ForceQueue`, `DisableArchive`, `MaxArchiveCount` (default 1000) and `MaxQueueCount` (default 50) but does not name the folders. [MS, WER Settings]
  - [obs] On Windows 11 (build 26300) the xReveal crash landed in the **machine-wide** folder: `C:\ProgramData\Microsoft\Windows\WER\ReportArchive\AppCrash_xreveal.exe_<hash>_<hash>_<guid>\Report.wer`. The per-user `WER\ReportArchive` did not exist. So xReveal must look in both.
  - [obs] `ReportArchive` grants `Authenticated Users` read; a medium-integrity (non-admin) shell read the report.
  - [obs] `Report.wer` is UTF-16LE `key=value` text. Useful keys: `EventTime` (FILETIME), `AppName` and `NsAppName` (`xreveal.exe`), `Sig[0..8]` (app name/version/timestamp, fault module name/version/timestamp, exception offset, **exception code**, exception data), `AppPath` (the exe path), `LoadedModule[n]`, `ReportIdentifier`, `IntegratorReportIdentifier`. The format isn't documented, so treat it as best-effort.
- **Event log.** Each crash also writes an `Application Error` event, ID **1000**, to the Application log (followed by a WER event 1001). It names the faulting app, module, exception code, offset, process ID, start time, path and Report Id. [MS, "Troubleshoot application service crashing behavior"]
  - [obs] `wevtutil qe Application "/q:*[System[(EventID=1000)]]"` from a non-admin shell returned the xReveal event; its `Report Id` equals `IntegratorReportIdentifier` in `Report.wer`, and its process ID (0x12A0 = 4768) matches the dump file name below. An app can do the same query with `EvtQuery` (wevtapi), no admin needed.
- **Dumps (LocalDumps).** Not on by default. When the `HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps` key exists, WER writes dumps to `DumpFolder` (default `%LOCALAPPDATA%\CrashDumps`), keeping `DumpCount` (default 10) of type `DumpType` (default 1, minidump). A per-app subkey `LocalDumps\xreveal.exe` overrides the global values. [MS, Collecting User-Mode Dumps]
  - "Applications that do their own custom crash reporting are not supported by this feature." [MS, same page] → installing our own unhandled-exception filter that handles the crash would stop these dumps.
  - [obs] On the dev machine another vendor's software had created `LocalDumps` (with only per-app subkeys), and that was enough for WER to write `%LOCALAPPDATA%\CrashDumps\xreveal.exe.4768.dmp` (37 MB) for the xReveal crash. Named `<exe>.<pid>.dmp`. On a clean machine there will be no dump.

### Can xReveal turn LocalDumps on for itself without admin?

**No.**
- "Enabling the feature requires administrator privileges", and the documented key is under `HKEY_LOCAL_MACHINE`. [MS, Collecting User-Mode Dumps]
- For each `LocalDumps` value (`DumpFolder`, `DumpCount`, `DumpType`, `CustomDumpFlags`): "This setting is not supported in the **HKEY_CURRENT_USER** registry hive." [MS, WER Settings]
- The per-user NSIS installer runs unelevated, so it can't write HKLM either. Elevating the installer only for this contradicts the per-user install.
- `WerRegisterAppLocalDump` is not a way round: it is for a **packaged** app (path relative to its `LocalFolder`) and covers a process that "stops responding", that is, a hang. [MS, WerRegisterAppLocalDump]
- A person who wants dumps can add the HKLM key themselves (an admin `reg add` line in a support doc or verification card). That's an option for the verification run, not for shipping.

### What a Rust crash looks like to WER

- With the default `panic = "unwind"` (xReveal's `Cargo.toml` sets no profile override), a panic that unwinds to the top of the **main thread** ends the program with exit code 101. [Rust, `panic!` docs] That is a normal exit, so **WER records nothing**.
- A panic on a **spawned thread** ends only that thread. The process keeps running. [Rust Reference, Panic]
- Since Rust 1.81, "the non-unwind ABIs (e.g., "C") will now abort on uncaught unwinds". [Rust blog, 1.81.0] Every COM/Win32 callback (Media Foundation `IMFSourceReaderCallback`, window procedures) is `extern "system"`, so **a panic inside one aborts the process**.
- On Windows, Rust's abort is `__fastfail(FAST_FAIL_FATAL_APP_EXIT)`, which "will terminate the process immediately without running any in-process exception handlers". [Rust, `library/std/src/sys/pal/windows/mod.rs`, `abort_internal`] A user-mode fast fail shows up as exception code **0xC0000409** with the fail code as the first parameter, and "no exception handlers are invoked". [MS, `__fastfail`] `FAST_FAIL_FATAL_APP_EXIT` is 7.
  - [obs] This is exactly the crash on disk: `Exception Code c0000409`, `Exception Data 0000000000000007`, fault module `xreveal.exe`. So that "native crash" was really a Rust panic aborting inside a callback, and **a panic hook would have caught its message**. 0xC0000409 means "stack buffer overrun" only for other fail codes; code 7 means an app-requested abort.
- [obs] WER shows `Application Version 0.0.0.0`: `xreveal.exe` has no `VERSIONINFO` resource. Adding one (a `build.rs` resource) would put the real version into every WER report and event 1000.

### Native crashes (Media Foundation, Direct3D, the GPU driver)

- An access violation inside `mfplat.dll`, `MFCORE.DLL`, `d3d11.dll` or a driver is an SEH exception, not a Rust panic. The panic hook never runs. WER still writes its report and event 1000, naming the faulting module.
- An app can see it in-process with `SetUnhandledExceptionFilter` (or a vectored handler), but the filter "is executed in the context of the thread that caused the fault. This can affect the exception handler's ability to recover from certain exceptions, such as an invalid stack." [MS, SetUnhandledExceptionFilter]
- Writing a dump from inside the crashed process is discouraged: `MiniDumpWriteDump` "should be called from a separate process if at all possible ... A loader deadlock is one of many potential side effects". [MS, MiniDumpWriteDump] Fast fails (Rust aborts, CRT security checks) bypass in-process handlers entirely. [MS, `__fastfail`]
- Handling the exception ourselves also turns off LocalDumps for that crash (see above).

---

## 2. macOS

### Where reports go, and what's in them

- ReportCrash writes a report for any process that crashes. Signing doesn't matter: ad hoc, self-signed and Developer ID apps all get one. The per-user folder is `~/Library/Logs/DiagnosticReports/` (system processes: `/Library/Logs/DiagnosticReports/`). Apple's guide only says to find them in **Console › Crash Reports** (listed by binary name), then "Reveal in Finder". [Apple, Acquiring crash reports and diagnostic logs] The path itself is stated by Apple staff and by Xcode's own error text ("The crash log is located in ~/Library/Logs/DiagnosticReports"). [Apple Developer Forums, Apple staff replies]
- xReveal's report will be named for the executable: `CFBundleExecutable` is `xreveal` (`packaging/macos/Info.plist`), so expect `xreveal-<YYYY-MM-DD-HHMMSS>.ips`. That naming is convention, not documented, so match on the JSON contents, not the file name.
- **Format.** "Starting with iOS 15 and macOS 12, apps that generate crash reports store the data as JSON in files with an .ips extension." The file holds **two JSON objects**: line 1 is metadata (`name`, `bug_type`, `bundleID`, `build_version`, `incident_id`, `timestamp`), and if `bug_type` is `309` the rest of the file is the crash report. [Apple, Interpreting the JSON format of a crash report] Useful crash-object fields: `procName`, `procPath`, `pid`, `procLaunch`, `captureTime`, `bundleInfo.CFBundleShortVersionString`, `exception` (`type`, `signal`), `termination`, `faultingThread`, `threads`/frames, `usedImages`, `asi` (Application Specific Information). [Apple, same page]
  - `isCorpse` / `EXC_CORPSE_NOTIFY` means "the crash didn't originate from a hardware trap, either because the process was explicitly quit by the operating system or the process called abort()". That is how a Rust panic-abort will look. [Apple, Examining the fields in a crash report]
- **Can xReveal read them?** xReveal isn't sandboxed (no entitlements file in `packaging/macos`), and `~/Library/Logs` is the user's own folder, so a plain directory listing and file read should work. **Unverified on a Mac:** confirm on the #118 MacBook that no TCC prompt appears.

### Native crashes (CoreMediaIO, AVFoundation, Metal)

- These arrive as Mach exceptions or signals (`EXC_BAD_ACCESS`/SIGSEGV/SIGBUS, SIGABRT), not as Rust panics. The `.ips` names the faulting thread, frames and binary images.
- Apple DTS strongly discourages an in-process crash reporter: "I strongly advise against implementing your own crash reporter ... It's impossible to implement a *good* crash reporter". A signal handler "can only use async signal safe functions" (no `malloc`, so no Swift, Objective-C or ordinary Rust allocation), and "You must ensure that your crash reporter doesn't disrupt the Apple crash reporter." [Apple DTS (Quinn), "Implementing Your Own Crash Reporter", forums thread 113742]
- On Unix, Rust's `std::process::abort` (and panic-abort) raises SIGABRT, so ReportCrash writes the report as usual.

---

## 3. The Rust panic hook

- `std::panic::set_hook`: "The panic hook is invoked when a thread panics, but before the panic runtime is invoked. As such, the hook will run with both the aborting and unwinding runtimes." It is global, and it runs on the panicking thread. [Rust, `set_hook`] So the hook also catches panics that then abort, whether inside an `extern "system"` callback or with `panic = "abort"`.
- What it gets from `PanicHookInfo`: `payload()` and `payload_as_str()` (stable since 1.91) for the message, and `location()` for the file, line and column. [Rust, `PanicHookInfo`] Whether this panic can unwind (`can_unwind`) is nightly only.
- Backtrace: `Backtrace::force_capture()` ignores `RUST_BACKTRACE`, but "capturing a backtrace can be an expensive operation". "Without debug information filenames/line numbers will not be reported", and accuracy is best-effort. [Rust, `std::backtrace`] In a release build without debug info, expect function names (from the PDB on Windows or the symbol table on macOS, if not stripped) but no file or line numbers. The panic `location()` is always exact, because it is compiled in.
- The hook can also add: the xReveal version (`CARGO_PKG_VERSION`), OS version, thread name, time, and app state (the active source, the video format, whether capture was running). Keep it short and allocation-light. The process is panicking, not corrupted, so normal Rust (format, open a file, write) is fine here. It must not panic itself: a panic inside a panic hook aborts.
- Keep the default hook's behaviour too (call the old hook from `take_hook`), so `stderr` and `cargo run` output stay the same.

**Limits.** The hook sees only Rust panics. It does not see: access violations or SIGSEGV in native frameworks or drivers; fast fails from the CRT or the OS; stack overflow (Rust prints a message; no hook runs); out-of-memory aborts; being killed (Task Manager, `kill -9`, jetsam); power loss or a BSOD. For all of these, the only records are the run marker (below) and the OS report.

**Out-of-process handlers (notes only).** `crash-handler` 0.8 [crate] installs SEH handlers on Windows, and Mach exception ports plus a SIGABRT handler on macOS. `minidumper` 0.11 [crate] pairs it with a separate monitor process that writes a minidump over IPC. That is the Breakpad/Crashpad model, and the only robust way to get our own dump of a native crash. Cost: a second process, a dependency stack (`minidump-writer`, `crash-context`), a minidump nobody but the developer can read, and on Windows it takes the crash away from WER/LocalDumps. On macOS it sits in front of ReportCrash, which Apple DTS warns against. Not worth it for v0.1.0.

---

## 4. "Did I crash last time?"

### Run marker (both OSes, the primary signal)

1. At start, before the window opens, look for `running.toml` in the settings folder (`%LOCALAPPDATA%\xReveal\` / `~/Library/Application Support/xReveal/`, as in `src/setup.rs`).
2. If it is there, the last run didn't end cleanly. Read it, then go on to match OS reports (below).
3. Write a fresh marker: version, pid, start time (UTC), exe path.
4. On a clean quit (the same path that flushes `setup.toml`), delete it. Also delete it before the update restart and before `--report` exits, so those don't count as crashes.
5. The panic hook writes `last-panic.txt` next to it (message, location, thread, backtrace, version, time) before the runtime unwinds or aborts.

What the marker can't tell apart: a crash, a kill from Task Manager or `kill`, a forced logout, power loss, or a BSOD. Matching the OS report is what separates "crashed" from "was stopped". A leftover marker with no OS report and no `last-panic.txt` should say "xReveal didn't close normally last time", not "crashed". Two instances running at once would share a marker; guard by checking whether the recorded pid is still alive and is xReveal.

### Matching the OS report

- **Windows:** in both `%LOCALAPPDATA%\Microsoft\Windows\WER\ReportArchive|ReportQueue` and `%ProgramData%\Microsoft\Windows\WER\ReportArchive|ReportQueue`, list `AppCrash_xreveal.exe_*` folders, read `Report.wer` (UTF-16LE), and keep those with `EventTime` after the marker's start time and `AppPath` under this install. Alternatively (simpler and documented): query the Application log for event 1000 with `Faulting application name: xreveal.exe` and pid = the marker's pid. Optionally, `%LOCALAPPDATA%\CrashDumps\xreveal.exe.<pid>.dmp` is a dump, if LocalDumps happened to be on.
- **macOS:** list `~/Library/Logs/DiagnosticReports/xreveal*.ips` modified after the marker's start time. Parse line 1 (`bug_type == "309"`), then the body; match `pid` and `procPath` (or `bundleID == io.github.pwn233.xreveal`). Pull out `exception.type`/`signal`, `termination`, the faulting thread's top frames and `asi`.
- Both: a match gives "crashed", the exception code or signal, and the faulting module. Together with `last-panic.txt`, that's enough for the note and a pre-filled issue. Nothing is uploaded; the person submits the issue themselves.

---

## 5. Recommendation

1. **Panic hook plus run marker, both OSes, no new dependencies.** The hook writes `last-panic.txt` (message, location, thread, `Backtrace::force_capture()`, version, OS, a small state summary); the marker is `running.toml` in the settings folder, removed on a clean quit, the update restart and `--report`. This alone would have explained the one real crash on record (a panic that aborted as 0xC0000409/7).
2. **Next launch:** if the marker is left over, look for a matching OS report (Windows: event 1000 by pid, falling back to `Report.wer` in both WER roots; macOS: the `.ips` by pid). Show one quiet note with what was found and an **Open an issue on GitHub** button that pre-fills title and body from the panic text and report summary. Never attach files or upload.
3. **Don't** install an SEH filter or signal handler, and don't ship `crash-handler`/`minidumper` for v0.1.0. They take crashes away from WER/LocalDumps and ReportCrash, and the OS reports already name the faulting module.
4. **Add a Windows `VERSIONINFO` resource** so WER reports and event 1000 carry the real version instead of 0.0.0.0.
5. **For the verification cards (#198):** on the Windows PC, turn on dumps once as admin for the soak (`reg add "HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\xreveal.exe" /v DumpType /t REG_DWORD /d 1`, and similar for `DumpFolder`/`DumpCount`); this is a step a script can't do unelevated. Ship release builds with symbols kept (a `.pdb` next to the release asset on Windows; don't strip on macOS) so reports and dumps can be symbolicated afterwards.
6. **Check on a Mac** that reading `~/Library/Logs/DiagnosticReports` raises no TCC prompt, and see what a forced Rust panic-abort produces there (`isCorpse`, SIGABRT).

---

## Sources

- [MS] Collecting User-Mode Dumps: https://learn.microsoft.com/en-us/windows/win32/wer/collecting-user-mode-dumps
- [MS] WER Settings (HKCU vs HKLM; "not supported in the HKEY_CURRENT_USER registry hive"): https://learn.microsoft.com/en-us/windows/win32/wer/wer-settings
- [MS] About WER: https://learn.microsoft.com/en-us/windows/win32/wer/about-wer
- [MS] WerRegisterAppLocalDump: https://learn.microsoft.com/en-us/windows/win32/api/werapi/nf-werapi-werregisterapplocaldump
- [MS] Troubleshoot application service crashing behavior (events 1000/1001): https://learn.microsoft.com/en-us/troubleshoot/windows-server/performance/troubleshoot-application-service-crashing-behavior
- [MS, archived blog] Windows Error Reporting (WER) for developers (ReportArchive/ReportQueue): https://learn.microsoft.com/en-us/archive/blogs/oanapl/windows-error-reporting-wer-for-developers
- [MS] `__fastfail`: https://learn.microsoft.com/en-us/cpp/intrinsics/fastfail
- [MS] SetUnhandledExceptionFilter: https://learn.microsoft.com/en-us/windows/win32/api/errhandlingapi/nf-errhandlingapi-setunhandledexceptionfilter
- [MS] MiniDumpWriteDump: https://learn.microsoft.com/en-us/windows/win32/api/minidumpapiset/nf-minidumpapiset-minidumpwritedump
- [Apple] Acquiring crash reports and diagnostic logs: https://developer.apple.com/documentation/xcode/acquiring-crash-reports-and-diagnostic-logs
- [Apple] Interpreting the JSON format of a crash report: https://developer.apple.com/documentation/xcode/interpreting-the-json-format-of-a-crash-report
- [Apple] Examining the fields in a crash report: https://developer.apple.com/documentation/xcode/examining-the-fields-in-a-crash-report
- [Apple DTS] Implementing Your Own Crash Reporter: https://developer.apple.com/forums/thread/113742
- [Apple staff, forums] `~/Library/Logs/DiagnosticReports` location, e.g. https://developer.apple.com/forums/thread/703253 and https://developer.apple.com/forums/thread/114418
- [Rust] `std::panic::set_hook`: https://doc.rust-lang.org/std/panic/fn.set_hook.html
- [Rust] `PanicHookInfo`: https://doc.rust-lang.org/std/panic/struct.PanicHookInfo.html
- [Rust] `panic!` (exit code 101): https://doc.rust-lang.org/std/macro.panic.html
- [Rust] `std::backtrace`: https://doc.rust-lang.org/std/backtrace/index.html
- [Rust] Reference, Panic: https://doc.rust-lang.org/reference/panic.html
- [Rust] Rust 1.81.0 (abort on unwind out of `extern "C"`): https://blog.rust-lang.org/2024/09/05/Rust-1.81.0/
- [Rust] `abort_internal` on Windows: https://github.com/rust-lang/rust/blob/master/library/std/src/sys/pal/windows/mod.rs
- [crate] crash-handler 0.8.1: https://docs.rs/crash-handler/
- [crate] minidumper 0.11.0: https://docs.rs/minidumper/
