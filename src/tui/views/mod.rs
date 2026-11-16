//! Rendering. Views read `App` and never change selection or run state.

mod content;
mod sidebar;
mod summary;
pub mod theme;

use super::app::{App, Mode};
use super::commands::{Scope, COMMANDS};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use std::time::Duration;
use theme::{centered, panel, ACCENT, SPINNER};

pub fn draw(app: &mut App, f: &mut Frame) {
    let [top, main, foot] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
    draw_top(app, f, top);
    let list_w = app.requests.iter().map(|r| r.rel.len()).max().unwrap_or(20) as u16 + 9;
    let list_w = list_w.min(main.width * 2 / 5).max(24);
    let [left, right] = Layout::horizontal([Constraint::Length(list_w), Constraint::Min(0)]).areas(main);
    sidebar::draw(app, f, left);
    let [summary, content] = Layout::vertical([Constraint::Length(6), Constraint::Min(0)]).areas(right);
    summary::draw(app, f, summary);
    content::draw(app, f, content);
    draw_footer(app, f, foot);
    if app.mode == Mode::Help {
        draw_help(f);
    }
}

fn draw_top(app: &App, f: &mut Frame, area: Rect) {
    let env = app.env();
    let env_style = match env {
        "production" => Style::new().bg(Color::Red).fg(Color::White).bold(),
        "staging" => Style::new().bg(Color::Yellow).fg(Color::Black).bold(),
        "local" => Style::new().bg(Color::Green).fg(Color::Black).bold(),
        _ => Style::new().bg(Color::Blue).fg(Color::White).bold(),
    };
    let mut spans = vec![
        Span::styled(" whurl ", Style::new().bg(Color::Indexed(238)).bold()),
        Span::raw(" "),
        Span::styled(format!(" {env} "), env_style),
        Span::raw(" "),
        if app.asserts {
            Span::styled(" asserts on ", Style::new().fg(Color::Black).bg(Color::Cyan))
        } else {
            Span::styled("asserts off", Style::new().dark_gray())
        },
        Span::raw("  "),
        Span::styled(app.ws.root.display().to_string(), Style::new().dark_gray()),
    ];
    if !app.running.is_empty() {
        spans.push(Span::styled(
            format!("   {} {} running", SPINNER[app.tick % SPINNER.len()], app.running.len()),
            Style::new().yellow(),
        ));
    }
    f.render_widget(Line::from(spans), area);
}

fn draw_footer(app: &App, f: &mut Frame, area: Rect) {
    if let Some((msg, err, at)) = &app.flash
        && at.elapsed() < Duration::from_secs(6)
    {
        let style = if *err { Style::new().red() } else { Style::new().green() };
        f.render_widget(Span::styled(format!(" {msg}"), style), area);
        return;
    }
    let mut spans = vec![Span::raw(" ")];
    for (i, (k, v)) in [("enter", "run"), ("?", "help"), ("q", "quit")].into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::new().dark_gray()));
        }
        spans.push(Span::styled(k, Style::new().cyan()));
        spans.push(Span::styled(format!(" {v}"), Style::new().dark_gray()));
    }
    f.render_widget(Line::from(spans), area);
}

/// Placeholder until phase 6's plantmon-style modal; generated from the registry so it can't drift.
fn draw_help(f: &mut Frame) {
    let column = |scopes: &[Scope]| -> Vec<Line<'static>> {
        let mut out = vec![];
        for &s in scopes {
            if !out.is_empty() {
                out.push(Line::default());
            }
            out.push(Line::from(Span::styled(s.title(), Style::new().fg(ACCENT).bold())));
            for c in COMMANDS.iter().filter(|c| c.scope == s) {
                out.push(Line::from(vec![
                    Span::styled(format!("{:<14}", c.key_labels()), Style::new().cyan()),
                    Span::raw(c.desc),
                ]));
            }
        }
        out
    };
    let left = column(&[Scope::Global]);
    let right = column(&[Scope::Sidebar, Scope::Main]);
    let h = left.len().max(right.len()) as u16 + 2;
    let area = centered(f.area(), 112, h);
    f.render_widget(Clear, area);
    let block = panel(true).title(" keys · esc closes ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [l, r] = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).spacing(2).areas(inner);
    f.render_widget(Paragraph::new(left), l);
    f.render_widget(Paragraph::new(right), r);
}
