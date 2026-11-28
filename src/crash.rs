//! After a crash (docs/spec/v1.md §2.11, #198, research #193).
//!
//! Three files in the settings folder, next to `setup.toml`, with nothing personal in them:
//! - `running.toml`, the run marker: written at launch, removed on a clean quit, before the update
//!   restart and when `--report` exits. Still there at the next launch: the last run didn't end
//!   cleanly.
//! - `last-panic.txt`, from the panic hook, before the process unwinds or aborts.
//! - `xreveal.log` (see [`crate::log`]).
//!
//! On the next launch a leftover marker is matched, off the UI thread, to `last-panic.txt` and to
//! the OS's own report of that process (Windows: event 1000 or `Report.wer`; macOS: the `.ips`).
//! What was found becomes a small note over the picture, and maybe a pre-filled GitHub issue that
//! the person checks and submits in their own browser. xReveal sends nothing itself.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::setup::Setup;

pub const MARKER: &str = "running.toml";
pub const PANIC_FILE: &str = "last-panic.txt";
/// The pre-filled issue's body, URL-encoded, stays under this: GitHub refuses very long URLs.
pub const BODY_CAP: usize = 6 * 1024;
/// The log lines and stack frames the issue carries at most.
pub const LOG_LINES: usize = 30;
pub const TOP_FRAMES: usize = 10;
/// The note goes by itself after this.
pub const NOTE_FOR: Duration = Duration::from_secs(30);
/// `XREVEAL_CRASH_TEST` crashes this long after the picture shows.
pub const CRASH_TEST_AFTER: Duration = Duration::from_secs(10);

pub const CRASHED: &str = "xReveal crashed last time. Reporting it helps fix it: the issue opens in your browser for you to check and send.";
pub const NOT_CLEAN: &str = "xReveal didn't close normally last time. If something went wrong, its log is in the settings folder.";
const FOOTER: &str = "Pre-filled by xReveal. Nothing was sent until you pressed Submit.";

/// The settings folder (`setup.toml`'s), where the crash files live.
pub fn folder() -> Option<PathBuf> {
    crate::setup::default_path()?.parent().map(Path::to_owned)
}

// ---------------------------------------------------------------------------------------------
// Start and end of a run.

/// What the run marker says about the last run, read at launch.
#[derive(Clone, Debug, PartialEq)]
pub enum LastRun {
    /// No marker: it quit cleanly (or this is the first run).
    Clean,
    /// A marker was left over: it didn't end cleanly. What it recorded, when it could be read.
    Leftover(Option<RunMarker>),
    /// The marker belongs to an xReveal that is still running: not a crash, and its marker is
    /// left alone.
    AlsoRunning,
}

/// `running.toml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RunMarker {
    pub version: String,
    pub pid: u32,
    /// UTC, `2026-11-28T12:01:29Z`.
    pub started: String,
}

impl RunMarker {
    pub fn now() -> Self {
        Self { version: crate::whats_new::VERSION.into(), pid: std::process::id(), started: utc(crate::update::now()) }
    }

    /// The start time in Unix seconds.
    pub fn started_at(&self) -> Option<u64> {
        parse_utc(&self.started)
    }
}

/// This run's marker, while it's ours to remove: its path and its text.
static OURS: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);

/// At launch: reads what the last run left, and writes this run's marker (unless another xReveal
/// is still running, whose marker it then leaves alone).
pub fn begin(folder: &Path) -> LastRun {
    let path = folder.join(MARKER);
    let previous = match std::fs::read_to_string(&path) {
        Ok(text) => Some(toml::from_str::<RunMarker>(&text).ok()),
        Err(_) => None,
    };
    if let Some(Some(marker)) = &previous
        && marker.pid != std::process::id()
        && crate::platform::process_is_xreveal(marker.pid)
    {
        return LastRun::AlsoRunning;
    }
    if let Ok(text) = toml::to_string(&RunMarker::now()) {
        *OURS.lock().unwrap_or_else(|e| e.into_inner()) = Some((path, text));
        resume();
    }
    match previous {
        None => LastRun::Clean,
        Some(marker) => LastRun::Leftover(marker),
    }
}

/// Writes this run's marker (again, after [`end`] when an update failed to start).
pub fn resume() {
    if let Some((path, text)) = OURS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
        let temporary = path.with_extension("toml.tmp");
        if std::fs::write(&temporary, text).is_ok() {
            let _ = std::fs::rename(&temporary, path);
        }
    }
}

/// A clean end: removes this run's marker (only if it's still this run's).
pub fn end() {
    if let Some((path, text)) = OURS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        && std::fs::read_to_string(path).is_ok_and(|on_disk| on_disk == *text)
    {
        let _ = std::fs::remove_file(path);
    }
}

/// What `state` and the log say about the last run.
pub fn describe(last: &LastRun) -> &'static str {
    match last {
        LastRun::Clean => "clean",
        LastRun::Leftover(_) => "not_clean",
        LastRun::AlsoRunning => "another xReveal is running",
    }
}

// ---------------------------------------------------------------------------------------------
// The panic hook.

/// The OS and its build, worked out once at launch (the panic hook mustn't go looking).
static OS: OnceLock<String> = OnceLock::new();
/// A short summary of the setup, kept current as it changes.
static SETUP: Mutex<String> = Mutex::new(String::new());
static FOLDER: OnceLock<PathBuf> = OnceLock::new();

pub fn os() -> &'static str {
    OS.get_or_init(crate::platform::os_version)
}

/// Starts the log and installs the panic hook. Call first thing in `main`.
pub fn init() {
    let _ = os();
    if let Some(folder) = folder() {
        crate::log::open(&folder);
        let _ = FOLDER.set(folder);
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Nothing in here may panic: a panic inside the hook aborts at once.
        record_panic(info);
        previous(info);
    }));
}

