//! Screens on macOS: their names, moving the window onto one, and Borderless (GPUI has no call for
//! any of them).

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSScreen, NSView, NSWindowStyleMask};
use objc2_foundation::{NSNumber, NSPoint, NSRect, NSSize, NSString};

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
///
/// The move runs a moment later from the main queue, not straight away: macOS reports the new
/// frame and screen back to GPUI during `setFrame`, and doing that while GPUI is still handling
/// the click that asked for it left the app unresponsive (#82).
pub fn move_window(view: &NSView, display: u64) {
    let view = view as *const NSView as usize;
    dispatch2::DispatchQueue::main().exec_async(move || {
        // SAFETY: the main queue runs on the main thread, and GPUI's view lives as long as its
        // window, which outlives this moment.
        let view = unsafe { &*(view as *const NSView) };
        move_now(view, display);
    });
}

fn move_now(view: &NSView, display: u64) {
    let Some(window) = view.window() else { return };
    let Some(screen) = screens().into_iter().find(|s| display_id(s) == Some(display)) else { return };
    let area = screen.visibleFrame();
    let mut frame = window.frame();
    frame.size.width = frame.size.width.min(area.size.width);
    frame.size.height = frame.size.height.min(area.size.height);
    let origin = NSPoint::new(area.origin.x + (area.size.width - frame.size.width) / 2.0, area.origin.y + (area.size.height - frame.size.height) / 2.0);
    window.setFrame_display(NSRect::new(origin, frame.size), true);
}

/// The frame GPUI gives xReveal's window (`main.rs`): a title bar with close, minimise and zoom,
/// and resizable edges.
const WINDOWED: NSWindowStyleMask = NSWindowStyleMask::Titled
    .union(NSWindowStyleMask::Closable)
    .union(NSWindowStyleMask::Miniaturizable)
    .union(NSWindowStyleMask::Resizable);

/// Borderless (docs/spec/v1.md §2.2): takes the title bar and border away from the window holding
/// `view`, keeping its content's size, and centres it on screen `display` (else the one it's on),
/// shrunk to fit; or gives them back around the same content. A moment later from the main queue,
/// like [`move_window`].
pub fn set_borderless(view: &NSView, borderless: bool, display: Option<u64>) {
    let view = view as *const NSView as usize;
    dispatch2::DispatchQueue::main().exec_async(move || {
        // SAFETY: as in `move_window`.
        let view = unsafe { &*(view as *const NSView) };
        set_borderless_now(view, borderless, display);
    });
}

fn set_borderless_now(view: &NSView, borderless: bool, display: Option<u64>) {
    let Some(window) = view.window() else { return };
    let mask = window.styleMask();
    // Native fullscreen (the green button) is left alone; a window that has its frame keeps it. An
    // already borderless one is only centred again (Monitor).
    if mask.contains(NSWindowStyleMask::FullScreen) || (!borderless && mask.contains(NSWindowStyleMask::Titled)) {
        return;
    }
    let content = window.contentRectForFrameRect(window.frame());
    let on = display.and_then(|d| screens().into_iter().find(|s| display_id(s) == Some(d))).or_else(|| window.screen());
    let frame = if borderless {
        window.setStyleMask(NSWindowStyleMask::Borderless);
        // Centred on what the menu bar and Dock leave of the screen, which would cover the picture.
        match on.map(|s| s.visibleFrame()) {
            Some(area) => {
                let size = NSSize::new(content.size.width.min(area.size.width), content.size.height.min(area.size.height));
                NSRect::new(NSPoint::new(area.origin.x + (area.size.width - size.width) / 2.0, area.origin.y + (area.size.height - size.height) / 2.0), size)
            }
            None => content,
        }
    } else {
        window.setStyleMask(WINDOWED);
        // The title bar goes above the same content; kept below the menu bar so it can be dragged.
        let mut frame = window.frameRectForContentRect(content);
        if let Some(area) = on.map(|s| s.visibleFrame()) {
            let over = (frame.origin.y + frame.size.height) - (area.origin.y + area.size.height);
            if over > 0.0 {
                frame.origin.y -= over;
            }
        }
        frame
    };
    window.setFrame_display(frame, true);
    // A style mask change makes AppKit take the keys and first responder away from the window (as
    // GPUI's own fullscreen notes): give them back, or Tab, the arrows, F11 and Esc stop working.
    window.makeKeyAndOrderFront(None);
    window.makeFirstResponder(Some(view));
}

