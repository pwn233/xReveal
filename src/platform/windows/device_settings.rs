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

/// The DirectShow moniker of the device whose path is `link` (MF's symbolic link is the same path).
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
                if path.eq_ignore_ascii_case(link) {
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