/// Keeps the panic file's setup line current. Called whenever the Remembered setup changes.
pub fn remember_setup(setup: &Setup) {
    let line = setup_summary(setup).iter().map(|(label, value)| format!("{label} {value}")).collect::<Vec<_>>().join(" · ");
    if let Ok(mut summary) = SETUP.try_lock() {
        *summary = line;
    }
}

fn record_panic(info: &std::panic::PanicHookInfo<'_>) {
    let message = info.payload_as_str().unwrap_or("(no message)");
    let location = info.location().map_or_else(|| "unknown".to_owned(), |l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
    let thread = std::thread::current();
    let thread = thread.name().unwrap_or("unnamed");
    crate::log::from_panic_hook(&format!("{message} at {location} on thread {thread}"));
    let Some(folder) = FOLDER.get() else { return };
    let setup = match SETUP.try_lock() {
        Ok(summary) => summary.clone(),
        Err(_) => String::new(),
    };
    let backtrace = std::backtrace::Backtrace::force_capture().to_string();
    let text = panic_text(&PanicText {
        time: utc(crate::update::now()),
        message,
        location: &location,
        thread,
        os: OS.get().map_or("", String::as_str),
        setup: &setup,
        backtrace: &backtrace,
    });
    let _ = std::fs::write(folder.join(PANIC_FILE), scrub(&text));
}

struct PanicText<'a> {
    time: String,
    message: &'a str,
    location: &'a str,
    thread: &'a str,
    os: &'a str,
    setup: &'a str,
    backtrace: &'a str,
}

/// `last-panic.txt`: a header of `key: value` lines, then the message, then the backtrace.
fn panic_text(p: &PanicText<'_>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "xReveal panicked");
    let _ = writeln!(out, "version: {}", crate::whats_new::VERSION);
    let _ = writeln!(out, "time: {}", p.time);
    let _ = writeln!(out, "location: {}", p.location);
    let _ = writeln!(out, "thread: {}", p.thread);
    let _ = writeln!(out, "os: {}", p.os);
    let _ = writeln!(out, "setup: {}", p.setup);
    let _ = writeln!(out, "\nmessage:\n{}", p.message.trim_end());
    let _ = writeln!(out, "\nbacktrace:\n{}", p.backtrace.trim_end());
    out
}

/// What `last-panic.txt` holds, read back.
#[derive(Clone, Debug, PartialEq)]
pub struct PanicReport {
    /// When it happened (Unix seconds).
    pub time: Option<u64>,
    pub message: String,
    /// `src/app.rs:120:5`.
    pub location: String,
    pub thread: String,
    /// The backtrace's frames from the code that panicked onwards, without the panic machinery.
    pub frames: Vec<String>,
}

pub fn parse_panic(text: &str) -> Option<PanicReport> {
    let (header, rest) = text.split_once("\nmessage:\n")?;
    let (message, backtrace) = rest.rsplit_once("\nbacktrace:\n").unwrap_or((rest, ""));
    let field = |key: &str| header.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(": ")).unwrap_or("").to_owned();
    Some(PanicReport {
        time: parse_utc(&field("time")),
        message: message.trim().to_owned(),
        location: field("location"),
        thread: field("thread"),
        frames: backtrace_frames(backtrace),
    })
}

/// The frames of a `std::backtrace::Backtrace` printout, one line each (`function (file:line)`),
/// starting at the first frame after the panic machinery and the hook.
fn backtrace_frames(backtrace: &str) -> Vec<String> {
    let machinery = ["std::", "core::panicking", "core::panic", "<std::", "<alloc::boxed::Box<", "rust_begin_unwind", "__rustc", "xreveal::crash::"];
    let mut frames: Vec<String> = Vec::new();
    for line in backtrace.lines() {
        let trimmed = line.trim();
        if let Some(at) = trimmed.strip_prefix("at ") {
            // The frame's place: keep the last two parts of the path.
            if let Some(last) = frames.last_mut() {
                let parts: Vec<&str> = at.rsplit(['/', '\\']).take(2).collect();
                let short: Vec<&str> = parts.into_iter().rev().collect();
                let _ = write!(last, " ({})", short.join("/"));
            }
        } else if let Some((number, name)) = trimmed.split_once(": ")
            && number.chars().all(|c| c.is_ascii_digit())
            && !number.is_empty()
        {
            frames.push(name.to_owned());
        }
    }
    let start = frames.iter().position(|f| !machinery.iter().any(|m| f.starts_with(m))).unwrap_or(0);
    frames.split_off(start)
}

// ---------------------------------------------------------------------------------------------
// The OS's own report.

