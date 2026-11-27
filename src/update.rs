//! Update available (#145, docs/spec/v1.md §2.10): once a day, after the picture is up, xReveal
//! asks GitHub for its latest release. When that's newer than this version, the drawer's footer
//! offers it: **Update to x.y.z**, with its notes and a button to its download page. xReveal never
//! installs anything itself.
//!
//! The request (in `crate::platform`) goes through each OS's own HTTP stack, so it follows the
//! system proxy and trusted roots and adds no crates. It sends only `User-Agent: xReveal`,
//! `Accept-Language: en` and `Accept`; nothing about the person or the machine.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The latest release's address on GitHub's API.
pub const LATEST_RELEASE: &str = "https://api.github.com/repos/pwn233/xReveal/releases/latest";
/// How often to ask.
pub const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// The most of an answer xReveal reads (a real one is about 10 KB plus the notes).
pub const READ_LIMIT: usize = 256 * 1024;

/// What's remembered in `setup.toml` (`[updates]`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Updates {
    /// The **Check for updates** switch (Interface tab).
    pub check: bool,
    /// When GitHub last answered (Unix seconds): a 200 or a 404. Failures aren't recorded, so the
    /// next launch tries again.
    pub last_check: Option<u64>,
    /// The newest version GitHub reported, and its notes, so the footer can offer it at once on the
    /// next launch, offline too.
    pub latest: Option<String>,
    pub notes: Vec<String>,
}

impl Default for Updates {
    fn default() -> Self {
        Self { check: true, last_check: None, latest: None, notes: Vec::new() }
    }
}

impl Updates {
    /// Whether it's time to ask again at `now` (Unix seconds). A last check in the future means the
    /// clock went back: ask.
    pub fn due(&self, now: u64) -> bool {
        self.check && self.last_check.is_none_or(|last| last > now || now - last >= EVERY.as_secs())
    }

    /// The version to offer, if it's newer than `current`.
    pub fn offer(&self, current: &str) -> Option<&str> {
        let latest = self.latest.as_deref()?;
        is_newer(latest, current).then_some(latest)
    }

    /// The switch turned off: no more daily checks, and an update already found is forgotten.
    /// (The update button can still be pressed to check by hand.)
    pub fn turn_off(&mut self) {
        self.check = false;
        self.latest = None;
        self.notes.clear();
    }

    /// Takes GitHub's answer at `now`.
    pub fn answered(&mut self, now: u64, answer: Answer) {
        self.last_check = Some(now);
        if let Answer::Release { version, notes } = answer
            && version_triple(&version).is_some()
        {
            self.latest = Some(version);
            self.notes = notes;
        }
    }
}

/// How the last check this run went, for the update button's tip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Check {
    #[default]
    NotYet,
    Checking,
    /// GitHub answered.
    Answered,
    /// No usable answer (offline, a proxy, rate-limited).
    Failed,
}

/// Where an in-app update stands (#163).
#[derive(Debug, Default)]
pub enum Updater {
    #[default]
    Idle,
    Downloading(std::sync::Arc<Progress>),
    /// Downloaded, checked and unpacked: what `platform::install_update` takes.
    Ready(std::path::PathBuf),
    Failed(String),
}

/// The same, as the update button shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum UpdaterView {
    #[default]
    Idle,
    /// From 0 to 1, or `None` while the size isn't known yet.
    Downloading(Option<f32>),
    Ready,
    Failed(String),
}

impl Updater {
    pub fn view(&self) -> UpdaterView {
        match self {
            Updater::Idle => UpdaterView::Idle,
            Updater::Downloading(progress) => UpdaterView::Downloading(progress.fraction()),
            Updater::Ready(_) => UpdaterView::Ready,
            Updater::Failed(why) => UpdaterView::Failed(why.clone()),
        }
    }
}

