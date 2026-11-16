//! Request tree (or flat list), with run status per row.

use super::theme::{method_color, panel, short_method, status_color, ACCENT, SPINNER};
use crate::tui::tree::{basename, Row};
use crate::tui::app::{App, Focus, Mode};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem};

pub fn draw(app: &mut App, f: &mut Frame, area: Rect) {
    let filtering = app.mode == Mode::Filter || !app.filter.is_empty();
    let block = panel(app.focus == Focus::Sidebar)
        .title(format!(" Requests {}/{} ", app.matched.len(), app.requests.len()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let (list_area, filter_area) = if filtering {
        let [a, b] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
        (a, Some(b))
    } else {
        (inner, None)
    };
    let env = app.env().to_string();
    let spin = SPINNER[app.tick % SPINNER.len()];
    let running_under = |dir: &str| {
        app.running.keys().any(|(rel, e)| *e == env && rel.starts_with(dir) && rel[dir.len()..].starts_with('/'))
    };
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|row| {
            let indent = Span::raw("  ".repeat(row.depth()));
            match row {
                Row::Dir { path, open, count, .. } => {
                    let arrow = if *open { "▾" } else { "▸" };
                    let mut spans = vec![
                        indent,
                        Span::styled(arrow, Style::new().fg(ACCENT)),
                        Span::raw(" "),
                        Span::styled(format!("{}/", basename(path)), Style::new().bold()),
                        Span::styled(format!(" {count}"), Style::new().dark_gray()),
                    ];
                    if !open && running_under(path) {
                        spans.push(Span::styled(format!(" {spin}"), Style::new().yellow()));
                    }
                    ListItem::new(Line::from(spans))
                }
                Row::Req { idx, .. } => {
                    let r = &app.requests[*idx];
                    let key = (r.rel.clone(), env.clone());
                    let mark = if app.running.contains_key(&key) {
                        Span::styled(spin, Style::new().yellow())
                    } else {
                        match app.runs.get(&key).and_then(Option::as_ref) {
                            Some(run) => {
                                let st = run.primary().and_then(|e| e.response.as_ref()).map(|r| r.status);
                                Span::styled("●", Style::new().fg(st.map(status_color).unwrap_or(Color::Red)))
                            }
                            None => Span::raw(" "),
                        }
                    };
                    let m = r.primary().map(|e| e.method.as_str()).unwrap_or("?");
                    let name = if app.tree { basename(&r.rel) } else { &r.rel };
                    ListItem::new(Line::from(vec![
                        indent,
                        mark,
                        Span::raw(" "),
                        Span::styled(format!("{:<4}", short_method(m)), Style::new().fg(method_color(m)).bold()),
                        Span::raw(" "),
                        Span::raw(name.to_string()),
                    ]))
                }
            }
        })
        .collect();
    let list = List::new(items)
        .highlight_style(Style::new().bg(Color::Indexed(237)).add_modifier(Modifier::BOLD))
        .highlight_symbol("▌");
    f.render_stateful_widget(list, list_area, &mut app.list);
    if let Some(a) = filter_area {
        let cursor = if app.mode == Mode::Filter { "▏" } else { "" };
        f.render_widget(
            Line::from(vec![Span::styled("/", Style::new().yellow()), Span::raw(format!("{}{cursor}", app.filter))]),
            a,
        );
    }
}