/// What the OS recorded about a crashed process.
#[derive(Clone, Debug, PartialEq)]
pub struct OsReport {
    pub pid: Option<u32>,
    /// When it crashed (Unix seconds).
    pub time: Option<u64>,
    /// The executable's file name (`xreveal`, `xreveal.exe`).
    pub app: String,
    /// The exception code or signal: `SIGSEGV`, `0xC0000005 (access violation)`.
    pub exception: String,
    /// The faulting module: `libsystem_kernel.dylib`, `mfplat.dll`.
    pub module: String,
    /// Extra facts worth reading (`EXC_BAD_ACCESS`, the termination reason, the offset).
    pub details: Vec<(&'static str, String)>,
    /// The faulting thread's top frames, when the report has them (macOS).
    pub frames: Vec<String>,
}

impl OsReport {
    pub fn is_xreveal(&self) -> bool {
        let stem = self.app.rsplit(['/', '\\']).next().unwrap_or("");
        let stem = stem.strip_suffix(".exe").or_else(|| stem.strip_suffix(".EXE")).unwrap_or(stem);
        stem.eq_ignore_ascii_case("xreveal")
    }
}

/// A macOS `.ips` crash report: line 1 is metadata (`bug_type` 309 for a crash), the rest the
/// report (Apple, "Interpreting the JSON format of a crash report").
#[cfg_attr(not(any(target_os = "macos", test)), expect(dead_code, reason = "only macOS reads its reports"))]
pub fn parse_ips(text: &str) -> Option<OsReport> {
    use serde_json::Value;
    let (first, body) = text.split_once('\n')?;
    let header: Value = serde_json::from_str(first).ok()?;
    if header.get("bug_type").and_then(Value::as_str) != Some("309") {
        return None;
    }
    let body: Value = serde_json::from_str(body).ok()?;
    let text_of = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or("").to_owned();
    let exception = body.get("exception");
    let kind = text_of(exception.and_then(|e| e.get("type")));
    let signal = text_of(exception.and_then(|e| e.get("signal")));
    let images: Vec<String> = body.get("usedImages").and_then(Value::as_array).map_or_else(Vec::new, |images| images.iter().map(|i| text_of(i.get("name"))).collect());
    let faulting = body.get("faultingThread").and_then(Value::as_u64).unwrap_or(0) as usize;
    let frames: Vec<(String, String)> = body
        .get("threads")
        .and_then(Value::as_array)
        .and_then(|threads| threads.get(faulting))
        .and_then(|thread| thread.get("frames"))
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |frames| {
            frames
                .iter()
                .map(|f| {
                    let image = f.get("imageIndex").and_then(Value::as_u64).and_then(|i| images.get(i as usize)).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| "???".into());
                    let place = match (f.get("symbol").and_then(Value::as_str), f.get("symbolLocation").and_then(Value::as_u64)) {
                        (Some(symbol), Some(offset)) => format!("{symbol} + {offset}"),
                        (Some(symbol), None) => symbol.to_owned(),
                        _ => format!("0x{:x}", f.get("imageOffset").and_then(Value::as_u64).unwrap_or(0)),
                    };
                    (image, place)
                })
                .collect()
        });
    let mut details = Vec::new();
    if !kind.is_empty() {
        let subtype = text_of(exception.and_then(|e| e.get("subtype")));
        details.push(("Exception", if subtype.is_empty() { kind.clone() } else { format!("{kind} ({subtype})") }));
    }
    let indicator = text_of(body.get("termination").and_then(|t| t.get("indicator")));
    if !indicator.is_empty() {
        details.push(("Termination", indicator));
    }
    let time = parse_ips_time(&text_of(body.get("captureTime")));
    let app = text_of(body.get("procPath")).rsplit('/').next().unwrap_or("").to_owned();
    Some(OsReport {
        pid: body.get("pid").and_then(Value::as_u64).map(|p| p as u32),
        time,
        app: if app.is_empty() { text_of(body.get("procName")) } else { app },
        exception: if signal.is_empty() { kind } else { signal },
        module: frames.first().map(|(image, _)| image.clone()).unwrap_or_default(),
        details,
        frames: frames.into_iter().map(|(image, place)| format!("{image}  {place}")).collect(),
    })
}

/// `2026-11-28 12:01:39.8123 +0000` (the `.ips` times) in Unix seconds.
#[cfg_attr(not(any(target_os = "macos", test)), expect(dead_code, reason = "only macOS reads its reports"))]
fn parse_ips_time(text: &str) -> Option<u64> {
    let mut parts = text.split(' ');
    let (date, time, zone) = (parts.next()?, parts.next()?, parts.next().unwrap_or("+0000"));
    let time = time.split('.').next()?;
    let local = parse_utc(&format!("{date}T{time}Z"))? as i64;
    let sign = if zone.starts_with('-') { -1 } else { 1 };
    let digits = zone.trim_start_matches(['+', '-']);
    let (hours, minutes) = (digits.get(0..2)?.parse::<i64>().ok()?, digits.get(2..4)?.parse::<i64>().ok()?);
    u64::try_from(local - sign * (hours * 3600 + minutes * 60)).ok()
}

/// Windows' Application event 1000 ("Application Error"), as `EvtRender` gives it in XML. Its
/// data, by name or (older records) in order: AppName, AppVersion, AppTimeStamp, ModuleName,
/// ModuleVersion, ModuleTimeStamp, ExceptionCode, FaultingOffset, ProcessId, ProcessCreationTime,
/// AppPath, ModulePath, IntegratorReportId, …
#[cfg_attr(not(any(target_os = "windows", test)), expect(dead_code, reason = "only Windows reads its reports"))]
pub fn parse_event_1000(xml: &str) -> Option<OsReport> {
    const ORDER: [&str; 11] = ["AppName", "AppVersion", "AppTimeStamp", "ModuleName", "ModuleVersion", "ModuleTimeStamp", "ExceptionCode", "FaultingOffset", "ProcessId", "ProcessCreationTime", "AppPath"];
    if xml_text(xml, "EventID").is_some_and(|id| id.trim() != "1000") {
        return None;
    }
    let mut named: Vec<(String, String)> = Vec::new();
    let mut rest = xml;
    let mut index = 0;
    while let Some(start) = rest.find("<Data") {
        rest = &rest[start + 5..];
        let Some(close) = rest.find('>') else { break };
        let attributes = &rest[..close];
        let self_closing = attributes.ends_with('/');
        let name = attributes.split_once("Name=").map(|(_, v)| v.trim_start_matches(['\'', '"']).split(['\'', '"']).next().unwrap_or("").to_owned());
        let value = if self_closing { String::new() } else { rest[close + 1..].split("</Data>").next().unwrap_or("").to_owned() };
        let name = name.unwrap_or_else(|| ORDER.get(index).copied().unwrap_or("").to_owned());
        named.push((name, xml_unescape(&value)));
        index += 1;
    }
    let get = |key: &str| named.iter().find(|(k, _)| k == key).map(|(_, v)| v.trim().to_owned()).unwrap_or_default();
    let app = get("AppName");
    if app.is_empty() {
        return None;
    }
    let pid = get("ProcessId");
    let pid = pid.strip_prefix("0x").or_else(|| pid.strip_prefix("0X")).map_or_else(|| pid.parse().ok(), |hex| u32::from_str_radix(hex, 16).ok());
    let time = xml.split_once("SystemTime=").and_then(|(_, v)| v.trim_start_matches(['\'', '"']).get(0..19)).and_then(|t| parse_utc(&format!("{t}Z")));
    let mut details = Vec::new();
    let offset = get("FaultingOffset");
    if !offset.is_empty() {
        details.push(("Offset", offset));
    }
    let version = get("AppVersion");
    if !version.is_empty() {
        details.push(("App version", version));
    }
    Some(OsReport { pid, time, app, exception: exception_name(&get("ExceptionCode")), module: get("ModuleName"), details, frames: Vec::new() })
}

