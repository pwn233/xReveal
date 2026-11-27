//! The capture device's own settings window (docs/spec/v1.md §2.3, §3.1): DirectShow property
//! pages (`ISpecifyPropertyPages` + `OleCreatePropertyFrame`), which drivers offer for brightness,
//! input selection, HDCP and the like. Media Foundation has nothing equivalent.
//!
//! The window opens on a thread of its own (property pages need a single-threaded apartment), so
//! xReveal keeps drawing while it is open.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use windows::Win32::Foundation::HWND;
use windows::Win32::Media::DirectShow::ICreateDevEnum;
use windows::Win32::Media::MediaFoundation::{CLSID_SystemDeviceEnum, CLSID_VideoInputDeviceCategory};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IMoniker};
use windows::Win32::System::Ole::{ISpecifyPropertyPages, OleCreatePropertyFrame};
use windows::Win32::System::Variant::{VARIANT, VT_BSTR, VariantClear};
use windows::core::{HSTRING, IUnknown, Interface, PCWSTR, w};

/// Where an open settings window stands.
pub const OPEN: u8 = 0;
pub const CLOSED: u8 = 1;
/// The driver refused while Media Foundation had the device: close the stream and try again.
pub const REFUSED: u8 = 2;

/// The device part of an interface path, without the interface class and reference string after it:
/// `\\?\usb#vid_345f&pid_2131&mi_00#8&9e76000&0&0000` from `…0000#{e5323777-…}\global`.
fn device_part(path: &str) -> &str {
    path.rfind("#{").map_or(path, |at| &path[..at])
}

/// Whether DirectShow's `DevicePath` and Media Foundation's symbolic link name the same device.
/// They can differ in their interface class: MF names a card that registers as a camera under
/// `{e5323777-…}` (camera), DirectShow under `{65e8773d-…}` (capture) or `{6994ad05-…}` (video).
fn same_device(directshow: &str, media_foundation: &str) -> bool {
    device_part(directshow).eq_ignore_ascii_case(device_part(media_foundation))
}

/// The DirectShow moniker of the device whose MF symbolic link is `link`.
unsafe fn moniker(link: &str) -> Option<IMoniker> {
    unsafe {
        let devices: ICreateDevEnum = CoCreateInstance(&CLSID_SystemDeviceEnum, None, CLSCTX_INPROC_SERVER).ok()?;
        let mut list = None;
        devices.CreateClassEnumerator(&CLSID_VideoInputDeviceCategory, &mut list, 0).ok()?;
        let list = list?;
        loop {
            let mut found = [None];
            if list.Next(&mut found, None).is_err() {
                return None;
            }
            let moniker = found[0].take()?;
            let Ok(bag) = moniker.BindToStorage::<_, _, IPropertyBag>(None, None) else { continue };
            let mut value = VARIANT::default();
            if bag.Read(w!("DevicePath"), &mut value, None).is_ok() && value.Anonymous.Anonymous.vt == VT_BSTR {
                let path = value.Anonymous.Anonymous.Anonymous.bstrVal.to_string();
                let _ = VariantClear(&mut value);
                if same_device(&path, link) {
                    return Some(moniker);
                }
            }
        }
    }
}

unsafe fn pages(link: &str) -> Option<(IUnknown, ISpecifyPropertyPages)> {
    unsafe {
        let filter: IUnknown = moniker(link)?.BindToObject(None, None).ok()?;
        let pages = filter.cast::<ISpecifyPropertyPages>().ok()?;
        Some((filter, pages))
    }
}

/// Whether the device offers a settings window. Runs its own short COM session.
pub fn available(link: &str) -> bool {
    let link = link.to_owned();
    // A thread of its own, so the check never meets the UI thread's COM apartment.
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let offered = pages(&link).and_then(|(_, p)| p.GetPages().ok()).is_some_and(|list| {
            let any = list.cElems > 0;
            CoTaskMemFree(Some(list.pElems as *const _));
            any
        });
        CoUninitialize();
        offered
    })
    .join()
    .unwrap_or(false)
}

/// Opens the settings window for device `link` over window `owner`; `state` goes from [`OPEN`] to
/// [`CLOSED`], or to [`REFUSED`] if the driver wouldn't open it.
pub fn open(link: &str, title: &str, owner: HWND, state: Arc<AtomicU8>) {
    let (link, title, owner) = (link.to_owned(), title.to_owned(), owner.0 as isize);
    state.store(OPEN, Ordering::SeqCst);
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let shown = (|| {
            let (filter, pages) = pages(&link)?;
            let list = pages.GetPages().ok()?;
            let caption = HSTRING::from(title);
            let result = OleCreatePropertyFrame(HWND(owner as *mut _), 0, 0, PCWSTR(caption.as_ptr()), 1, &Some(filter), list.cElems, list.pElems, 0, None, None);
            CoTaskMemFree(Some(list.pElems as *const _));
            result.ok()
        })();
        state.store(if shown.is_some() { CLOSED } else { REFUSED }, Ordering::SeqCst);
        CoUninitialize();
    });
}

#[cfg(test)]
mod tests {
    use super::same_device;

    const MF: &str = r"\?\usb#vid_345f&pid_2131&mi_00#8&9e76000&0&0000#{e5323777-f976-4f5b-9b55-b94699c46e44}\global";

    #[test]
    fn a_camera_matches_its_directshow_path_under_another_interface_class() {
        let capture = r"\?\USB#VID_345F&PID_2131&MI_00#8&9e76000&0&0000#{65e8773d-8f56-11d0-a3b9-00a0c9223196}\global";
        let video = r"\?\usb#vid_345f&pid_2131&mi_00#8&9e76000&0&0000#{6994ad05-93ef-11d0-a3cc-00a0c9223196}\global";
        assert!(same_device(capture, MF));
        assert!(same_device(video, MF));
        assert!(same_device(MF, MF));
    }

    #[test]
    fn another_device_or_interface_does_not_match() {
        let other_interface = r"\?\usb#vid_345f&pid_2131&mi_02#8&9e76000&0&0002#{65e8773d-8f56-11d0-a3b9-00a0c9223196}\global";
        let other_card = r"\?\usb#vid_345f&pid_2131&mi_00#8&1111111&0&0000#{65e8773d-8f56-11d0-a3b9-00a0c9223196}\global";
        assert!(!same_device(other_interface, MF));
        assert!(!same_device(other_card, MF));
    }
}
