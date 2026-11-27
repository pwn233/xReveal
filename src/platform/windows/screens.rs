//! Monitors on Windows: their names, and moving the window onto one (GPUI has no call for either).

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
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

fn text(wide: &[u16]) -> String {
    let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..len])
}

/// A monitor's name as Windows Settings shows it ("DELL U2720Q"), or `None`. That's the monitor's
/// own (EDID) name from the display configuration; the driver's name, which is often just
/// "Generic PnP Monitor", is the fallback.
pub fn name(monitor: u64) -> Option<String> {
    let info = info(monitor)?;
    let gdi_name = text(&info.szDevice);
    if let Some(name) = friendly_name(&gdi_name).filter(|n| !n.is_empty()) {
        return Some(name);
    }
    let mut device = DISPLAY_DEVICEW { cb: size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
    unsafe { EnumDisplayDevicesW(PCWSTR(info.szDevice.as_ptr()), 0, &mut device, 0) }.as_bool().then(|| text(&device.DeviceString))
}

/// The EDID name of the monitor showing GDI device `gdi_name` (`\\.\DISPLAY1`).
fn friendly_name(gdi_name: &str) -> Option<String> {
    let (mut paths, mut modes) = (0u32, 0u32);
    unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths, &mut modes) }.ok().ok()?;
    let mut path_list = vec![DISPLAYCONFIG_PATH_INFO::default(); paths as usize];
    let mut mode_list = vec![DISPLAYCONFIG_MODE_INFO::default(); modes as usize];
    unsafe { QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut paths, path_list.as_mut_ptr(), &mut modes, mode_list.as_mut_ptr(), None) }.ok().ok()?;
    path_list.truncate(paths as usize);
    path_list.iter().find_map(|path| {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                adapterId: path.sourceInfo.adapterId,
                id: path.sourceInfo.id,
            },
            ..Default::default()
        };
        if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != 0 || !text(&source.viewGdiDeviceName).eq_ignore_ascii_case(gdi_name) {
            return None;
        }
        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                adapterId: path.targetInfo.adapterId,
                id: path.targetInfo.id,
            },
            ..Default::default()
        };
        (unsafe { DisplayConfigGetDeviceInfo(&mut target.header) } == 0).then(|| text(&target.monitorFriendlyDeviceName))
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
/// `owner` keeps the look and feel of focus while the pop-up covers it ([`follow_focus`]).
pub fn own_popup(popup: HWND, owner: HWND) {
    use windows::Win32::Graphics::Dwm::{
        DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY, DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
        DwmSetWindowAttribute,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GWLP_HWNDPARENT, GetWindowLongPtrW, HWND_NOTOPMOST, SWP_NOMOVE, SWP_NOSIZE, SetWindowLongPtrW, WS_EX_TOPMOST};
    follow_focus(owner);
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

/// Focus for xReveal's main window `owner`, whose content its pop-ups cover:
/// - Its title bar stays lit while another window of xReveal's (the drawer) has the keys, the way
///   it does for a menu or a drop-down list. Windows would otherwise draw it as if xReveal had lost
///   focus whenever the drawer opens. It still dims when another app is chosen.
/// - A click on the picture brings xReveal to the front. The click reaches `owner` through the
///   overlay window, which never takes focus ([`make_overlay`]), so on its own it wouldn't.
pub fn follow_focus(owner: HWND) {
    unsafe {
        let _ = windows::Win32::UI::Shell::SetWindowSubclass(owner, Some(main_window_focus), 2, 0);
    }
}

unsafe extern "system" fn main_window_focus(
    hwnd: HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Shell::DefSubclassProc;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow, WM_ACTIVATEAPP, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_NCACTIVATE, WM_RBUTTONDOWN,
    };
    let ours = |window: HWND| unsafe { GetWindowThreadProcessId(window, None) == GetCurrentThreadId() };
    match message {
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            if !ours(unsafe { GetForegroundWindow() }) {
                let _ = unsafe { SetForegroundWindow(hwnd) };
            }
            unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
        }
        // Losing the keys to a window of xReveal's own (`lparam`, still unowned while it opens):
        // draw the title bar as active anyway.
        WM_NCACTIVATE if wparam.0 == 0 && lparam.0 != 0 => {
            let active = ours(HWND(lparam.0 as *mut _));
            unsafe { DefSubclassProc(hwnd, message, WPARAM(active as usize), lparam) }
        }
        // xReveal as a whole gains or loses focus, maybe while the drawer has the keys, when no
        // WM_NCACTIVATE reaches this window: light or dim its title bar to match.
        WM_ACTIVATEAPP => {
            unsafe { DefSubclassProc(hwnd, WM_NCACTIVATE, WPARAM((wparam.0 != 0) as usize), LPARAM(0)) };
            unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}