#[cfg_attr(not(any(target_os = "windows", test)), expect(dead_code, reason = "only Windows reads its reports"))]
fn xml_text<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = xml.find(&format!("<{tag}"))?;
    let after = &xml[open..];
    let start = after.find('>')? + 1;
    let end = after.find(&format!("</{tag}>"))?;
    after.get(start..end)
}

#[cfg_attr(not(any(target_os = "windows", test)), expect(dead_code, reason = "only Windows reads its reports"))]
fn xml_unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// A `Report.wer` (UTF-16 `key=value` lines, already decoded): the app, the time (`EventTime`, a
/// FILETIME), and its signature (`Sig[n].Name` / `Sig[n].Value`). Undocumented: best effort.
#[cfg_attr(not(any(target_os = "windows", test)), expect(dead_code, reason = "only Windows reads its reports"))]
pub fn parse_report_wer(text: &str) -> Option<OsReport> {
    let pairs: Vec<(&str, &str)> = text.lines().filter_map(|l| l.trim_start_matches('\u{feff}').split_once('=')).collect();
    let get = |key: &str| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| v.trim().to_owned());
    let signature = |name: &str| {
        pairs.iter().find(|(k, v)| k.starts_with("Sig[") && k.ends_with("].Name") && v.trim() == name).and_then(|(k, _)| get(&k.replace("].Name", "].Value")))
    };
    let app = get("AppName").or_else(|| get("NsAppName")).or_else(|| signature("Application Name"))?;
    let time = get("EventTime").and_then(|t| t.parse::<u64>().ok()).and_then(|filetime| (filetime / 10_000_000).checked_sub(11_644_473_600));
    let mut details = Vec::new();
    if let Some(offset) = signature("Exception Offset") {
        details.push(("Offset", offset));
    }
    if let Some(data) = signature("Exception Data") {
        details.push(("Exception data", data));
    }
    Some(OsReport {
        pid: None,
        time,
        app,
        exception: exception_name(&signature("Exception Code").unwrap_or_default()),
        module: signature("Fault Module Name").unwrap_or_default(),
        details,
        frames: Vec::new(),
    })
}

/// `c0000005` → `0xC0000005 (access violation)`, for the few codes worth naming.
#[cfg_attr(not(any(target_os = "windows", test)), expect(dead_code, reason = "only Windows reads its reports"))]
fn exception_name(code: &str) -> String {
    let hex = code.trim().trim_start_matches("0x").trim_start_matches("0X").to_ascii_uppercase();
    if hex.is_empty() {
        return String::new();
    }
    let name = match hex.as_str() {
        "C0000005" => Some("access violation"),
        "C0000409" => Some("fast fail"),
        "C00000FD" => Some("stack overflow"),
        "C000001D" => Some("illegal instruction"),
        "C0000374" => Some("heap corruption"),
        "80000003" => Some("breakpoint"),
        _ => None,
    };
    match name {
        Some(name) => format!("0x{hex} ({name})"),
        None => format!("0x{hex}"),
    }
}

// ---------------------------------------------------------------------------------------------
// The next launch.

/// What the next launch found about a run that didn't end cleanly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Evidence {
    pub panic: Option<PanicReport>,
    pub os: Option<OsReport>,
}

impl Evidence {
    pub fn crashed(&self) -> bool {
        self.panic.is_some() || self.os.is_some()
    }
}

/// Looks for what the run in `marker` left behind: a `last-panic.txt` from after it started, and
/// the OS's report for its process. Blocking (it reads folders and, on Windows, the event log):
/// call it off the UI thread.
pub fn look_for_evidence(folder: &Path, marker: Option<&RunMarker>) -> Evidence {
    let Some((pid, started)) = marker.and_then(|m| Some((m.pid, m.started_at()?))) else { return Evidence::default() };
    let panic = std::fs::read_to_string(folder.join(PANIC_FILE)).ok().and_then(|t| parse_panic(&t)).filter(|p| p.time.is_some_and(|t| t >= started));
    Evidence { panic, os: crate::platform::find_crash_report(pid, started) }
}

/// The OS writes its report a while after the crash (macOS's ReportCrash took about 30 s on the
/// M3), so after a quick relaunch it may not be there yet: looks again every few seconds, for up
/// to [`OS_REPORT_WAIT`]. Blocking: call it off the UI thread.
pub fn wait_for_os_report(marker: &RunMarker) -> Option<OsReport> {
    let started = marker.started_at()?;
    let deadline = std::time::Instant::now() + OS_REPORT_WAIT;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_secs(5));
        if let Some(report) = crate::platform::find_crash_report(marker.pid, started) {
            return Some(report);
        }
    }
    None
}

