//! The `?` modal, after plantmon's: a centered rounded popup with Commands and Config tabs,
//! a selectable table, a description for the selected row, and a hint line.

use super::theme::{centered, panel, ACCENT};
use crate::config::Config;
use crate::tui::app::App;
use crate::tui::commands::COMMANDS;
use crate::tui::help::{describe, settings, HelpTab};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Cell, Clear, Paragraph, Row, Table, TableState, Tabs, Wrap};

pub fn draw(app: &App, f: &mut Frame) {
    let area = f.area();
    let rect = centered(area, 100.min(area.width.saturating_sub(2)), 36.min(area.height.saturating_sub(2)));
    f.render_widget(Clear, rect);
    let title = match app.help.tab {
        HelpTab::Commands => " Help ",
        HelpTab::Config => " Config ",
    };
    let block = panel(true).title(title);
    let inner = block.inner(rect).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
    f.render_widget(block, rect);
    let [tabs, _, body] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    f.render_widget(
        Tabs::new(["Commands", "Config"])
            .select(if app.help.tab == HelpTab::Config { 1 } else { 0 })
            .style(Style::new().dark_gray())
            .highlight_style(Style::new().fg(ACCENT).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)),
        tabs,
    );
    match app.help.tab {
        HelpTab::Commands => commands(app, f, body),
        HelpTab::Config => config(app, f, body),
    }
}

fn highlight() -> Style {
    Style::new().fg(ACCENT).bg(ratatui::style::Color::Indexed(237)).add_modifier(Modifier::BOLD)
}

/// Summary on top, the table in the middle, then the selected row's description and a hint.
fn frame(f: &mut Frame, area: Rect, summary: &str, desc: &str, hint: &str) -> Rect {
    let lines = |s: &str| Paragraph::new(s).wrap(Wrap { trim: false }).line_count(area.width).min(3) as u16;
    let compact = area.height < 14;
    let [top, table, d, h] = Layout::vertical([
        Constraint::Length(if compact { 0 } else { lines(summary) + 1 }),
        Constraint::Min(3),
        Constraint::Length(if compact || desc.is_empty() { 0 } else { lines(desc) + 1 }),
        Constraint::Length(lines(hint)),
    ])
    .areas(area);
    f.render_widget(Paragraph::new(summary).wrap(Wrap { trim: false }), top);
    let [_, d] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(d);
    f.render_widget(Paragraph::new(desc).style(Style::new().dark_gray()).wrap(Wrap { trim: false }), d);
    f.render_widget(Paragraph::new(hint).style(Style::new().fg(ACCENT)).wrap(Wrap { trim: false }), h);
    table
}

fn commands(app: &App, f: &mut Frame, area: Rect) {
    let sel = app.help.cmd.min(COMMANDS.len().saturating_sub(1));
    let table_area = frame(
        f,
        area,
        "Keys act on the focused pane first (tab switches focus), then anywhere. Pick a row and press enter to run it. `:` runs any of these by name, plus env, set, set!, export, tab and cancel, which take arguments.",
        "",
        "tab pages · ↑↓ choose · enter run · ? / esc close",
    );
    let rows = COMMANDS.iter().map(|c| {
        Row::new([
            Cell::from(c.key_labels()).style(Style::new().fg(ACCENT)),
            Cell::from(c.desc),
            Cell::from(c.scope.title()).style(Style::new().dark_gray()),
        ])
    });
    let table = Table::new(rows, [Constraint::Length(16), Constraint::Min(10), Constraint::Length(10)])
        .header(
            Row::new(["Key".to_string(), format!("Command · {} of {}", sel + 1, COMMANDS.len()), "Where".into()])
                .style(Style::new().dark_gray())
                .bottom_margin(1),
        )
        .column_spacing(2)
        .highlight_symbol("› ")
        .row_highlight_style(highlight());
    f.render_stateful_widget(table, table_area, &mut TableState::default().with_selected(Some(sel)));
}

fn config(app: &App, f: &mut Frame, area: Rect) {
    let rows = settings(&app.cfg);
    let sel = app.help.cfg.min(rows.len().saturating_sub(1));
    let described: Vec<_> = rows.iter().map(|s| describe(app, s)).collect();
    let path = Config::display_path();
    let summary = format!("Changes apply now and are saved to {path}; comments in the file are kept.");
    let desc = described.get(sel).map(|d| d.2.as_str()).unwrap_or("");
    let table_area =
        frame(f, area, &summary, desc, "tab pages · ↑↓ select · ←→ / enter change · J/K reorder sections · esc close");
    let table = Table::new(
        described.iter().map(|(label, value, _)| {
            let style = if value.starts_with("[ ]") { Style::new().dark_gray() } else { Style::new() };
            Row::new([Cell::from(label.as_str()), Cell::from(Line::styled(value.as_str(), style))])
        }),
        [Constraint::Length(26), Constraint::Min(10)],
    )
    .column_spacing(2)
    .highlight_symbol("› ")
    .row_highlight_style(highlight());
    f.render_stateful_widget(table, table_area, &mut TableState::default().with_selected(Some(sel)));
}
