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

/// Key hints, then the latest toast after them. The toast only covers the hints when the
/// terminal is too narrow to fit both, so nothing shifts.
fn draw_footer(app: &App, f: &mut Frame, area: Rect) {
    const MIN_TOAST: usize = 20;
    let sep = || Span::styled(" · ", Style::new().dark_gray());
    let mut hints = Line::from(Span::raw(" "));
    for (i, (k, v)) in [("enter", "run"), ("?", "help"), ("q", "quit")].into_iter().enumerate() {
        if i > 0 {
            hints.push_span(sep());
        }
        hints.push_span(Span::styled(k, Style::new().cyan()));
        hints.push_span(Span::styled(format!(" {v}"), Style::new().dark_gray()));
    }
    let toast = app.flash.as_ref().filter(|(_, _, at)| at.elapsed() < Duration::from_secs(5));
    let Some((msg, err, _)) = toast else {
        f.render_widget(hints, area);
        return;
    };
    let style = if *err { Style::new().red() } else { Style::new().green() };
    let msg = msg.lines().next().unwrap_or_default();
    let room = (area.width as usize).saturating_sub(hints.width() + 3);
    let line = if room >= MIN_TOAST.min(msg.chars().count()) {
        hints.push_span(sep());
        hints.push_span(Span::styled(ellipsize(msg, room), style));
        hints
    } else {
        Line::from(Span::styled(format!(" {}", ellipsize(msg, (area.width as usize).saturating_sub(1))), style))
    };
    f.render_widget(line, area);
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
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