/// How long the next launch keeps looking for the OS's report.
pub const OS_REPORT_WAIT: Duration = Duration::from_secs(90);

/// The note shown after a run that didn't end cleanly.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    /// The pre-filled issue's URL; `None` for "didn't close normally".
    pub issue: Option<String>,
    /// What **Show the files** reveals: the panic file, else the log, else the folder.
    pub reveal: PathBuf,
}

/// The note for what was found, with the issue built from it, the setup and the log.
pub fn note(folder: &Path, evidence: &Evidence, setup: &Setup) -> Note {
    let panic_file = folder.join(PANIC_FILE);
    let log_file = folder.join(crate::log::FILE);
    let reveal = if evidence.panic.is_some() && panic_file.exists() {
        panic_file
    } else if log_file.exists() {
        log_file
    } else {
        folder.to_owned()
    };
    let issue = evidence.crashed().then(|| {
        let mut setup_lines = vec![("xReveal", crate::whats_new::VERSION.to_owned()), ("OS", os().to_owned())];
        setup_lines.extend(setup_summary(setup));
        let log = previous_run_log(crate::log::tail(folder, LOG_LINES + 10));
        let title = issue_title(evidence);
        let body = issue_body(&setup_lines, evidence, &log, BODY_CAP);
        issue_url(&title, &body)
    });
    Note { issue, reveal }
}

/// The log's lines up to this run's own launch line, so the issue shows the run that crashed.
fn previous_run_log(mut lines: Vec<String>) -> Vec<String> {
    let this_run = format!("(pid {})", std::process::id());
    if let Some(at) = lines.iter().rposition(|l| l.contains(" app: started ") && l.contains(&this_run)) {
        lines.truncate(at);
    }
    let skip = lines.len().saturating_sub(LOG_LINES);
    lines.split_off(skip)
}

/// The setup lines the panic file and the issue carry.
pub fn setup_summary(setup: &Setup) -> Vec<(&'static str, String)> {
    let source = setup.source.as_ref().map_or_else(|| "none".to_owned(), source_model);
    let format = setup.video_format.map_or_else(|| "none".to_owned(), |f| format!("{}×{}@{}", f.width, f.height, f.fps.round()));
    let buffering = match setup.delay.buffering_frames {
        0 => "Off".to_owned(),
        1 => "1 frame".to_owned(),
        n => format!("{n} frames"),
    };
    vec![("Source", source), ("Video format", format), ("Window mode", setup.window_mode.label().to_owned()), ("Buffering", buffering)]
}

/// A Source by its model: the name the device gives itself and its USB IDs, never its serial.
pub fn source_model(identity: &crate::source::SourceIdentity) -> String {
    match identity.model {
        Some((vendor, product)) => format!("{} ({vendor:04x}:{product:04x})", identity.name),
        None => identity.name.clone(),
    }
}

// ---------------------------------------------------------------------------------------------
// The pre-filled issue.

/// `Crash: <the panic message's first line> (<file:line>)`, or `Crash: <code or signal> in <module>`.
pub fn issue_title(evidence: &Evidence) -> String {
    let title = if let Some(panic) = &evidence.panic {
        let first = panic.message.lines().next().unwrap_or("").trim();
        let first: String = if first.chars().count() > 120 { first.chars().take(119).chain(['…']).collect() } else { first.to_owned() };
        let file_line = panic.location.rsplit_once(':').map_or(panic.location.as_str(), |(file_line, _)| file_line);
        format!("Crash: {first} ({file_line})")
    } else if let Some(os) = &evidence.os {
        let what = if os.exception.is_empty() { "unknown exception" } else { &os.exception };
        if os.module.is_empty() { format!("Crash: {what}") } else { format!("Crash: {what} in {}", os.module) }
    } else {
        "Crash".to_owned()
    };
    scrub(&title)
}

/// The issue's body (docs/spec/v1.md §2.11), kept under `cap` bytes once URL-encoded: log lines
/// go first, then frames, then the message is cut short.
pub fn issue_body(setup: &[(&str, String)], evidence: &Evidence, log: &[String], cap: usize) -> String {
    let frames = evidence.panic.as_ref().map(|p| p.frames.len()).unwrap_or(0).max(evidence.os.as_ref().map_or(0, |o| o.frames.len())).min(TOP_FRAMES);
    let fits = |body: &String| encode(body).len() <= cap;
    let mut log_lines = log.len();
    let mut frame_count = frames;
    loop {
        let body = scrub(&body_text(setup, evidence, &log[log.len() - log_lines..], frame_count, None));
        if fits(&body) {
            return body;
        }
        if log_lines > 0 {
            log_lines -= 1;
        } else if frame_count > 0 {
            frame_count -= 1;
        } else {
            break;
        }
    }
    // Still too long (a very long message): cut the message until it fits.
    let mut limit = 2000;
    loop {
        let body = scrub(&body_text(setup, evidence, &[], 0, Some(limit)));
        if fits(&body) || limit == 0 {
            return body;
        }
        limit /= 2;
    }
}

