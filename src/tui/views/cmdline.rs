//! The `:` bar on the statusbar line, and its completion list just above it.

use super::theme::{panel, ACCENT};
use crate::tui::app::App;
use crate::tui::cmdline::{completing, completions};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Clear, Row, Table, TableState};

pub fn input(app: &App) -> Line<'static> {
    Line::from(vec![
        Span::styled(" :", Style::new().fg(ACCENT).bold()),
        Span::raw(app.cmdline.input.clone()),
        Span::styled("▏", Style::new().fg(ACCENT)),
    ])
}

/// `foot` is the statusbar line; the list sits on top of it, left-aligned under the cursor.
pub fn draw_list(app: &App, f: &mut Frame, foot: Rect) {
    let cands = completions(app);
    if cands.is_empty() {
        return;
    }
    let area = f.area();
    let w = 120.min(area.width);
    let h = (cands.len() as u16 + 2).min(14).min(foot.y.saturating_sub(area.y));
    let rect = Rect { x: area.x, y: foot.y - h, width: w, height: h };
    let title = match completing(app) {
        Some((name, usage)) => format!(" :{name} {usage} "),
        None => " Commands ".into(),
    };
    let block = panel(true).title(title).title_bottom(
        Line::from(Span::styled(" tab complete · ↑↓ choose · enter run · esc close ", Style::new().dark_gray()))
            .right_aligned(),
    );
    f.render_widget(Clear, rect);
    let name_w = cands.iter().map(|c| c.text.chars().count() + c.usage.chars().count() + 1).max().unwrap_or(8).min(26);
    let keys_w = cands.iter().map(|c| c.keys.chars().count()).max().unwrap_or(0);
    let rows = cands.iter().map(|c| {
        let name = Line::from(vec![
            Span::raw(c.text.clone()),
            Span::styled(if c.usage.is_empty() { String::new() } else { format!(" {}", c.usage) }, Style::new().dark_gray()),
        ]);
        Row::new([
            Cell::from(name),
            Cell::from(c.desc.clone()).style(Style::new().dark_gray()),
            Cell::from(Line::from(c.keys.clone()).right_aligned()).style(Style::new().fg(ACCENT)),
        ])
    });
    let table = Table::new(rows, [Constraint::Length(name_w as u16), Constraint::Min(10), Constraint::Length(keys_w as u16)])
        .block(block)
        .column_spacing(2)
        .highlight_symbol("› ")
        .row_highlight_style(Style::new().bg(Color::Indexed(237)).add_modifier(Modifier::BOLD));
    let sel = app.cmdline.sel.min(cands.len() - 1);
    f.render_stateful_widget(table, rect, &mut TableState::default().with_selected(Some(sel)));
}
