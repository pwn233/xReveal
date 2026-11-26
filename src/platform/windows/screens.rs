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

/// Makes `hwnd` (a window of this thread) the active one, so it takes the keys. Synchronously:
/// call it outside a GPUI update, so GPUI sees the change.
pub fn activate(hwnd: HWND) {
    unsafe {
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow(hwnd);
    }
}

/// Makes pop-up `popup` (the drawer's or the overlays' window) part of `owner`: owned by it, so it
/// stays just above it and minimises with it, never over other apps; and without the frame,
/// rounded corners, shadow and open and close animations Windows gives a window of its kind.
pub fn own_popup(popup: HWND, owner: HWND) {
    use windows::Win32::Graphics::Dwm::{
        DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY, DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
        DwmSetWindowAttribute,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GWLP_HWNDPARENT, GetWindowLongPtrW, HWND_NOTOPMOST, SWP_NOMOVE, SWP_NOSIZE, SetWindowLongPtrW, WS_EX_TOPMOST};
    unsafe {
        SetWindowLongPtrW(popup, GWLP_HWNDPARENT, owner.0 as isize);
        let style = GetWindowLongPtrW(popup, GWL_EXSTYLE);
        SetWindowLongPtrW(popup, GWL_EXSTYLE, style & !(WS_EX_TOPMOST.0 as isize));
        let _ = SetWindowPos(popup, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        let set = |attribute, value: u32| {
            let _ = DwmSetWindowAttribute(popup, attribute, (&raw const value).cast(), size_of::<u32>() as u32);
        };
        set(DWMWA_NCRENDERING_POLICY, DWMNCRP_DISABLED.0 as u32);
        set(DWMWA_TRANSITIONS_FORCEDISABLED, 1);
        set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND.0 as u32);
        set(DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE);
    }
}

/// Moves pop-up `popup` so its content covers `area` (left, top, right, bottom, in device pixels
/// from the top-left of `owner`'s content), keeping its place in the stack and without focusing it.
/// GPUI places a pop-up by its frame, which puts the content a few pixels off. Synchronously: call
/// it outside a GPUI update, so GPUI handles the move and resize as they happen.
pub fn place_popup(popup: HWND, owner: HWND, area: [i32; 4]) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
    let (mut origin, mut inner) = (POINT::default(), POINT::default());
    let (mut frame, mut client) = (RECT::default(), RECT::default());
    unsafe {
        if !ClientToScreen(owner, &mut origin).as_bool()
            || !ClientToScreen(popup, &mut inner).as_bool()
            || GetWindowRect(popup, &mut frame).is_err()
            || GetClientRect(popup, &mut client).is_err()
        {
            return;
        }
    }
    // The pop-up's frame around its content, on each side.
    let (left, top) = (inner.x - frame.left, inner.y - frame.top);
    let (right, bottom) = (frame.right - inner.x - client.right, frame.bottom - inner.y - client.bottom);
    let x = origin.x + area[0] - left;
    let y = origin.y + area[1] - top;
    let width = area[2] - area[0] + left + right;
    let height = area[3] - area[1] + top + bottom;
    if [x, y, width, height] == [frame.left, frame.top, frame.right - frame.left, frame.bottom - frame.top] {
        return;
    }
    unsafe {
        let _ = SetWindowPos(popup, None, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Turns `overlay` into a layer over `owner` (xReveal's overlays on Windows): part of it (see
/// [`own_popup`]); never takes focus; and lets every click and mouse move through to `owner`
/// underneath.
pub fn make_overlay(overlay: HWND, owner: HWND) {
    use windows::Win32::UI::Shell::SetWindowSubclass;
    use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_NOACTIVATE};
    own_popup(overlay, owner);
    unsafe {
        let style = GetWindowLongPtrW(overlay, GWL_EXSTYLE);
        SetWindowLongPtrW(overlay, GWL_EXSTYLE, style | WS_EX_NOACTIVATE.0 as isize);
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