/// Enters or leaves macOS's own fullscreen for the window holding `view` (#56). A moment later from
/// the main queue, like [`move_window`]. A borderless window gets its frame back first, because
/// macOS's fullscreen needs a titled window; leaving goes back to that frame, and the app makes it
/// borderless again.
pub fn toggle_fullscreen(view: &NSView) {
    let view = view as *const NSView as usize;
    dispatch2::DispatchQueue::main().exec_async(move || {
        // SAFETY: as in `move_window`.
        let view = unsafe { &*(view as *const NSView) };
        let Some(window) = view.window() else { return };
        let mask = window.styleMask();
        if !mask.contains(NSWindowStyleMask::FullScreen) && !mask.contains(NSWindowStyleMask::Titled) {
            window.setStyleMask(WINDOWED);
        }
        window.setCollectionBehavior(window.collectionBehavior() | objc2_app_kit::NSWindowCollectionBehavior::FullScreenPrimary);
        window.toggleFullScreen(None);
    });
}

/// Watches macOS's own fullscreen slide for the window holding `view` (#203): `on_change(true)`
/// when macOS starts sliding it into or out of fullscreen, `on_change(false)` when the slide has
/// finished. Called on the main thread, from AppKit's notification: send it on rather than update
/// GPUI from it. Watches for as long as the app runs.
pub fn watch_fullscreen(view: &NSView, on_change: impl Fn(bool) + Send + Sync + 'static) {
    use objc2_app_kit::{NSWindowDidEnterFullScreenNotification, NSWindowDidExitFullScreenNotification, NSWindowWillEnterFullScreenNotification, NSWindowWillExitFullScreenNotification};
    use objc2_foundation::{NSNotification, NSNotificationCenter};

    let Some(window) = view.window() else { return };
    let on_change = std::sync::Arc::new(on_change);
    let center = NSNotificationCenter::defaultCenter();
    // SAFETY: AppKit's own constants, set before any app code runs.
    let names = unsafe {
        [
            (NSWindowWillEnterFullScreenNotification, true),
            (NSWindowWillExitFullScreenNotification, true),
            (NSWindowDidEnterFullScreenNotification, false),
            (NSWindowDidExitFullScreenNotification, false),
        ]
    };
    for (name, started) in names {
        let on_change = on_change.clone();
        let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| on_change(started));
        let object: &objc2::runtime::AnyObject = &window;
        // SAFETY: the block is Send + Sync (its closure is), and with no queue it runs on the
        // thread that posts the notification: AppKit's main thread.
        let observer = unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), Some(object), None, &block) };
        // The window lives as long as the app: so does the observer.
        std::mem::forget(observer);
    }
}

/// Turns the window holding `overlay` into a layer over the one holding `owner`: a child window
/// (it moves with it and stays just above it, not over other apps) that lets the mouse through.
pub fn make_overlay(overlay: &NSView, owner: &NSView) {
    let (Some(overlay), Some(owner)) = (overlay.window(), owner.window()) else { return };
    overlay.setIgnoresMouseEvents(true);
    overlay.setLevel(objc2_app_kit::NSNormalWindowLevel);
    unsafe { owner.addChildWindow_ordered(&overlay, objc2_app_kit::NSWindowOrderingMode::Above) };
}

/// Makes GPUI's `CAMetalLayer` (the layer of `view`) drop the drawables it no longer needs (#225),
/// a moment later from the main queue. While the drawer slides, GPUI draws every frame and its
/// layer's pool grows to five window-size surfaces (17 MB each in a 1280×887 pt window); once the
/// UI is still again it keeps them until the window is resized. Changing the drawable size frees
/// the pool, as a resize does, and setting it straight back means GPUI never draws at the wrong
/// size: both happen in one main-queue block, between two of its frames.
pub fn trim_drawables(view: &NSView) {
    let view = view as *const NSView as usize;
    dispatch2::DispatchQueue::main().exec_async(move || {
        // SAFETY: as in `move_window`.
        let view = unsafe { &*(view as *const NSView) };
        let Some(layer) = view.layer() else { return };
        let Ok(layer) = layer.downcast::<objc2_quartz_core::CAMetalLayer>() else { return };
        let size = layer.drawableSize();
        if size.height <= 2.0 {
            return;
        }
        layer.setDrawableSize(objc2_core_foundation::CGSize { width: size.width, height: size.height - 2.0 });
        layer.setDrawableSize(size);
    });
}
