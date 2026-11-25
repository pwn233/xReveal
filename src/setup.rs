//! The Remembered setup and where it lives (docs/spec/v1.md §3.7).
//!
//! One `setup.toml` per machine, never roaming: macOS `~/Library/Application Support/xReveal/`,
//! Windows `%LOCALAPPDATA%\xReveal\`. Saved about a second after the last change and on quit,
//! always by writing a temporary file and renaming it over the old one. Read tolerantly: missing
//! fields take their defaults, unknown ones are ignored, and an unreadable file is set aside as
//! `setup.toml.bad` so xReveal starts as on first run.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::source::{SourceIdentity, VideoFormat};
use crate::video::picture::{Choice, ColourRange, ColourSpace, PictureSettings};
use crate::video::timing::DelaySettings;

pub const VERSION: u32 = 1;

/// The single saved set of choices xReveal applies on launch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Setup {
    pub version: u32,
    /// The Source last shown.
    pub source: Option<SourceIdentity>,
    pub video_format: Option<VideoFormat>,
    /// Target ratio, Fill mode, Smart stretch strength and Picture adjustments. Colour range and
    /// space are per Source (in `sources`), not here.
    pub picture: PictureSettings,
    /// Buffering and the A/V sync offset.
    pub delay: DelaySettings,
    pub audio: AudioRoute,
    /// The monitor to go fullscreen on, by the display's own identity.
    pub monitor: Option<String>,
    pub drawer_side: DrawerSide,
    /// Every Source ever seen, with its own colour choices. Never trimmed.
    pub sources: Vec<SourceEntry>,
    /// Audio buffer size per device ID, in frames, grown after dropouts.
    pub audio_buffers: BTreeMap<String, u32>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            version: VERSION,
            source: None,
            video_format: None,
            picture: PictureSettings::default(),
            delay: DelaySettings::default(),
            audio: AudioRoute::default(),
            monitor: None,
            drawer_side: DrawerSide::Right,
            sources: Vec::new(),
            audio_buffers: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct AudioRoute {
    /// A chosen input device; `None` means the Source's own audio.
    pub input: Option<String>,
    /// A chosen output device; `None` means the system default.
    pub output: Option<String>,
    /// 0..=1.
    pub volume: f32,
    pub muted: bool,
}

impl Default for AudioRoute {
    fn default() -> Self {
        Self { input: None, output: None, volume: 1.0, muted: false }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DrawerSide {
    #[default]
    Right,
    Left,
}

/// One Source xReveal has shown, with the choices that belong to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct SourceEntry {
    pub identity: SourceIdentity,
    pub colour_range: Choice<ColourRange>,
    pub colour_space: Choice<ColourSpace>,
    /// The one-time "Blacks look grey?" tip has been shown for this Source.
    pub tip_shown: bool,
}

impl Default for SourceEntry {
    fn default() -> Self {
        Self {
            identity: SourceIdentity { os_id: String::new(), serial: None, model: None, name: String::new() },
            colour_range: Choice::Auto,
            colour_space: Choice::Auto,
            tip_shown: false,
        }
    }
}

impl Setup {
    /// The entry for `identity` (same OS ID, or same serial), created if this Source is new.
    /// Its identity is refreshed, so a card moved to another port keeps its colour choices.
    pub fn source_entry(&mut self, identity: &SourceIdentity) -> &mut SourceEntry {
        let same = |e: &SourceEntry| {
            e.identity.os_id == identity.os_id || (identity.serial.is_some() && e.identity.serial == identity.serial)
        };
        let index = match self.sources.iter().position(same) {
            Some(i) => i,
            None => {
                self.sources.push(SourceEntry { identity: identity.clone(), ..Default::default() });
                self.sources.len() - 1
            }
        };
        let entry = &mut self.sources[index];
        entry.identity = identity.clone();
        entry
    }

