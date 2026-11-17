//! Rendering. Views read `App` and never change selection or run state.

mod content;
mod help;
mod picker;
mod sidebar;
mod summary;
pub mod theme;

use super::app::{App, Mode};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::time::Duration;
use theme::{ACCENT, SPINNER};

pub fn draw(app: &mut App, f: &mut Frame) {
    let [main, foot] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
    let right = if app.show_sidebar {
        let list_w = app.requests.iter().map(|r| r.rel.len()).max().unwrap_or(20) as u16 + 9;
        let list_w = list_w.min(main.width * 2 / 5).max(24);
        let [left, right] = Layout::horizontal([Constraint::Length(list_w), Constraint::Min(0)]).areas(main);
        sidebar::draw(app, f, left);
        right
    } else {
        main
    };
    let [summary, content] = Layout::vertical([Constraint::Length(6), Constraint::Min(0)]).areas(right);
    summary::draw(app, f, summary);
    content::draw(app, f, content);
    draw_footer(app, f, foot);
    match app.mode {
        Mode::Help => help::draw(app, f),
        Mode::Picker => picker::draw(app, f),
        _ => {}
    }
}

/// Right side of the statusbar: workspace, env badge, asserts, and in-flight runs.
fn state_line(app: &App) -> Line<'static> {
    let env = app.env().to_string();
    let env_style = match env.as_str() {
        "production" => Style::new().bg(Color::Red).fg(Color::White).bold(),
        "staging" => Style::new().bg(Color::Yellow).fg(Color::Black).bold(),
        "local" => Style::new().bg(Color::Green).fg(Color::Black).bold(),
        _ => Style::new().bg(Color::Blue).fg(Color::White).bold(),
    };
    let mut spans = vec![];
    if !app.running.is_empty() {
        spans.push(Span::styled(
            format!("{} {} running  ", SPINNER[app.tick % SPINNER.len()], app.running.len()),
            Style::new().yellow(),
        ));
    }
    let ws = app.ws.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    spans.extend([
        Span::styled(format!("{ws}  "), Style::new().dark_gray()),
        if app.asserts {
            Span::styled(" asserts ", Style::new().fg(Color::Black).bg(ACCENT))
        } else {
            Span::styled("asserts off", Style::new().dark_gray())
        },
        Span::raw(" "),
        Span::styled(format!(" {env} "), env_style),
    ]);
    Line::from(spans)
}

/// Key hints, the latest toast after them, and state on the right. The toast only covers the
/// hints (then the state) when the terminal is too narrow to fit them side by side, so nothing
/// shifts.
fn draw_footer(app: &App, f: &mut Frame, area: Rect) {
    const MIN_TOAST: usize = 20;
    let width = area.width as usize;
    let sep = || Span::styled(" · ", Style::new().dark_gray());
    let mut hints = Line::from(Span::raw(" "));
    let keys: &[(&str, &str)] = if app.show_sidebar {
        &[("enter", "run"), ("?", "help"), ("q", "quit")]
    } else {
        &[("enter", "run"), ("/", "find"), ("?", "help"), ("q", "quit")]
    };
    for (i, &(k, v)) in keys.iter().enumerate() {
        if i > 0 {
            hints.push_span(sep());
        }
        hints.push_span(Span::styled(k, Style::new().cyan()));
        hints.push_span(Span::styled(format!(" {v}"), Style::new().dark_gray()));
    }
    let state = state_line(app);
    let state_w = state.width() + 1;
    let toast = app.flash.as_ref().filter(|(_, _, at)| at.elapsed() < Duration::from_secs(4));
    let left = match toast {
        None => hints,
        Some((msg, err, _)) => {
            let style = if *err { Style::new().red() } else { Style::new().green() };
            let msg = msg.lines().next().unwrap_or_default();
            let need = MIN_TOAST.min(msg.chars().count());
            let after_hints = width.saturating_sub(hints.width() + 3 + state_w + 1);
            let alone = width.saturating_sub(state_w + 2);
            if after_hints >= need {
                hints.push_span(sep());
                hints.push_span(Span::styled(ellipsize(msg, after_hints), style));
                hints
            } else if alone >= need {
                Line::from(Span::styled(format!(" {}", ellipsize(msg, alone)), style))
            } else {
                let line = Line::from(Span::styled(format!(" {}", ellipsize(msg, width.saturating_sub(1))), style));
                return f.render_widget(line, area);
            }
        }
    };
    // The env badge wins over hints: they are clipped, it never is.
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(state_w as u16)]).areas(area);
    f.render_widget(left, l);
    f.render_widget(state, r);
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
