//! Installing a downloaded update on macOS (#163): the release's `.zip` holds `xReveal.app`.
//! It's unpacked next to the running app, its signature checked against this app's own designated
//! requirement (the stable certificate, #162), then the two bundles are swapped and the new one
//! launched. The old bundle is removed on the next launch.

use std::path::{Path, PathBuf};
use std::process::Command;

const STAGE: &str = ".xReveal-update";

/// The running `.app` bundle.
fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bundle = exe.ancestors().nth(3)?.to_owned();
    (bundle.extension()? == "app").then_some(bundle)
}

fn old_bundle(bundle: &Path) -> PathBuf {
    let mut name = bundle.file_name().unwrap_or_default().to_owned();
    name.push(".old");
    bundle.with_file_name(name)
}

/// Unpacks `zip` beside the running app and checks the new app is signed like this one. Returns
/// the new app, ready to swap in.
pub fn prepare(zip: &Path) -> Result<PathBuf, String> {
    let bundle = bundle().ok_or("xReveal isn't running from an app bundle")?;
    // Opened straight from a download (macOS runs it from a read-only copy): it can't replace itself.
    if bundle.to_string_lossy().contains("/AppTranslocation/") {
        return Err("Move xReveal to Applications first, then update".into());
    }
    let parent = bundle.parent().ok_or("no folder around the app")?;
    let stage = parent.join(STAGE);
    if stage.exists() {
        std::fs::remove_dir_all(&stage).map_err(|_| "couldn't clear an earlier update")?;
    }
    std::fs::create_dir(&stage).map_err(|_| "xReveal's folder can't be written to")?;
    let unpacked = Command::new("/usr/bin/ditto").arg("-x").arg("-k").arg(zip).arg(&stage).status().map_err(|e| e.to_string())?;
    let new = stage.join("xReveal.app");
    if !unpacked.success() || !new.exists() {
        return Err("the download didn't hold the app".into());
    }
    // The new app must be signed with the same certificate as this one.
    let requirement = Command::new("/usr/bin/codesign").arg("-d").arg("-r-").arg(&bundle).output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&requirement.stdout).into_owned() + &String::from_utf8_lossy(&requirement.stderr);
    let designated = text.lines().find_map(|l| l.split_once("designated => ")).map(|(_, r)| r.trim().to_owned()).ok_or("couldn't read this app's signature")?;
    let verified = Command::new("/usr/bin/codesign").arg("--verify").arg("--deep").arg("--strict").arg(format!("-R={designated}")).arg(&new).status().map_err(|e| e.to_string())?;
    if !verified.success() {
        return Err("the new version isn't signed like this one".into());
    }
    Ok(new)
}

/// Swaps the new app in and launches it. The caller quits right after.
pub fn install_and_relaunch(new: &Path) -> Result<(), String> {
    let bundle = bundle().ok_or("xReveal isn't running from an app bundle")?;
    let old = old_bundle(&bundle);
    if old.exists() {
        let _ = std::fs::remove_dir_all(&old);
    }
    std::fs::rename(&bundle, &old).map_err(|_| "couldn't move this version aside")?;
    if std::fs::rename(new, &bundle).is_err() {
        let _ = std::fs::rename(&old, &bundle);
        return Err("couldn't put the new version in place".into());
    }
    Command::new("/usr/bin/open").arg("-n").arg(&bundle).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// After an update: removes the previous version and the unpacking folder, if they're there.
pub fn clean_up() {
    let Some(bundle) = bundle() else { return };
    let old = old_bundle(&bundle);
    if old.exists() {
        let _ = std::fs::remove_dir_all(old);
    }
    if let Some(stage) = bundle.parent().map(|p| p.join(STAGE))
        && stage.exists()
    {
        let _ = std::fs::remove_dir_all(stage);
    }
}
