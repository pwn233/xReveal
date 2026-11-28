//! The driver's control channel (#204, decided in #196): unattended runs drive xReveal through
//! stdin, so one script works on Windows and macOS with no OS input permissions.
//!
//! Built into every build, active only when `XREVEAL_DRIVE=1` (docs/spec/v1.md §5). It reads one
//! command per line on stdin and answers each with one line on stdout starting `drive:`, so answers
//! can be told from other output:
//!
//! - `key <key>`: one of xReveal's own keys ([`KEYS`]: `tab`, `escape`, `f3`, `f11`, `ctrl-cmd-f`,
//!   the arrows, `enter`, `space`), through GPUI's own input path ([`Window::dispatch_keystroke`]):
//!   key-binding matching, focus and the action handlers run as for a real key. Only the OS's key
//!   event and its translation are skipped. It goes to the window that has xReveal's keys: the
//!   drawer's own window while it's open there (Windows), else the main one.
//! - `move <x> <y>` / `click <x> <y>`: the pointer moves to (or left-clicks at) a point in the main
//!   window's content, in logical pixels, through [`Window::dispatch_event`]: hit-testing, hover and
//!   click handlers run. A point over the drawer's own window goes to that window.
//! - `state`: one JSON line ([`XReveal::drive_state`]) with what a script needs to see that a step
//!   has finished.
//! - `quit`: quits as Cmd+Q does.
//!
//! Commands arrive from a thread that blocks on stdin, through a channel the UI task awaits: a
//! driven xReveal that isn't being sent anything is idle.

use std::io::{BufRead as _, Write as _};

use futures::StreamExt as _;
use gpui_kit::*;

use crate::app::XReveal;

/// The keys a driver may press: xReveal's own bindings (src/app.rs, src/drawer/nav.rs).
/// `Keystroke::parse` wants a `&'static str`: keys are looked up here rather than leaking a string
/// per command, which a leak-per-cycle measurement would see.
const KEYS: &[&str] = &["tab", "escape", "f3", "f11", "ctrl-cmd-f", "up", "down", "left", "right", "shift-left", "shift-right", "enter", "space"];

/// Whether this run is driven: `XREVEAL_DRIVE=1`.
pub fn wanted() -> bool {
    std::env::var_os("XREVEAL_DRIVE").is_some_and(|v| v == "1")
}

#[derive(Debug, PartialEq)]
enum Command {
    Key(&'static str),
    Move(f32, f32),
    Click(f32, f32),
    State,
    Quit,
}

fn parse(line: &str) -> Result<Command, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["key", key] => KEYS.iter().find(|k| *k == key).map(|k| Command::Key(k)).ok_or_else(|| format!("unknown key {key}")),
        [verb @ ("move" | "click"), x, y] => match (x.parse::<f32>(), y.parse::<f32>()) {
            (Ok(x), Ok(y)) if x.is_finite() && y.is_finite() => Ok(if *verb == "move" { Command::Move(x, y) } else { Command::Click(x, y) }),
            _ => Err(format!("bad position {x} {y}")),
        },
        ["state"] => Ok(Command::State),
        ["quit"] => Ok(Command::Quit),
        _ => Err(format!("unknown command {line:?}")),
    }
}

/// One answer line. A driver that has gone away (a closed pipe) isn't xReveal's problem.
fn say(answer: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "drive: {answer}");
    let _ = out.flush();
}

