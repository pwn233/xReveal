//! After a crash on macOS (docs/spec/v1.md §2.11): whether a process is a running xReveal, the OS
//! version, and ReportCrash's `.ips` report for a crashed run.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::PathBuf;

use crate::crash::OsReport;

unsafe extern "C" {
    // libproc (libSystem): the executable's path of a running process.
    fn proc_pidpath(pid: c_int, buffer: *mut c_void, size: u32) -> c_int;
    fn sysctlbyname(name: *const c_char, old: *mut c_void, old_len: *mut usize, new: *mut c_void, new_len: usize) -> c_int;
}

/// Whether process `pid` is running and is xReveal (its executable is `xreveal`).
pub fn process_is_xreveal(pid: u32) -> bool {
    let Ok(pid) = c_int::try_from(pid) else { return false };
    let mut buffer = [0u8; 4096];
    // SAFETY: the buffer is writable for its whole length, which is what's passed.
    let length = unsafe { proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    let Some(path) = usize::try_from(length).ok().filter(|l| *l > 0).and_then(|l| buffer.get(..l)) else { return false };
    std::str::from_utf8(path).is_ok_and(|p| p.rsplit('/').next().is_some_and(|name| name.eq_ignore_ascii_case("xreveal")))
}

fn sysctl_text(name: &CStr) -> Option<String> {
    let mut buffer = [0u8; 256];
    let mut length = buffer.len();
    // SAFETY: `name` is NUL-terminated, and `length` is the buffer's writable size.
    let result = unsafe { sysctlbyname(name.as_ptr(), buffer.as_mut_ptr().cast(), &mut length, std::ptr::null_mut(), 0) };
    (result == 0).then(|| CStr::from_bytes_until_nul(&buffer).ok().map(|s| s.to_string_lossy().into_owned())).flatten()
}

/// `macOS 26.0 (25A354)`.
pub fn os_version() -> String {
    let version = sysctl_text(c"kern.osproductversion").unwrap_or_else(|| "?".into());
    match sysctl_text(c"kern.osversion") {
        Some(build) => format!("macOS {version} ({build})"),
        None => format!("macOS {version}"),
    }
}

/// ReportCrash's report of process `pid`, if it crashed after `started` (Unix seconds):
/// `~/Library/Logs/DiagnosticReports/xreveal*.ips`, matched by its process ID and time.
pub fn find_report(pid: u32, started: u64) -> Option<OsReport> {
    let folder = PathBuf::from(std::env::var_os("HOME")?).join("Library/Logs/DiagnosticReports");
    let since = std::time::UNIX_EPOCH + std::time::Duration::from_secs(started.saturating_sub(5));
    std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with("xreveal") && name.ends_with(".ips")
        })
        .filter(|entry| entry.metadata().and_then(|m| m.modified()).is_ok_and(|modified| modified >= since))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| crate::crash::parse_ips(&text))
        .find(|report| report.pid == Some(pid) && report.is_xreveal())
}
