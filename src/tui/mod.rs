//! Interactive view: pick a request, fire it, inspect the response.

mod app;
mod commands;
mod jobs;
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
        if let Some(path) = app.edit.take() {
            ratatui::restore();
            let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
            let status = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("{editor} \"$1\""))
                .arg("sh")
                .arg(&path)
                .status();
            *term = ratatui::init();
            if let Err(e) = status {
                app.say(format!("editor: {e}"), true);
            }
            app.reload();
        }
        app.tick = app.tick.wrapping_add(1);
    }
    Ok(())
}

/// Overlays and text inputs see keys first; everything else goes through the registry.
fn on_key(app: &mut App, k: KeyEvent) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && k.code == KeyCode::Char('c') {
        app.quit = true;
        return;
    }
    match app.mode {
        Mode::Help => {
            if matches!(k.code, KeyCode::Esc | KeyCode::Char('?' | 'q') | KeyCode::Enter) {
                app.mode = Mode::Normal;
            }
        }
        Mode::Filter => match k.code {
            KeyCode::Esc => {
                app.filter.clear();
                app.mode = Mode::Normal;
                app.refilter();
            }
            KeyCode::Enter => app.mode = Mode::Normal,
            KeyCode::Backspace => {
                app.filter.pop();
                app.refilter();
            }
            KeyCode::Down => app.move_sel(1),
            KeyCode::Up => app.move_sel(-1),
            KeyCode::Char(c) if !ctrl => {
                app.filter.push(c);
                app.refilter();
            }
            _ => {}
        },
        Mode::Normal => {
            commands::dispatch(app, &k);
        }
    }
}
