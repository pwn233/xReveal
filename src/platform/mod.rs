//! Platform-specific capture and video surfaces. Each OS exposes the same `VideoPipeline`.

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub use macos::VideoPipeline;
#[cfg(target_os = "windows")]
pub use windows::VideoPipeline;

/// Keeps the display from idling to sleep while it lives (#215). Display only, never the system.
#[cfg(target_os = "macos")]
pub use macos::awake::DisplayAwake;
#[cfg(target_os = "windows")]
pub use windows::awake::DisplayAwake;

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

/// Moves `window` to the middle of screen `display`, keeping its size. Call it outside a GPUI
/// update: on Windows the move happens at once, and GPUI can only take note of it (its observers,
/// the drawer and overlay windows following) when it isn't in the middle of one.
pub fn move_window(window: raw_window_handle::RawWindowHandle, display: u64) {
    #[cfg(target_os = "macos")]
    if let raw_window_handle::RawWindowHandle::AppKit(appkit) = window {
        // SAFETY: GPUI's raw window handle on macOS is a valid NSView for the window's lifetime.
        let view: &objc2_app_kit::NSView = unsafe { appkit.ns_view.cast().as_ref() };
        macos::screens::move_window(view, display);
    }
    #[cfg(target_os = "windows")]
    if let raw_window_handle::RawWindowHandle::Win32(win32) = window {
        windows::screens::move_window(::windows::Win32::Foundation::HWND(win32.hwnd.get() as *mut _), display);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = (window, display);
}

/// Borderless (docs/spec/v1.md §2.2): takes the title bar and border away
/// from `window`, keeping its content's size, and centres it on screen `display` (else the one it's
/// on); or gives them back. GPUI has no call for it. Call it outside a GPUI update.
pub fn set_borderless(window: raw_window_handle::RawWindowHandle, borderless: bool, display: Option<u64>) {
    #[cfg(target_os = "macos")]
    if let raw_window_handle::RawWindowHandle::AppKit(appkit) = window {
        // SAFETY: GPUI's raw window handle on macOS is a valid NSView for the window's lifetime.
        let view: &objc2_app_kit::NSView = unsafe { appkit.ns_view.cast().as_ref() };
        macos::screens::set_borderless(view, borderless, display);
    }
    #[cfg(target_os = "windows")]
    if let raw_window_handle::RawWindowHandle::Win32(win32) = window {
        windows::screens::set_borderless(::windows::Win32::Foundation::HWND(win32.hwnd.get() as *mut _), borderless, display);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = (window, borderless, display);
}

/// Frees the window-size drawables GPUI's layer piled up while the UI animated (#225); macOS
/// only, where the drawer and overlays are drawn in the main window. Call it once the UI is still.
pub fn trim_drawables(window: raw_window_handle::RawWindowHandle) {
    #[cfg(target_os = "macos")]
    if let raw_window_handle::RawWindowHandle::AppKit(appkit) = window {
        // SAFETY: GPUI's raw window handle on macOS is a valid NSView for the window's lifetime.
        let view: &objc2_app_kit::NSView = unsafe { appkit.ns_view.cast().as_ref() };
        macos::screens::trim_drawables(view);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// Enters or leaves macOS's own fullscreen for `window` (#56), a moment later from the main queue.
/// A borderless window gets its title bar back first: macOS's fullscreen needs a normal window.
#[cfg(target_os = "macos")]
pub fn toggle_fullscreen(window: raw_window_handle::RawWindowHandle) {
    if let raw_window_handle::RawWindowHandle::AppKit(appkit) = window {
        // SAFETY: GPUI's raw window handle on macOS is a valid NSView for the window's lifetime.
        let view: &objc2_app_kit::NSView = unsafe { appkit.ns_view.cast().as_ref() };
        macos::screens::toggle_fullscreen(view);
    }
}

/// Watches macOS's fullscreen slide for `window` (#203): `on_change(true)` when it starts,
/// `on_change(false)` when it has finished. Called from AppKit: send it on, don't update GPUI in it.
#[cfg(target_os = "macos")]
pub fn watch_fullscreen(window: raw_window_handle::RawWindowHandle, on_change: impl Fn(bool) + Send + Sync + 'static) {
    if let raw_window_handle::RawWindowHandle::AppKit(appkit) = window {
        // SAFETY: GPUI's raw window handle on macOS is a valid NSView for the window's lifetime.
        let view: &objc2_app_kit::NSView = unsafe { appkit.ns_view.cast().as_ref() };
        macos::screens::watch_fullscreen(view, on_change);
    }
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

/// Gives the keys back to `window` (Windows), from another window of xReveal's that has them.
/// Call it outside a GPUI update.
pub fn activate(window: raw_window_handle::RawWindowHandle) {
    #[cfg(target_os = "windows")]
    if let raw_window_handle::RawWindowHandle::Win32(win32) = window {
        windows::screens::activate(::windows::Win32::Foundation::HWND(win32.hwnd.get() as *mut _));
    }
    #[cfg(not(target_os = "windows"))]
    let _ = window;
}

/// Makes the drawer's window `popup` part of `owner`, so it stays just above it and never floats
/// over other apps, without the frame and animations Windows gives it (Windows).
pub fn own_popup(popup: &gpui_kit::Window, owner: raw_window_handle::RawWindowHandle) {
    #[cfg(target_os = "windows")]
    if let (Some(raw_window_handle::RawWindowHandle::Win32(top)), raw_window_handle::RawWindowHandle::Win32(below)) = (raw_window(popup), owner) {
        use ::windows::Win32::Foundation::HWND;
        windows::screens::own_popup(HWND(top.hwnd.get() as *mut _), HWND(below.hwnd.get() as *mut _));
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (popup, owner);
}

/// Puts pop-up `popup` exactly over `area` of `owner`'s content (GPUI's pixels from its top-left,
/// at `scale`), keeping its place in the stack and without focusing it (Windows). Call it outside
/// a GPUI update.
pub fn place_popup(popup: raw_window_handle::RawWindowHandle, owner: raw_window_handle::RawWindowHandle, area: gpui_kit::Bounds<gpui_kit::Pixels>, scale: f32) {
    #[cfg(target_os = "windows")]
    if let (raw_window_handle::RawWindowHandle::Win32(top), raw_window_handle::RawWindowHandle::Win32(below)) = (popup, owner) {
        use ::windows::Win32::Foundation::HWND;
        let edges = [area.left(), area.top(), area.right(), area.bottom()].map(|e| (e.as_f32() * scale).round() as i32);
        windows::screens::place_popup(HWND(top.hwnd.get() as *mut _), HWND(below.hwnd.get() as *mut _), edges);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (popup, owner, area, scale);
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


/// Whether the scroll event being handled moves the way the fingers did, so the Drawer swipe
/// (#141) can go by the fingers whatever the scrolling-direction setting.
/// - **macOS:** GPUI's delta follows the "natural scrolling" setting, and the native event says
///   whether that setting turned it around (`isDirectionInvertedFromDevice`). Read during the
///   event (GPUI handles `scrollWheel:` synchronously), so `currentEvent` is that event.
/// - **Windows:** assumed to follow the fingers. To be checked on a precision touchpad (#140).
pub fn scroll_follows_fingers() -> bool {
    #[cfg(target_os = "macos")]
    {
        let Some(mtm) = objc2::MainThreadMarker::new() else { return true };
        objc2_app_kit::NSApplication::sharedApplication(mtm).currentEvent().is_none_or(|event| event.isDirectionInvertedFromDevice())
    }
    #[cfg(not(target_os = "macos"))]
    true
}

/// Asks GitHub for xReveal's latest release (#145), blocking the calling background thread for up
/// to about 15 s. `None` when there was no usable answer.
pub fn latest_release() -> Option<crate::update::Answer> {
    #[cfg(target_os = "macos")]
    return macos::http::latest_release();
    #[cfg(target_os = "windows")]
    return windows::http::latest_release();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    None
}

/// A GET of `url` (status and body), for the update's checksums (#163).
pub fn http_get(url: &str) -> Option<(isize, Vec<u8>)> {
    #[cfg(target_os = "macos")]
    return macos::http::get(url, "text/plain");
    #[cfg(target_os = "windows")]
    return windows::http::get(url, "text/plain");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    None
}

/// Downloads `url` to `dest` with `progress` (#163). Blocking: run it on a background thread.
pub fn download(url: &str, dest: &std::path::Path, progress: &crate::update::Progress) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos::http::download(url, dest, progress);
    #[cfg(target_os = "windows")]
    return windows::http::download(url, dest, progress);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err("not on this OS".into())
}

/// Gets a verified download ready to install (#163): returns what [`install_update`] takes.
pub fn prepare_update(download: &std::path::Path) -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "macos")]
    return macos::install::prepare(download);
    #[cfg(target_os = "windows")]
    return windows::install::prepare(download);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err("not on this OS".into())
}

/// Installs it and starts the new version; xReveal quits right after (#163).
pub fn install_update(prepared: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos::install::install_and_relaunch(prepared);
    #[cfg(target_os = "windows")]
    return windows::install::install_and_relaunch(prepared);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err("not on this OS".into())
}

/// Removes what an update left behind (the previous version), at launch.
pub fn clean_up_update() {
    #[cfg(target_os = "macos")]
    macos::install::clean_up();
    #[cfg(target_os = "windows")]
    windows::install::clean_up();
}

/// Whether process `pid` is running and is xReveal (docs/spec/v1.md §2.11: two xReveals would
/// share the run marker).
pub fn process_is_xreveal(pid: u32) -> bool {
    #[cfg(target_os = "macos")]
    return macos::crash::process_is_xreveal(pid);
    #[cfg(target_os = "windows")]
    return windows::crash::process_is_xreveal(pid);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = pid;
        false
    }
}

/// The OS, its version and build, for the panic file and the pre-filled issue.
pub fn os_version() -> String {
    #[cfg(target_os = "macos")]
    return macos::crash::os_version();
    #[cfg(target_os = "windows")]
    return windows::crash::os_version();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    std::env::consts::OS.to_owned()
}

/// The OS's own report of xReveal's process `pid` crashing after `started` (Unix seconds), if
/// there is one. Blocking: call it off the UI thread.
pub fn find_crash_report(pid: u32, started: u64) -> Option<crate::crash::OsReport> {
    #[cfg(target_os = "macos")]
    return macos::crash::find_report(pid, started);
    #[cfg(target_os = "windows")]
    return windows::crash::find_report(pid, started);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (pid, started);
        None
    }
}