fn body_text(setup: &[(&str, String)], evidence: &Evidence, log: &[String], frames: usize, message_limit: Option<usize>) -> String {
    let cut = |text: &str| match message_limit {
        Some(limit) if text.chars().count() > limit => text.chars().take(limit).chain(['…']).collect(),
        _ => text.to_owned(),
    };
    let mut out = String::new();
    let _ = writeln!(out, "### What were you doing?\n\n\n");
    let _ = writeln!(out, "### Setup\n");
    for (label, value) in setup {
        let _ = writeln!(out, "- {label}: {value}");
    }
    let _ = writeln!(out, "\n### Crash\n\n```text");
    if let Some(panic) = &evidence.panic {
        let _ = writeln!(out, "Panic: {}", cut(&panic.message));
        let _ = writeln!(out, "At: {}", panic.location);
        let _ = writeln!(out, "Thread: {}", panic.thread);
    }
    if let Some(os) = &evidence.os {
        if !os.exception.is_empty() {
            let _ = writeln!(out, "Signal or exception: {}", os.exception);
        }
        if !os.module.is_empty() {
            let _ = writeln!(out, "Faulting module: {}", os.module);
        }
        for (label, value) in &os.details {
            let _ = writeln!(out, "{label}: {value}");
        }
    }
    // The panic's own frames say most; else the OS report's faulting thread.
    let list = evidence.panic.as_ref().map(|p| &p.frames).filter(|f| !f.is_empty()).or(evidence.os.as_ref().map(|o| &o.frames));
    if let Some(list) = list.filter(|_| frames > 0) {
        let _ = writeln!(out, "\nTop frames:");
        for (i, frame) in list.iter().take(frames).enumerate() {
            let _ = writeln!(out, "{i:>2}: {frame}");
        }
    }
    let _ = writeln!(out, "```\n\n### Log\n");
    if log.is_empty() {
        let _ = writeln!(out, "(left out)");
    } else {
        let _ = writeln!(out, "```text");
        for line in log {
            let _ = writeln!(out, "{line}");
        }
        let _ = writeln!(out, "```");
    }
    let _ = write!(out, "\n{FOOTER}\n");
    out
}

/// `github.com/pwn233/xReveal/issues/new?title=…&body=…`.
pub fn issue_url(title: &str, body: &str) -> String {
    format!("{}/issues/new?title={}&body={}", crate::whats_new::REPOSITORY, encode(title), encode(body))
}

/// Percent-encodes everything but the unreserved characters (RFC 3986).
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(byte as char),
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Nothing personal.

/// The home folder and the user name, as they'd appear in paths and messages.
static PRIVATE: OnceLock<(Option<String>, Option<String>)> = OnceLock::new();

/// Takes out what's personal: the home folder becomes `~`, and the user name `<user>`.
pub fn scrub(text: &str) -> String {
    let (home, user) = PRIVATE.get_or_init(|| {
        let home = std::env::var("HOME").ok().or_else(|| std::env::var("USERPROFILE").ok()).filter(|h| h.len() > 1);
        let user = std::env::var("USER").ok().or_else(|| std::env::var("USERNAME").ok());
        (home, user)
    });
    scrub_with(text, home.as_deref(), user.as_deref())
}

fn scrub_with(text: &str, home: Option<&str>, user: Option<&str>) -> String {
    let mut out = text.to_owned();
    if let Some(home) = home.map(|h| h.trim_end_matches(['/', '\\'])).filter(|h| !h.is_empty()) {
        out = out.replace(home, "~");
        // The same folder written with the other slash (Windows paths in Rust messages).
        let flipped = if home.contains('\\') { home.replace('\\', "/") } else { home.replace('/', "\\") };
        out = out.replace(&flipped, "~");
    }
    // The user name on its own (not inside a longer word); too short a name would hit everything.
    if let Some(user) = user.filter(|u| u.chars().count() >= 3) {
        let mut scrubbed = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(at) = rest.find(user) {
            let before = rest[..at].chars().next_back();
            let after = rest[at + user.len()..].chars().next();
            let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            scrubbed.push_str(&rest[..at]);
            scrubbed.push_str(if word(before) || word(after) { user } else { "<user>" });
            rest = &rest[at + user.len()..];
        }
        scrubbed.push_str(rest);
        out = scrubbed;
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Time.

/// Unix seconds as `2026-11-28T12:01:29Z`.
pub fn utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest / 60 % 60, rest % 60)
}

/// `2026-11-28T12:01:29Z` in Unix seconds.
pub fn parse_utc(text: &str) -> Option<u64> {
    let text = text.trim().strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (year, month, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|p| p.parse::<i64>().ok());
    let (hour, minute, second) = (t.next()??, t.next()??, t.next()??);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    u64::try_from(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second).ok()
}

/// Howard Hinnant's `days_from_civil`: days since 1970-01-01.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

// ---------------------------------------------------------------------------------------------
// The verification cards' crash.

/// `XREVEAL_CRASH_TEST=panic | abort | native`: how to crash, if asked to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashTest {
    Panic,
    Abort,
    /// A write to an address that's never mapped: SIGSEGV, or an access violation.
    Native,
}

pub fn crash_test() -> Option<CrashTest> {
    match std::env::var("XREVEAL_CRASH_TEST").ok()?.trim() {
        "panic" => Some(CrashTest::Panic),
        "abort" => Some(CrashTest::Abort),
        "native" => Some(CrashTest::Native),
        _ => None,
    }
}