/// Downloads `version`'s asset for this OS, checks it against the release's `SHA256SUMS`, and gets
/// it ready to install. Blocking: run on a background thread.
pub fn fetch_update(version: &str, progress: &Progress) -> Result<std::path::PathBuf, String> {
    let name = asset_name(version);
    let dir = std::env::temp_dir().join(format!("xreveal-update-{version}"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(&name);
    crate::platform::download(&asset_url(version, &name), &file, progress)?;
    let sums = match crate::platform::http_get(&asset_url(version, "SHA256SUMS")) {
        Some((200, body)) => String::from_utf8_lossy(&body).into_owned(),
        _ => return Err("couldn't get the release's checksums".into()),
    };
    let expected = checksum_for(&sums, &name).ok_or("the release lists no checksum for this download")?;
    if !file_matches(&file, expected) {
        let _ = std::fs::remove_file(&file);
        return Err("the download didn't match its checksum".into());
    }
    crate::platform::prepare_update(&file)
}

/// A download's progress, shared between the thread downloading and the button's ring.
#[derive(Debug, Default)]
pub struct Progress {
    received: std::sync::atomic::AtomicU64,
    total: std::sync::atomic::AtomicU64,
    cancelled: std::sync::atomic::AtomicBool,
}

impl Progress {
    pub fn set(&self, received: u64, total: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        self.received.store(received, Relaxed);
        self.total.store(total, Relaxed);
    }

    /// From 0 to 1, or `None` while the size isn't known yet.
    pub fn fraction(&self) -> Option<f32> {
        use std::sync::atomic::Ordering::Relaxed;
        let total = self.total.load(Relaxed);
        (total > 0).then(|| (self.received.load(Relaxed) as f64 / total as f64).min(1.) as f32)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// The release asset the in-app update installs on this OS (#163): what `scripts/package-*` make.
pub fn asset_name(version: &str) -> String {
    if cfg!(target_os = "windows") { format!("xReveal-{version}-windows-x64-setup.exe") } else { format!("xReveal-{version}-macos-arm64.zip") }
}

/// Where a release asset (or its `SHA256SUMS`) downloads from. `XREVEAL_UPDATE_FEED` points it at
/// another server (`<feed>/v<version>/<name>`), to try the in-app update before a release exists;
/// the checksum, and on macOS the signature, are still checked.
pub fn asset_url(version: &str, name: &str) -> String {
    let feed = std::env::var("XREVEAL_UPDATE_FEED").unwrap_or_else(|_| "https://github.com/pwn233/xReveal/releases/download".into());
    format!("{}/v{version}/{name}", feed.trim_end_matches('/'))
}

/// `name`'s checksum in a `SHA256SUMS` file (`<hex>  <name>` lines, as `sha256sum` writes them).
pub fn checksum_for<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let (hex, file) = line.split_once(char::is_whitespace)?;
        (file.trim_start().trim_start_matches('*') == name && hex.len() == 64).then_some(hex)
    })
}

/// Whether the file at `path` has the SHA-256 `expected` (hex).
pub fn file_matches(path: &std::path::Path, expected: &str) -> bool {
    use sha2::Digest;
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            Err(_) => return false,
        }
    }
    let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    actual.eq_ignore_ascii_case(expected)
}

/// What GitHub said.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    /// The latest release: its version (the tag without `v`) and its notes.
    Release { version: String, notes: Vec<String> },
    /// No release published yet (404), or an answer xReveal can't read: nothing newer.
    Nothing,
}

/// Reads a 200's body: `tag_name` and the release's notes (`body`, whose `- ` lines are the
/// version's CHANGELOG.md section).
pub fn parse(json: &[u8]) -> Answer {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        #[serde(default)]
        body: Option<String>,
    }
    let Ok(release) = serde_json::from_slice::<Release>(json) else { return Answer::Nothing };
    let version = release.tag_name.strip_prefix('v').unwrap_or(&release.tag_name).to_owned();
    let notes = release.body.unwrap_or_default().lines().filter_map(|l| l.trim().strip_prefix("- ")).map(str::to_owned).collect();
    Answer::Release { version, notes }
}

