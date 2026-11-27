//! Camera and Microphone permission on macOS (docs/spec/v1.md §2.1): read the status, ask for both
//! (macOS shows its two prompts one after the other), and open the matching privacy page.

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

/// Asks for Camera, then Microphone once the first prompt is answered. Returns at once; the
/// answers show up in [`access`].
pub fn request() {
    let (Some(video), Some(audio)) = (media(Privacy::Camera), media(Privacy::Microphone)) else { return };
    let then_microphone = RcBlock::new(move |_granted: Bool| {
        let done = RcBlock::new(|_granted: Bool| {});
        unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &done) };
    });
    unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(video, &then_microphone) };
}

/// Opens System Settings › Privacy & Security on the Camera or Microphone page.
pub fn open_settings(privacy: Privacy) {
    let page = match privacy {
        Privacy::Camera => "Privacy_Camera",
        Privacy::Microphone => "Privacy_Microphone",
    };
    let _ = std::process::Command::new("open").arg(format!("x-apple.systempreferences:com.apple.preference.security?{page}")).spawn();
}

/// A camera effect: the class property that says it's on, its name, and the call that reads it.
type Effect = (objc2::runtime::Sel, &'static str, unsafe fn() -> bool);

/// The macOS camera effects switched on for xReveal (Control Center › Video Effects), by name
/// (#118). Each one came with a later macOS than xReveal's minimum, so each is asked only where
/// macOS knows it.
pub fn video_effects_on() -> Vec<&'static str> {
    use objc2::{ClassType, msg_send, sel};
    let class = AVCaptureDevice::class();
    let known = |selector| -> bool { unsafe { msg_send![class, respondsToSelector: selector] } };
    let effects: [Effect; 5] = [
        (sel!(reactionEffectsEnabled), "Reactions", AVCaptureDevice::reactionEffectsEnabled),
        (sel!(isPortraitEffectEnabled), "Portrait", AVCaptureDevice::isPortraitEffectEnabled),
        (sel!(isStudioLightEnabled), "Studio Light", AVCaptureDevice::isStudioLightEnabled),
        (sel!(isBackgroundReplacementEnabled), "Background", AVCaptureDevice::isBackgroundReplacementEnabled),
        (sel!(isCenterStageEnabled), "Center Stage", AVCaptureDevice::isCenterStageEnabled),
    ];
    effects.into_iter().filter(|(selector, _, on)| known(*selector) && unsafe { on() }).map(|(_, name, _)| name).collect()
}

/// Opens macOS's Video Effects panel for xReveal, where the person can turn the effects off.
pub fn show_video_effects() {
    use objc2::{ClassType, msg_send, sel};
    let known: bool = unsafe { msg_send![AVCaptureDevice::class(), respondsToSelector: sel!(showSystemUserInterface:)] };
    if known {
        unsafe { AVCaptureDevice::showSystemUserInterface(objc2_av_foundation::AVCaptureSystemUserInterface::VideoEffects) };
    }
}
