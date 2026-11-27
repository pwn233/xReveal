//! Camera and Microphone access on Windows (docs/spec/v1.md §2.1, #176). Windows never asks: its
//! privacy settings say whether desktop apps such as xReveal may use the camera or microphone,
//! and a card opened while they're off fails with `E_ACCESSDENIED` or simply stops.
//!
//! The switches live in the capability access manager's consent store, read here as they are
//! now, so a switch flipped while xReveal runs is seen at the next look:
//! - `HKLM\…\ConsentStore\<capability>`: the device-wide "Camera access" / "Microphone access",
//!   which only an administrator can change;
//! - `HKCU\…\ConsentStore\<capability>`: "Let apps access your camera" for this user;
//! - `HKCU\…\ConsentStore\<capability>\NonPackaged`: "Let desktop apps access your camera".
//!
//! Each holds `Value` = `Allow` or `Deny`. Any `Deny` turns access off; a missing key or value
//! (an older Windows) doesn't.

use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::HSTRING;

use crate::permission::{Access, Privacy};

const STORE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore";

/// Whether Windows lets desktop apps use the camera or the microphone.
pub fn access(privacy: Privacy) -> Access {
    let capability = match privacy {
        Privacy::Camera => "webcam",
        Privacy::Microphone => "microphone",
    };
    let keys = [
        (HKEY_LOCAL_MACHINE, format!(r"{STORE}\{capability}")),
        (HKEY_CURRENT_USER, format!(r"{STORE}\{capability}")),
        (HKEY_CURRENT_USER, format!(r"{STORE}\{capability}\NonPackaged")),
    ];
    if keys.iter().any(|(root, key)| consent(*root, key).is_some_and(|value| value.eq_ignore_ascii_case("Deny"))) {
        Access::Denied
    } else {
        Access::Granted
    }
}

/// The `Value` string under `key`, if there is one.
fn consent(root: HKEY, key: &str) -> Option<String> {
    let mut buffer = [0u16; 16];
    let mut size = size_of_val(&buffer) as u32;
    let result = unsafe {
        RegGetValueW(root, &HSTRING::from(key), &HSTRING::from("Value"), RRF_RT_REG_SZ, None, Some(buffer.as_mut_ptr().cast()), Some(&mut size))
    };
    if result.is_err() {
        return None;
    }
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..len]))
}
