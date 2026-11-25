# What does xReveal need to capture on macOS and install cleanly on macOS and Windows?

Research for [#29](https://github.com/pwn233/xReveal/issues/29) (map: [#2](https://github.com/pwn233/xReveal/issues/2)).
Feeds the grilling ticket [#30](https://github.com/pwn233/xReveal/issues/30) (how v1 is packaged and shipped).
Researched 2026-11-25. The dev Mac runs **macOS 26.6.2 with Xcode 27.2**. The current Apple release is macOS 27
"Golden Gate". Sources are Apple developer documentation (read as Apple's DocC JSON), Apple DTS posts on the
Apple Developer Forums, Microsoft Learn / Microsoft Support, and tool READMEs and source code. Crate versions are
from crates.io on the same date. "Verified" means read in a primary source. "Inference" means reasoned from
sources but not tested.

## Short answer

**macOS: xReveal must ship as a real `.app` bundle with `NSCameraUsageDescription` and
`NSMicrophoneUsageDescription` in its `Info.plist`, or the system kills it on first capture. Once it is signed
with the hardened runtime (required for notarization), it also needs the `com.apple.security.device.camera` and
`com.apple.security.device.audio-input` entitlements. Only a Developer ID-signed, notarized app opens on a
stranger's Mac without a trip to System Settings. That needs the Apple Developer Program, US$99 a year.
Windows: no manifest or entitlement is needed to capture, but the user's global "Let desktop apps access your
camera/microphone" switches can block xReveal. Every unsigned or newly signed download gets a SmartScreen
warning. Signing costs from ~US$9.99 a month (Azure Artifact Signing, if eligible) to US$150–300 a year (OV
certificate), and it is the only way past Smart App Control. Tooling: `cargo-packager` is the one Rust tool that
builds a signed, notarized `.app`/`.dmg` and a signed Windows NSIS/MSI installer from one config.**

- **Capture card video asks for Camera and its audio asks for Microphone on macOS.** cpal reads the card's
  sound as a Core Audio input device ([#6](https://github.com/pwn233/xReveal/issues/6)), so xReveal needs
  **both** permissions. That means two prompts on first run.
- **`cargo run` is not a good way to test capture on macOS.** TCC (macOS's privacy-permission system) gives the
  request to the *responsible* app: the terminal or editor that launched the binary, not xReveal. With an ad-hoc
  signature (what `rustc`'s linker produces) TCC can't recognise the next build as the same app. Develop with a
  small `.app` bundle launched through `open`, signed with a stable identity (see
  [§2](#2-the-dev-loop-cargo-run-vs-a-bundle)).
- **Gatekeeper on macOS 15 and later:** an ad-hoc-signed app, or one signed with Developer ID but not notarized,
  is **blocked** on first open. The user has to go to *System Settings › Privacy & Security › Open Anyway* and
  enter an admin password. Control-click › Open no longer gets around this. A notarized app gets only the usual
  "downloaded from the Internet" confirmation. Official Homebrew casks must pass Gatekeeper since 2026-09-01.
- **Windows privacy switches apply to desktop apps.** Unpackaged apps such as xReveal can't be switched on one by
  one, and Windows shows **no prompt**. If camera access is off, Media Foundation returns `E_ACCESSDENIED`, and
  the app should send the user to `ms-settings:privacy-webcam`.
- **SmartScreen:** an unsigned exe gets "Windows protected your PC" with a hidden "Run anyway". Signed files
  still get a warning until they build reputation, but they show the publisher name. **EV certificates stopped
  skipping this step in 2024.** Smart App Control on Windows 11 **blocks unsigned apps outright, and users can't
  make an exception for one app.**
- **Installer forms:** a portable `.exe`/`.zip`, an MSI (WiX), an NSIS `.exe`, MSIX or winget. None of them adds
  a runtime *if* the exe is built with a static C runtime (`+crt-static`). Otherwise the app needs the VC++
  Redistributable. MSIX must always be signed. The Microsoft Store re-signs MSIX packages for free.

## 1. macOS: what capture needs at all

### Info.plist usage strings (verified)

Apple's capture authorization article says:

> If your app uses device cameras, include the `NSCameraUsageDescription` key in your app's `Info.plist` file.
> If your app uses device microphones, include the `NSMicrophoneUsageDescription` key …
> In macOS, you also need to enable the following entitlements … `com.apple.security.device.camera` …
> `com.apple.security.device.audio-input`.
> Your app needs to contain the appropriate key in its `Info.plist` file, and the appropriate entitlement enabled
> in macOS, before it requests authorization or attempts to use a capture device. **Otherwise, the system
> terminates your app.**

— [Apple: Requesting authorization to capture and save media](https://developer.apple.com/documentation/avfoundation/requesting-authorization-to-capture-and-save-media)

The key pages say the same: "This key is required if your app uses APIs that access the device's camera" /
"…microphone" ([NSCameraUsageDescription](https://developer.apple.com/documentation/bundleresources/information-property-list/nscamerausagedescription),
[NSMicrophoneUsageDescription](https://developer.apple.com/documentation/bundleresources/information-property-list/nsmicrophoneusagedescription)).

Flow: call `AVCaptureDevice.authorizationStatus(for:)` first. If it returns `notDetermined`, call
`requestAccess(for:completionHandler:)` "at a time that's appropriate for your app". The system remembers the
answer. After that the user can change it only in System Settings › Privacy & Security (same Apple article).
For xReveal that fits the Welcome screen: explain, then call `requestAccess` for `.video` and then `.audio`.
Both are reachable through `objc2-av-foundation`.

**Which permission covers the card's audio (inference, high confidence).** cpal opens the card's audio as a
Core Audio input device. The audio-input entitlement is described as covering "record audio using the built-in
microphone **and access audio input using Core Audio**"
([com.apple.security.device.audio-input](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.audio-input)).
TCC files it under Microphone, so a capture card's sound needs the Microphone permission. Test it on the
Reference card.

### Entitlements and the hardened runtime (verified)

- The camera entitlement is added "after you enable the App Sandbox **or Hardened Runtime** capability". The
  audio-input entitlement is added under Hardened Runtime › Resource Access
  ([camera](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.camera),
  [audio-input](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.audio-input)).
- "To upload a macOS app to be notarized, you must enable the Hardened Runtime capability"
  ([Hardened Runtime](https://developer.apple.com/documentation/security/hardened-runtime)). In `codesign` terms
  that is `--options runtime`
  ([Resolving common notarization issues](https://developer.apple.com/documentation/security/resolving-common-notarization-issues)).
- Entitlements belong on executables only. Libraries inherit them from the host, and an entitlement left out is
  false (same Hardened Runtime page).
- Notarization also rejects `com.apple.security.get-task-allow`, needs a secure timestamp (`--timestamp`), and
  needs XML entitlements without a BOM (same page).
- **Plug-in note:** Apple warns that apps using "drivers for professional mirrorless cameras and legacy DSLR
  cameras that don't conform to the driverless USB video device class (UVC) standard" must declare
  `com.apple.security.cs.disable-library-validation` to load those plug-ins under the hardened runtime
  ([Notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)).
  UVC cards don't need it. A **driver-based macOS capture card that ships a DAL/CoreMediaIO plug-in might**. That
  is the "best effort" case from [#12](https://github.com/pwn233/xReveal/issues/12).

So the minimum `xreveal.entitlements` for a notarized build is:

```xml
<key>com.apple.security.device.camera</key><true/>
<key>com.apple.security.device.audio-input</key><true/>
```

Zed, a GPUI app, ships a hardened-runtime entitlements file with exactly these two keys among others
([zed.entitlements](https://github.com/zed-industries/zed/blob/main/crates/zed/resources/zed.entitlements)).

**Inference:** a build signed *without* the hardened runtime (ad-hoc or Apple Development without
`--options runtime`) doesn't enforce the resource-access entitlements. It still needs the Info.plist strings.
Apple's text requires "the appropriate entitlement enabled in macOS" without naming a mode. The entitlement
pages, though, tie the entitlements to the Sandbox and Hardened Runtime capabilities. Not tested.

## 2. The dev loop: `cargo run` vs a bundle

**An unbundled binary has no Info.plist (verified).** Apple DTS: "You can't put this in a `Info.plist` file,
because the command-line tool isn't packaged, but you can add it to a custom section in your executable … Create
Info.plist Section in Binary" ([forum 697278](https://developer.apple.com/forums/thread/697278), Dec 2021). An
Apple Frameworks Engineer suggested the same embedded `__TEXT,__info_plist` section for a command-line tool that
crashed for lack of `NSCameraUsageDescription`
([forum 111100](https://developer.apple.com/forums/thread/111100), Mar 2021). Without Xcode this is the linker flag
`-sectcreate __TEXT __info_plist <file>`, which a `build.rs` can pass with `cargo:rustc-link-arg`.

**TCC attributes the request to the responsible code (verified).** Quinn (Apple DTS): "The MAC privilege
mechanism is heavily dependent on the concept of *responsible code*." If an app's helper triggers a prompt, the
app's name and usage description appear, and the decision is recorded for the app
([On File System Permissions, forum 678819](https://developer.apple.com/forums/thread/678819), rev. 2026-09-18).
A binary started by `cargo run` is a child of the terminal or editor, so the terminal app is the responsible code
(inference, consistent with the above).

**Local check on the dev Mac (2026-11-25):**

| Launcher | `NSCameraUsageDescription` / `NSMicrophoneUsageDescription` in its Info.plist |
| --- | --- |
| Terminal.app (macOS 26.6.2) | **absent** |
| Zed.app | present: "An application in Zed wants to use the camera." |
| T3 Code (Nightly).app | present: "T3 Code lets commands run by terminals and coding agents use the camera." |

Zed and T3 Code declare these strings *so that their child processes can prompt*. What happens to a camera
request from a process under Terminal.app is **not verified** (open question). One third-party report says the
parent denies it silently when it lacks the key
([coven-cave PR 5702](https://github.com/OpenCoven/coven-cave/pull/5702), not authoritative).

**Ad-hoc signatures defeat TCC's memory (verified).** Quinn: "Either your program is not signed or it's ad hoc
signed. This means that TCC can't record a stable signing identity for it, so it doesn't know that version N+1 is
'the same' as version N. The fix … is to sign your code with a stable identity, either Apple Development for
day-to-day work or Developer ID when you distribute it"
([forum 697278](https://developer.apple.com/forums/thread/697278), Jul 2023). An **Apple Development** identity
can come from a *free* Apple Account through Xcode's Personal Team. DTS recommends that route for people without
the paid program, and the identity can then be used with `codesign` from the command line
([forum 712555](https://developer.apple.com/forums/thread/712555)). The certificate lasts one year.

**Recommendation for the dev loop.** Add a `just`/`cargo xtask` recipe that builds the binary, wraps it in a
minimal `xReveal.app` (Info.plist with bundle ID and both usage strings), and signs it with the dev Mac's Apple
Development identity. Then launch it with `open xReveal.app` so that LaunchServices makes xReveal its own
responsible app. Inference: the permission then sticks across rebuilds, and the prompts show xReveal's own text.
Verify both on the dev Mac (see [§10](#10-open-questions)).

## 3. macOS: Gatekeeper outcome per signing level

**Background (verified):**

- On Apple silicon, "any executable must be signed before it's allowed to run … a simple ad-hoc signature is
  sufficient", and the Xcode 12+ linker ad-hoc signs automatically. Tools that change a binary after linking
  (`strip`, `install_name_tool`) need a `codesign` re-sign. "Binaries signed this way **cannot pass through
  Gatekeeper**"
  ([macOS Big Sur 11.0.1 Universal Apps release notes](https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes)).
- "Beginning in macOS 10.15, all software built after June 1, 2019, and distributed with Developer ID must be
  notarized." Only a **Developer ID** certificate can be notarized. Ad-hoc, Apple Development and Mac Distribution
  certificates are rejected
  ([Notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)).
- "In macOS Sequoia, users will no longer be able to Control-click to override Gatekeeper when opening software
  that isn't signed correctly or notarized. They'll need to visit System Settings > Privacy & Security"
  ([Apple Developer News, 2024-08-06](https://developer.apple.com/news/?id=saqachfa)).
- Gatekeeper checks downloaded software "the first time it's opened"
  ([Apple Platform Security: Gatekeeper](https://support.apple.com/guide/security/gatekeeper-and-runtime-protection-sec5599b66df/web)).
  The user path is *Open Anyway* in Privacy & Security, after which the app "is saved as an exception"
  ([Apple Support 102445](https://support.apple.com/en-us/102445), published 2026-05-27).
- macOS 26 and 27 add no new developer-facing Gatekeeper rule. The macOS 27 notes add only `spctl --purge`
  ([macOS 27 release notes](https://developer.apple.com/documentation/macos-release-notes/macos-27-release-notes)).

**Outcome for a downloaded (quarantined) copy on macOS 15 / 26 / 27:**

| Signing level | First open | Cost |
| --- | --- | --- |
| Unsigned arm64 | Does not run at all. The kernel requires a signature. Rare in practice because the linker ad-hoc signs. | free |
| Ad-hoc (`codesign -s -`) | Blocked: "could not verify … free of malware". The user must go to System Settings › Privacy & Security › **Open Anyway** and enter an admin password once per install. The terminal alternative is `xattr -d com.apple.quarantine`. | free |
| Apple Development (Personal Team) | Same as ad-hoc for other people ("unidentified developer"). Its value is a stable identity for TCC on the dev Mac. | free |
| Developer ID, **not notarized** | Blocked the same way as ad-hoc (must be notarized since 10.15) | US$99/yr |
| Developer ID + **notarized** (+ stapled) | Opens after the normal "downloaded from the Internet" confirmation. Gatekeeper finds the ticket online or stapled. | US$99/yr |

A copy built on the Mac itself has no quarantine flag, so Gatekeeper doesn't check it (inference from the
"downloaded software" wording). That is why local builds just run.

**Homebrew.** Homebrew 5.0.0 (2025-11-12) deprecated unsigned casks and `--no-quarantine`, and "will disable all
Homebrew/homebrew-cask casks that fail Gatekeeper checks in September 2026"
([brew.sh 5.0.0](https://brew.sh/2025/11/12/homebrew-5.0.0/),
[Homebrew/brew#20755](https://github.com/Homebrew/brew/issues/20755)). An official cask therefore needs a
notarized build. A personal tap can still carry an unsigned cask.

**Intel Macs.** macOS 27 lists Intel apps "that will no longer run in macOS 28.0", and "All Intel-based software
will no longer be compatible with macOS 28.0, excluding legacy games"
([macOS 27 release notes](https://developer.apple.com/documentation/macos-release-notes/macos-27-release-notes)).
That makes an arm64-only macOS build reasonable. Universal is optional and costs size.

## 4. macOS: cost

- **Apple Developer Program: US$99 per membership year** (local price may vary). Nonprofits, schools and
  government bodies can ask for a fee waiver. An individual needs an Apple Account with two-factor
  authentication and must enrol under their **legal name**, which is shown as the seller/publisher name.
  Organizations need a D-U-N-S number
  ([Apple: Enrollment](https://developer.apple.com/programs/enroll/)).
- The program gives Developer ID certificates and the notary service (`notarytool`, which accepts an App Store
  Connect API key, the usual choice for CI). A free account gets only the Apple Development identity: no
  Developer ID and no notarization ([forum 712555](https://developer.apple.com/forums/thread/712555)).
- Notarization itself has no per-use fee. Scans "usually take less than an hour"
  ([Notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)).
  Apple forums in 2026 report occasional long "In Progress" queues.

## 5. Windows: privacy switches

- Windows 11: *Settings › Privacy & security › Camera / Microphone* has a device-wide switch, per-app switches
  for **Store (packaged) apps**, and one **"Let desktop apps access your camera"** (and microphone) switch.
  "Desktop apps cannot be individually toggled." On Windows 10 (1903+) the switch is called "Allow desktop apps
  to access your camera". Microsoft also says desktop apps "may not always appear in the list" and "might still
  be able to access your camera or microphone even when these settings are turned off"
  ([Microsoft Support: Windows camera, microphone, and privacy](https://support.microsoft.com/en-us/windows/privacy/windows-camera-microphone-and-privacy)).
  Windows 10 support ended 2025-10-14 (same page).
- Microsoft's developer guidance: "Camera access can be disabled for the entire device, for all unpackaged apps,
  or for individual packaged apps." With access off, "the Windows camera capture APIs will return the error
  **E_ACCESSDENIED**". The sample shows this coming from `IMFActivate::ActivateObject` on the device found by
  `MFEnumDeviceSources`, which is xReveal's exact path. The fix is to tell the user and launch
  `ms-settings:privacy-webcam`, then retry when they return
  ([Microsoft Learn: Handle the Windows camera privacy setting](https://learn.microsoft.com/en-us/windows/apps/develop/camera/camera-privacy-setting), 2026-09-08).
- No Info.plist-style declaration or first-run prompt exists for unpackaged apps. Neither Microsoft page
  describes a prompt. Packaged (MSIX) apps can check first with `AppCapability.Create("Webcam").CheckAccess()`
  (same page).
- **Microphone:** the same desktop-apps switch exists. The exact error cpal's WASAPI backend surfaces when it is
  off was **not verified**. Handle a capture-stream build error the same way and link
  `ms-settings:privacy-microphone`.

## 6. Windows: SmartScreen and Smart App Control

From [Microsoft Learn: SmartScreen reputation for Windows app developers](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)
(2026-05-04):

| Signing | First-download behaviour |
| --- | --- |
| Microsoft Store | No warning (Microsoft's certificate) |
| OV / EV / Artifact Signing | "Warning — app flagged as unrecognized until reputation accumulates; verified publisher name is displayed" |
| No signature | "'Windows protected your PC'; User must choose 'Run anyway'". Enterprise policy can block it completely. |
| Self-signed | Same as no signature |

- "**EV certificates no longer bypass SmartScreen** … Paying a premium for EV solely to avoid SmartScreen warnings
  is no longer justified."
- Reputation builds from download volume: "it can take several weeks and hundreds of clean installs". There is
  no way to submit a file for review. "Unsigned files must build reputation anew with every update". Signed files
  can inherit the certificate's reputation.
- **Smart App Control (Windows 11)** "may supersede SmartScreen" and "will block execution of unsigned files
  unless the file has a positive reputation", for all executables, not only downloads (same page). Microsoft
  Support: "There is currently no way to bypass Smart App Control protection for individual apps". Recent updates
  let users turn it on without a clean install
  ([Microsoft Support: What is Smart App Control?](https://support.microsoft.com/en-us/windows/what-is-smart-app-control-285ea03d-fa88-4d56-882e-6698afdb7003)).
  **On a PC with SAC on, an unsigned xReveal can't run at all.**

## 7. Windows: signing options and cost

From [Microsoft Learn: Code signing options for Windows app developers](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options)
(2026-10-29), unless noted:

| Option | Cost | Who can use it | Notes |
| --- | --- | --- | --- |
| Microsoft Store, **MSIX** | free (Store account is free: [Partner Center](https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/open-a-developer-account)) | worldwide | Store re-signs the package, so there are no SmartScreen warnings. Needs ID check (government ID + selfie). |
| Microsoft Store, MSI/EXE | needs your own trusted certificate | worldwide | Installer and every PE file signed to a Trusted Root CA, silent install, offline installer ([requirements](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/app-package-requirements)) |
| **Azure Artifact Signing** (was Trusted Signing) | **~US$9.99/month** (Basic: 5,000 signatures) | organizations in the US, Canada, EU, UK, Australia, NZ, Japan, South Korea, Singapore, Switzerland, Norway, Israel; **individuals: US and Canada only** ([quickstart](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart), 2026-05-21) | No hardware token. Built for CI (GitHub Actions). Identity check takes 1–30 business days. Individuals need an Azure billing account of type Individual. Zed signs this way ([sign.ps1](https://github.com/zed-industries/zed/blob/main/crates/zed/resources/windows/sign.ps1)). |
| OV certificate (DigiCert, Sectigo, GlobalSign, …) | US$150–300/yr | worldwide | Private key must be on an HSM or hardware token (CA/B Forum, June 2023). Cloud-HSM offerings make CI possible. |
| EV certificate | US$400+/yr | worldwide | Same SmartScreen behaviour as OV since 2024 |
| SignPath Foundation | free | qualifying open-source projects | OV-level signing through a managed pipeline. xReveal is public but has **no licence file yet**. |
| Self-signed / none | free | — | Blocked by SAC. SmartScreen "Run anyway". |

The Azure pricing page shows the tier structure (Basic 5,000 / Premium 100,000 signatures a month) but no dollar
amounts. The ~US$9.99 figure comes from Microsoft Learn
([Azure pricing](https://azure.microsoft.com/en-us/pricing/details/artifact-signing/)).

## 8. Windows: installer forms

| Form | Install experience | Size / runtime | Signing | Tool |
| --- | --- | --- | --- | --- |
| Portable `.exe` (or `.zip` of it) | Download and run. Zip extraction carries the download mark, so SmartScreen still applies (inference). | Smallest: the exe alone | Optional but recommended | none |
| MSI (WiX) | Standard per-machine or per-user install, Add/Remove Programs, `msiexec /q` silent | Small wrapper | Sign the exe and the MSI | `cargo-packager` (WiX), `cargo-wix` 0.3.9, `cargo-dist` 0.34 (`msi`), `cargo-bundle` (experimental) |
| NSIS `.exe` | Classic setup wizard, `/S` silent | Small wrapper | Sign the exe and the installer | `cargo-packager` |
| MSIX | Clean install/uninstall, differential updates, package identity, per-app privacy switches. Apps run in a light container. | No added runtime | **"All MSIX packages must be signed before installation"** ([MSIX overview](https://learn.microsoft.com/en-us/windows/msix/overview)). Free if through the Store. | WinApp CLI / MakeAppx (not in the Rust packagers) |
| winget | `winget install` from a manifest PR to `microsoft/winget-pkgs` pointing at a release URL | Same as the installer it points to | Not required, but Defender and several antivirus engines scan every submission. The installer must install silently and come straight from the publisher's URL. | manifest (`wingetcreate`). Types: msix, msi, exe, inno, nullsoft, wix, burn, plus portable (winget 1.3+) and zip (1.5+) ([repository](https://learn.microsoft.com/en-us/windows/package-manager/package/repository), [installer schema](https://github.com/microsoft/winget-pkgs/blob/master/doc/manifest/schema/1.12.0/installer.md)) |

**C runtime (verified).** Rust's MSVC targets link the C runtime **dynamically** by default. Statically linking
it takes `-C target-feature=+crt-static`
([Rust reference: Linkage](https://doc.rust-lang.org/reference/linkage.html)). Linked dynamically, the app needs
the Visual C++ Redistributable "at least as recent as the build tools used to build your app"
([Microsoft Learn: VC++ Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)).
winget even has a `Validation-VCRuntime-Dependency` failure label for this. **Recommendation: build Windows
releases with `+crt-static`, so that a portable exe has no runtime dependency.** Check the size cost in CI
(expected tens of KB to a few hundred KB, not measured).

## 9. Rust tooling

| Tool (crates.io, 2026-11-25) | macOS output | macOS signing | Windows output | Windows signing | Notes |
| --- | --- | --- | --- | --- | --- |
| **`cargo-packager` 0.11.8** (release 2025-11-27, repo active Nov 2026) | `.app`, `.dmg` | **Yes.** `codesign --options runtime --timestamp` with your entitlements file, then `notarytool submit --wait` and `stapler staple`. Credentials come from env: `APPLE_CERTIFICATE`(+`_PASSWORD`), `APPLE_API_KEY`/`APPLE_API_ISSUER`/`APPLE_API_KEY_PATH`, or `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` ([source](https://github.com/crabnebula-dev/cargo-packager/blob/main/crates/packager/src/codesign/macos.rs)) | NSIS `.exe`, MSI (WiX) | Yes. `signtool` by thumbprint, **or a custom `sign_command` with `%1`**, which fits Artifact Signing ([WindowsConfig](https://docs.rs/cargo-packager/latest/cargo_packager/config/struct.WindowsConfig.html)) | Config in `[package.metadata.packager]` or `Packager.toml`. Takes your own `info_plist_path` and `entitlements` ([MacOsConfig](https://docs.rs/cargo-packager/latest/cargo_packager/config/struct.MacOsConfig.html)). Same team as Tauri's bundler. |
| `cargo-bundle` 0.12.0 (2026-09-20) | `.app`, `.dmg` | Signs in-process (`apple-codesign`) but "**does not notarize**" | MSI (experimental) | PKCS#12 only, behind the GPL `windows-signing` feature | "Very early alpha". Extra Info.plist keys via `osx_info_plist_exts`. **Zed uses a fork** of it, then signs and notarizes in its own script ([bundle-mac](https://github.com/zed-industries/zed/blob/main/script/bundle-mac)). |
| `cargo-dist` (`dist`) 0.34.0 (2026-11-02) | **no `.app`/`.dmg`** ("future installers … no specific timeline") | n/a | MSI, PowerShell/shell installers, Homebrew formula | limited | Aimed at CLIs. Good at the GitHub Release + CI workflow, wrong for a GUI app bundle ([installers](https://axodotdev.github.io/cargo-dist/book/installers/index.html)) |
| `cargo-wix` 0.3.9 (2025-03-13) | — | — | MSI | via signtool | Windows only |

**Recommendation:** use **`cargo-packager`**: one config for `.app` + `.dmg` + NSIS/MSI, with Apple notarization
and a pluggable Windows sign command, run from two GitHub Actions jobs (macOS and Windows runners). Fall back to
Zed's pattern (`cargo-bundle` plus hand-written `codesign`/`notarytool`/`stapler` lines) if cargo-packager falls
behind. Ship the portable exe as well, since it is just the signed `xreveal.exe`.

### GPUI-specific bundling needs

- **Metal shaders are compiled at build time.** GPUI's Apple backend runs `xcrun metal` → `shaders.metallib`
  in its `build.rs`, unless the `runtime_shaders` feature is on
  (`gpui-fast-apple-0.1.19/build.rs`, the GPUI that GPUI Kit 0.11 resolves to in `Cargo.lock`). The macOS build
  host, CI included, needs **full Xcode with the Metal toolchain**. Since **Xcode 26 the Metal toolchain is a
  separate download** (`xcodebuild -downloadComponent MetalToolchain`), and Apple forum threads report it going
  missing on command-line builds
  ([forum 787294](https://developer.apple.com/forums/thread/787294), [forum 792841](https://developer.apple.com/forums/thread/792841)).
  The dev Mac's Xcode 27.2 resolves `xcrun --find metal`. CI images need checking.
- **Windows shaders need `fxc.exe`** from the Windows SDK at build time (`gpui-fast-windows-0.1.19/build.rs`,
  overridable with `GPUI_FXC_PATH`). GitHub's Windows runners ship the SDK (unverified for the current image).
- **xReveal's own shaders** (Metal + HLSL, [#18](https://github.com/pwn233/xReveal/issues/18)) need the same
  toolchains, or precompiling and embedding.
- The GPUI app is one executable. GPUI Kit's default assets are compiled in (`gpui-kit-assets`), so the `.app`
  needs only the binary, Info.plist and icon. A `minimum_system_version` of at least **12.0** matches the
  `-mmacosx-version-min=12.0` that GPUI's shader build uses.
- Zed's macOS release flow is the reference: sign helpers, then the main binary with entitlements, then the
  `.app`, all with `--options runtime --timestamp`, then `hdiutil` the `.dmg`, sign it, notarize it and staple it
  ([bundle-mac](https://github.com/zed-industries/zed/blob/main/script/bundle-mac)). Zed builds its Windows
  installer with Inno Setup and signs with Azure Trusted/Artifact Signing
  ([bundle-windows.ps1](https://github.com/zed-industries/zed/blob/main/script/bundle-windows.ps1)).

## 10. Open questions

1. **`cargo run` from Terminal.app:** does a camera request prompt (attributed to Terminal), get denied
   silently, or crash? Terminal.app 26.6.2 declares no usage strings. Test once on the dev Mac before
   settling the dev loop.
2. **Bundled dev build launched with `open`:** confirm that xReveal is its own responsible app, that the prompts
   show xReveal's strings, and that an Apple Development signature keeps the grant across rebuilds.
3. **Card audio = Microphone permission:** confirm on the Reference card that the Microphone permission covers
   cpal's Core Audio input from the card (expected), and that Camera covers the video.
4. **Ad-hoc build without the hardened runtime:** does capture work with the Info.plist strings alone (expected),
   or does it also need the entitlements?
5. **Windows microphone switch off:** what error cpal/WASAPI returns, so xReveal can show the right message.
   Needs a Windows PC (dry lab, [#23](https://github.com/pwn233/xReveal/issues/23)).
6. **Eligibility:** Azure Artifact Signing for individuals is US/Canada only. If the developer isn't there, the
   choice is an OV certificate (cloud HSM for CI), SignPath Foundation (needs an OSS licence), or Store MSIX.
7. **Driver-based macOS cards** with CoreMediaIO/DAL plug-ins may need `disable-library-validation` under the
   hardened runtime. Untested, and best effort per #12.
8. **GitHub Actions macOS runner:** is the Metal toolchain present for the Xcode version selected?

## Sources

Apple (developer.apple.com, read 2026-11-25):
[Requesting authorization to capture and save media](https://developer.apple.com/documentation/avfoundation/requesting-authorization-to-capture-and-save-media) ·
[NSCameraUsageDescription](https://developer.apple.com/documentation/bundleresources/information-property-list/nscamerausagedescription) ·
[NSMicrophoneUsageDescription](https://developer.apple.com/documentation/bundleresources/information-property-list/nsmicrophoneusagedescription) ·
[Camera entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.camera) ·
[Audio input entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.audio-input) ·
[Hardened Runtime](https://developer.apple.com/documentation/security/hardened-runtime) ·
[Notarizing macOS software before distribution](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution) ·
[Resolving common notarization issues](https://developer.apple.com/documentation/security/resolving-common-notarization-issues) ·
[macOS Big Sur 11.0.1 Universal Apps release notes](https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes) ·
[macOS 27 release notes](https://developer.apple.com/documentation/macos-release-notes/macos-27-release-notes) ·
[Updates to runtime protection in macOS Sequoia (2024-08-06)](https://developer.apple.com/news/?id=saqachfa) ·
[Apple Developer Program enrollment](https://developer.apple.com/programs/enroll/) ·
[Apple Support 102445: Safely open apps](https://support.apple.com/en-us/102445) ·
[Apple Platform Security: Gatekeeper](https://support.apple.com/guide/security/gatekeeper-and-runtime-protection-sec5599b66df/web) ·
DTS forum threads [678819](https://developer.apple.com/forums/thread/678819), [697278](https://developer.apple.com/forums/thread/697278),
[698337](https://developer.apple.com/forums/thread/698337), [111100](https://developer.apple.com/forums/thread/111100),
[712555](https://developer.apple.com/forums/thread/712555), [787294](https://developer.apple.com/forums/thread/787294).

Microsoft (read 2026-11-25):
[SmartScreen reputation](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation) ·
[Code signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options) ·
[Artifact Signing quickstart](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart) ·
[Artifact Signing pricing](https://azure.microsoft.com/en-us/pricing/details/artifact-signing/) ·
[Handle the Windows camera privacy setting](https://learn.microsoft.com/en-us/windows/apps/develop/camera/camera-privacy-setting) ·
[Windows camera, microphone, and privacy](https://support.microsoft.com/en-us/windows/privacy/windows-camera-microphone-and-privacy) ·
[What is Smart App Control?](https://support.microsoft.com/en-us/windows/what-is-smart-app-control-285ea03d-fa88-4d56-882e-6698afdb7003) ·
[MSIX overview](https://learn.microsoft.com/en-us/windows/msix/overview) ·
[Store MSI/EXE requirements](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/app-package-requirements) ·
[Open a Store developer account](https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/open-a-developer-account) ·
[winget: submit manifest](https://learn.microsoft.com/en-us/windows/package-manager/package/repository) ·
[winget: manifest](https://learn.microsoft.com/en-us/windows/package-manager/package/manifest) ·
[VC++ Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist).

Tools: [cargo-packager](https://github.com/crabnebula-dev/cargo-packager) ·
[cargo-bundle](https://github.com/burtonageo/cargo-bundle) ·
[dist installers](https://axodotdev.github.io/cargo-dist/book/installers/index.html) ·
[Zed bundle-mac](https://github.com/zed-industries/zed/blob/main/script/bundle-mac) ·
[Zed bundle-windows.ps1](https://github.com/zed-industries/zed/blob/main/script/bundle-windows.ps1) ·
[Homebrew 5.0.0](https://brew.sh/2025/11/12/homebrew-5.0.0/) ·
[Rust reference: Linkage](https://doc.rust-lang.org/reference/linkage.html).
Local: `~/.cargo/registry/src/*/gpui-fast-apple-0.1.19/build.rs`, `gpui-fast-windows-0.1.19/build.rs`.
