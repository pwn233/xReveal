//! After a crash on Windows (docs/spec/v1.md §2.11, research #193): whether a process is a
//! running xReveal, the Windows version, and Windows Error Reporting's record of a crashed run:
//! Application event 1000 ("Application Error"), else a `Report.wer` in either WER folder.

use std::path::PathBuf;

use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, STILL_ACTIVE};
use windows::Win32::System::EventLog::{EVT_HANDLE, EvtClose, EvtNext, EvtQuery, EvtQueryChannelPath, EvtQueryReverseDirection, EvtRender, EvtRenderEventXml};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::core::{HSTRING, PWSTR};

use crate::crash::OsReport;

/// Whether process `pid` is running and is xReveal (its executable is `xreveal.exe`).
pub fn process_is_xreveal(pid: u32) -> bool {
    // SAFETY: plain Win32 calls on a handle opened here and closed before returning.
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut code = 0u32;
        let running = GetExitCodeProcess(process, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let mut buffer = [0u16; 1024];
        let mut length = buffer.len() as u32;
        let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut length).is_ok();
        let _ = CloseHandle(process);
        let path = String::from_utf16_lossy(buffer.get(..length as usize).unwrap_or_default());
        running && named && path.rsplit('\\').next().is_some_and(|name| name.eq_ignore_ascii_case("xreveal.exe"))
    }
}

fn registry_text(value: &str) -> Option<String> {
    let mut buffer = [0u16; 128];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: the buffer is writable for `size` bytes.
    let result = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, &HSTRING::from(CURRENT_VERSION), &HSTRING::from(value), RRF_RT_REG_SZ, None, Some(buffer.as_mut_ptr().cast()), Some(&mut size)) };
    (result == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buffer[..(size as usize / 2).min(buffer.len())]).trim_end_matches('\0').to_owned())
}

fn registry_number(value: &str) -> Option<u32> {
    let mut number = 0u32;
    let mut size = 4u32;
    // SAFETY: `number` is writable for 4 bytes.
    let result = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, &HSTRING::from(CURRENT_VERSION), &HSTRING::from(value), RRF_RT_REG_DWORD, None, Some((&mut number as *mut u32).cast()), Some(&mut size)) };
    (result == ERROR_SUCCESS).then_some(number)
}

const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

/// `Windows 11 24H2 (build 26100.2605)`.
pub fn os_version() -> String {
    let build = registry_text("CurrentBuildNumber").unwrap_or_default();
    // ProductName still says "Windows 10" on 11; the build number tells them apart.
    let name = if build.parse::<u32>().is_ok_and(|b| b >= 22000) { "Windows 11" } else { "Windows 10" };
    let release = registry_text("DisplayVersion").map(|v| format!(" {v}")).unwrap_or_default();
    let revision = registry_number("UBR").map(|r| format!(".{r}")).unwrap_or_default();
    format!("{name}{release} (build {build}{revision})")
}

/// WER's record of process `pid`, if it crashed after `started` (Unix seconds).
pub fn find_report(pid: u32, started: u64) -> Option<OsReport> {
    event_1000(pid, started).or_else(|| report_wer(started))
}

/// Closes an event log handle when dropped.
struct Evt(EVT_HANDLE);

impl Drop for Evt {
    fn drop(&mut self) {
        // SAFETY: the handle came from EvtQuery or EvtNext and is closed once.
        let _ = unsafe { EvtClose(self.0) };
    }
}

/// The newest Application event 1000 for xreveal.exe with process ID `pid`, from after `started`.
/// A plain user can read the Application log; no admin needed.
fn event_1000(pid: u32, started: u64) -> Option<OsReport> {
    // `timediff` is in ms from now; reach back to a little before the run started.
    let window_ms = crate::update::now().saturating_sub(started).saturating_add(60) * 1000;
    let query = format!("*[System[(EventID=1000) and TimeCreated[timediff(@SystemTime) <= {window_ms}]]]");
    // SAFETY: plain Win32 calls; every handle is closed by `Evt`, and the render buffer's size is
    // what's passed.
    unsafe {
        let results = Evt(EvtQuery(None, &HSTRING::from("Application"), &HSTRING::from(query), EvtQueryChannelPath.0 | EvtQueryReverseDirection.0).ok()?);
        loop {
            let mut events = [0isize; 16];
            let mut returned = 0u32;
            if EvtNext(results.0, &mut events, 1000, 0, &mut returned).is_err() || returned == 0 {
                return None;
            }
            let events: Vec<Evt> = events.iter().take(returned as usize).map(|e| Evt(EVT_HANDLE(*e))).collect();
            for event in &events {
                let mut buffer = vec![0u16; 8192];
                let mut used = 0u32;
                let mut properties = 0u32;
                if EvtRender(None, event.0, EvtRenderEventXml.0, (buffer.len() * 2) as u32, Some(buffer.as_mut_ptr().cast()), &mut used, &mut properties).is_err() {
                    let needed = (used as usize).div_ceil(2);
                    buffer = vec![0u16; needed];
                    if EvtRender(None, event.0, EvtRenderEventXml.0, (buffer.len() * 2) as u32, Some(buffer.as_mut_ptr().cast()), &mut used, &mut properties).is_err() {
                        continue;
                    }
                }
                let xml = String::from_utf16_lossy(&buffer[..(used as usize / 2).min(buffer.len())]);
                if let Some(report) = crate::crash::parse_event_1000(xml.trim_end_matches('\0'))
                    && report.is_xreveal()
                    && report.pid == Some(pid)
                {
                    return Some(report);
                }
            }
        }
    }
}

/// The newest `Report.wer` for xreveal.exe from after `started`, in the per-user or the
/// machine-wide WER folders (the research found it in the machine-wide one).
fn report_wer(started: u64) -> Option<OsReport> {
    let roots = ["LOCALAPPDATA", "ProgramData"].into_iter().filter_map(std::env::var_os).map(PathBuf::from);
    roots
        .flat_map(|root| ["ReportArchive", "ReportQueue"].map(|kind| root.join(r"Microsoft\Windows\WER").join(kind)))
        .filter_map(|folder| std::fs::read_dir(folder).ok())
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase().starts_with("appcrash_xreveal.exe_"))
        .filter_map(|entry| std::fs::read(entry.path().join("Report.wer")).ok())
        .filter_map(|bytes| crate::crash::parse_report_wer(&utf16_text(&bytes)))
        .filter(|report| report.is_xreveal() && report.time.is_some_and(|t| t + 5 >= started))
        .max_by_key(|report| report.time)
}

/// `Report.wer` is UTF-16LE (with a byte-order mark).
fn utf16_text(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    String::from_utf16_lossy(&units)
}
