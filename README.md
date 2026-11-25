# xReveal

A desktop app written in Rust. The UI is built with [GPUI Kit](https://gpui-kit.com), which bundles
[GPUI](https://www.gpui.rs/) (Zed's GPU-accelerated UI framework) and the `gpui-component` library.

## Requirements

- Rust 1.93 or newer. `rust-toolchain.toml` pins the version, and rustup installs it on the first build.
- macOS: Xcode Command Line Tools (`xcode-select --install`).
- Linux: the usual GPUI system libraries (X11/Wayland, xkbcommon, fontconfig, Vulkan).

## Run

```sh
cargo run
```

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

`use gpui_kit::*;` brings in GPUI itself. Components are under `gpui_kit::component::*`
(see the [component catalog](https://gpui-kit.com/component)).