/// Crashes the way `test` says. On the UI thread, so a panic takes xReveal down.
pub fn crash_now(test: CrashTest) -> ! {
    crate::log::warn("app", format!("XREVEAL_CRASH_TEST={test:?}: crashing on purpose"));
    match test {
        CrashTest::Panic => panic!("XREVEAL_CRASH_TEST: a deliberate panic"),
        CrashTest::Abort => std::process::abort(),
        CrashTest::Native => {
            // Address 8 sits in the never-mapped first page. (Not 0 itself: std checks for null.)
            let address = std::hint::black_box(8usize) as *mut u8;
            // SAFETY: it isn't. This is the deliberate native crash the verification cards ask for.
            unsafe { std::ptr::write_volatile(address, 1) };
            std::process::abort()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_round_trip_through_utc_text() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(parse_utc("2026-11-28T12:01:29Z"), Some(1_795_867_289));
        assert_eq!(utc(1_795_867_289), "2026-11-28T12:01:29Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(parse_utc("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_utc("yesterday"), None);
        assert_eq!(parse_ips_time("2026-11-28 13:01:29.8123 +0100"), Some(1_795_867_289));
    }

    #[test]
    fn scrubbing_takes_out_the_home_folder_and_the_user_name() {
        let home = Some("/Users/jordan");
        let user = Some("jordan");
        assert_eq!(scrub_with("at /Users/jordan/xReveal/src/app.rs:12", home, user), "at ~/xReveal/src/app.rs:12");
        assert_eq!(scrub_with("hello jordan, jordanian", home, user), "hello <user>, jordanian");
        let home = Some(r"C:\Users\Jordan");
        let user = Some("Jordan");
        assert_eq!(scrub_with(r"C:\Users\Jordan\AppData\Local\xReveal", home, user), r"~\AppData\Local\xReveal");
        assert_eq!(scrub_with("C:/Users/Jordan/x", home, user), "~/x");
        // A very short name would match everything: left alone.
        assert_eq!(scrub_with("a b", None, Some("a")), "a b");
    }

    fn panic_fixture() -> String {
        panic_text(&PanicText {
            time: "2026-11-28T12:01:29Z".into(),
            message: "index out of bounds: the len is 3 but the index is 7\nsecond line",
            location: "src/video/slot.rs:88:17",
            thread: "main",
            os: "macOS 26.0 (25A354)",
            setup: "Source USB2 Video (345f:2131) · Video format 1920×1080@50",
            backtrace: "   0: std::backtrace::Backtrace::force_capture\n             at /rustc/abc/library/std/src/backtrace.rs:312:9\n   1: <std::backtrace::Backtrace>::create\n   2: xreveal::crash::record_panic\n   2: <alloc::boxed::Box<dyn for<'a> core::ops::function::Fn<(&'a std::panic::PanicHookInfo,)>> as core::ops::function::Fn<(&std::panic::PanicHookInfo,)>>::call\n   2: __rustc::rust_begin_unwind\n   2: std::panicking::panic_with_hook\n   3: core::panicking::panic_bounds_check\n   4: xreveal::video::slot::Slot::take\n             at /Users/jordan/xReveal/src/video/slot.rs:88:17\n   5: xreveal::app::XReveal::watch_signal\n",
        })
    }

    #[test]
    fn the_panic_file_reads_back_without_the_panic_machinery() {
        let report = parse_panic(&panic_fixture()).unwrap();
        assert_eq!(report.time, Some(1_795_867_289));
        assert_eq!(report.message, "index out of bounds: the len is 3 but the index is 7\nsecond line");
        assert_eq!(report.location, "src/video/slot.rs:88:17");
        assert_eq!(report.thread, "main");
        assert_eq!(report.frames, vec!["xreveal::video::slot::Slot::take (video/slot.rs:88:17)".to_owned(), "xreveal::app::XReveal::watch_signal".to_owned()]);
        assert_eq!(parse_panic("not a panic file"), None);
    }

    const IPS: &str = r#"{"app_name":"xreveal","timestamp":"2026-11-28 12:01:40.00 +0000","app_version":"0.1.0","bug_type":"309","name":"xreveal"}
{
  "pid" : 4321,
  "procName" : "xreveal",
  "procPath" : "/Users/jordan/xReveal.app/Contents/MacOS/xreveal",
  "captureTime" : "2026-11-28 12:01:39.8123 +0000",
  "exception" : {"codes":"0x0000000000000001, 0x0000000000000008","type":"EXC_BAD_ACCESS","signal":"SIGSEGV","subtype":"KERN_INVALID_ADDRESS at 0x0000000000000008"},
  "termination" : {"indicator":"Segmentation fault: 11"},
  "faultingThread" : 1,
  "threads" : [
    {"frames":[{"imageOffset":100,"symbol":"mach_msg2_trap","symbolLocation":8,"imageIndex":0}]},
    {"triggered":true,"frames":[{"imageOffset":4096,"symbol":"xreveal::crash::crash_now","symbolLocation":36,"imageIndex":1},{"imageOffset":8192,"imageIndex":1}]}
  ],
  "usedImages" : [{"name":"libsystem_kernel.dylib","path":"/usr/lib/system/libsystem_kernel.dylib"},{"name":"xreveal","path":"/Users/jordan/xReveal.app/Contents/MacOS/xreveal"}]
}"#;

    #[test]
    fn an_ips_report_gives_the_process_signal_module_and_frames() {
        let report = parse_ips(IPS).unwrap();
        assert_eq!(report.pid, Some(4321));
        assert_eq!(report.time, Some(1_795_867_299));
        assert!(report.is_xreveal());
        assert_eq!(report.exception, "SIGSEGV");
        assert_eq!(report.module, "xreveal");
        assert_eq!(report.frames, vec!["xreveal  xreveal::crash::crash_now + 36".to_owned(), "xreveal  0x2000".to_owned()]);
        assert_eq!(report.details[0], ("Exception", "EXC_BAD_ACCESS (KERN_INVALID_ADDRESS at 0x0000000000000008)".to_owned()));
        // Anything but a crash report (bug_type 309) is passed over.
        assert_eq!(parse_ips(&IPS.replacen("\"309\"", "\"288\"", 1)), None);
        assert_eq!(parse_ips("{}"), None);
    }

    #[test]
    fn a_report_wer_gives_the_app_time_code_and_module() {
        let text = "\u{feff}Version=1\r\nEventType=APPCRASH\r\nEventTime=134089000000000000\r\nSig[0].Name=Application Name\r\nSig[0].Value=xreveal.exe\r\nSig[3].Name=Fault Module Name\r\nSig[3].Value=mfplat.dll\r\nSig[6].Name=Exception Code\r\nSig[6].Value=c0000005\r\nSig[7].Name=Exception Offset\r\nSig[7].Value=000000000001a2b3\r\nAppName=xreveal.exe\r\nAppPath=C:\\Users\\Jordan\\AppData\\Local\\Programs\\xReveal\\xreveal.exe\r\n";
        let report = parse_report_wer(text).unwrap();
        assert!(report.is_xreveal());
        assert_eq!(report.time, Some(134_089_000_000_000_000 / 10_000_000 - 11_644_473_600));
        assert_eq!(report.exception, "0xC0000005 (access violation)");
        assert_eq!(report.module, "mfplat.dll");
        assert_eq!(report.details, vec![("Offset", "000000000001a2b3".to_owned())]);
    }

    #[test]
    fn event_1000_gives_the_pid_by_name_or_by_position() {
        let named = "<Event><System><EventID Qualifiers='0'>1000</EventID><TimeCreated SystemTime='2026-11-28T12:01:40.1234567Z'/></System><EventData><Data Name='AppName'>xreveal.exe</Data><Data Name='AppVersion'>0.1.0.0</Data><Data Name='AppTimeStamp'>0</Data><Data Name='ModuleName'>xreveal.exe</Data><Data Name='ModuleVersion'>0.1.0.0</Data><Data Name='ModuleTimeStamp'>0</Data><Data Name='ExceptionCode'>c0000409</Data><Data Name='FaultingOffset'>0x1234</Data><Data Name='ProcessId'>0x12a0</Data></EventData></Event>";
        let report = parse_event_1000(named).unwrap();
        assert_eq!(report.pid, Some(4768));
        assert_eq!(report.time, Some(1_795_867_300));
        assert_eq!(report.exception, "0xC0000409 (fast fail)");
        assert_eq!(report.module, "xreveal.exe");
        let positional = "<Event><System><EventID>1000</EventID></System><EventData><Data>xreveal.exe</Data><Data>0.0.0.0</Data><Data>0</Data><Data>mfplat.dll</Data><Data>10.0</Data><Data>0</Data><Data>c0000005</Data><Data>0x99</Data><Data>0x10</Data></EventData></Event>";
        let report = parse_event_1000(positional).unwrap();
        assert_eq!((report.pid, report.module.as_str(), report.app.as_str()), (Some(16), "mfplat.dll", "xreveal.exe"));
        assert_eq!(parse_event_1000("<Event><System><EventID>1001</EventID></System></Event>"), None);
    }

    fn evidence() -> Evidence {
        Evidence { panic: parse_panic(&panic_fixture()), os: parse_ips(IPS) }
    }

    #[test]
    fn the_title_names_the_panic_or_else_the_signal_and_module() {
        assert_eq!(issue_title(&evidence()), "Crash: index out of bounds: the len is 3 but the index is 7 (src/video/slot.rs:88)");
        let native = Evidence { panic: None, os: parse_ips(IPS) };
        assert_eq!(issue_title(&native), "Crash: SIGSEGV in xreveal");
    }

    #[test]
    fn the_body_has_its_sections_and_stays_under_the_cap() {
        let setup = vec![("xReveal", "0.1.0".to_owned()), ("Source", "USB2 Video (345f:2131)".to_owned())];
        let log: Vec<String> = (0..30).map(|i| format!("2026-11-28T12:00:{i:02}Z INFO signal: live {}", "x".repeat(60))).collect();
        let body = issue_body(&setup, &evidence(), &log, BODY_CAP);
        for part in ["### What were you doing?", "### Setup", "- Source: USB2 Video (345f:2131)", "### Crash", "Panic: index out of bounds", "At: src/video/slot.rs:88:17", "Thread: main", "Signal or exception: SIGSEGV", "Top frames:", "### Log", FOOTER] {
            assert!(body.contains(part), "missing {part:?}");
        }
        assert!(encode(&body).len() <= BODY_CAP);
        // A tight cap cuts the log first (oldest lines first), then the frames.
        let tight = issue_body(&setup, &evidence(), &log, 2000);
        assert!(encode(&tight).len() <= 2000);
        assert!(tight.contains("Top frames:"), "frames go only after the log");
        assert!(!tight.contains("12:00:00Z"), "the oldest log lines go first");
        // Just too small for every frame with no log at all: the frames go too.
        let no_log = encode(&issue_body(&setup, &evidence(), &[], BODY_CAP)).len();
        let tighter = issue_body(&setup, &evidence(), &log, no_log - 1);
        assert!(encode(&tighter).len() < no_log);
        assert!(tighter.contains("(left out)"));
        assert!(tighter.contains("Slot::take") && !tighter.contains("watch_signal"), "the last frame goes first");
        assert!(tighter.contains(FOOTER));
    }

    #[test]
    fn the_issue_url_is_encoded() {
        let url = issue_url("Crash: a & b", "line 1\nline 2 ✓");
        assert_eq!(url, "https://github.com/pwn233/xReveal/issues/new?title=Crash%3A%20a%20%26%20b&body=line%201%0Aline%202%20%E2%9C%93");
    }

    #[test]
    fn the_log_for_the_issue_stops_at_this_runs_launch() {
        let me = std::process::id();
        let lines = vec!["a".to_owned(), "b".to_owned(), format!("t INFO app: started xReveal 0.1.0 (pid {me})"), "c".to_owned()];
        assert_eq!(previous_run_log(lines), vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn the_marker_round_trips() {
        let marker = RunMarker { version: "0.1.0".into(), pid: 42, started: "2026-11-28T12:01:29Z".into() };
        let text = toml::to_string(&marker).unwrap();
        assert_eq!(text, "version = \"0.1.0\"\npid = 42\nstarted = \"2026-11-28T12:01:29Z\"\n");
        assert_eq!(toml::from_str::<RunMarker>(&text).unwrap().started_at(), Some(1_795_867_289));
    }
}