    /// The picture settings for showing `identity`: the shared ones plus that Source's colour.
    pub fn picture_for(&self, identity: &SourceIdentity) -> PictureSettings {
        let mut picture = self.picture;
        if let Some(entry) = self
            .sources
            .iter()
            .find(|e| e.identity.os_id == identity.os_id || (identity.serial.is_some() && e.identity.serial == identity.serial))
        {
            picture.colour_range = entry.colour_range;
            picture.colour_space = entry.colour_space;
        }
        picture
    }
}

/// What was found on disk at launch.
#[derive(Debug, PartialEq)]
pub enum Loaded {
    Setup(Box<Setup>),
    /// No setup yet: the Welcome screen.
    FirstRun,
    /// The file couldn't be read; it was renamed to `setup.toml.bad`. Start as on first run and
    /// say so (`notice`).
    Recovered { notice: String },
}

/// `setup.toml` in the per-machine app data folder, or `None` if the OS gives no such folder.
pub fn default_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"));
    base.map(|dir| dir.join("xReveal").join("setup.toml"))
}

pub fn load(path: &Path) -> Loaded {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Loaded::FirstRun,
        Err(error) => return set_aside(path, &error.to_string()),
    };
    match toml::from_str::<Setup>(&text) {
        Ok(setup) => Loaded::Setup(Box::new(setup)),
        Err(error) => set_aside(path, &error.to_string()),
    }
}

fn set_aside(path: &Path, why: &str) -> Loaded {
    let bad = path.with_extension("toml.bad");
    let _ = std::fs::rename(path, &bad);
    Loaded::Recovered { notice: format!("Your saved settings couldn't be read, so xReveal started fresh ({why}).") }
}

/// Writes `setup` to `path` atomically: a temporary file next to it, then a rename over it.
pub fn save(path: &Path, setup: &Setup) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string_pretty(setup).map_err(io::Error::other)?;
    let temporary = path.with_extension("toml.tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path)
}

