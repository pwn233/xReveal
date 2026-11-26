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

/// Makes `overlay` a click-through layer over `owner`, for the overlays that would otherwise sit
/// under the video surface (Windows) — see `app::OverlayWindow`.
pub fn make_overlay(overlay: &gpui_kit::Window, owner: raw_window_handle::RawWindowHandle) {
    #[cfg(target_os = "macos")]
    if let (Some(raw_window_handle::RawWindowHandle::AppKit(top)), raw_window_handle::RawWindowHandle::AppKit(below)) = (raw_window(overlay), owner) {
        // SAFETY: GPUI's raw window handles on macOS are valid NSViews for the windows' lifetimes.
        let (top, below): (&objc2_app_kit::NSView, &objc2_app_kit::NSView) = unsafe { (top.ns_view.cast().as_ref(), below.ns_view.cast().as_ref()) };
        macos::screens::make_overlay(top, below);
    }
    #[cfg(target_os = "windows")]
    if let (Some(raw_window_handle::RawWindowHandle::Win32(top)), raw_window_handle::RawWindowHandle::Win32(below)) = (raw_window(overlay), owner) {
        use ::windows::Win32::Foundation::HWND;
        windows::screens::make_overlay(HWND(top.hwnd.get() as *mut _), HWND(below.hwnd.get() as *mut _));
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = (overlay, owner);
}

/// The native handle of `window` (for [`make_overlay`]).
pub fn window_handle(window: &gpui_kit::Window) -> Option<raw_window_handle::RawWindowHandle> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    return raw_window(window);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = window;
        None
    }
}
