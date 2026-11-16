//! Request list. (Phase 5 turns this into a tree.)

use super::theme::{method_color, panel, short_method, status_color, SPINNER};
use crate::tui::app::{App, Focus, Mode};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem};

pub fn draw(app: &mut App, f: &mut Frame, area: Rect) {
    let filtering = app.mode == Mode::Filter || !app.filter.is_empty();
    let block = panel(app.focus == Focus::Sidebar)
        .title(format!(" Requests {}/{} ", app.visible.len(), app.requests.len()));
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
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .map(|&i| {
            let r = &app.requests[i];
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
            ListItem::new(Line::from(vec![
                mark,
                Span::raw(" "),
                Span::styled(format!("{:<4}", short_method(m)), Style::new().fg(method_color(m)).bold()),
                Span::raw(" "),
                Span::raw(r.rel.clone()),
            ]))
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