/// Starts reading commands from stdin and running them in `window`, the main window.
pub fn start(window: &mut Window, cx: &mut Context<XReveal>) {
    // One line for the run's log, not one per command (#198 adds the log file).
    eprintln!("xReveal is driven (XREVEAL_DRIVE=1): commands on stdin, answers on stdout");
    // One line in the log for the whole run, never one per command (docs/spec/v1.md §2.11).
    crate::log::info("drive", "driven (XREVEAL_DRIVE=1): commands on stdin, answers on stdout");
    let (sender, mut commands) = futures::channel::mpsc::unbounded::<String>();
    let reader = std::thread::Builder::new().name("xreveal-drive".into()).spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if sender.unbounded_send(line).is_err() {
                return;
            }
        }
    });
    if let Err(error) = reader {
        eprintln!("xReveal: the driver's stdin reader didn't start: {error}");
        return;
    }
    cx.spawn_in(window, async move |this, cx| {
        say("ready");
        // Ends when stdin closes: the driver has gone.
        while let Some(line) = commands.next().await {
            let answer = match parse(&line) {
                Ok(Command::Key(key)) => {
                    let keystroke = Keystroke::parse(key).expect("KEYS are valid keystrokes");
                    // Outside any entity update: the action handlers update XReveal themselves.
                    let target = this.read_with(cx, |this, _| this.keys_window()).ok().flatten();
                    let handled = cx.update(|window, cx| match target {
                        Some(drawer) => drawer.update(cx, |_, window, cx| window.dispatch_keystroke(keystroke, cx)).unwrap_or(false),
                        None => window.dispatch_keystroke(keystroke, cx),
                    });
                    handled.map_or_else(|_| "error window gone".into(), |handled| format!("ok key {key} handled={handled}"))
                }
                Ok(Command::Move(x, y)) => pointer_command(&this, point(px(x), px(y)), false, cx),
                Ok(Command::Click(x, y)) => pointer_command(&this, point(px(x), px(y)), true, cx),
                Ok(Command::State) => this.update_in(cx, |this, window, cx| this.drive_state(window, cx)).unwrap_or_else(|_| "error window gone".into()),
                Ok(Command::Quit) => {
                    say("ok quit");
                    let _ = cx.update(|_, cx| cx.quit());
                    return;
                }
                Err(error) => format!("error {error}"),
            };
            say(&answer);
        }
    })
    .detach();
}

/// `move` or `click` at `position` in the main window's content: sent to the window under it.
fn pointer_command(this: &WeakEntity<XReveal>, position: Point<Pixels>, click: bool, cx: &mut AsyncWindowContext) -> String {
    let target = this.update_in(cx, |this, window, _| this.pointer_target(position, window)).ok().flatten();
    let done = cx.update(|window, cx| match target {
        Some((drawer, inside)) => drawer.update(cx, |_, window, cx| pointer(window, inside, click, cx)).is_ok(),
        None => {
            pointer(window, position, click, cx);
            true
        }
    });
    let verb = if click { "click" } else { "move" };
    match done {
        Ok(true) => format!("ok {verb} {} {}", position.x.as_f32(), position.y.as_f32()),
        _ => "error window gone".into(),
    }
}

/// Moves the pointer to `position` in `window`, and left-clicks there if `click`.
fn pointer(window: &mut Window, position: Point<Pixels>, click: bool, cx: &mut App) {
    let modifiers = Modifiers::default();
    window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers }), cx);
    if click {
        let down = MouseDownEvent { button: MouseButton::Left, position, modifiers, click_count: 1, first_mouse: false };
        window.dispatch_event(PlatformInput::MouseDown(down), cx);
        let up = MouseUpEvent { button: MouseButton::Left, position, modifiers, click_count: 1 };
        window.dispatch_event(PlatformInput::MouseUp(up), cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, KEYS, parse};

    #[test]
    fn commands_are_parsed() {
        assert_eq!(parse("key f11"), Ok(Command::Key("f11")));
        assert_eq!(parse("  key   ctrl-cmd-f "), Ok(Command::Key("ctrl-cmd-f")));
        assert_eq!(parse("move 20 680.5"), Ok(Command::Move(20.0, 680.5)));
        assert_eq!(parse("click 1 2"), Ok(Command::Click(1.0, 2.0)));
        assert_eq!(parse("state"), Ok(Command::State));
        assert_eq!(parse("quit"), Ok(Command::Quit));
    }

    #[test]
    fn anything_else_is_an_error() {
        for line in ["", "key", "key cmd-q", "key f3 f3", "move 1", "move a b", "click NaN 2", "state now", "launch"] {
            assert!(parse(line).is_err(), "{line:?} should be an error");
        }
    }

    #[test]
    fn every_key_is_a_keystroke() {
        for key in KEYS {
            assert!(gpui_kit::Keystroke::parse(key).is_ok(), "{key}");
        }
    }
}
