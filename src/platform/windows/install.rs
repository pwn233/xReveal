//! Installing a downloaded update on Windows (#163): the release's installer, run silently once
//! xReveal has quit. `/RELAUNCH` makes it wait for xReveal to close and start it again when done
//! (packaging/windows/installer.nsi).

use std::path::{Path, PathBuf};

/// Nothing to unpack: the installer itself is what runs. (Its checksum is already checked.)
pub fn prepare(installer: &Path) -> Result<PathBuf, String> {
    Ok(installer.to_owned())
}

/// Starts the installer, which waits for xReveal to quit. The caller quits right after.
pub fn install_and_relaunch(installer: &Path) -> Result<(), String> {
    std::process::Command::new(installer).args(["/S", "/RELAUNCH"]).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Nothing left behind to remove: the installer lives in the temporary folder.
pub fn clean_up() {}
