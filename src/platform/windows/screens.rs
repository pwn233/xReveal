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
        let _ = SetWindowPos(hwnd, None, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}
