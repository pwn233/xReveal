# What does a once-a-day Update available check cost and need?

Research for [#136](https://github.com/pwn233/xReveal/issues/136) (map: [#131](https://github.com/pwn233/xReveal/issues/131)).
Feeds the grilling ticket [#138](https://github.com/pwn233/xReveal/issues/138).
Researched 2026-11-27 on the dev Mac (macOS 26.6, Rust 1.100.0, xReveal at `f67d38b`, GPUI Kit 0.11.0 /
gpui-pre 0.3.14). Sources: GitHub REST docs, Apple developer documentation (read as DocC JSON), Microsoft Learn,
crate source in `~/.cargo/registry`, and measurements made here. "Measured" means run on this Mac. "Inference"
means reasoned from sources, not run. Nothing was built into xReveal.

Already settled, not re-decided here: at most one check a day, after the picture is up, while a **Check for
updates** switch is on (default on), sending nothing but the request; the drawer footer's version line becomes
**Update to x.y.z**, which opens the release's download page. No self-installing updater.

## Short answer

**Use each OS's own HTTP stack: `NSURLSession` (ephemeral session) on macOS through `objc2-foundation`, and
WinHTTP (synchronous, on GPUI's background executor) on Windows through the `windows` crate. It adds about
0.06 MB to the binary, against +1.9 MB for `ureq` with rustls and +0.7 MB for `ureq` with native-tls. It needs no
new crate: `objc2-foundation` already compiles `NSURLSession` in xReveal's graph, and `windows` needs one more
feature, `Win32_Networking_WinHttp`. Both stacks follow the system proxy and the OS's trusted roots, so
corporate proxies and TLS inspection work without extra code. One `GET
https://api.github.com/repos/pwn233/xReveal/releases/latest` a day costs about 2–3 KB gzipped plus headers and
the TLS handshake, about 3 MB of transient memory, and well under a second of wall time, mostly waiting.**

- **xReveal makes no network requests today (verified).** GPUI Kit links no HTTP client on macOS or Windows.
  The only `reqwest` in `Cargo.lock` (`gpui-pre-reqwest`, with hyper, rustls, tokio and h2) is behind
  `cfg(target_family = "wasm")` in `gpui-kit-assets`. GPUI's default client is `NullHttpClient`, which always
  fails. `src/` contains no networking code.
- **The endpoint:** `GET /repos/{owner}/{repo}/releases/latest` returns the newest published release that is not
  a prerelease or a draft. It returns **404** while no release exists, which is xReveal's state today.
  Unauthenticated callers get **60 requests per hour per IP**. A daily check uses 1.
- **The response is about 9 KB plus the release notes** for xReveal's 4 assets, and about 2 KB gzipped. Measured
  on real repositories: 35–66 KB raw, 4.4–7.3 KB gzipped, almost all of it asset metadata (~1.7 KB per asset).
- **Version comparison:** strip the `v` and compare `major.minor.patch` as numbers against
  `CARGO_PKG_VERSION`. Ignore any tag with a `-pre` suffix. Today `release.yml` would publish a `v1.1.0-beta.1`
  tag as a *full* release, so it would become "latest" (see §3).
- **Where state lives:** `setup.toml` through `Setup` in `src/setup.rs`, with a new `[updates]` table
  (`check`, `last-check`, `latest`). Reading is already tolerant, so older files get the defaults.
- **Failure:** every error is silent and leaves the footer unchanged: offline, DNS, proxy, TLS, 403/429, 404,
  or bad JSON. Record `last-check` only after a 200 or a 404, so an offline launch tries again next launch.
- **What to open:** the release page (`https://github.com/pwn233/xReveal/releases/tag/vX.Y.Z`), not an asset.
  It shows the notes (CHANGELOG.md), lets the user pick installer or portable, and doesn't depend on asset names.
  Open it with GPUI's `cx.open_url`.

## 1. The GitHub endpoint

### What it returns (verified)

> View the latest published full release for the repository. The latest release is the most recent
> non-prerelease, non-draft release, sorted by the `created_at` attribute. The `created_at` attribute is the date
> of the commit used for the release, and not the date when the release was drafted or published.

— [GitHub REST: Get the latest release](https://docs.github.com/en/rest/releases/releases#get-the-latest-release)
(`GET /repos/{owner}/{repo}/releases/latest`, status codes 200 and 404).

Measured today: `https://api.github.com/repos/pwn233/xReveal/releases/latest` returns **404** with a 130-byte
JSON body (`{"message":"Not Found",…}`), because no release is published yet. The client must treat 404 as
"nothing newer", not as an error worth retrying.

Required headers (verified):
- `User-Agent` — "All API requests must include a valid `User-Agent` header. Requests with no `User-Agent`
  header will be rejected." GitHub suggests the app's name
  ([Getting started with the REST API](https://docs.github.com/en/rest/using-the-rest-api/getting-started-with-the-rest-api)).
- `Accept: application/vnd.github+json` is recommended (same page).
- `X-GitHub-Api-Version` is optional: "Requests without the `X-GitHub-Api-Version` header will default to use
  the `2022-11-28` version" ([API versions](https://docs.github.com/en/rest/about-the-rest-api/api-versions)).
  Skip it. `tag_name` and `html_url` have been stable for years. Inference: if 2022-11-28 is ever retired,
  unversioned requests move to the next oldest supported version, and those fields would still be there.

### Rate limits (verified, plus a measurement)

> The primary rate limit for unauthenticated requests is 60 requests per hour.

Unauthenticated requests are counted per originating IP. When the limit is exceeded the response is **403 or
429** with `x-ratelimit-remaining: 0`
([Rate limits for the REST API](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)).
Measured response headers: `x-ratelimit-limit: 60`, `x-ratelimit-resource: core`,
`cache-control: public, max-age=60`.

A daily check uses 1 of 60 per hour. It could only fail when 60 or more GitHub API callers share one public IP
within the hour (a large office, a school, carrier-grade NAT). It then fails silently and succeeds the next day.

**ETags don't help unauthenticated callers.** The best-practices page says a 304 is free "if … the request was
made while correctly authorized with an `Authorization` header"
([Best practices](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api)).
Measured: an unauthenticated `If-None-Match` request got `304` with an empty body, and
`x-ratelimit-remaining` still went down. A 304 saves only the ~2 KB gzipped body, and it means storing the ETag.
Not worth it.

### Response size (measured)

`curl` with only `User-Agent: update-check-test` and the `Accept` header, 2026-11-27:

| Repository | Assets | Notes (body) | Raw JSON | Gzipped | Time |
|---|---:|---:|---:|---:|---:|
| sharkdp/bat | 21 | 1.5 KB | 38.4 KB | 4.4 KB | 0.12 s |
| obsproject/obs-studio | 12 | 12.3 KB | 35.0 KB | 6.5 KB | 0.12 s |
| zed-industries/zed | 14 | 13.5 KB | 38.9 KB | 7.3 KB | 0.12 s |
| BurntSushi/ripgrep | 34 | 6.3 KB | 66.4 KB | 7.0 KB | 0.12 s |

Response headers are about 1.35 KB. Taking each response apart, the release object without assets or body is
about **2.0 KB**, and **each asset adds about 1.7 KB**, mostly the uploader's user object with its URL
templates. The REST endpoint has no field selection, and GraphQL needs a token.

**xReveal's expected response:** 4 assets (`xReveal-X.Y.Z-macos-arm64.dmg` and `.zip`,
`xReveal-X.Y.Z-windows-x64-setup.exe` and `.zip`, from `scripts/package-*`), so ≈ 2.0 + 4 × 1.7 ≈ **8.8 KB plus
the notes**. That is roughly 1.5–3 KB gzipped (inference from the 5–9× ratios above). The TLS handshake adds a
few KB of certificates (inference). The body is the CHANGELOG.md section for that version, so it stays small
**as long as only that version's section goes in**. If the whole CHANGELOG.md became every release's body, each
check would download the full history, and the download would grow with every release.

Measured TLS timing to api.github.com from here: DNS 4 ms, connect 38 ms, TLS 79 ms, total 0.12–0.24 s over HTTP/2.

### Rejected: the web redirect instead of the API

`https://github.com/{owner}/{repo}/releases/latest` answers **302** with
`location: …/releases/tag/<tag>`. That gives the tag with no JSON and no API rate limit. But the measured headers
are 5.5 KB, more than the gzipped API body, and the redirect is website behaviour, not a documented API. Not
worth it.

## 2. Which HTTP/TLS client

### What's already linked (verified with `cargo tree`)

- **macOS and Windows: no HTTP client, no TLS library.** `cargo tree -i hyper|rustls|tokio|h2|ring
  --target aarch64-apple-darwin` and `--target x86_64-pc-windows-msvc` both print nothing.
  `Cargo.lock` lists `gpui-pre-reqwest 0.12.15` (with hyper, hyper-rustls, rustls, ring, tokio, h2), but
  `gpui-kit-assets-0.11.0/Cargo.toml` pulls it in only under `[target.'cfg(target_family = "wasm")'.dependencies]`.
- GPUI has an `HttpClient` trait (`gpui-pre-http-client`, just the trait plus `http`, `url`, `serde_json`).
  `App` starts with `Arc::new(NullHttpClient)`, whose `send` is `bail!("No HttpClient available")`
  (`gpui-pre-0.3.14/src/app.rs:246, 3717`). No implementation ships for native targets.
- `src/` has no `std::net`, sockets, or HTTP of any kind. The only "open a URL" calls launch `explorer
  ms-settings:…` and `open x-apple.systempreferences:…` (`src/permission.rs`,
  `src/platform/macos/access.rs`). **Verified: xReveal makes no network requests.** Spec §1 lists "An
  auto-updater, or any network access" as out of v1. This feature changes that line.
- `objc2-foundation 0.3.2` in xReveal's graph already has the features `NSURLSession`, `NSURLRequest`,
  `NSURLResponse`, `NSURL`, `NSURLError` and `block2` (`cargo tree -e features`). The bindings are compiled
  already, and only the call sites would be new.
- `windows 0.62` is a direct dependency. WinHTTP needs one more feature, `Win32_Networking_WinHttp`.
- Already linked and useful: `serde_json` (via GPUI) and `serde`. `semver` is linked on macOS only (through
  `gpui-pre-macos`), so the version comparison should be hand-written (§3).

### Binary size and memory (measured on macOS arm64)

A probe binary (`serde` + `serde_json`, fetches `sharkdp/bat`'s `releases/latest`, parses `tag_name` and
`html_url`) was built `--release` once per option, with the same toolchain. Footprint was read with
`footprint <pid>` about 0.5 s after the request finished.

| Option | Binary, stripped | Added vs. no client | Footprint after the request | Wall time |
|---|---:|---:|---:|---:|
| No client (baseline) | 0.47 MB | — | 0.9 MB | — |
| `ureq 3.5` + rustls (ring) + webpki-roots | 2.39 MB | **+1.92 MB** | 2.9 MB (+1.9) | 0.46 s |
| same + `gzip` (ureq's default features) | 2.45 MB | +1.99 MB | — | — |
| `ureq` + rustls + `platform-verifier` (OS roots) | 2.42 MB | +1.95 MB | 3.2 MB (+1.8) | 0.18 s |
| `ureq` + `native-tls` (Security.framework / SChannel) | 1.15 MB | **+0.68 MB** | 4.1 MB (+2.7) | 0.19 s |
| **`NSURLSession`** via `objc2-foundation` | 0.53 MB | **+0.06 MB** | 4.5 MB (+3.1) | 0.14 s |
| Shell out to `/usr/bin/curl` | 0.50 MB | +0.04 MB | 1.1 MB in-process, plus a curl child of 4.3 MB peak | 0.20 s |

For scale, today's `target/release/xreveal` is **29.1 MB**, and xReveal's footprint on the reference Mac is
~170 MB (#118). ureq with rustls would grow the binary by **6.6%**, ureq with native-tls by 2.3%, and the OS
stacks by about 0.2%. The NSURLSession row overstates xReveal's cost: the probe's 0.06 MB includes objc2
runtime glue that xReveal already links. The footprint numbers are first-use costs in a tiny process. The OS
stacks load CFNetwork/Security pages that the system shares. In every case the memory is a one-off, under 2% of
xReveal's footprint, happening once a day.

**Windows (inference; the Mac has no Windows linker).** A WinHTTP version of the same probe type-checks against
`windows 0.62` with `Win32_Networking_WinHttp` (`cargo check --target x86_64-pc-windows-msvc`). WinHTTP lives
in `winhttp.dll`, which ships with Windows, and the crate emits only the functions called, so the added size
should be tens of KB, like NSURLSession. ureq + rustls on Windows carries the same ring/rustls/webpki-roots
code, so expect about +1.9 MB there too.

### How the options behave

| | ureq + rustls | ureq + native-tls | NSURLSession / WinHTTP | `curl` child process |
|---|---|---|---|---|
| New crates | ureq, rustls, ring, webpki-roots, … | ureq, native-tls, security-framework / schannel | none (one `windows` feature) | none |
| System proxy (PAC, per-user) | No. Reads `ALL_PROXY`/`HTTPS_PROXY`/`HTTP_PROXY` env vars, which macOS GUI apps don't get. `win-system-proxy` reads only the static IE registry proxy (`ureq-3.5.2/src/proxy.rs`). | Same | **Yes.** NSURLSession: "The default value is `NULL`, which means that tasks use the default system settings" ([`connectionProxyDictionary`](https://developer.apple.com/documentation/foundation/urlsessionconfiguration/connectionproxydictionary)). WinHTTP `WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY`: "Uses system and per-user proxy settings (including the Internet Explorer proxy configuration) … Supported in Windows 8.1 and newer" ([WinHttpOpen](https://learn.microsoft.com/en-us/windows/win32/api/winhttp/nf-winhttp-winhttpopen)). | Env vars only |
| Corporate TLS inspection (custom root) | Fails: bundled Mozilla roots | Works: OS roots | Works: OS roots | Works on macOS; on Windows curl.exe uses Schannel |
| Gzip | Optional feature | Optional | NSURLSession: automatic. WinHTTP: opt in with `WINHTTP_OPTION_DECOMPRESSION` (8.1+); without it, ~9 KB plain | Flag |
| Code in xReveal | ~15 lines, both OSes | ~20 lines | ~50 lines per OS, `unsafe` FFI, behind `crate::platform` like the rest of xReveal's OS code | ~15 lines. A Windows GUI app needs `CREATE_NO_WINDOW`, or a console flashes. |
| Risk | — | — | — | Spawning `curl` from an unsigned app is a pattern security software looks for (inference). Proxy settings ignored. |

**What NSURLSession sends (measured).** Pointed at a local listener, an ephemeral NSURLSession sent
`Accept`, `User-Agent` (as set), `Connection: keep-alive`, `Accept-Encoding: gzip, deflate`, and
**`Accept-Language: en-GB,en-US;q=0.9,en;q=0.8`**, taken from the user's locale. Setting `Accept-Language: en`
on the request replaced it (measured). Do that, or the request leaks the user's language list, which goes
against "sending nothing but the request". With no `User-Agent` set, CFNetwork adds its own, which includes the
CFNetwork and Darwin versions (inference). Set it explicitly. WinHTTP sends only the agent string given to
`WinHttpOpen` and the headers added (inference from the API).

**Use the ephemeral configuration on macOS.** It "doesn't store caches, credential stores, or any
session-related data to disk. Instead, session-related data is stored in RAM"
([`URLSessionConfiguration.ephemeral`](https://developer.apple.com/documentation/foundation/urlsessionconfiguration/ephemeral)).
`NSURLSession.sharedSession` would leave a cache under `~/Library/Caches`.

**App sandbox / entitlements:** none needed. xReveal isn't sandboxed (`packaging/macos` has only `Info.plist`;
the outgoing-network entitlement matters only in the App Sandbox). App Transport Security allows
`https://api.github.com` (TLS 1.2+).

### Off the UI thread (verified in GPUI source)

- `cx.background_executor().spawn(future)` "Enqueues the given future to be run to completion on a background
  thread" (`gpui-pre-0.3.14/src/executor.rs:118`). On macOS that is GCD's global queue
  (`gpui-pre-apple/src/dispatcher.rs:46`). On Windows it is the system thread pool
  (`TrySubmitThreadpoolCallback`, `gpui-pre-windows/src/dispatcher.rs:68`). Both pools grow, so one blocking
  WinHTTP call (≤ the 10 s timeout) on a worker doesn't starve GPUI.
- **macOS:** NSURLSession is asynchronous by itself. The completion block runs on the session's own background
  queue and sends the result through a oneshot channel (`futures` is already linked). A foreground task
  (`cx.spawn`) awaits it and updates the footer. Nothing blocks.
- **Windows:** synchronous WinHTTP (no `WINHTTP_FLAG_ASYNC`) inside `cx.background_executor().spawn`, then the
  result goes back to the foreground task. Microsoft "strongly recommend[s]" async mode, but async mode needs a
  status callback state machine. For one request a day, a background worker is the lighter code.
- **"After the picture is up":** xReveal already delays work with `cx.background_executor().timer(…)` in
  `cx.spawn` (`src/app.rs:64`). Start the check a few seconds after the first frame is drawn. Open no
  connection before that, so capture start-up never competes with DNS/TLS.
- Opening the page: `cx.open_url(url)` (`gpui-pre/src/app.rs:2049`). On macOS that is
  `NSWorkspace.openURL`. On Windows it is `ShellExecuteW("open")`, already on the background executor
  (`gpui-pre-windows/src/platform.rs:764, 1522`). xReveal's own `explorer`/`open` calls could move to it too.

## 3. Version comparison

- Tags are `v` + `MAJOR.MINOR.PATCH`, with an optional `-pre`/`+build` suffix (the regex in
  `.github/workflows/release.yml`), and the tag must equal `Cargo.toml`'s version at that tag. So
  `env!("CARGO_PKG_VERSION")` is the running version, and `tag_name` minus `v` is the latest.
- Compare `(u64, u64, u64)` tuples. Treat anything that doesn't parse as "no update". About ten lines, no crate
  (`semver` isn't linked on Windows):

  ```rust
  fn triple(v: &str) -> Option<(u64, u64, u64)> {
      let mut it = v.strip_prefix('v').unwrap_or(v).split('.');
      let t = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
      it.next().is_none().then_some(t) // "1.2.3-beta.1" fails the third parse: ignored
  }
  // update when triple(tag) > triple(CARGO_PKG_VERSION)
  ```
- **A gap in `release.yml`:** it accepts `v1.1.0-beta.1` and runs `gh release create` without `--prerelease`,
  so a beta would be a full release and `releases/latest` would return it. The `triple` above ignores it, and the
  footer stays quiet. But if prereleases should never be "latest", `release.yml` should pass `--prerelease`
  when the tag has a `-`.
- Running a newer build than the latest release (a dev build): the comparison is `>`, so nothing shows.

## 4. Where state lives

`src/setup.rs` already gives what's needed. One `setup.toml` per machine (macOS `~/Library/Application
Support/xReveal/`, Windows `%LOCALAPPDATA%\xReveal\`), `#[serde(default)]` everywhere, atomic save about a
second after a change. A new table:

```toml
[updates]
check = true            # the "Check for updates" switch
last-check = 1795791647 # Unix seconds of the last answered check (200 or 404)
latest = "1.2.0"        # newest version seen, so the footer shows it at once on the next launch, offline too
```

- **When to check:** `check` is on, and either `now − last-check ≥ 24 h` or `last-check` is in the future (the
  clock was set back). Use `SystemTime`. Store seconds, not a date string.
- **The footer** shows **Update to {latest}** whenever `latest > CARGO_PKG_VERSION`, with no network needed.
  After the user updates, the running version catches up and the line goes back to the plain version. Nothing
  needs clearing.
- `Setup`'s doc says it holds "choices xReveal applies on launch". `last-check` and `latest` are state, not
  choices. Keeping them in `setup.toml` is still the least machinery. A separate file would mean a second
  reader, writer and path.

## 5. Failure behaviour

Everything fails silently, with no dialog, toast or log noise for the user:

| Case | What happens | Record `last-check`? |
|---|---|---|
| 200, newer tag | store `latest`, footer becomes **Update to x.y.z** | yes |
| 200, same or older, or a `-pre` tag | nothing | yes |
| 404 (no release yet) | nothing | yes |
| 403 / 429 (rate limit), 5xx | nothing | no: try next launch |
| Offline, DNS, proxy refused, TLS error, timeout | nothing | no |
| JSON without a parseable `tag_name` | nothing | yes (a broken answer won't fix itself in an hour) |

- **Timeouts:** 10 s (`NSURLRequest.timeoutInterval`, `WinHttpSetTimeouts`). Offline, NSURLSession fails at once
  with `waitsForConnectivity` at its default of `false` (inference from Apple's docs).
- **Proxies:** both OS stacks use the system proxy, so most proxied networks just work (§2). An authenticating
  proxy that wants credentials fails, silently.
- **Read limit:** cap the body at 256 KB, so a misbehaving middlebox can't make xReveal buffer a lot.
- **A long session:** a single check per launch never re-checks while xReveal stays open for days. A 24 h timer
  in the same task would cover that, at no measurable cost.

## 6. What to open

| | Release page `…/releases/tag/vX.Y.Z` | Asset (`browser_download_url`) |
|---|---|---|
| Shows the notes (CHANGELOG.md section) | yes | no |
| Installer vs portable | user picks | xReveal must guess: Windows installed copy → `-setup.exe`, else `.zip`; macOS `.dmg` vs `.zip` |
| Coupled to asset names | no | yes, if `xReveal-X.Y.Z-<os>-<arch>.<ext>` ever changes, the link breaks |
| Clicks to update | 2 (open page, click asset) | 1, but a file lands in Downloads without context |

**Recommend the release page on both OSes.** Build the URL from the tag:
`https://github.com/pwn233/xReveal/releases/tag/{tag_name}`. That equals the response's `html_url` (verified on
the measured repos). Building it from the tag means xReveal only ever opens a page under its own repository's
releases, whatever the response contains.

## 7. Recommendation

**Use the OS's own HTTP stack behind one small seam** (`crate::platform::latest_release_tag() -> Option<String>`).
macOS uses an ephemeral `NSURLSession` with `User-Agent: xReveal` and `Accept-Language: en`. Windows uses
synchronous WinHTTP with `WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY` on GPUI's background executor. Both send
`Accept: application/vnd.github+json`, time out after 10 s, read at most 256 KB, and parse `tag_name` with
`serde_json`.

| Cost | Value |
|---|---|
| Binary | ~+0.06 MB on macOS (measured upper bound), tens of KB on Windows (inference). Against +1.9 MB for ureq/rustls. |
| New dependencies | none. One `windows` feature. |
| Network | 1 request a day, ~2–3 KB gzipped body + ~1.4 KB headers + TLS handshake. 1 of the 60/hour per-IP limit. |
| Memory | ~3 MB one-off at first use (measured in a tiny process), <2% of xReveal's ~170 MB |
| CPU / time | one TLS handshake, ~0.14–0.24 s wall here, off the UI thread |
| Code | ~50 lines per OS + ~40 shared (schedule, compare, setup fields, footer) |

Second choice, if per-OS FFI is unwelcome: **`ureq` with `native-tls-no-default`** (+0.68 MB, OS roots, one code
path for both OSes). It doesn't follow system proxy settings on either OS. ureq with rustls is the heaviest
option and the worst at corporate TLS inspection, so it is ruled out.

## Things #138 should decide

1. **The User-Agent string:** `xReveal` alone, or `xReveal/x.y.z`? The version tells GitHub what xReveal runs,
   and nothing about the user. Nothing else is sent: no locale (`Accept-Language: en` forced), no OS version.
2. **A failed check:** retry on every launch until one succeeds (recommended above), or count it as the day's
   check?
3. **Turning the switch on** in the drawer: check immediately, or wait for the next launch? Turning it off:
   hide an already-found **Update to x.y.z**, or keep showing it?
4. **A session open for days:** re-check every 24 h while running, or once per launch only?
5. **Prereleases:** should `release.yml` mark `-` tags `--prerelease` (they then never appear as "latest"), or
   is client-side ignoring enough?
6. **The release body:** only that version's CHANGELOG.md section, so the daily download doesn't grow with
   history. `release.yml` currently uses `--generate-notes`.
7. **Spec §1** says "An auto-updater, or any network access" is out of v1. That line needs rewording: one opt-out
   daily request to api.github.com.
8. **`last-check` / `latest` in `setup.toml`** (simplest), even though `Setup` is described as choices.
