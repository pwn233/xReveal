//! Monitors on Windows: their names, and moving the window onto one (GPUI has no call for either).

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{DISPLAY_DEVICEW, EnumDisplayDevicesW, GetMonitorInfoW, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos};
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
        // Asynchronously: the window's messages are handled after GPUI finishes the click that
        // asked for the move, not in the middle of it (#82).
        let _ = SetWindowPos(hwnd, None, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS);
    }
}

/// Borderless (docs/spec/v1.md §2.2): takes window `hwnd`'s title bar and border away, keeping its
/// content's size, and centres it on `monitor` (else the one it's on), shrunk to fit. With
/// `borderless` false, gives them back around the same content. Synchronously: call it outside a
/// GPUI update, so GPUI handles the resize as it happens.
pub fn set_borderless(hwnd: HWND, borderless: bool, monitor: Option<u64>) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{ClientToScreen, MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
    use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetWindowLongPtrW, IsZoomed, SW_RESTORE, SWP_FRAMECHANGED, SetWindowLongPtrW, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
        WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
    };
    // A normal window's frame: GPUI's main window is a standard overlapped window.
    let frame = (WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX).0 as isize;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        if !borderless && style & frame == frame {
            return;
        }
        if IsZoomed(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let (mut origin, mut client) = (POINT::default(), RECT::default());
        if !ClientToScreen(hwnd, &mut origin).as_bool() || GetClientRect(hwnd, &mut client).is_err() {
            return;
        }
        let mut rect = RECT { left: origin.x, top: origin.y, right: origin.x + client.right, bottom: origin.y + client.bottom };
        let style = if borderless { style & !frame } else { style | frame };
        let on = monitor.unwrap_or(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST).0 as u64);
        if borderless {
            // Centred on the whole monitor, as near as its size allows.
            if let Some(info) = info(on) {
                let screen = info.monitorInfo.rcMonitor;
                let width = (rect.right - rect.left).min(screen.right - screen.left);
                let height = (rect.bottom - rect.top).min(screen.bottom - screen.top);
                let (x, y) = (screen.left + (screen.right - screen.left - width) / 2, screen.top + (screen.bottom - screen.top - height) / 2);
                rect = RECT { left: x, top: y, right: x + width, bottom: y + height };
            }
        } else {
            // The frame around the same content, its title bar kept on the screen.
            let exstyle = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let _ = AdjustWindowRectExForDpi(&mut rect, WINDOW_STYLE(style as u32), false, WINDOW_EX_STYLE(exstyle as u32), GetDpiForWindow(hwnd));
            if let Some(info) = info(on) {
                let below = (info.monitorInfo.rcWork.top - rect.top).max(0);
                rect.top += below;
                rect.bottom += below;
            }
        }
        SetWindowLongPtrW(hwnd, GWL_STYLE, style);
        let _ = SetWindowPos(hwnd, None, rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
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
