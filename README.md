# xReveal

A lightweight, low-latency viewer for playing a console live through a capture card, for Windows and
macOS. Written in Rust; the UI is built with [GPUI Kit](https://gpui-kit.com), which bundles
[GPUI](https://www.gpui.rs/) (Zed's GPU-accelerated UI framework) and the `gpui-component` library.

What v1 does and how: [`docs/spec/v1.md`](docs/spec/v1.md). Terms: [`GLOSSARY.md`](GLOSSARY.md).
Architecture decisions: [`docs/adr/`](docs/adr/).

## Requirements

- Rust 1.93 or newer. `rust-toolchain.toml` pins the version, and rustup installs it on the first build.
- macOS: full Xcode (GPUI compiles its Metal shaders at build time with `xcrun metal`).
- Windows: Visual Studio Build Tools with the Windows SDK (release builds use its `fxc.exe`).
- Checking the Windows build from a Mac: `brew install llvm` (for `llvm-rc`).

## Run

```sh
cargo run
```

## Run as a macOS app (needed for capture)

macOS grants camera and microphone access to the app that launches xReveal, and kills it on its
first capture if that app doesn't declare why it needs them. `scripts/dev-app.sh` builds xReveal,
wraps it in a minimal `xReveal.app` with those declarations, signs it and launches it with `open`:

```sh
scripts/dev-app.sh            # debug build
scripts/dev-app.sh --release
```

Sign with an Apple Development identity so macOS remembers the permission across rebuilds: in
Xcode › Settings › Accounts add your Apple ID (a free Personal Team is enough), then Manage
Certificates › + › Apple Development. Without one the script ad-hoc signs and macOS may ask again
after each build. `XREVEAL_SIGN_IDENTITY` picks a specific identity.

## Check the Windows build from a Mac

All development happens on a Mac. Windows code is type-checked and linted locally, then built and
tested on Windows CI:

```sh
scripts/check-win.sh                  # clippy for x86_64-pc-windows-msvc
scripts/check-win.sh -- -D warnings   # as strict as CI
```

`bacon check-win` runs the same thing on every save.

## Develop with auto-reload

[bacon](https://dystroy.org/bacon/) rebuilds and relaunches the window every time you save:

```sh
cargo install bacon --locked   # one time
bacon                          # runs the `run` job from bacon.toml
```

A rebuild takes a few seconds. The process restarts, so in-memory UI state (such as the counter)
resets on each reload. While bacon is running, press `c` for clippy, `t` for tests, `r` to go back
to run, and `q` to quit.

## Layout

| Path          | Purpose                                                 |
| ------------- | ------------------------------------------------------- |
| `src/main.rs` | Starts the app, initializes GPUI Kit, opens the window  |
| `src/app.rs`  | `XReveal` root view (title bar, sample controls, theme) |
| `scripts/`    | `dev-app.sh` (macOS .app launcher), `check-win.sh` (Windows dry lab) |
| `packaging/`  | Platform packaging files (macOS `Info.plist`)            |
| `.github/workflows/ci.yml` | Build, clippy and tests on macOS and Windows for every PR |

`use gpui_kit::*;` brings in GPUI itself. Components are under `gpui_kit::component::*`
(see the [component catalog](https://gpui-kit.com/component)).

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT)
at your option.
