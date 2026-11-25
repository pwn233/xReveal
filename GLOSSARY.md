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

**Picture adjustments**
xReveal's own brightness, contrast and saturation (always shown), plus gamma and hue (in a collapsed "More" group), applied to the Source picture the same way on every card and OS.
_Avoid_: filters, colour correction, card settings

**Card settings**
The settings window supplied by the capture card's maker, opened from xReveal only where the operating system offers one.
_Avoid_: vendor dialog, properties

**Audio route**
Which audio input (usually the Source's own audio) xReveal plays, and which output device it plays to.

**A/V sync offset**
A delay, in milliseconds, applied to the sound or the picture so they line up.

**Remembered setup**
The single saved set of choices (Source, Video format, Target ratio, Fill mode and Smart stretch strength, Picture adjustments, Audio route, A/V sync offset and the monitor to go fullscreen on) that xReveal applies automatically on launch. There is one, not several named ones.
_Avoid_: profile, preset, scene

## Relationships

- A **Source** delivers frames in one **Video format** at a time.
- The **Target ratio** and **Fill mode** together decide how those frames appear on screen.
- The **Remembered setup** holds exactly one choice of each.

## Flagged ambiguities

- "Profile" was first used for a saved setup. Resolved as **Remembered setup**: there is one, and named profiles are out of scope.
- "Scale ratio" was used for the on-screen shape. Resolved as **Target ratio**. How the picture is fitted into it is a separate term: **Fill mode**.
