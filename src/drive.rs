//! PROTOTYPE for #196, never merged: a local control channel for unattended runs.
//!
//! Built only with `--features drive`, and active only when `XREVEAL_DRIVE=1`. It reads one command
//! per line on stdin and answers on stdout, so the same driver script works on Windows and macOS
//! with no OS input permissions:
//!
//! - `key <keystroke>`: a keystroke as GPUI parses it (`f3`, `tab`, `escape`, `right`, `ctrl-cmd-f`),
//!   dispatched into GPUI's own input path ([`Window::dispatch_keystroke`]): key-binding matching,
//!   focus and the action handlers all run as for a real key. Only the OS's key event and its
//!   translation are skipped.
//! - `move <x> <y>` / `click <x> <y>`: a mouse move or a left click at window coordinates (logical
//!   pixels), through [`Window::dispatch_event`]: hit-testing, hover and click handlers run.
//! - `state`: one JSON line with what a script needs to see that a step has finished.
//! - `quit`: quits as Cmd+Q does.
//!
//! Every answer is one line starting `drive:`, so it can be told from other output.

use std::io::BufRead as _;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use gpui_kit::*;

use crate::app::XReveal;

/// The keys a driver may press: xReveal's own bindings (src/app.rs, src/drawer/nav.rs).
const KEYS: &[&str] = &["tab", "escape", "f3", "f11", "ctrl-cmd-f", "up", "down", "left", "right", "shift-left", "shift-right", "enter", "space"];

/// Whether this run is driven: the feature is built in and `XREVEAL_DRIVE=1`.
pub fn wanted() -> bool {
    std::env::var_os("XREVEAL_DRIVE").is_some_and(|v| v == "1")
}

/// Starts reading commands from stdin and running them in `window`.
pub fn start(window: &mut Window, cx: &mut Context<XReveal>) {
    let (sender, receiver) = mpsc::channel::<String>();
    std::thread::Builder::new()
        .name("xreveal-drive".into())
        .spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    return;
                }
            }
        })
        .ok();
    println!("drive: ready");
    cx.spawn_in(window, async move |this, cx| run(this, receiver, cx).await).detach();
}

async fn run(this: WeakEntity<XReveal>, receiver: Receiver<String>, cx: &mut AsyncWindowContext) {
    loop {
        let line = match receiver.try_recv() {
            Ok(line) => line,
            Err(TryRecvError::Empty) => {
                cx.background_executor().timer(Duration::from_millis(10)).await;
                continue;
            }
            Err(TryRecvError::Disconnected) => return,
        };
        let mut words = line.split_whitespace();
        let answer = match (words.next(), words.next(), words.next()) {
            // `Keystroke::parse` wants a `&'static str`: look the key up rather than leak a string
            // per command, which would show up in a leak-per-cycle measurement.
            (Some("key"), Some(key), None) => match KEYS.iter().find(|k| **k == key).and_then(|k| Keystroke::parse(k).ok()) {
                // Outside any entity update: the action handlers update XReveal themselves.
                Some(keystroke) => cx.update(|window, cx| format!("ok key {key} handled={}", window.dispatch_keystroke(keystroke, cx))).unwrap_or_else(|_| "error window gone".into()),
                None => format!("error unknown key {key}"),
            },
            (Some(verb @ ("move" | "click")), Some(x), Some(y)) => match (x.parse::<f32>(), y.parse::<f32>()) {
                (Ok(x), Ok(y)) => {
                    let position = point(px(x), px(y));
                    cx.update(|window, cx| {
                        window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }), cx);
                        if verb == "click" {
                            let down = MouseDownEvent { button: MouseButton::Left, position, modifiers: Modifiers::default(), click_count: 1, first_mouse: false };
                            window.dispatch_event(PlatformInput::MouseDown(down), cx);
                            let up = MouseUpEvent { button: MouseButton::Left, position, modifiers: Modifiers::default(), click_count: 1 };
                            window.dispatch_event(PlatformInput::MouseUp(up), cx);
                        }
                    })
                    .map_or_else(|_| "error window gone".into(), |()| format!("ok {verb} {x} {y}"))
                }
                _ => format!("error bad position {x} {y}"),
            },
            (Some("state"), None, None) => this
                .update_in(cx, |this, window, _| this.drive_state(window))
                .unwrap_or_else(|_| "error window gone".into()),
            (Some("quit"), None, None) => {
                println!("drive: ok quit");
                let _ = cx.update(|_, cx| cx.quit());
                return;
            }
            _ => format!("error unknown command {line:?}"),
        };
        println!("drive: {answer}");
    }
}
