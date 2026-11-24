//! whurl-gui: a native window over a whurl workspace. Same `requests/`, `env/` and `.whurl/` as
//! the CLI and TUI, through whurl-core, so runs made here show up in `whurl history` and back.
//!
//! `whurl-gui [--workspace DIR] [-e ENV]`; `WHURL_WORKSPACE` and `HURL_ENV` work as in whurl.

mod app;
mod commands;
mod hurl;
mod view;

use app::State;
use rune::{App, AppSpec, Key, KeyEvent, Tx, Waker};
use std::path::PathBuf;
use std::sync::Mutex;
use whurl_core::config::Config;
use whurl_core::workspace::Workspace;

/// Startup from the command line, handed to `init` (a fn pointer, so it can't capture).
type Boot = Result<(Workspace, Config, String), String>;
static BOOT: Mutex<Option<Boot>> = Mutex::new(None);

fn init() -> State {
    let boot = BOOT
        .lock()
        .unwrap()
        .take()
        .unwrap_or_else(|| Err("not started".into()));
    match boot {
        Ok((ws, cfg, env)) => State::new(ws, cfg, env, None),
        Err(e) => State::new(
            Workspace {
                root: PathBuf::new(),
            },
            Config::default(),
            String::new(),
            Some(e),
        ),
    }
}

fn setup(s: &mut State, w: Waker) {
    s.waker = Some(w);
}

fn poll(s: &mut State, t: f64, _tx: &mut Tx) -> bool {
    let changed = s.drain();
    if s.fatal.is_some() {
        return changed;
    }
    let before = s.requests.len();
    s.watch(t);
    s.persist();
    changed || s.requests.len() != before
}

fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if s.fatal.is_some() {
        return false;
    }
    // Escape closes whatever is on top.
    if ev.key == Key::Escape {
        if s.palette.take().is_some() {
            s.query.focused = false;
        } else if s.naming.take().is_some() || s.confirm.take().is_some() {
        } else if s.env_edit.is_some() {
            s.close_env_edit();
        } else if s.settings_open {
            s.settings_open = false;
        } else if s.menu_open {
            s.menu_open = false;
        } else if s.method_open {
            s.method_open = false;
        } else {
            return false;
        }
        return true;
    }
    // Dialogs take their own keys; the palette still lets ⌘K/⌘P switch modes.
    if s.naming.is_some() || s.confirm.is_some() || s.settings_open || s.env_edit.is_some() {
        return false;
    }
    commands::dispatch(s, ev, tx)
}

/// `--workspace DIR` / `-e ENV` from the arguments Rune didn't take.
fn args() -> (Option<PathBuf>, Option<String>) {
    let mut ws = std::env::var_os("WHURL_WORKSPACE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let mut env = std::env::var("HURL_ENV").ok().filter(|e| !e.is_empty());
    let mut it = rune::extra_args().iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--workspace" | "-w" => ws = it.next().map(PathBuf::from),
            "--env" | "-e" => env = it.next().cloned(),
            s if s.starts_with("--workspace=") => ws = Some(PathBuf::from(&s[12..])),
            s if s.starts_with("--env=") => env = Some(s[6..].to_owned()),
            _ => {}
        }
    }
    (ws, env)
}

fn boot() -> Boot {
    let (ws, env) = args();
    let cfg = Config::load().map_err(|e| format!("{e:#}"))?;
    let ws = Workspace::discover(ws, cfg.workspace()).map_err(|e| format!("{e:#}"))?;
    let env = cfg.env(env);
    Ok((ws, cfg, env))
}

fn main() {
    *BOOT.lock().unwrap() = Some(boot());
    // The Dock and app-switcher icon (a bundled .app's Info.plist icon would be used without it).
    let _ = rune::set_app_icon(include_bytes!("../assets/icon.png").as_slice());
    App::new(AppSpec {
        min_size: Some((640.0, 400.0)),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        families: ("Inter", "ui-monospace"),
        ..AppSpec::new("whurl", (1100.0, 720.0), init, view::view)
    })
    // AppKit saves the window's size and place and reopens it there (no-op off macOS).
    .on_window(|_| {
        rune_mac::remember_frame("whurl-gui");
    })
    // Saves the draft and UI state when the window closes or the app quits.
    .on_close(|s: &mut State, _| {
        s.save();
        s.persist();
    })
    .run();
}