/// Reset all settings: forget the Remembered setup and every Source. The next launch is a first run.
pub fn reset(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// The Remembered setup in memory, saved in the background as it changes.
pub struct Store {
    path: Option<PathBuf>,
    pub setup: Setup,
    /// No setup was found: the first launch (the Welcome screen, once it exists).
    pub first_run: bool,
    /// Shown once when a broken file had to be set aside.
    pub notice: Option<String>,
    autosave: Option<Autosave>,
}

impl Store {
    /// Loads `setup.toml` from the per-machine folder.
    pub fn open() -> Self {
        Self::open_at(default_path())
    }

    pub fn open_at(path: Option<PathBuf>) -> Self {
        let (setup, first_run, notice) = match path.as_deref().map(load) {
            Some(Loaded::Setup(setup)) => (*setup, false, None),
            Some(Loaded::Recovered { notice }) => (Setup::default(), true, Some(notice)),
            Some(Loaded::FirstRun) | None => (Setup::default(), true, None),
        };
        let autosave = path.clone().map(|p| Autosave::new(p, Duration::from_secs(1)));
        Self { path, setup, first_run, notice, autosave }
    }

    /// Changes the setup; it is saved about a second after the last change.
    pub fn update(&mut self, change: impl FnOnce(&mut Setup)) {
        let before = self.setup.clone();
        change(&mut self.setup);
        if self.setup != before
            && let Some(autosave) = &self.autosave
        {
            autosave.changed(&self.setup);
        }
    }

    /// Saves any pending change now (on quit).
    pub fn flush(&mut self) {
        if let Some(autosave) = &mut self.autosave {
            autosave.flush();
        }
        self.autosave = self.path.clone().map(|p| Autosave::new(p, Duration::from_secs(1)));
    }

    /// Reset all settings (docs/spec/v1.md §2.3): forgets everything; the next launch is a first run.
    pub fn reset_all(&mut self) -> io::Result<()> {
        if let Some(autosave) = &mut self.autosave {
            autosave.flush();
        }
        if let Some(path) = &self.path {
            reset(path)?;
        }
        self.setup = Setup::default();
        self.first_run = true;
        self.autosave = self.path.clone().map(|p| Autosave::new(p, Duration::from_secs(1)));
        Ok(())
    }
}

enum Message {
    Changed(Box<Setup>),
    Flush,
}

/// Saves on a background thread once changes have stopped for `quiet` (so dragging a slider
/// doesn't write 60 times a second), and immediately on [`flush`](Self::flush) or drop.
pub struct Autosave {
    sender: Option<Sender<Message>>,
    thread: Option<JoinHandle<()>>,
}

impl Autosave {
    pub fn new(path: PathBuf, quiet: Duration) -> Self {
        let (sender, receiver) = mpsc::channel::<Message>();
        let thread = std::thread::Builder::new()
            .name("xreveal-autosave".into())
            .spawn(move || {
                let mut pending: Option<Box<Setup>> = None;
                loop {
                    let message = match pending {
                        Some(_) => receiver.recv_timeout(quiet),
                        None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
                    };
                    match message {
                        Ok(Message::Changed(setup)) => pending = Some(setup),
                        Ok(Message::Flush) | Err(RecvTimeoutError::Timeout) => write(&path, pending.take()),
                        Err(RecvTimeoutError::Disconnected) => {
                            write(&path, pending.take());
                            return;
                        }
                    }
                }
            })
            .ok();
        Self { sender: Some(sender), thread }
    }

    /// Remembers `setup`; it is written once nothing has changed for a moment.
    pub fn changed(&self, setup: &Setup) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(Message::Changed(Box::new(setup.clone())));
        }
    }

    /// Writes any pending change now and waits for it (on quit).
    pub fn flush(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(Message::Flush);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Autosave {
    fn drop(&mut self) {
        self.flush();
    }
}

fn write(path: &Path, setup: Option<Box<Setup>>) {
    if let Some(setup) = setup
        && let Err(error) = save(path, &setup)
    {
        eprintln!("setup: couldn't save {}: {error}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::picture::{FillMode, TargetRatio};

    /// A fresh folder per test, removed afterwards.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("xreveal-setup-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn file(&self) -> PathBuf {
            self.0.join("setup.toml")
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn onten(os_id: &str) -> SourceIdentity {
        SourceIdentity { os_id: os_id.into(), serial: Some("93869598".into()), model: Some((0x345f, 0x2131)), name: "USB2 Video".into() }
    }

    fn sample() -> Setup {
        let mut setup = Setup {
            source: Some(onten("port-A")),
            video_format: Some(VideoFormat { width: 1280, height: 720, fps: 60.0 }),
            monitor: Some("ultrawide-uuid".into()),
            ..Default::default()
        };
        setup.picture.fill_mode = FillMode::SmartStretch;
        setup.picture.target_ratio = TargetRatio(64.0 / 27.0);
        setup.delay.av_sync_offset_ms = -20;
        setup.source_entry(&onten("port-A")).colour_range = Choice::Manual(ColourRange::Limited);
        setup.audio_buffers.insert("USB2 Digital Audio".into(), 256);
        setup
    }

    #[test]
    fn round_trips_and_leaves_no_temporary_file() {
        let dir = TempDir::new("round-trip");
        save(&dir.file(), &sample()).unwrap();
        assert_eq!(load(&dir.file()), Loaded::Setup(Box::new(sample())));
        assert!(!dir.file().with_extension("toml.tmp").exists());
        let text = std::fs::read_to_string(dir.file()).unwrap();
        assert!(text.contains("version = 1"), "{text}");
    }

    #[test]
    fn missing_file_is_a_first_run() {
        let dir = TempDir::new("missing");
        assert_eq!(load(&dir.file()), Loaded::FirstRun);
    }

    #[test]
    fn missing_fields_take_defaults_and_unknown_fields_are_ignored() {
        let dir = TempDir::new("tolerant");
        std::fs::write(dir.file(), "version = 7\nfrom-a-newer-xreveal = true\n[delay]\nbuffering-frames = 1\n").unwrap();
        let Loaded::Setup(setup) = load(&dir.file()) else { panic!("should load") };
        assert_eq!(setup.version, 7);
        assert_eq!(setup.delay.buffering_frames, 1);
        assert_eq!(setup.picture, PictureSettings::default());
        assert_eq!(setup.drawer_side, DrawerSide::Right);
    }

    #[test]
    fn unreadable_file_is_set_aside() {
        let dir = TempDir::new("corrupt");
        std::fs::write(dir.file(), "this is [not toml").unwrap();
        let Loaded::Recovered { notice } = load(&dir.file()) else { panic!("should recover") };
        assert!(notice.contains("couldn't be read"));
        assert!(!dir.file().exists());
        assert!(dir.file().with_extension("toml.bad").exists());
        assert_eq!(load(&dir.file()), Loaded::FirstRun, "the next launch starts fresh");
    }

    #[test]
    fn source_entries_follow_a_card_to_another_port() {
        let mut setup = sample();
        let moved = onten("port-B");
        assert_eq!(setup.picture_for(&moved).colour_range, Choice::Manual(ColourRange::Limited), "found by serial");
        setup.source_entry(&moved).tip_shown = true;
        assert_eq!(setup.sources.len(), 1, "same card, one entry");
        assert_eq!(setup.sources[0].identity.os_id, "port-B");
        let other = SourceIdentity { os_id: "elgato".into(), serial: None, model: None, name: "HD60 X".into() };
        assert_eq!(setup.picture_for(&other).colour_range, Choice::Auto);
        setup.source_entry(&other);
        assert_eq!(setup.sources.len(), 2, "every Source ever seen is kept");
    }

    #[test]
    fn autosave_waits_for_quiet_then_writes_the_latest() {
        let dir = TempDir::new("autosave");
        let mut autosave = Autosave::new(dir.file(), Duration::from_millis(150));
        let mut setup = Setup::default();
        for offset in [-1, -2, -3] {
            setup.delay.av_sync_offset_ms = offset;
            autosave.changed(&setup);
        }
        std::thread::sleep(Duration::from_millis(50));
        assert!(!dir.file().exists(), "not written while changes are still coming");
        std::thread::sleep(Duration::from_millis(400));
        let Loaded::Setup(saved) = load(&dir.file()) else { panic!("written after the quiet period") };
        assert_eq!(saved.delay.av_sync_offset_ms, -3);

        setup.delay.av_sync_offset_ms = -4;
        autosave.changed(&setup);
        autosave.flush();
        let Loaded::Setup(saved) = load(&dir.file()) else { panic!("written on flush") };
        assert_eq!(saved.delay.av_sync_offset_ms, -4, "flush writes immediately (quit)");
    }

    #[test]
    fn store_saves_changes_and_resets() {
        let dir = TempDir::new("store");
        let mut store = Store::open_at(Some(dir.file()));
        assert!(store.first_run);
        store.update(|s| s.monitor = Some("ultrawide".into()));
        store.flush();
        let reopened = Store::open_at(Some(dir.file()));
        assert!(!reopened.first_run);
        assert_eq!(reopened.setup.monitor.as_deref(), Some("ultrawide"));
        store.reset_all().unwrap();
        assert!(store.first_run);
        assert!(Store::open_at(Some(dir.file())).first_run);
    }

    #[test]
    fn reset_forgets_everything() {
        let dir = TempDir::new("reset");
        save(&dir.file(), &sample()).unwrap();
        reset(&dir.file()).unwrap();
        assert_eq!(load(&dir.file()), Loaded::FirstRun);
        reset(&dir.file()).unwrap();
    }

    #[test]
    fn default_path_is_per_machine() {
        let path = default_path().unwrap();
        assert!(path.ends_with("xReveal/setup.toml") || path.ends_with("xReveal\\setup.toml"));
        #[cfg(target_os = "macos")]
        assert!(path.to_string_lossy().contains("Library/Application Support"));
    }
}
