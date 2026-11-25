//! xReveal's own video surface on macOS: a Metal layer in a native view placed *under* GPUI's
//! view, so GPUI draws only the UI on top (ADR 0001, docs/spec/v1.md §3.4).

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::{MainThreadMarker, MainThreadOnly};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindowOrderingMode};
use objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice, MTLPixelFormat};
use objc2_quartz_core::CAMetalLayer;

/// What the video thread draws into.
pub struct RenderTarget {
    pub layer: Retained<CAMetalLayer>,
    pub device: Retained<ProtocolObject<dyn MTLDevice>>,
}

// SAFETY: Metal devices are thread-safe, and CAMetalLayer is designed to be drawn into (nextDrawable,
// drawableSize) from a dedicated render thread. The view hierarchy is only touched on the main thread.
unsafe impl Send for RenderTarget {}

/// Adds a full-size view backed by a `CAMetalLayer` beneath `gpui_view` (GPUI's `NSView`) in the
/// window's content view. The GPUI window must use a transparent background to show it.
pub fn attach_below(gpui_view: NonNull<c_void>) -> Result<RenderTarget, String> {
    let mtm = MainThreadMarker::new().ok_or("the video surface must be created on the main thread")?;
    // SAFETY: GPUI's raw window handle on macOS is a valid NSView for the window's lifetime.
    let gpui_view: &NSView = unsafe { gpui_view.cast().as_ref() };
    let content = unsafe { gpui_view.superview() }.ok_or("GPUI's view has no parent view")?;
    let device = MTLCreateSystemDefaultDevice().ok_or("no Metal device")?;

    let layer = CAMetalLayer::new();
    layer.setDevice(Some(&device));
    layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
    layer.setFramebufferOnly(true);
    // Hand each frame to the window server as soon as it is drawn: the window server already
    // composites in step with the display, so the layer's own vsync only added a second wait
    // (~45 ms → ~8 ms arrival → screen, issue #54). Three drawables so drawing never waits for one
    // to come back. The picture doesn't tear: the window is composited (docs/spec/v1.md §3.3).
    layer.setDisplaySyncEnabled(false);
    layer.setMaximumDrawableCount(3);
    layer.setOpaque(true);
    if let Some(window) = gpui_view.window() {
        layer.setContentsScale(window.backingScaleFactor());
    }

    let view = NSView::initWithFrame(NSView::alloc(mtm), content.bounds());
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    // Layer-hosting: set the layer before asking for one, so AppKit uses ours.
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    content.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, Some(gpui_view));

    Ok(RenderTarget { layer, device })
}
