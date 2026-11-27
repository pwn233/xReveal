//! Embeds the app icon in the Windows exe (#130). macOS gets its icon from the .app bundle
//! (packaging/macos/AppIcon.icns).

fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/xreveal.rc");
    println!("cargo:rerun-if-changed=packaging/windows/xreveal.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("packaging/windows/xreveal.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
