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
