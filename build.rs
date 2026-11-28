//! Windows exe resources: the app icon (#130) and a version resource (#206), so Explorer's Details
//! tab, Task Manager, WER reports and Event Viewer show xReveal and its real version instead of
//! 0.0.0.0. macOS gets both from the .app bundle (packaging/macos).

use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/xreveal.rc");
    println!("cargo:rerun-if-changed=packaging/windows/xreveal.ico");
    println!("cargo:rerun-if-changed=LICENSE-MIT");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        write_version_header();
        // embed-resource puts OUT_DIR on the include path, where xreveal.rc finds xreveal-version.h.
        embed_resource::compile("packaging/windows/xreveal.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}

/// Writes `$OUT_DIR/xreveal-version.h` for xreveal.rc's VERSIONINFO: the version from Cargo.toml and
/// the copyright line from LICENSE-MIT, so the resource never needs editing for a release.
fn write_version_header() {
    let part = |name: &str| env::var(name).unwrap().parse::<u16>().unwrap();
    let (major, minor, patch) =
        (part("CARGO_PKG_VERSION_MAJOR"), part("CARGO_PKG_VERSION_MINOR"), part("CARGO_PKG_VERSION_PATCH"));
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let copyright = fs::read_to_string("LICENSE-MIT")
        .ok()
        .and_then(|text| text.lines().find(|line| line.starts_with("Copyright")).map(str::to_owned))
        .unwrap_or_default();
    let header = format!(
        "#define XREVEAL_VERSION_BIN {major},{minor},{patch},0\n\
         #define XREVEAL_VERSION_STR {}\n\
         #define XREVEAL_COPYRIGHT_STR {}\n",
        rc_string(&version),
        rc_string(&copyright),
    );
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("xreveal-version.h"), header).unwrap();
}

/// An RC string literal: quoted, with `""` for a quote inside.
fn rc_string(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
