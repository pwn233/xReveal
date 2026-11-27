//! Camera and Microphone permission on macOS (docs/spec/v1.md §2.1): read the status, ask for one
//! (#143: each has its own button), and open the matching privacy page.

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaType, AVMediaTypeAudio, AVMediaTypeVideo};

use crate::permission::{Access, Privacy};

fn media(privacy: Privacy) -> Option<&'static AVMediaType> {
    unsafe {
        match privacy {
            Privacy::Camera => AVMediaTypeVideo,
            Privacy::Microphone => AVMediaTypeAudio,
        }
    }
}

pub fn access(privacy: Privacy) -> Access {
    let Some(media) = media(privacy) else { return Access::Granted };
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(media) } {
        AVAuthorizationStatus::Authorized => Access::Granted,
        AVAuthorizationStatus::NotDetermined => Access::NotAsked,
        _ => Access::Denied,
    }
}

/// Asks for `privacy` (macOS shows its prompt once, ever). Returns at once; the answer shows up
/// in [`access`].
pub fn request(privacy: Privacy) {
    let Some(media) = media(privacy) else { return };
    let done = RcBlock::new(|_granted: Bool| {});
    unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(media, &done) };
}

/// Opens System Settings › Privacy & Security on the Camera or Microphone page.
pub fn open_settings(privacy: Privacy) {
    let page = match privacy {
        Privacy::Camera => "Privacy_Camera",
        Privacy::Microphone => "Privacy_Microphone",
    };
    let _ = std::process::Command::new("open").arg(format!("x-apple.systempreferences:com.apple.preference.security?{page}")).spawn();
}