/// `x.y.z` as numbers; anything else (a beta like `1.1.0-beta.1`, junk) is `None`, never offered.
fn version_triple(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.strip_prefix('v').unwrap_or(version).split('.');
    let triple = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    parts.next().is_none().then_some(triple)
}

/// Whether `latest` is a newer release than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((version_triple(latest), version_triple(current)), (Some(l), Some(c)) if l > c)
}

/// This version's release page for `version` (the download page Update opens).
pub fn release_page(version: &str) -> String {
    format!("https://github.com/pwn233/xReveal/releases/tag/v{version}")
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::{Answer, Updates, is_newer, parse};

    #[test]
    fn only_a_plainly_newer_release_is_offered() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.9"), "numbers, not text");
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"), "a dev build ahead of the latest release");
        assert!(!is_newer("0.2.0-beta.1", "0.1.0"), "betas are never offered");
        assert!(!is_newer("junk", "0.1.0"));
    }

    #[test]
    fn asks_once_a_day_and_when_the_clock_went_back() {
        let day = 24 * 60 * 60;
        let mut updates = Updates::default();
        assert!(updates.due(1_000), "never asked");
        updates.last_check = Some(1_000);
        assert!(!updates.due(1_000 + day - 1));
        assert!(updates.due(1_000 + day));
        assert!(updates.due(500), "the clock went back");
        updates.check = false;
        assert!(!updates.due(1_000 + 2 * day), "the switch is off");
    }

    #[test]
    fn an_answer_is_remembered_and_offered_while_the_switch_is_on() {
        let mut updates = Updates::default();
        updates.answered(7, parse(br#"{"tag_name":"v0.2.0","body":"Intro\n\n- New thing.\n- Fixed thing.\n","assets":[]}"#));
        assert_eq!(updates.last_check, Some(7));
        assert_eq!(updates.offer("0.1.0"), Some("0.2.0"));
        assert_eq!(updates.notes, ["New thing.", "Fixed thing."]);
        assert_eq!(updates.offer("0.2.0"), None, "after updating, nothing to offer");
        updates.turn_off();
        assert_eq!(updates.offer("0.1.0"), None, "turning it off forgets it");
    }

    #[test]
    fn nothing_published_or_a_beta_changes_nothing_but_the_time() {
        let mut updates = Updates::default();
        updates.answered(7, Answer::Nothing);
        assert_eq!((updates.last_check, updates.latest.as_deref()), (Some(7), None));
        updates.answered(8, parse(br#"{"tag_name":"v0.2.0-beta.1"}"#));
        assert_eq!(updates.latest, None, "a beta is never remembered");
        assert_eq!(parse(b"not json"), Answer::Nothing);
    }
}

#[cfg(test)]
mod install_tests {
    use super::{asset_url, checksum_for, file_matches};

    #[test]
    fn finds_an_assets_checksum() {
        let sums = "aaaa  other\n0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  xReveal-0.2.0-macos-arm64.zip\n";
        assert_eq!(checksum_for(sums, "xReveal-0.2.0-macos-arm64.zip"), Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));
        assert_eq!(checksum_for(sums, "missing.zip"), None);
        assert_eq!(checksum_for(sums, "other"), None, "not a full checksum");
    }

    #[test]
    fn checks_a_files_sha256() {
        let path = std::env::temp_dir().join(format!("xreveal-sha-test-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        assert!(file_matches(&path, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        assert!(!file_matches(&path, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ae"));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(asset_url("0.2.0", "SHA256SUMS"), "https://github.com/pwn233/xReveal/releases/download/v0.2.0/SHA256SUMS");
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod network {
    /// The real request, against GitHub. Run by hand: `cargo test -- --ignored github_answers`.
    #[test]
    #[ignore = "needs the network"]
    fn github_answers() {
        let answer = crate::platform::latest_release();
        eprintln!("{answer:?}");
        assert!(answer.is_some(), "GitHub should answer (a release, or 404 before the first one)");
    }
}
