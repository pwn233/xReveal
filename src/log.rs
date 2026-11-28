//! `xreveal.log` (docs/spec/v1.md §2.11): what happened, one plain line per event, never per
//! frame. `2026-11-28T12:01:29Z INFO source: USB2 Video (345f:2131) 1920×1080@50`: UTC time,
//! level, area, message. Capped at 256 KB; the full file becomes `xreveal.old.log`, so one older
//! file is kept. Nothing personal goes in: every message passes through [`crate::crash::scrub`].
//!
//! A few lines of std rather than the `log`/`tracing` crates: xReveal writes a handful of events
//! per session to one file, and needs none of their filtering, formatting or subscribers.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FILE: &str = "xreveal.log";
/// The previous file, once the log reached its cap.
pub const OLDER: &str = "xreveal.old.log";
pub const CAP: u64 = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

/// The open log: where it is and how big it is now.
struct Log {
    path: PathBuf,
    size: u64,
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// Starts logging to `xreveal.log` in `folder` (the settings folder). Until then events go nowhere.
pub fn open(folder: &Path) {
    let _ = std::fs::create_dir_all(folder);
    let path = folder.join(FILE);
    let size = std::fs::metadata(&path).map_or(0, |m| m.len());
    *LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(Log { path, size });
}

pub fn info(area: &str, message: impl AsRef<str>) {
    write(Level::Info, area, message.as_ref(), false);
}

pub fn warn(area: &str, message: impl AsRef<str>) {
    write(Level::Warn, area, message.as_ref(), false);
}

pub fn error(area: &str, message: impl AsRef<str>) {
    write(Level::Error, area, message.as_ref(), false);
}

/// From the panic hook: never waits for the log (the panic may have happened while writing it).
pub fn from_panic_hook(message: &str) {
    write(Level::Error, "panic", message, true);
}

fn write(level: Level, area: &str, message: &str, never_wait: bool) {
    let mut guard = if never_wait {
        match LOG.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return,
        }
    } else {
        LOG.lock().unwrap_or_else(|e| e.into_inner())
    };
    let Some(log) = guard.as_mut() else { return };
    let text = line(crate::crash::utc(crate::update::now()), level, area, &crate::crash::scrub(message));
    append(log, &text, CAP);
}

/// One event's line: time, level, area, message, all on one line (line breaks become spaces).
pub fn line(time: String, level: Level, area: &str, message: &str) -> String {
    let message: String = message.split(['\r', '\n']).filter(|part| !part.trim().is_empty()).map(str::trim_end).collect::<Vec<_>>().join(" ");
    format!("{time} {} {area}: {message}\n", level.label())
}

/// Adds `text` to the log, first moving a log that would pass `cap` aside to [`OLDER`].
fn append(log: &mut Log, text: &str, cap: u64) {
    if log.size > 0 && log.size + text.len() as u64 > cap {
        let _ = std::fs::rename(&log.path, log.path.with_file_name(OLDER));
        log.size = 0;
    }
    let written = std::fs::OpenOptions::new().create(true).append(true).open(&log.path).and_then(|mut file| file.write_all(text.as_bytes()));
    if written.is_ok() {
        log.size += text.len() as u64;
    }
}

/// The last `count` lines of the log in `folder` (for the pre-filled issue), oldest first.
pub fn tail(folder: &Path, count: usize) -> Vec<String> {
    let current = std::fs::read_to_string(folder.join(FILE)).unwrap_or_default();
    let mut lines: Vec<String> = current.lines().map(str::to_owned).collect();
    if lines.len() < count {
        let older = std::fs::read_to_string(folder.join(OLDER)).unwrap_or_default();
        let mut before: Vec<String> = older.lines().map(str::to_owned).collect();
        before.append(&mut lines);
        lines = before;
    }
    let skip = lines.len().saturating_sub(count);
    lines.split_off(skip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_time_level_area_and_message() {
        let text = line("2026-11-28T12:01:29Z".into(), Level::Info, "source", "USB2 Video 1920×1080@50 NV12");
        assert_eq!(text, "2026-11-28T12:01:29Z INFO source: USB2 Video 1920×1080@50 NV12\n");
        // Always one line, whatever the message holds.
        let text = line("2026-11-28T12:01:29Z".into(), Level::Error, "panic", "first\nsecond\r\n\nthird");
        assert_eq!(text, "2026-11-28T12:01:29Z ERROR panic: first second third\n");
    }

    #[test]
    fn a_full_log_moves_aside_and_only_one_older_file_is_kept() {
        let dir = std::env::temp_dir().join(format!("xreveal-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = Log { path: dir.join(FILE), size: 0 };
        let event = "2026-11-28T12:01:29Z INFO app: 0123456789\n"; // 42 bytes
        for _ in 0..3 {
            append(&mut log, event, 100);
        }
        // The third would pass 100 bytes: the first two went to the older file.
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap(), event);
        assert_eq!(std::fs::read_to_string(dir.join(OLDER)).unwrap(), event.repeat(2));
        for _ in 0..3 {
            append(&mut log, &event.replace("0123", "abcd"), 100);
        }
        // The older file is replaced, never a third file.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        assert!(std::fs::read_to_string(dir.join(OLDER)).unwrap().contains("abcd"));
        assert_eq!(log.size, 2 * event.len() as u64);
        // The tail reaches into the older file when the current one is short.
        let tail = tail(&dir, 3);
        assert_eq!(tail.len(), 3);
        assert!(tail.iter().all(|l| l.contains("abcd")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
