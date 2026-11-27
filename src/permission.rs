//! Camera and Microphone access (docs/spec/v1.md §2.1). macOS asks the person; Windows has no
//! prompt, only a global privacy switch that makes opening a card fail with `E_ACCESSDENIED`.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Access {
    #[default]
    Granted,
    /// Never asked: the Welcome screen explains, then asks.
    NotAsked,
    Denied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Privacy {
    Camera,
    Microphone,
}

#[cfg(target_os = "macos")]
pub use crate::platform::macos::access::{access, open_settings, request};

/// What an **Allow access** button does (#113): asks, when macOS hasn't asked yet; once it has, it
/// won't ask again, so it opens the privacy settings instead.
pub fn allow(privacy: Privacy) {
    match access(privacy) {
        Access::NotAsked => request(),
        Access::Denied => open_settings(privacy),
        Access::Granted => {}
    }
}

#[cfg(not(target_os = "macos"))]
pub fn access(_privacy: Privacy) -> Access {
    Access::Granted
}

#[cfg(not(target_os = "macos"))]
pub fn request() {}

/// Opens Windows' privacy settings for the camera or microphone.
#[cfg(not(target_os = "macos"))]
pub fn open_settings(privacy: Privacy) {
    let page = match privacy {
        Privacy::Camera => "ms-settings:privacy-webcam",
        Privacy::Microphone => "ms-settings:privacy-microphone",
    };
    let _ = std::process::Command::new("explorer").arg(page).spawn();
}
