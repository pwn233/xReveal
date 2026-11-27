//! Which sound devices are the Mac's own microphone and speakers, for the echo guard (#112,
//! docs/spec/v1.md §2.7). CoreAudio says it for certain: a built-in device's data source is `imic`
//! (the internal microphone), `ispk` (the internal speakers) or `hdpn` (headphones in the jack).

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectID, AudioObjectPropertyAddress, AudioObjectPropertyScope, AudioObjectPropertySelector,
    kAudioDevicePropertyDataSource, kAudioDevicePropertyTransportType, kAudioDeviceTransportTypeBuiltIn,
    kAudioHardwarePropertyTranslateUIDToDevice, kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeInput, kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
};
use objc2_core_foundation::CFString;

use crate::audio::DeviceKind;

const INTERNAL_MICROPHONE: u32 = u32::from_be_bytes(*b"imic");
const INTERNAL_SPEAKERS: u32 = u32::from_be_bytes(*b"ispk");

/// What the device with CoreAudio UID `uid` (cpal's device ID on macOS) is, as an input or output.
pub fn kind(uid: &str, input: bool) -> DeviceKind {
    let Some(device) = device_for(uid) else { return DeviceKind::Other };
    let built_in = read::<u32>(device, kAudioDevicePropertyTransportType, kAudioObjectPropertyScopeGlobal) == Some(kAudioDeviceTransportTypeBuiltIn);
    let scope = if input { kAudioObjectPropertyScopeInput } else { kAudioObjectPropertyScopeOutput };
    match (built_in, read::<u32>(device, kAudioDevicePropertyDataSource, scope)) {
        (true, Some(INTERNAL_MICROPHONE)) if input => DeviceKind::BuiltInMicrophone,
        (true, Some(INTERNAL_SPEAKERS)) if !input => DeviceKind::BuiltInSpeakers,
        _ => DeviceKind::Other,
    }
}

/// The CoreAudio device with UID `uid`, if it's there.
fn device_for(uid: &str) -> Option<AudioObjectID> {
    let uid = CFString::from_str(uid);
    let qualifier: *const CFString = &*uid;
    let address = address(kAudioHardwarePropertyTranslateUIDToDevice, kAudioObjectPropertyScopeGlobal);
    let mut device: AudioObjectID = 0;
    let mut size = size_of::<AudioObjectID>() as u32;
    // SAFETY: the qualifier is a CFStringRef, as this property asks for, alive for the call; the
    // output is one AudioObjectID.
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&address),
            size_of::<*const CFString>() as u32,
            (&raw const qualifier).cast::<c_void>(),
            NonNull::from(&mut size),
            NonNull::from(&mut device).cast(),
        )
    };
    (status == 0 && device != 0).then_some(device)
}

/// One plain-data property of `object`.
fn read<T: Copy + Default>(object: AudioObjectID, selector: AudioObjectPropertySelector, scope: AudioObjectPropertyScope) -> Option<T> {
    let address = address(selector, scope);
    let mut value = T::default();
    let mut size = size_of::<T>() as u32;
    // SAFETY: `value` has room for `size` bytes, and these selectors return a `u32`.
    let status = unsafe { AudioObjectGetPropertyData(object, NonNull::from(&address), 0, std::ptr::null(), NonNull::from(&mut size), NonNull::from(&mut value).cast()) };
    (status == 0 && size as usize == size_of::<T>()).then_some(value)
}

fn address(selector: AudioObjectPropertySelector, scope: AudioObjectPropertyScope) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain }
}
