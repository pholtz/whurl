//! Interactive view: pick a request, fire it, inspect the response.

mod app;
mod cmdline;
mod commands;
mod help;
mod jobs;
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
use std::time::Duration;

pub fn run(ws: Workspace, cfg: Config, query: Option<String>, env: String) -> Result<()> {
    let mut app = App::new(ws, cfg, query, env)?;
    let mut term = ratatui::init();
    let res = event_loop(&mut app, &mut term);
    app.shutdown();
    ratatui::restore();
    res
}

fn event_loop(app: &mut App, term: &mut DefaultTerminal) -> Result<()> {
    while !app.quit {
        term.draw(|f| views::draw(app, f))?;
        if event::poll(Duration::from_millis(80))?
            && let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
        {
            on_key(app, k);
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
