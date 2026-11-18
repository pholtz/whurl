//! Request line, status, latency waterfall, and sparkline above the tabs.

use super::theme::{
    SPINNER, ago, draw_spark, latency_color, method_color, panel, phase_legend, status_color,
    waterfall_line,
};
use crate::export::{bytes, ms, reason};
use crate::tui::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Padding, Paragraph};

pub fn draw(app: &App, f: &mut Frame, area: Rect) {
    let Some(req) = app.current() else {
        let title = match app.row() {
            Some(crate::tui::tree::Row::Dir { path, .. }) => format!(" {path}/ "),
            _ => " no requests ".into(),
        };
        f.render_widget(panel(false).title(title), area);
        return;
    };
    let entry = app.current_entry();
    let (method, url) = match (entry, req.primary()) {
        (Some(e), _) => (e.method.clone(), e.url.clone()),
        (None, Some(p)) => (
            p.method.clone(),
            app.ws.interpolate(&p.url, &app.ws.vars(app.env())),
        ),
        _ => ("?".into(), String::new()),
    };
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(req.rel.clone(), Style::new().bold()),
        Span::styled(
            req.title
                .as_ref()
                .map(|t| format!("  {t} "))
                .unwrap_or(" ".into()),
            Style::new().dark_gray(),
        ),
    ]);
    let key = app.key().unwrap_or_default();
    let mut block = panel(false).padding(Padding::horizontal(1)).title(title);
    // On the border, so a run starting or ending never moves the lines below.
    if let Some(job) = app.running.get(&key) {
        block = block.title_top(
            Line::from(vec![
                Span::styled(
                    format!(
                        " {} running {:.1}s",
                        SPINNER[app.tick % SPINNER.len()],
                        job.started.elapsed().as_secs_f32()
                    ),
                    Style::new().yellow(),
                ),
                Span::styled(" · esc to cancel ", Style::new().dark_gray()),
            ])
            .right_aligned(),
        );
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    let lat = app.latencies.get(&key).cloned().unwrap_or_default();
    let spark_w = if lat.len() > 1 && inner.width > 70 {
        32
    } else {
        0
    };
    let [left, spark] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(spark_w)]).areas(inner);

    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{method} "),
            Style::new().fg(method_color(&method)).bold(),
        ),
        Span::raw(url),
    ])];
    let run = app.current_run();
    match (run, entry) {
        (Some(run), Some(e)) => {
            let mut status = vec![];
            match &e.response {
                Some(r) => {
                    status.push(Span::styled(
                        format!(
                            " {} ",
                            format!("{} {}", r.status, reason(r.status)).trim_end()
                        ),
                        Style::new()
                            .bg(status_color(r.status))
                            .fg(Color::Black)
                            .bold(),
                    ));
                    status.push(Span::styled(
                        format!("  {}", e.total_us().map(ms).unwrap_or_default()),
                        Style::new()
                            .bold()
                            .fg(latency_color(e.total_us().unwrap_or(0))),
                    ));
                    status.push(Span::raw(format!("  {}", bytes(r.size))));
                    status.push(Span::styled(
                        format!("  {}", r.http_version),
                        Style::new().dark_gray(),
                    ));
                }
                None => status.push(Span::styled(
                    " no response ",
                    Style::new().bg(Color::Red).fg(Color::White).bold(),
                )),
            }
            if run.entries.len() > 1 {
                status.push(Span::styled(
                    format!("  entry {}/{} [ ]", app.entry + 1, run.entries.len()),
                    Style::new().cyan(),
                ));
            }
            let failed = e.failed_asserts().count();
            if failed > 0 {
                status.push(Span::styled(
                    format!("  {failed} assert(s) failed"),
                    Style::new().red().bold(),
                ));
            }
            status.push(Span::styled(
                format!("  {}", ago(&run.started_at)),
                Style::new().dark_gray(),
            ));
            lines.push(Line::from(status));
            if let Some(t) = &e.timings {
                lines.push(waterfall_line(
                    t,
                    (left.width as usize).saturating_sub(2).min(60),
                ));
                lines.push(phase_legend(t));
            }
        }
        _ if !app.running.contains_key(&key) => lines.push(Line::from(Span::styled(
            format!("not run in {} yet — press enter", app.env()),
            Style::new().dark_gray(),
        ))),
        _ => {}
    }
    f.render_widget(Paragraph::new(lines), left);
    if spark_w > 0 {
        draw_spark(f, spark, &lat);
    }
}
