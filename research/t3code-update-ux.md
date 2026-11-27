# How does T3 Code show its version, release notes and update offer?

Research for [#137](https://github.com/pwn233/xReveal/issues/137) (map: [#131](https://github.com/pwn233/xReveal/issues/131)).
Researched 2026-11-27. Feeds [#138 Update available flow](https://github.com/pwn233/xReveal/issues/138) and
[#135 What's new look](https://github.com/pwn233/xReveal/issues/135).

**Source.** T3 Code is the open-source desktop app at [pingdotgg/t3code](https://github.com/pingdotgg/t3code)
(Electron shell in `apps/desktop`, React UI in `apps/web`). Read at commit
[`e3cd83a`](https://github.com/pingdotgg/t3code/tree/e3cd83a299d02ddb1c19fd6baf6e79b395a0bfaf) (2026-11-27),
desktop version 0.1.13 (`apps/desktop/package.json:3`). Every `path:line` below is at that commit. Release bodies
were read from the repo's GitHub Releases (latest stable: v0.1.13, 2026-11-26). The app was not run; the UI is
described from source.

Labels: **[source]** read in T3 Code's code or release data. **[inference]** my own reasoning.

## Short answer

- **Version:** only in **Settings › About**: a "Version" row with the number as small muted monospace text that
  copies itself when clicked. The row's button checks, downloads or installs. The sidebar never shows the number.
  [source]
- **Update signal:** a quiet, round, icon-only button at the right end of the **sidebar footer** row (after
  Settings, Pull requests, Issues and Usage). Idle it is a muted refresh glyph. When an update is found it becomes
  a download glyph with a small dot, on a filled circle. There is **no toast, dialog or badge elsewhere** when an
  update is found. [source]
- **What the button does:** electron-updater 6.8.9 over GitHub Releases, with **auto-download and
  install-on-quit both off**. First press downloads, with a progress ring on the button and a "Update downloaded"
  toast. The next press asks "Install update X and restart T3 Code?" and then quits and installs. It checks 15 s
  after launch and every 4 h after that. [source]
- **Release notes:** shown **before** updating, in a popover that opens when the pointer **rests on** the update
  button or keyboard focus lands on it (pressing it does the action instead). The popover has a title, "0.1.13 · 3h
  ago", bullets with a bold lead phrase, a link to the GitHub release, and up to 6 skipped releases. The text is the
  GitHub release body, parsed in the app. [source]
- **After updating:** **no "What's new"**. There is at most a one-line toast on the next launch, and only after an
  "update everything" run. If an install didn't take, there is an error toast "Update did not apply". [source]
- **Dismiss / skip:** none. No "skip this version" and no "remind me later" for the offer. The dot stays until the
  update is installed. "Later" exists only in the macOS menu's dialogs and on a restart countdown. [source]
- **For xReveal:** take the quiet in-place signal, the version-and-age line, the short bold-lead bullet notes with
  a link to the release, and grouping several skipped releases. Leave out the icon-only button, the
  download/progress/restart states, popovers that open on hover, the 4-hourly polling and channels. T3 Code gives
  **no reference for What's new** (#135). [inference]

## 1. Where the version is shown

- **Settings › About › Version row.** The title is the word "Version" followed by the number. The number is
  monospace, extra-small and muted, and it is an inline button that copies the number, with a "Copy version" tooltip
  (`apps/web/src/components/settings/AboutVersionTitle.tsx:28-51`). [source]
- The same row carries the update state ([source]):
  - Description (`apps/web/src/components/settings/SettingsPanels.tsx:336-340`):
    - "Current version of the application." normally.
    - "Update available." when there is one.
    - The reason updates are off, when they are off.
  - Control: one small outline button labelled **Check for Updates**, **Checking…**, **Up to Date**, **Download**,
    **Downloading…** or **Install**. Its tooltip holds the longer status (`SettingsPanels.tsx:320-366`).
- **macOS app menu:** "About T3 Code" and **Check for Updates...**
  (`apps/desktop/src/window/DesktopApplicationMenu.ts:240-249`). The menu item answers with native message boxes
  (`DesktopApplicationMenu.ts:83-155`) [source]:
  - "You're up to date! T3 Code X is currently the newest version available."
  - "Update available, T3 Code X is available. You're running T3 Code Y." with **Download** / **Later**.
  - "Update ready … T3 Code will restart to install it. Any running tasks will be interrupted." with **Restart** /
    **Later**.
- The sidebar never shows the version number, not even next to the update button. [source]

## 2. How an available update is signalled

- **Placement:** `SidebarUpdatePill` is the last item of the sidebar footer's utility row, after Settings, Pull
  requests, Issues and Usage. It is pushed to the right end (`apps/web/src/components/sidebar/SidebarChrome.tsx:283-310`,
  `SidebarUpdatePill.tsx:525`). [source]
- **Shape:** a 32 px round, icon-only button (`SidebarUpdatePill.tsx:483`). It has no text label. The wording is in
  the tooltip and the accessible label. [source]
- **States** (`apps/web/src/components/sidebar/DesktopUpdateStatusIcon.tsx:8-14, 103-127`;
  `SidebarUpdatePill.tsx:83-118, 485-494`) [source]:

  | State | Look | Tooltip / popover title |
  |---|---|---|
  | Idle | Muted refresh glyph, no fill | "Check for updates" |
  | Checking | The same glyph spins. The spin is latched so it completes a turn, and it is off with reduced motion. | "Checking for updates…" |
  | Available | Download glyph with a tiny solid dot at its top-right corner, on a filled circle | "Update 0.1.14 ready to download" |
  | Downloading | Download glyph inside a thin ring that fills with the percent | "Downloading update (42%)" |
  | Downloaded | Restart glyph with a small check badge | "Update X downloaded. Click to restart and install." |
  | Updates off (policy) | Idle glyph, dimmed | "Updates unavailable", with the reason |

- **When a check finds an update, nothing pops up.** No toast, dialog or system notification. The only change is
  the button's glyph and fill. Searching the UI code turns up no toast for the "available" status. [source]
- **When a check fails:**
  - A background check that fails is silent. The control keeps its idle look. [source]
  - A **manual** check that can't run shows an error toast "Could not check for updates" with the reason
    (`SidebarUpdatePill.tsx:345-368`). [source]
- **Error and retry wording goes in the tooltip,** for example "Download failed for X. Click to retry." or "Update
  X did not apply. Click to retry." (`apps/web/src/components/desktopUpdate.logic.ts:114-196`). [source]

## 3. What the update button does

- **Updater:** `electron-updater` 6.8.9 (`apps/desktop/package.json:28`), patched in
  `patches/electron-updater@6.8.9.patch`. The feed is GitHub Releases: provider `github`, release type `release`,
  or `prerelease` for nightly (`scripts/build-desktop-artifact.ts:2748-2753`). [source]
- **When it checks:** 15 s after launch, then every 4 hours (`apps/desktop/src/updates/DesktopUpdates.ts:65-66,
  1093-1125`). [source]
- **Nothing happens on its own:** `autoDownload = false` and `autoInstallOnAppQuit = false`
  (`DesktopUpdates.ts:1371-1372`). [source]
- **One press does one step** (`desktopUpdate.logic.ts:54-74`; `SidebarUpdatePill.tsx:293-372`) [source]:
  1. **Idle:** the press checks now, along with its separate updates for helper tools. Those are T3 Code-specific
     and not relevant here.
  2. **Available:** the press downloads. When the download completes, a success toast says **"Update downloaded"**,
     "Restart the app from the update button to install it.", with a **Read more ↗** link that opens the GitHub
     release page in the browser (`apps/web/src/components/desktopUpdate.toast.tsx:14-22, 49-63`).
  3. **Downloaded:** the press asks for confirmation: "Install update X and restart T3 Code? Any running tasks will
     be interrupted. Make sure you're ready before continuing." (`desktopUpdate.logic.ts:198-215`). Then it calls
     `autoUpdater.quitAndInstall` (`apps/desktop/src/electron/ElectronUpdater.ts:170-178`).
- **Channels:** a Settings select switches between Stable (`latest`) and Nightly (`SettingsPanels.tsx:367-420`;
  `DesktopUpdates.ts:575-591`). [source]
- **Managed policy:** an administrator's policy can turn updates off or point them at a mirror. The UI then shows
  the reason in place of the actions (`apps/desktop/src/updates/DesktopUpdatePolicy.ts:20-48`;
  `desktopUpdate.logic.ts:263-285`). [source]

## 4. How release notes are shown

### Where and when

- **Before updating only,** in a popover anchored above the sidebar update button. The popover is offered only
  while an update is pending and has notes (`SidebarUpdatePill.tsx:59-64, 280-281`). [source]
- **What opens it:** the pointer resting on the button (150 ms close delay), or keyboard focus landing on it. Tab
  moves focus into the popover. **Pressing the button never opens the notes:** that press is cancelled for the
  popover and does the update action instead (`SidebarUpdatePill.tsx:66-81, 497-518, 526-599`). Escape closes it.
  [source]

### Layout

From `apps/web/src/components/sidebar/SidebarUpdateReleaseNotes.tsx:78-177` [source]:

- At most 24 rem wide. The list scrolls past about 28 rem.
- **Header:**
  - Title "Update ready to download" in medium weight.
  - Under it, muted: **"0.1.14 · 3h ago"**, the version and the release's age. Nightlies get "today" or "2d ago".
- **One section per release, newest first:**
  - The offered release has no heading. An older skipped release gets a small muted "X · 2d ago" heading.
  - Then either a one-paragraph intro or a bullet list. A bullet that starts with a bold phrase keeps the bold, as
    in "**Stop devices.** Stop individual simulators…".
  - An optional bold footer line, for example "Plus 380 more bug fixes and performance improvements."
  - A muted link with an external-link glyph. Its text is:
    - the release's own stats line, "401 changes from 52 contributors", when it has one;
    - "N more changes on GitHub" when bullets were cut;
    - "View release on GitHub" otherwise.
- **Older releases:** at most 6 release sections. Then "N older releases on GitHub", linking to the releases list.

### Where the text comes from

- It is the **GitHub release body**, delivered by the updater feed and parsed in the app
  (`apps/desktop/src/updates/releaseNotes.ts`) [source]:
  - HTML is stripped and entities decoded (`:57-69`).
  - "What's changed", compare links and the "Full changelog" footer are dropped (`:86-97`).
  - Only the summary above the `<details><summary>Full changelog</summary>` block is kept (`:101-160`).
  - At most 8 items per release, each at most 220 characters (`:19-21, 71-77`).
  - Only releases on the user's channel are kept (`:196-224`).
- **The release body is written for this view.** CI writes a short model summary over GitHub's generated list
  (`.github/workflows/release.yml:951-970`). v0.1.13's body has this shape [source]:
  - `**v0.1.13** · 401 changes from 52 contributors`
  - seven bullets like `**Workflow subjects.** Link workflow runs to an issue…`
  - `### Plus 380 more bug fixes and performance improvements.`
  - GitHub's full list folded under `<details>`.

### After updating

- There is **no "What's new" screen, card or banner.** [source]
- The only post-restart messages are toasts:
  - **After an "update everything" run:** a success toast on the next launch, e.g. "Updated this app and 2
    servers". The result is saved to `localStorage` and shown only if the restart happens within 10 minutes
    (`apps/web/src/components/sidebar/updateEverythingRun.tsx:207-208, 276-304, 425-432`;
    `updateEverything.logic.ts:725-766`). The toast has no notes in it. [source]
  - **When an install didn't take:** the app compares its running version with the version it tried to install.
    If they differ, it shows an error toast "Update did not apply. T3 Code is still on X. Install Y again from the
    update button." (`updateEverythingRun.tsx:434-455`). [source]

## 5. Dismiss / skip

- **No "skip this version" and no "remind me later" for the offer.** The available indicator stays until the user
  installs. A newer release simply replaces the offered one (`DesktopUpdates.ts:1129-1170`). [source]
- **"Later"** appears in only two places [source]:
  - the macOS menu's Update available and Update ready message boxes (`DesktopApplicationMenu.ts:103-106, 133`);
  - the "update everything" capsule, where it postpones a restart countdown
    (`SidebarUpdateCapsule.tsx:109-112`; `updateEverythingRun.tsx:203-205`).
- The notes popover closes on Escape or when the pointer leaves. Nothing about the offer is remembered as
  dismissed. [source]

## 6. What fits xReveal, and what doesn't

xReveal's settled flow:

- The version is small muted text in the Settings drawer footer, beside "Reset all settings" and "Quit xReveal"
  (`src/drawer.rs:1044-1057`).
- When a newer GitHub release is found (checked at most once a day), that line reads **Update to x.y.z**. It shows
  the notes and opens the download page.
- There is no installer, and the game must not be covered.

### Fits

- **The signal is a quiet change in place, and nothing else.**
  - T3 Code announces a found update only by changing one small footer control: no toast, no dialog.
  - That matches "never cover the game".
  - It is evidence that a mature app manages without an announcement outside its sidebar. [inference]
- **Version and age together.** "1.2.0 · 3d ago" is a cheap, useful line above the notes. GitHub's release API
  gives `published_at`. [inference]
- **Short notes with a link to the rest.**
  - Bullets that open with a bold phrase, at most about 8 of them.
  - A last link, "View release on GitHub", which for xReveal is the same download page.
  - This suits a narrow drawer better than rendering a whole Markdown body. [inference]
- **Grouping skipped releases.**
  - A user two versions behind sees both releases' notes, newest first, each under a small "x.y.z · age" heading.
  - The count is capped, then "N older releases on GitHub".
  - xReveal's once-a-day check against `releases` can return this. [inference]
- **The release body is written to be read in the app.**
  - T3 Code shapes its GitHub release body so the in-app view can parse it.
  - xReveal can make the GitHub release body the version's `CHANGELOG.md` section, written once.
  - The Update to x.y.z preview and the built-in What's new would then show the same text. [inference]
- **Manual and background checks fail differently.**
  - A background failure stays silent.
  - A failure the user asked for, if xReveal has a "check now" at all, says so briefly. [inference]

### Doesn't fit

- **The icon-only round button with a dot.**
  - xReveal's footer is text links, and "Update to x.y.z" says more than a glyph with a dot.
  - T3 Code needs the icon because its row is full of icons. [inference]
- **The download, progress-ring, restart-badge and install-confirmation states, the "Update downloaded" toast and
  electron-updater.**
  - These exist only because T3 Code installs itself.
  - xReveal needs two states: plain version, and Update to x.y.z. "Checking…" needs no visible state for a
    once-a-day background check. [inference]
- **Notes that open on hover.**
  - The drawer is driven by keyboard focus too (`src/drawer/nav.rs`).
  - A popover anchored to a footer link would also cramp the drawer.
  - Better: pressing Update to x.y.z shows the notes inside the drawer, with an "Open download page" button there.
  - T3 Code's "press does the action, hover shows notes" split would hide the notes from keyboard and controller
    users. [inference]
- **Checking every 4 hours, checking 15 s after launch, Stable/Nightly channels and managed policy.** xReveal has
  settled on once a day, after the picture is up, with no channels. [inference]
- **The macOS "Check for Updates..." menu item with modal dialogs.** Modal boxes go against "don't cover the
  game". A menu item could only open the drawer. [inference]
- **"Update did not apply" detection.** xReveal doesn't install, so it has nothing to compare against. [inference]

### For #135 (What's new)

- T3 Code has **no post-update notes surface at all**. Its only post-update feedback is a one-line toast, and only
  after a multi-machine update run. So T3 Code doesn't settle #135's choice between a top bar, a card and the drawer
  opening by itself. [source]
- One part carries over: its **notes layout** fits whichever surface wins:
  - version · age heading;
  - bold-lead bullets, capped;
  - a link to the full release.
  [inference]
