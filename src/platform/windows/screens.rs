//! Monitors on Windows: their names, and moving the window onto one (GPUI has no call for either).

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{DISPLAY_DEVICEW, EnumDisplayDevicesW, GetMonitorInfoW, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos};
use windows::core::PCWSTR;

fn info(monitor: u64) -> Option<MONITORINFOEXW> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let ok = unsafe { GetMonitorInfoW(HMONITOR(monitor as _), &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) };
    ok.as_bool().then_some(info)
}

/// A monitor's name as Windows lists it ("DELL U2720Q"), or `None`.
pub fn name(monitor: u64) -> Option<String> {
    let info = info(monitor)?;
    let mut device = DISPLAY_DEVICEW { cb: size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
    unsafe { EnumDisplayDevicesW(PCWSTR(info.szDevice.as_ptr()), 0, &mut device, 0) }.as_bool().then(|| {
        let len = device.DeviceString.iter().position(|&c| c == 0).unwrap_or(device.DeviceString.len());
        String::from_utf16_lossy(&device.DeviceString[..len])
    })
}

/// Moves window `hwnd` to the middle of `monitor`'s work area, keeping its size (shrunk to fit).
pub fn move_window(hwnd: HWND, monitor: u64) {
    let Some(info) = info(monitor) else { return };
    let area = info.monitorInfo.rcWork;
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return;
    }
    let width = (rect.right - rect.left).min(area.right - area.left);
    let height = (rect.bottom - rect.top).min(area.bottom - area.top);
    let x = area.left + (area.right - area.left - width) / 2;
    let y = area.top + (area.bottom - area.top - height) / 2;
    unsafe {
        // Synchronously, from outside a GPUI update (see `platform::move_window`): with
        // SWP_ASYNCWINDOWPOS on its own thread the move still happened at once, inside the update
        // that asked for it, so GPUI missed it and nothing followed the window.
        let _ = SetWindowPos(hwnd, None, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Turns `overlay` into a layer over `owner` (xReveal's overlays on Windows): owned by it, so it
/// stays just above it and minimises with it, not over other apps; never takes focus; and lets
/// every click and mouse move through to `owner` underneath.
pub fn make_overlay(overlay: HWND, owner: HWND) {
    use windows::Win32::UI::Shell::SetWindowSubclass;
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWLP_HWNDPARENT, GetWindowLongPtrW, HWND_NOTOPMOST, SWP_NOMOVE, SWP_NOSIZE, SetWindowLongPtrW, WS_EX_NOACTIVATE, WS_EX_TOPMOST,
    };
    unsafe {
        SetWindowLongPtrW(overlay, GWLP_HWNDPARENT, owner.0 as isize);
        let style = GetWindowLongPtrW(overlay, GWL_EXSTYLE);
        SetWindowLongPtrW(overlay, GWL_EXSTYLE, (style | WS_EX_NOACTIVATE.0 as isize) & !(WS_EX_TOPMOST.0 as isize));
        let _ = SetWindowPos(overlay, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        let _ = SetWindowSubclass(overlay, Some(pass_mouse_through), 1, 0);
    }
}

/// Every mouse message goes to the window underneath (`owner`, which is on the same thread).
unsafe extern "system" fn pass_mouse_through(
    hwnd: HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{HTTRANSPARENT, WM_NCHITTEST};
    if message == WM_NCHITTEST {
        return windows::Win32::Foundation::LRESULT(HTTRANSPARENT as isize);
    }
    unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, message, wparam, lparam) }
}
