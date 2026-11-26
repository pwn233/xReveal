//! Screens on macOS: their names, and moving the window onto one (GPUI has no call for either).

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSScreen, NSView};
use objc2_foundation::{NSNumber, NSPoint, NSRect, NSString};

/// A screen's display ID (GPUI's `DisplayId` on macOS is the same `CGDirectDisplayID`).
fn display_id(screen: &NSScreen) -> Option<u64> {
    let description = screen.deviceDescription();
    let number = description.objectForKey(&NSString::from_str("NSScreenNumber"))?;
    let number = number.downcast::<NSNumber>().ok()?;
    Some(number.unsignedIntValue() as u64)
}

fn screens() -> Vec<Retained<NSScreen>> {
    MainThreadMarker::new().map(|mtm| NSScreen::screens(mtm).to_vec()).unwrap_or_default()
}

/// Each screen's display ID and the name macOS shows for it ("Built-in Retina Display").
pub fn names() -> Vec<(u64, String)> {
    screens().iter().filter_map(|s| Some((display_id(s)?, s.localizedName().to_string()))).collect()
}

/// Moves the window holding `view` to the middle of screen `display`, keeping its size (shrunk to
/// fit if the screen is smaller).
pub fn move_window(view: &NSView, display: u64) {
    let Some(window) = view.window() else { return };
    let Some(screen) = screens().into_iter().find(|s| display_id(s) == Some(display)) else { return };
    let area = screen.visibleFrame();
    let mut frame = window.frame();
    frame.size.width = frame.size.width.min(area.size.width);
    frame.size.height = frame.size.height.min(area.size.height);
    let origin = NSPoint::new(area.origin.x + (area.size.width - frame.size.width) / 2.0, area.origin.y + (area.size.height - frame.size.height) / 2.0);
    window.setFrame_display(NSRect::new(origin, frame.size), true);
}

/// Turns the window holding `overlay` into a layer over the one holding `owner`: a child window
/// (it moves with it and stays just above it, not over other apps) that lets the mouse through.
pub fn make_overlay(overlay: &NSView, owner: &NSView) {
    let (Some(overlay), Some(owner)) = (overlay.window(), owner.window()) else { return };
    overlay.setIgnoresMouseEvents(true);
    overlay.setLevel(objc2_app_kit::NSNormalWindowLevel);
    unsafe { owner.addChildWindow_ordered(&overlay, objc2_app_kit::NSWindowOrderingMode::Above) };
}
