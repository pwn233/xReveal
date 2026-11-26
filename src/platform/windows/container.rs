//! Container IDs (docs/spec/v1.md §2.1): Windows gives every device node of one physical device the
//! same container ID, so a card's video and its own audio can be matched without guessing by name.

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Device_Interface_PropertyW, CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CR_SUCCESS,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_ContainerId, DEVPKEY_Device_InstanceId, DEVPROPTYPE};
use windows::core::{GUID, HSTRING, PCWSTR};

/// The container every root-enumerated (built-in, virtual) device shares: no real device.
const NO_CONTAINER: GUID = GUID::from_u128(0x00000000_0000_0000_ffff_ffffffffffff);

/// The container ID of the device node with instance ID `instance`.
fn of_instance(instance: &str) -> Option<GUID> {
    let mut node = 0u32;
    let wide = HSTRING::from(instance);
    if unsafe { CM_Locate_DevNodeW(&mut node, PCWSTR(wide.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) } != CR_SUCCESS {
        return None;
    }
    let mut guid = GUID::zeroed();
    let mut kind = DEVPROPTYPE::default();
    let mut size = size_of::<GUID>() as u32;
    let result = unsafe { CM_Get_DevNode_PropertyW(node, &DEVPKEY_Device_ContainerId, &mut kind, Some(&mut guid as *mut GUID as *mut u8), &mut size, 0) };
    (result == CR_SUCCESS && guid != NO_CONTAINER && guid != GUID::zeroed()).then_some(guid)
}

/// The container ID of the capture device with symbolic link `link` (a Source's OS ID).
pub fn of_video(link: &str) -> Option<GUID> {
    let mut buffer = [0u16; 512];
    let mut kind = DEVPROPTYPE::default();
    let mut size = (buffer.len() * 2) as u32;
    let wide = HSTRING::from(link);
    let result = unsafe {
        CM_Get_Device_Interface_PropertyW(PCWSTR(wide.as_ptr()), &DEVPKEY_Device_InstanceId, &mut kind, Some(buffer.as_mut_ptr() as *mut u8), &mut size, 0)
    };
    if result != CR_SUCCESS {
        return None;
    }
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    of_instance(&String::from_utf16_lossy(&buffer[..len]))
}

/// The container ID of the audio endpoint with ID `endpoint` (cpal's WASAPI device ID).
pub fn of_audio_endpoint(endpoint: &str) -> Option<GUID> {
    of_instance(&format!("SWD\\MMDEVAPI\\{endpoint}"))
}
