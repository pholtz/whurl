//! The fuzzy request picker: a query line over ranked matches, fzf style.

use super::theme::{method_color, panel, short_method, status_color, ACCENT};
use crate::tui::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState};

pub fn draw(app: &App, f: &mut Frame) {
    let area = f.area();
    let w = 90.min(area.width.saturating_sub(4));
    let h = (app.picker.results.len() as u16 + 4).clamp(6, 22).min(area.height.saturating_sub(2));
    // Pinned near the top so the box grows downward as results change, not around its center.
    let rect = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height / 6).min(4), width: w, height: h };
    f.render_widget(Clear, rect);
    let block = panel(true).title(" Find request ").title_bottom(
        Line::from(Span::styled(" ↑↓ choose · enter open · esc close ", Style::new().dark_gray())).right_aligned(),
    );
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let [input, _, list] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    let count = format!("{}/{} ", app.picker.results.len(), app.requests.len());
    f.render_widget(
        Line::from(vec![
            Span::styled(" › ", Style::new().fg(ACCENT).bold()),
            Span::raw(app.picker.query.clone()),
            Span::styled("▏", Style::new().fg(ACCENT)),
        ]),
        input,
    );
    f.render_widget(Line::styled(count, Style::new().dark_gray()).right_aligned(), input);
    let env = app.env().to_string();
    let items: Vec<ListItem> = app
        .picker
        .results
        .iter()
        .map(|&i| {
            let r = &app.requests[i];
            let m = r.primary().map(|e| e.method.as_str()).unwrap_or("?");
            let key = (r.rel.clone(), env.clone());
            let dot = match app.runs.get(&key).and_then(Option::as_ref) {
                Some(run) => {
                    let st = run.primary().and_then(|e| e.response.as_ref()).map(|r| r.status);
                    Span::styled("●", Style::new().fg(st.map(status_color).unwrap_or(Color::Red)))
                }
                None => Span::raw(" "),
            };
            let mut spans = vec![
                dot,
                Span::raw(" "),
                Span::styled(format!("{:<4}", short_method(m)), Style::new().fg(method_color(m)).bold()),
                Span::raw(" "),
                Span::raw(r.rel.clone()),
            ];
            if let Some(t) = &r.title {
                spans.push(Span::styled(format!("  {t}"), Style::new().dark_gray()));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let list_widget = List::new(items)
        .highlight_style(Style::new().bg(Color::Indexed(237)).add_modifier(Modifier::BOLD))
        .highlight_symbol("▌");
    let mut state = ListState::default().with_selected((!app.picker.results.is_empty()).then_some(app.picker.sel));
    f.render_stateful_widget(list_widget, list, &mut state);
}
