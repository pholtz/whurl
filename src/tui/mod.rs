//! Interactive view: pick a request, fire it, inspect the response.

mod app;
mod clipboard;
mod cmdline;
mod commands;
mod help;
mod jobs;
mod mouse;
mod settings;
mod state;
mod tree;
mod views;

use crate::config::Config;
use crate::workspace::Workspace;
use anyhow::Result;
use app::{App, Mode};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Whether the terminal currently reports mouse events to us.
static CAPTURED: AtomicBool = AtomicBool::new(false);

/// Turn mouse capture on or off, if that changes anything.
fn set_capture(on: bool) {
    if CAPTURED.swap(on, Ordering::SeqCst) != on {
        use ratatui::crossterm::{event::{DisableMouseCapture, EnableMouseCapture}, execute};
        let _ = if on { execute!(std::io::stdout(), EnableMouseCapture) } else { execute!(std::io::stdout(), DisableMouseCapture) };
    }
}

pub fn run(ws: Workspace, cfg: Config, query: Option<String>, env: String) -> Result<()> {
    let mut app = App::new(ws, cfg, query, env)?;
    let mut term = ratatui::init();
    // ratatui's hook restores the terminal on panic but leaves mouse capture on.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        set_capture(false);
        hook(info)
    }));
    let res = event_loop(&mut app, &mut term);
    app.shutdown();
    set_capture(false);
    ratatui::restore();
    res
}

fn event_loop(app: &mut App, term: &mut DefaultTerminal) -> Result<()> {
    while !app.quit {
        set_capture(app.cfg.ui.mouse);
        term.draw(|f| views::draw(app, f))?;
        if event::poll(Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => on_key(app, k),
                Event::Mouse(m) => mouse::on_mouse(app, m),
                _ => {}
            }
        }
        while let Ok(msg) = app.rx.try_recv() {
            app.on_msg(msg);
        }
        if let Some((file, cmd)) = app.viewing.take() {
            match suspend(term, &cmd, file.path()) {
                Ok(s) if !s.success() => {
                    app.say(format!("viewer exited with {}", s.code().map_or("a signal".into(), |c| c.to_string())), true)
                }
                Ok(_) => {}
                Err(e) => app.say(format!("{cmd}: {e}"), true),
            }
        }
        if let Some(path) = app.edit.take() {
            let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
            if let Err(e) = suspend(term, &editor, &path) {
                app.say(format!("editor: {e}"), true);
            }
            // A new request left empty was abandoned.
            if app.created.take().as_ref() == Some(&path)
                && std::fs::read_to_string(&path).is_ok_and(|s| s.trim().is_empty())
                && std::fs::remove_file(&path).is_ok()
            {
                let top = app.ws.requests_dir();
                for d in path.ancestors().skip(1).take_while(|d| d.starts_with(&top) && *d != top) {
                    let _ = std::fs::remove_dir(d); // only if the new request left it empty
                }
                app.say("empty request discarded", false);
            }
            if Some(&path) == crate::config::Config::path().as_ref() {
                app.reload_config();
            } else {
                app.reload();
            }
        }
        app.tick = app.tick.wrapping_add(1);
    }
    Ok(())
}

/// Leave the TUI to run `cmd <path>` through the shell, then come back.
fn suspend(term: &mut DefaultTerminal, cmd: &str, path: &std::path::Path) -> std::io::Result<std::process::ExitStatus> {
    set_capture(false);
    ratatui::restore();
    let status = std::process::Command::new("sh").arg("-c").arg(format!("{cmd} \"$1\"")).arg("sh").arg(path).status();
    *term = ratatui::init();
    status
}

/// Overlays and text inputs see keys first; everything else goes through the registry.
fn on_key(app: &mut App, k: KeyEvent) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && k.code == KeyCode::Char('c') {
        app.quit = true;
        return;
    }
    match app.mode {
        Mode::Help => help::on_key(app, &k),
        Mode::Command => match k.code {
            KeyCode::Esc => app.mode = Mode::Normal,
            KeyCode::Enter => cmdline::submit(app),
            KeyCode::Tab => cmdline::accept(app),
            KeyCode::Down | KeyCode::BackTab => cmdline::move_sel(app, 1),
            KeyCode::Up => cmdline::move_sel(app, -1),
            KeyCode::Char('n' | 'j') if ctrl => cmdline::move_sel(app, 1),
            KeyCode::Char('p' | 'k') if ctrl => cmdline::move_sel(app, -1),
            KeyCode::Backspace if app.cmdline.input.is_empty() => app.mode = Mode::Normal,
            KeyCode::Backspace => {
                app.cmdline.input.pop();
                cmdline::edited(app);
            }
            KeyCode::Char('u') if ctrl => {
                app.cmdline.input.clear();
                cmdline::edited(app);
            }
            KeyCode::Char('w') if ctrl => {
                let s = app.cmdline.input.trim_end();
                let keep = s.rfind(char::is_whitespace).map_or(0, |i| i + 1);
                app.cmdline.input.truncate(keep);
                cmdline::edited(app);
            }
            KeyCode::Char(c) if !ctrl => {
                app.cmdline.input.push(c);
                cmdline::edited(app);
            }
            _ => {}
        },
        Mode::Picker => match k.code {
            KeyCode::Esc => app.mode = Mode::Normal,
            KeyCode::Enter => app.picker_choose(),
            KeyCode::Down => app.picker_move(1),
            KeyCode::Up => app.picker_move(-1),
            KeyCode::Char('n' | 'j') if ctrl => app.picker_move(1),
            KeyCode::Char('p' | 'k') if ctrl => app.picker_move(-1),
            KeyCode::PageDown => app.picker_move(10),
            KeyCode::PageUp => app.picker_move(-10),
            KeyCode::Char('u') if ctrl => {
                app.picker.query.clear();
                app.picker_refilter();
            }
            KeyCode::Backspace => {
                app.picker.query.pop();
                app.picker_refilter();
            }
            KeyCode::Char(c) if !ctrl => {
                app.picker.query.push(c);
                app.picker_refilter();
            }
            _ => {}
        },
        Mode::Filter => match k.code {
            KeyCode::Esc => {
                app.filter.clear();
                app.mode = Mode::Normal;
                app.refilter(false);
            }
            KeyCode::Enter => app.mode = Mode::Normal,
            KeyCode::Backspace => {
                app.filter.pop();
                app.refilter(true);
            }
            KeyCode::Down => app.move_sel(1),
            KeyCode::Up => app.move_sel(-1),
            KeyCode::Char(c) if !ctrl => {
                app.filter.push(c);
                app.refilter(true);
            }
            _ => {}
        },
        Mode::Normal => {
            commands::dispatch(app, &k);
        }
    }
}
