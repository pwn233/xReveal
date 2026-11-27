# Glossary

xReveal is a lightweight desktop viewer for playing a console live through a capture card on a computer monitor.

## Terms

**Source**
The capture device the picture and sound come from, such as a USB or PCIe capture card with a PS5 plugged into it.
_Avoid_: input, camera, device (alone)

**Reference card**
The specific capture card (the user's Onten card) that xReveal is tested against. xReveal supports capture cards in general; the Reference card is just the one every claim is checked on.

**Video format**
The resolution, frame rate and pixel encoding that xReveal asks the Source to deliver.
_Avoid_: mode, quality

**Buffering**
A Source setting that holds back the newest frames to even out uneven delivery from the card: Off (always show the newest frame, the default), 1 frame or 2 frames. Each frame held back adds that much input lag.
_Avoid_: queue, latency mode

**Target ratio**
The shape xReveal fills on screen, such as 4:3, 16:9, 16:10, 21:9 or a custom ratio. It is independent of the Source's own 16:9 shape.
_Avoid_: aspect, scale, resolution

**Fill mode**
How the Source picture is fitted into the Target ratio. One of Fit, Stretch, Crop or Smart stretch.
_Avoid_: scaling mode, zoom

**Fit**
The Fill mode that keeps the picture's true shape and adds black bars where it doesn't reach the Target ratio.
_Avoid_: letterbox, pillarbox

**Stretch**
The Fill mode that distorts the picture evenly so it exactly fills the Target ratio.

**Crop**
The Fill mode that enlarges the picture until it fills the Target ratio, cutting off whatever falls outside.
_Avoid_: zoom

**Smart stretch**
The Fill mode that fills the Target ratio with a stretch that grows smoothly from the centre of the picture towards the edges, so the centre keeps closer to its true shape than with Stretch. When the Target ratio is narrower than the Source, the edges are squeezed instead.
_Avoid_: panorama, non-linear scale

**Smart stretch strength**
How far Smart stretch moves away from plain Stretch: 0% looks exactly like Stretch, 100% keeps the centre fully undistorted and puts all the stretching at the edges. Starts at 50%.

**Colour range**
Whether the Source's levels use the full scale (Full) or reserve the ends for headroom (Limited). Auto assumes Full. Each Source remembers its own choice.
_Avoid_: RGB range, levels, black level

**Colour space**
Which colour formula the Source's picture follows: BT.709 for HD, BT.601 for standard definition. Auto picks by picture size (720 lines and up is BT.709). Each Source remembers its own choice.
_Avoid_: colour matrix, colour profile

**Picture adjustments**
xReveal's own brightness, contrast and saturation (always shown), plus gamma and hue (in a collapsed "More" group), applied to the Source picture the same way on every card and OS.
_Avoid_: filters, colour correction, card settings

**Card settings**
The settings window supplied by the capture card's maker, opened from xReveal only where the operating system offers one.
_Avoid_: vendor dialog, properties

**Settings drawer**
The see-through panel that slides in over the live picture (it floats on top, never pushes or shrinks the picture) to change settings. Opens from the right by default; the side can be changed. Looks the same on every OS.
_Avoid_: sidebar, settings window, overlay menu

**Drawer swipe**
A sideways two-finger swipe on a trackpad (or Magic Mouse) that pulls the Settings drawer in or pushes it away: fingers moving away from the drawer's side open it, fingers moving toward it close it. On by default; it can be turned off.
_Avoid_: gesture, swipe (alone)

**Welcome screen**
The centred screen shown on the very first launch, when there is no Remembered setup yet, where the user picks a Source.
_Avoid_: onboarding, setup wizard

**Card-not-found picker**
The smaller picker shown at launch when the Remembered setup's Source isn't plugged in. It waits for the user's click.
_Avoid_: error screen, welcome screen

**No-signal screen**
The black screen with a centred message that xReveal shows when no frame has arrived from the Source for a second. It goes away by itself when frames return. A picture that is merely black (a loading screen, a card's own blank output) is shown as it is, not as a No-signal screen.
_Avoid_: error screen, blank screen

**Source busy**
When another app holds the Source so xReveal cannot open it. xReveal says so and keeps retrying until the other app lets go.
_Avoid_: device locked, in use error

**Card-unplugged screen**
The last picture, dimmed, with a "waiting for it" message, shown when the Source is unplugged during a session. When the same Source comes back, xReveal resumes by itself.
_Avoid_: disconnected error

**Stats overlay**
A small corner readout, toggled by a key, showing how long frames take from the Source to the screen, the real frame rate, dropped frames and the active Video format. Off by default.
_Avoid_: debug overlay, FPS counter

**Audio route**
Which audio input (usually the Source's own audio) xReveal plays, and which output device it plays to.

**A/V sync offset**
A delay, in milliseconds, applied to the sound or the picture so they line up. Positive delays the sound (free); negative delays the picture, which adds the same amount of input lag. Ranges from −500 to +500 ms.

**Remembered setup**
The single saved set of choices (Source, Video format, Target ratio, Fill mode and Smart stretch strength, Picture adjustments, Audio route, A/V sync offset and the monitor to go fullscreen on) that xReveal applies automatically on launch. There is one, not several named ones.
_Avoid_: profile, preset, scene

**What's new**
The notes for the version just installed, shown once on the first launch after xReveal is updated. They come with the app, so showing them needs no network.
_Avoid_: changelog popup, release notes (alone)

**Update available**
A newer xReveal release that xReveal has found by asking GitHub, at most once a day and only while **Check for updates** is on. The drawer then offers its notes and an **Update** button, which opens that release's download page.
_Avoid_: auto-update, new version (alone)

## Relationships

- A **Source** delivers frames in one **Video format** at a time.
- The **Target ratio** and **Fill mode** together decide how those frames appear on screen.
- The **Remembered setup** holds exactly one choice of each.

## Flagged ambiguities

- "Profile" was first used for a saved setup. Resolved as **Remembered setup**: there is one, and named profiles are out of scope.
- "Scale ratio" was used for the on-screen shape. Resolved as **Target ratio**. How the picture is fitted into it is a separate term: **Fill mode**.
