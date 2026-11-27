# Two-finger horizontal trackpad swipe through GPUI (macOS and Windows)

Research for [#132](https://github.com/pwn233/xReveal/issues/132), feeding the grilling ticket #133.
Question: can xReveal see a two-finger horizontal swipe, with its direction and its start and end, on
macOS trackpads and Windows precision touchpads, from what GPUI Kit 0.11 already hands the app?

Versions read: `gpui-pre`, `gpui-pre-macos` and `gpui-pre-windows` 0.3.14, `gpui-component` 0.11.0
(GPUI Kit 0.11), `objc2-app-kit` 0.3.2, in `~/.cargo/registry/src/index.crates.io-*/`. GPUI paths
below are relative to the crate's `src/`. xReveal paths are relative to the repo root at `dd5d5fb`.

Labels: **[code]** means I read it in the source. **[docs]** means it comes from Apple or Microsoft
documentation (linked). **[inferred]** means my own reasoning from the two, not checked on a device.
Nothing here was run on hardware.

## Short answer

- **macOS: yes, through GPUI alone, with one gap.** Every two-finger trackpad scroll reaches GPUI as a
  `ScrollEvent` with pixel deltas and a phase: `Started`, then `Moved`, then `Ended` when the fingers
  lift. This happens anywhere in the window, including over the picture. The gap is direction. The
  deltas already include the user's "natural scrolling" setting, and GPUI drops
  `isDirectionInvertedFromDevice` and `momentumPhase`. To know which way the fingers physically moved,
  xReveal has to read the `NSEvent` itself. It can do that with a one-line
  `NSApp.currentEvent` read inside the GPUI handler, or with an `NSEvent` local monitor.
- **Windows: yes in GPUI's code, not yet proven over the picture.** GPUI does not use
  `WM_MOUSEHWHEEL` for precision touchpads. It runs its own **Direct Manipulation** viewport and turns
  touchpad pans into the same `ScrollEvent`: pixel deltas, `Started`, `Moved`…, and `Ended`. Two
  differences matter. `Ended` comes only after inertia has finished, not when the fingers lift. And
  whether the touchpad contact reaches GPUI's window at all depends on hit-testing through xReveal's
  video child window and the overlay pop-up. That has to be tested on a device.
- **Cost:** no per-frame work. One listener closure, registered when GPUI paints (GPUI paints only
  when its UI changes, not for each video frame). A few comparisons per scroll event. On macOS, at
  most two Objective-C property reads per scroll event.

## 1. What GPUI's scroll event carries

`ScrollEvent` (`ScrollWheelEvent` is a deprecated alias), `gpui-pre/interactive.rs:522–543` **[code]**:

| Field | Meaning |
|---|---|
| `position` | Pointer position in the window, in logical px |
| `delta: ScrollDelta` | `Pixels(Point)` for precise devices, `Lines(Point)` for wheel notches (`:575–580`). `delta.precise()` is true for `Pixels` (`:627`) |
| `modifiers` | Keys held down |
| `touch_phase: TouchPhase` | `Started`, `Moved`, `Ended` or `Cancelled` (`:88–100`) |
| `source: ScrollSource` | `Pointer` for mouse or trackpad, `Touch` for GPUI's own touchscreen pans (`:546–557`). It is always `Pointer` on both desktops (see below) |

There is **no momentum field** and **no "direction inverted" field**. GPUI has no swipe event for
trackpads. The only "swipe" GPUI handles is AppKit's `NSEventTypeSwipe`, which it turns into a
Back/Forward mouse button (section 2).

### Delivery: anywhere in the window, over the picture too

- GPUI hands every mouse-type event to **all** listeners registered with `Window::on_mouse_event`,
  first in a capture pass and then in a bubble pass (`gpui-pre/window.rs:6457–6490`, capture loop at
  `:6474`). A listener that ignores hitboxes therefore sees every scroll in that GPUI window, wherever
  the pointer is. **[code]**
- `on_scroll` on an element is narrower. It runs only in the bubble pass, and only when that element's
  hitbox is under the pointer (`elements/div.rs:390–398`, `window.rs:834`). Some GPUI Kit components
  stop scroll propagation in the bubble pass: the dialog (`gpui-component/dialog/dialog.rs:905`) and the
  context menu (`menu/context_menu.rs:178`). A root-level `on_scroll` would miss scrolls over those. A
  **capture-phase** `window.on_mouse_event::<ScrollEvent>` listener runs before any of them and misses
  nothing. **[code]**
- GPUI's own scroll containers do **not** stop propagation (`elements/div.rs:3448–3492`), so a swipe
  over the drawer's list is seen by both the list and the window listener. **[code]**
- **Gotcha [code]:** a vertical-only scroll container turns a purely horizontal delta into vertical
  scrolling unless it has `restrict_scroll_to_axis` set (`elements/div.rs:3473–3478`; the style method
  is at `:1590`). xReveal's drawer pane is `.overflow_y_scroll()` without it (`src/drawer.rs:1247`).
  So a horizontal swipe over the drawer would also move its list up or down. #133 should decide
  whether that matters.
- macOS: xReveal's video is an `NSView` added **below** GPUI's view in the same content view
  (`src/platform/macos/surface.rs:26–59`). GPUI's view covers the whole window. Its `hitTest:` gives
  the event away only to native views that GPUI registered itself (`gpui-pre-macos/window/native_views.rs:809–825`),
  and xReveal's video view is not one of those. So scrolls over the picture land on GPUI's view.
  **[code]** On macOS the drawer and the overlays are drawn in the main window
  (`src/app.rs:189–191`: separate windows only on Windows, or behind an environment variable). **[code]**
- Windows: the arrangement is different, and the ticket's assumption ("under GPUI's view") is wrong
  there. The video is a `WS_CHILD` window **over** GPUI's window. It answers `WM_NCHITTEST` with
  `HTTRANSPARENT` (`src/platform/windows/surface.rs:24–90`). The drawer is a **separate focusable
  GPUI pop-up window** (`src/app.rs:635–680`). The overlays are another GPUI pop-up that also passes
  the mouse through with `HTTRANSPARENT` (`src/app.rs:682–715`, `src/platform/windows/screens.rs:216–240`).
  Each GPUI window has its **own** Direct Manipulation viewport (`gpui-pre-windows/window.rs:159`).
  So on Windows the listener has to be in each of the main, drawer and overlay windows, not only the
  main one. **[code]**

## 2. macOS

### How AppKit's event becomes GPUI's event [code]

GPUI's `GPUIView` class sends `scrollWheel:`, `magnifyWithEvent:` and `swipeWithEvent:` to
`handle_view_event` (`gpui-pre-macos/window.rs:166–188`). That function converts the event and calls
the window's event callback **synchronously**, still inside AppKit's `scrollWheel:` (`window.rs:3051`,
callback at `:3204`). The conversion is `platform_input_from_native`, `gpui-pre-macos/events.rs:269–296`:

- Phase: `MayBegin` and `Began` become `Started`. `Ended` becomes `Ended`. **Everything else**
  becomes `Moved`, including `Changed`, `Stationary`, **`Cancelled`** and **`None`** (`:270–274`).
- `delta` is `scrollingDeltaX/Y`, as `Pixels` when `hasPreciseScrollingDeltas`, otherwise as `Lines`
  (`:276–285`).
- `momentumPhase` is never read. `isDirectionInvertedFromDevice` is never read.
- `NSEventTypeSwipe` (`:224–247`) is used only for the `Ended` phase, and becomes a
  `MouseDown` of `MouseButton::Navigate(Back|Forward)`. The comment there is about Logitech mice.

What Apple documents about these phases ([NSEvent.phase](https://developer.apple.com/documentation/appkit/nsevent/phase-swift.property),
[momentumPhase](https://developer.apple.com/documentation/appkit/nsevent/momentumphase)) **[docs]**:

- Two fingers on a trackpad give `mayBegin`, then `began`, and the gesture ends with `ended` or
  `cancelled`. A Magic Mouse does not send `mayBegin`.
- **Momentum events, and legacy wheel events, have `phase == None`.** Momentum is reported in
  `momentumPhase`. "All the gesture events are sent to the view under the cursor when the `began`
  occurred."

What that means for xReveal through GPUI alone **[code + docs → inferred]**:

- A gesture arrives as `Started` (sometimes twice: `mayBegin`, then `began`), then `Moved`…, then
  `Ended` **when the fingers lift**.
- **Momentum** after the lift arrives as more `Moved` events with no `Started` before them. A detector
  that only counts deltas between a `Started` and the next `Ended` ignores momentum without needing
  `momentumPhase`.
- A two-finger touch that never moves ends with `cancelled`, which reaches xReveal as a zero `Moved`
  and **no `Ended`**. The next `Started` has to reset the detector.
- A mouse wheel gives `Lines` and `Moved` with no `Started`. A Magic Mouse gives `Pixels` with phases,
  so through GPUI it looks like a trackpad. GPUI cannot tell them apart.
- **Direction:** Apple says `scrollingDeltaX/Y` "are automatically inverted … according to the user's
  preferences", and that `isDirectionInvertedFromDevice` is how to undo that
  ([docs](https://developer.apple.com/documentation/appkit/nsevent/isdirectioninvertedfromdevice)).
  GPUI passes the inverted delta and drops the flag. Natural scrolling is on by default, and then the
  fingers moving right give a positive `delta.x` **[inferred]**. A user with natural scrolling off gets
  the opposite sign. So "swipe toward the drawer" in terms of finger movement **cannot be decided from
  GPUI's event alone** on macOS.

### What xReveal would read natively, if it wants the finger direction or real momentum flags

xReveal already turns GPUI's raw handle into an `NSView` and its `NSWindow`
(`src/platform/mod.rs:136`, `src/platform/macos/screens.rs`). There are two cheap ways to see the
`NSEvent` itself:

1. **Inside the GPUI listener, read `NSApplication::sharedApplication(mtm).currentEvent()`.** GPUI's
   callback runs synchronously inside `scrollWheel:` **[code]**, and `currentEvent` is "the last event
   object that the app retrieved from the event queue"
   ([docs](https://developer.apple.com/documentation/appkit/nsapplication/currentevent)). That should
   be this same scroll event **[inferred]**. Read `isDirectionInvertedFromDevice()`, `momentumPhase()`
   and the raw `phase()` (this also tells `Cancelled` apart from `Changed`). Everything is in
   `objc2-app-kit` 0.3, which xReveal already depends on (`NSEvent.rs:824–842`, `NSApplication.rs:903`).
   **[code]**
2. **An `NSEvent` local monitor:** `addLocalMonitorForEventsMatchingMask_handler(NSEventMask::ScrollWheel, block)`.
   It sees every scroll event "prior to … dispatch" through `sendEvent:`, and it can let the event
   through unchanged ([docs](https://developer.apple.com/documentation/appkit/nsevent/addlocalmonitorforevents(matching:handler:))).
   The method needs the `block2` feature, which is in `objc2-app-kit`'s default features. xReveal
   already depends on `block2` (`Cargo.toml`). **[code]** This works independently of GPUI but needs
   its own handoff into GPUI state. Option 1 is simpler.

A cheaper workaround that needs no per-event native call **[inferred]**: read the
natural-scrolling preference once (or decide that the swipe follows the content's scroll direction,
like a page swipe in Safari, and not the fingers). That is a product question for #133.

### Clashes with macOS's own gestures [docs]

From [Apple's gesture list](https://support.apple.com/en-us/102482):

- **Swipe between pages: two fingers left or right.** The app implements this, not the system: an
  app calls `trackSwipeEvent` from its scroll handling, or `NSScrollView` does it, when
  `isSwipeTrackingFromScrollEventsEnabled` is true
  ([docs](https://developer.apple.com/documentation/appkit/nsevent/isswipetrackingfromscrolleventsenabled)).
  GPUI does neither (no `trackSwipeEvent` or `wantsScrollEventsForSwipeTracking` anywhere in
  `gpui-pre-macos`) **[code]**. So in xReveal the system takes nothing; the two-finger swipe comes in
  as plain scroll events. Users do expect "back and forward" from this gesture, but nothing else in
  xReveal uses it. If the user sets page swipes to **three** fingers, three-finger swipes arrive as
  `swipeWithEvent:`, which GPUI turns into Navigate Back/Forward mouse-downs (`events.rs:224–247`)
  **[code]**. That is harmless unless xReveal binds those buttons.
- **Swipe between full-screen apps / desktops:** three or four fingers. No clash with two fingers.
  This matters because xReveal's Fullscreen is macOS's own fullscreen Space (#123).
- **Notification Center: "Swipe left from the right edge with two fingers."** The system takes this
  gesture. With the drawer on the right, a swipe that starts at the trackpad's right edge and moves
  left (the closing direction, in finger terms) opens Notification Center and never reaches xReveal
  **[docs → inferred]**. A swipe that starts away from the edge is unaffected.

## 3. Windows

### Precision touchpads go through Direct Manipulation, not `WM_MOUSEHWHEEL` [code]

- Each GPUI window creates a Direct Manipulation viewport on its `HWND` (`gpui-pre-windows/window.rs:159`,
  `direct_manipulation.rs:31–90`). The configuration is: translation in X and Y, **inertia**,
  **rails in X and Y** (a gesture locks to one axis), and scaling (`:41–48`).
- When Windows sends `DM_POINTERHITTEST` (`events.rs:158`), GPUI checks that the pointer is a
  `PT_TOUCHPAD` and calls `viewport.SetContact(pointer_id)` (`direct_manipulation.rs:97–105`).
  Touchscreens are left alone.
- GPUI turns content-transform updates into `ScrollEvent { delta: Pixels(dx, dy), source: Pointer }`.
  The first update of a gesture is `Started` and the following ones are `Moved`
  (`direct_manipulation.rs:262–345`, phase at `:317`/`:324`). Deltas are divided by the scale factor,
  so they are logical px.
- **`Ended` comes when the viewport returns to `READY`** (`:223–252`), which is **after inertia has
  run out**, not when the fingers lift. If a new gesture interrupts inertia, GPUI sends `Ended` and
  then the new `Started` (`:218–221`). Inertia steps look like ordinary `Moved` events. GPUI keeps the
  `INERTIA` status to itself, and xReveal cannot reach GPUI's viewport handler.
- When the update is delivered: GPUI polls Direct Manipulation inside its draw handler
  (`events.rs:1378–1388`), and that runs every vsync because GPUI's vsync thread invalidates every
  window (`platform.rs:466–480`). So the events come at most one refresh late, and they cost xReveal
  no polling of its own.
- `WM_MOUSEWHEEL` and `WM_MOUSEHWHEEL` (`events.rs:147–148`, `552–641`) always become `Lines` deltas
  with `TouchPhase::Moved`. They have no phases, and fractional "fine" deltas are just fractional
  lines. On Windows, then, **`delta.precise()` (i.e. `Pixels`) means "came through Direct
  Manipulation"**, which is a clean way to accept only precision touchpads.

Microsoft's documentation is thin here. [`DM_POINTERHITTEST`](https://learn.microsoft.com/en-us/windows/win32/inputmsg/dm-pointerhittest)
is "sent to a window, when pointer input is first detected, in order to determine the most probable
input target for Direct Manipulation." It does not say how the target window is chosen.
[`WM_MOUSEHWHEEL`](https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-mousehwheel) goes to the
active window, `DefWindowProc` passes it up to the parent, and deltas smaller than 120 are allowed for
finer devices. **[docs]** Chromium's notes say Direct Manipulation "takes over the following
scrolling" once `SetContact` is called on `DM_POINTERHITTEST`, and that without a Direct Manipulation
consumer Windows falls back to precise `WM_MOUSEWHEEL` messages
([direct_manipulation_helper_win.h](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/content/browser/renderer_host/direct_manipulation_helper_win.h)).
That is a first-party consumer, but not Microsoft's own documentation.

### Open risk: does the touchpad contact reach GPUI's window over the picture? [inferred, test on device]

The pointer over the picture is over xReveal's **video child window**, and on top of that the
**overlay pop-up**. Both answer `WM_NCHITTEST` with `HTTRANSPARENT`, and clicks already pass through
them to GPUI (the code comments say so). I found no documentation on whether the
`DM_POINTERHITTEST` target is chosen by the same hit-test. The possible outcomes:

- **It honours `HTTRANSPARENT`:** the main window gets the contact and everything above holds.
- **It goes to the video child:** that window's procedure is `DefWindowProc`, so no `SetContact`
  happens. Windows then falls back to precise wheel messages, which `DefWindowProc` passes up to GPUI,
  and xReveal gets `Lines`/`Moved` events with **no `Started` or `Ended`** over the picture. The fix
  would be small: in `surface_proc`, forward `DM_POINTERHITTEST` to the parent so that GPUI's handler
  calls `SetContact`. Whether a parent's viewport accepts a contact that was hit-tested to its child is
  itself unverified.
- **It goes to the overlay pop-up:** that is a GPUI window with its own viewport, so it gets the
  `ScrollEvent`s. A listener in the overlay window covers this case.

The one-hour device test: log `touch_phase` and `delta` from a capture listener in all three GPUI
windows, and swipe over the picture, over the drawer and over the edge handle.

### Clashes with Windows' own gestures [docs]

Microsoft's [touchpad interactions](https://learn.microsoft.com/en-us/windows/apps/design/input/touchpad-interactions)
page lists the system gestures: three-finger tap and slide (Task View, desktops, app switching) and
"two finger slide to pan". No system gesture uses a two-finger horizontal slide. Edge swipes belong to
touchscreens, not touchpads. **[docs]**

Direction on Windows: GPUI passes the change in Direct Manipulation's content offset. I did not find
whether Windows' "scrolling direction" touchpad setting changes the sign that Direct Manipulation
reports. **[unverified; check on device alongside the hit-test]**

## 4. Cost

- **Per video frame: nothing.** The picture is native (Metal or D3D) and GPUI does not redraw for it.
  **[code, ADR 0001]**
- **Per GPUI paint:** a `window.on_mouse_event` listener is re-registered on every paint (one boxed
  closure pushed onto a list, `window.rs:5948–5962`). That is the same cost as any `on_click`. GPUI
  paints only when its UI changes or animates. **[code]**
- **Per scroll event:** a phase check, an accumulator, and a threshold comparison. On macOS, reading
  `currentEvent` and two properties is a few `objc_msgSend`s. Scroll events arrive at the trackpad's
  report rate on macOS, and at most once per vsync per window on Windows (polled in `draw_window`).
  **[code + inferred]**
- **When the gesture fires:** the existing drawer open or close path (`ToggleDrawer`, with its slide
  animation), the same as pressing Tab.
- Nothing has to be added to the Windows message loop unless the hit-test risk above turns out to need
  the `DM_POINTERHITTEST` forward. That forward would run once per touch-down, not per event.

## 5. What changes for #133

1. **Momentum and "fires once" behave differently on each OS.** On macOS the gesture ends at finger
   lift and momentum comes afterwards, outside the gesture. On Windows, `Ended` waits until inertia
   has finished. Firing once when the threshold is crossed and then ignoring everything until the next
   `Started` works the same on both. A rule that fires on `Ended` would feel late on Windows.
2. **The meaning of "direction" has to be chosen.** Is it the fingers' physical direction (on macOS
   this needs a native read of `isDirectionInvertedFromDevice`) or the content's scroll direction (what
   GPUI gives as is, and how Safari's page swipe behaves)?
3. **Swipes over the drawer:** its list (`src/drawer.rs:1247`) turns horizontal deltas into vertical
   scrolling, and on Windows the drawer is its own window. Decide whether a swipe over the drawer
   closes it, scrolls it, or both. Setting `restrict_scroll_to_axis` on the pane separates the two.
4. **Notification Center** takes two-finger leftward swipes that start at the trackpad's right edge
   on macOS. This affects the right-side drawer's closing swipe if it starts at the edge.
5. **Which devices count:** on Windows only precision touchpads (`Pixels`); on macOS trackpads **and**
   the Magic Mouse (GPUI cannot tell them apart). Mouse wheels and tilt wheels (`Lines`) can be
   excluded on both.
6. A **device check on Windows** (section 3) should come before building the feature, or be the first
   step of it.
