//! Platform-specific capture and video surfaces. Each OS exposes the same `VideoPipeline`.

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub use macos::VideoPipeline;
#[cfg(target_os = "windows")]
pub use windows::VideoPipeline;

/// The window's native handle, for the few things GPUI can't do (moving it to another screen).
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn raw_window(window: &gpui_kit::Window) -> Option<raw_window_handle::RawWindowHandle> {
    use raw_window_handle::HasWindowHandle;
    HasWindowHandle::window_handle(window).ok().map(|h| h.as_raw())
}

/// The OS's name for screen `display` (a GPUI display ID).
pub fn screen_name(display: u64) -> Option<String> {
    #[cfg(target_os = "macos")]
    return macos::screens::names().into_iter().find(|(id, _)| *id == display).map(|(_, name)| name);
    #[cfg(target_os = "windows")]
    return windows::screens::name(display);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = display;
        None
    }
}

/// Moves `window` to the middle of screen `display`, keeping its size.
pub fn move_window(window: &gpui_kit::Window, display: u64) {
    #[cfg(target_os = "macos")]
    if let Some(raw_window_handle::RawWindowHandle::AppKit(appkit)) = raw_window(window) {
        // SAFETY: GPUI's raw window handle on macOS is a valid NSView for the window's lifetime.
        let view: &objc2_app_kit::NSView = unsafe { appkit.ns_view.cast().as_ref() };
        macos::screens::move_window(view, display);
    }
    #[cfg(target_os = "windows")]
    if let Some(raw_window_handle::RawWindowHandle::Win32(win32)) = raw_window(window) {
        windows::screens::move_window(::windows::Win32::Foundation::HWND(win32.hwnd.get() as *mut _), display);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = (window, display);
}
