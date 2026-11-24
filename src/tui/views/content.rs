//! The tabbed main pane: Request · Response · Meta · Source.

use super::theme::{body_lines, header_lines, latency_color, method_color, panel, section, vars};
use crate::config::{REQUEST_SECTIONS, RESPONSE_SECTIONS};
use crate::export::{bytes, ms};
use crate::print::PHASE_COLORS;
use crate::run::{Entry, Run};
use crate::tui::app::{App, Focus, TABS, Tab, View, ViewKey};
use crate::workspace::RequestFile;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Padding, Paragraph, Tabs, Wrap};
use std::collections::HashSet;

pub fn draw(app: &mut App, f: &mut Frame, area: Rect) {
    let block = panel(app.focus == Focus::Main).padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [tabs_area, _, body] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);
    let titles: Vec<Line> = TABS
        .iter()
        .enumerate()
        .map(|(i, t)| Line::from(format!("{} {}", i + 1, t.title())))
        .collect();
    let mut tabs_line = vec![];
    if !app.wrap {
        tabs_line.push(Span::styled("nowrap", Style::new().dark_gray()));
    }
    f.render_widget(
        Tabs::new(titles)
            .select(TABS.iter().position(|t| *t == app.tab).unwrap_or(0))
            .style(Style::new().dark_gray())
            .highlight_style(Style::new().fg(Color::White).bold().underlined())
            .divider(" "),
        tabs_area,
    );
    // Tabs pads each title with a space either side and joins them with a 1-wide divider.
    let mut x = tabs_area.x;
    for (i, t) in TABS.iter().enumerate() {
        let w = format!("{} {}", i + 1, t.title()).chars().count() as u16 + 2;
        app.hits.tabs.push((
            Rect {
                x,
                y: tabs_area.y,
                width: w,
                height: 1,
            }
            .intersection(tabs_area),
            *t,
        ));
        x += w + 1;
    }
    if !tabs_line.is_empty() {
        f.render_widget(Line::from(tabs_line).right_aligned(), tabs_area);
    }
    app.content_height = body.height as usize;
    // A folder gets a pseudo key (the trailing `/` can't clash with a request) so its hint shows.
    let key = match (app.key(), app.row()) {
        (Some(k), _) => k,
        (None, Some(whurl_core::tree::Row::Dir { path, .. })) => {
            (format!("{path}/"), String::new())
        }
        _ => return,
    };

    let vk = ViewKey {
        running: app.running.contains_key(&key),
        key,
        run: app.current_run().map(|r| r.id.clone()),
        entry: app.entry,
        tab: app.tab,
        width: if app.tab == Tab::Meta { body.width } else { 0 },
    };
    if app.view.as_ref().map(|v| &v.key) != Some(&vk) {
        let lines = build_lines(app, body.width);
        app.view = Some(View {
            key: vk,
            lines,
            rows: vec![],
            rows_for: (0, false),
        });
    }
    let view = app.view.as_mut().unwrap();
    let measure_for = (body.width, app.wrap);
    if view.rows_for != measure_for {
        // Keep the same top line across wrap toggles and resizes.
        let (top, _) = locate(&view.rows, app.scroll.0);
        view.rows = measure(&view.lines, body.width, app.wrap);
        view.rows_for = measure_for;
        app.scroll.0 = view.rows[..top.min(view.rows.len())].iter().sum();
    }
    let total: usize = view.rows.iter().sum();
    app.content_rows = total;
    app.scroll.0 = app.scroll.0.min(total.saturating_sub(body.height as usize));

    let (start, offset) = locate(&view.rows, app.scroll.0);
    let mut end = start;
    let mut covered = 0;
    while end < view.lines.len() && covered < offset + body.height as usize {
        covered += view.rows[end];
        end += 1;
    }
    let mut p = Paragraph::new(view.lines[start..end].to_vec());
    p = if app.wrap {
        p.wrap(Wrap { trim: false }).scroll((offset as u16, 0))
    } else {
        p.scroll((0, app.scroll.1))
    };
    f.render_widget(p, body);

    if total > body.height as usize {
        let last = if app.scroll.0 + body.height as usize >= total {
            view.lines.len()
        } else {
            end.saturating_sub(1).max(start + 1)
        };
        let pos = format!(" {}-{}/{} ", start + 1, last, view.lines.len());
        let w = pos.len() as u16;
        if area.width > w + 2 {
            let r = Rect {
                x: area.x + area.width - w - 2,
                y: area.y + area.height - 1,
                width: w,
                height: 1,
            };
            f.render_widget(Span::styled(pos, Style::new().dark_gray()), r);
        }
    }
}

/// Screen rows each line takes at `width`.
fn measure(lines: &[Line<'static>], width: u16, wrap: bool) -> Vec<usize> {
    if !wrap {
        return vec![1; lines.len()];
    }
    lines
        .iter()
        .map(|l| {
            Paragraph::new(l.clone())
                .wrap(Wrap { trim: false })
                .line_count(width)
                .max(1)
        })
        .collect()
}

/// (line index, rows into that line) for screen row `row`; past the end → (len, 0).
fn locate(rows: &[usize], row: usize) -> (usize, usize) {
    let mut acc = 0;
    for (i, &r) in rows.iter().enumerate() {
        if row < acc + r {
            return (i, row - acc);
        }
        acc += r;
    }
    (rows.len(), 0)
}

fn hint(s: &str) -> Line<'static> {
    Line::from(Span::styled(s.to_string(), Style::new().dark_gray()))
}

fn build_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    if let Some(whurl_core::tree::Row::Dir { path, count, .. }) = app.row() {
        return vec![
            Line::from(Span::styled(format!("{path}/"), Style::new().bold())),
            Line::default(),
            hint(&format!(
                "{count} request(s) · enter/space toggles · ←→ close/open"
            )),
        ];
    }
    let run = app.current_run();
    let entry = app.current_entry();
    let running = app.key().is_some_and(|k| app.running.contains_key(&k));
    let not_run = || {
        hint(&if running {
            "running…".to_string()
        } else {
            format!("not run in {} yet — press enter", app.env())
        })
    };
    match app.tab {
        Tab::Source => app
            .current()
            .map(|r| source_lines(r, entry.map(|e| e.line)))
            .unwrap_or_default(),
        Tab::Request => match entry {
            Some(e) => request_lines(e, &app.cfg.request.or(REQUEST_SECTIONS)),
            None => {
                let mut out = vec![];
                if let Some(p) = app.current().and_then(RequestFile::primary) {
                    out.push(Line::from(vec![
                        Span::styled(
                            format!("{} ", p.method),
                            Style::new().fg(method_color(&p.method)).bold(),
                        ),
                        Span::raw(app.ws.interpolate(&p.url, &app.ws.vars(app.env()))),
                    ]));
                    out.push(Line::default());
                }
                out.push(not_run());
                out.push(hint(
                    "headers and body appear after a run · 4 shows the .hurl source",
                ));
                out
            }
        },
        Tab::Response => match (run, entry) {
            (Some(run), Some(e)) => response_lines(run, e, &app.cfg.response.or(RESPONSE_SECTIONS)),
            (Some(run), None) if !run.stderr.is_empty() => stderr_lines(run),
            _ => vec![not_run()],
        },
        Tab::Meta => meta_lines(app, width),
    }
}

fn stderr_lines(run: &Run) -> Vec<Line<'static>> {
    let mut out: Vec<Line> = run
        .stderr
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), Style::new().red())))
        .collect();
    out.push(Line::default());
    out
}

/// Section titles only when more than one section is shown.
fn titled(out: &mut Vec<Line<'static>>, many: bool, title: String) {
    if many {
        if !out.is_empty() {
            out.push(Line::default());
        }
        out.push(section(&title));
    }
}

fn request_lines(e: &Entry, sections: &[&str]) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(vec![
        Span::styled(
            format!("{} ", e.method),
            Style::new().fg(method_color(&e.method)).bold(),
        ),
        Span::raw(e.url.clone()),
    ])];
    for &s in sections {
        match s {
            "headers" => {
                titled(
                    &mut out,
                    true,
                    format!("Headers ({})", e.request.headers.len()),
                );
                out.extend(header_lines(&e.request.headers));
            }
            "body" => {
                if let Some(b) = &e.request.body {
                    titled(&mut out, true, format!("Body ({})", bytes(b.len() as u64)));
                    out.extend(body_lines(b));
                }
            }
            "curl" => {
                titled(&mut out, true, "curl".into());
                out.extend(
                    e.curl
                        .lines()
                        .map(|l| Line::from(Span::styled(l.to_string(), Style::new().dark_gray()))),
                );
            }
            _ => {}
        }
    }
    out
}

fn response_lines(run: &Run, e: &Entry, sections: &[&str]) -> Vec<Line<'static>> {
    let mut out = if run.stderr.is_empty() {
        vec![]
    } else {
        stderr_lines(run)
    };
    let Some(r) = &e.response else {
        out.push(hint("no response"));
        return out;
    };
    let many = sections.len() > 1;
    for &s in sections {
        match s {
            "headers" => {
                titled(&mut out, many, format!("Headers ({})", r.headers.len()));
                out.extend(header_lines(&r.headers));
            }
            "body" => {
                titled(&mut out, many, format!("Body ({})", bytes(r.size)));
                match r.body.as_deref() {
                    _ if r.binary => out.push(hint(&format!("<binary body, {}>", bytes(r.size)))),
                    Some(b) if !b.is_empty() => {
                        out.extend(body_lines(b));
                        if r.truncated {
                            out.push(hint("<truncated>"));
                        }
                    }
                    _ => out.push(hint("<empty body>")),
                }
            }
            _ => {}
        }
    }
    out
}

fn kv(k: &str, v: impl Into<Span<'static>>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{k:<10}"), Style::new().dark_gray()),
        v.into(),
    ])
}

/// Run facts, latency waterfall, history, asserts, and captures. Laid out to `width`.
fn meta_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let mut out = vec![];
    if let (Some(run), Some(e)) = (app.current_run(), app.current_entry()) {
        out.push(section("Run"));
        out.push(kv("id", Span::raw(run.id.clone())));
        out.push(kv(
            "started",
            Span::raw(format!(
                "{}  ({})",
                run.started_at,
                super::theme::ago(&run.started_at)
            )),
        ));
        out.push(kv("env", Span::raw(run.env.clone())));
        if run.entries.len() > 1 {
            out.push(kv(
                "entry",
                Span::raw(format!(
                    "{}/{}  (line {})",
                    app.entry + 1,
                    run.entries.len(),
                    e.line
                )),
            ));
        }
        if let Some(r) = &e.response {
            out.push(kv("http", Span::raw(r.http_version.clone())));
        }
        out.push(kv(
            "hurl",
            Span::raw(format!(
                "exit {} · {}ms wall time (includes startup)",
                run.exit_code, run.wall_ms
            )),
        ));

        if let Some(t) = &e.timings {
            out.push(Line::default());
            out.push(section("Timing"));
            let label_w = 10usize;
            let bar_w = (width as usize).saturating_sub(label_w + 12).min(70);
            let total = t.total.max(1) as f64;
            let mut offset = 0u64;
            for (i, p) in t.phases().iter().enumerate() {
                // Waterfall: each phase starts where the previous one ended.
                let start = ((offset as f64 / total) * bar_w as f64).round() as usize;
                let w = if p.us == 0 {
                    0
                } else {
                    (((p.us as f64 / total) * bar_w as f64).round() as usize).max(1)
                };
                let start = start.min(bar_w);
                let w = w.min(bar_w - start);
                offset += p.us;
                out.push(Line::from(vec![
                    Span::styled(
                        format!("{:<label_w$}", p.name),
                        Style::new().fg(Color::Indexed(PHASE_COLORS[i])),
                    ),
                    Span::styled("·".repeat(start), Style::new().fg(Color::Indexed(236))),
                    Span::styled(
                        "█".repeat(w),
                        Style::new().fg(Color::Indexed(PHASE_COLORS[i])),
                    ),
                    Span::styled(
                        "·".repeat(bar_w - start - w),
                        Style::new().fg(Color::Indexed(236)),
                    ),
                    Span::raw(format!(" {:>9}", ms(p.us))),
                ]));
            }
            out.push(Line::from(vec![
                Span::styled(format!("{:<label_w$}", "total"), Style::new().bold()),
                Span::raw(" ".repeat(bar_w)),
                Span::styled(
                    format!(" {:>9}", ms(t.total)),
                    Style::new().bold().fg(latency_color(t.total)),
                ),
            ]));
        }

        if !e.asserts.is_empty() {
            let failed: Vec<_> = e.failed_asserts().collect();
            out.push(Line::default());
            out.push(section(&format!(
                "Asserts {}/{} passed",
                e.asserts.len() - failed.len(),
                e.asserts.len()
            )));
            for a in failed {
                let msg = a.message.as_deref().unwrap_or("failed");
                out.extend(
                    msg.lines()
                        .map(|l| Line::from(Span::styled(l.to_string(), Style::new().red()))),
                );
            }
        } else if !run.asserts {
            out.push(Line::default());
            out.push(hint("asserts were off for this run · t toggles"));
        }
        if !e.captures.is_empty() {
            out.push(Line::default());
            out.push(section("Captures"));
            out.extend(header_lines(&e.captures));
        }
    }

    let lat = app
        .key()
        .and_then(|k| app.latencies.get(&k).cloned())
        .unwrap_or_default();
    if lat.len() > 1 {
        let mut sorted = lat.clone();
        sorted.sort_unstable();
        let pct = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
        let max = *sorted.last().unwrap();
        if !out.is_empty() {
            out.push(Line::default());
        }
        out.push(section(&format!(
            "Last {} runs in {}",
            lat.len(),
            app.env()
        )));
        out.push(Line::from(Span::styled(
            format!(
                "min {}  p50 {}  p90 {}  max {}",
                ms(sorted[0]),
                ms(pct(0.5)),
                ms(pct(0.9)),
                ms(max)
            ),
            Style::new().dark_gray(),
        )));
        const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let spark: String = lat
            .iter()
            .rev()
            .take(width as usize)
            .rev()
            .map(|&v| BARS[((v as f64 / max.max(1) as f64) * 7.0).round() as usize])
            .collect();
        out.push(Line::from(Span::styled(
            spark,
            Style::new().fg(Color::Indexed(214)),
        )));
    }
    if out.is_empty() {
        out.push(hint(&format!("not run in {} yet — press enter", app.env())));
    }
    out
}

/// The .hurl file with light highlighting; the selected entry's request line is marked.
fn source_lines(r: &RequestFile, mark: Option<usize>) -> Vec<Line<'static>> {
    let src = std::fs::read_to_string(&r.path).unwrap_or_default();
    let entry_lines: HashSet<usize> = r.entries.iter().map(|e| e.line).collect();
    let mut in_fence = false;
    src.lines()
        .enumerate()
        .map(|(i, l)| {
            let n = i + 1;
            let gutter = Span::styled(
                format!("{n:>4} {} ", if Some(n) == mark { "▶" } else { " " }),
                Style::new().fg(Color::Indexed(if Some(n) == mark { 214 } else { 239 })),
            );
            let t = l.trim_start();
            let content = if t.starts_with("```") {
                in_fence = !in_fence;
                vec![Span::styled(l.to_string(), Style::new().dark_gray())]
            } else if in_fence {
                vars(l, Style::new())
            } else if t.starts_with('#') {
                vec![Span::styled(
                    l.to_string(),
                    Style::new().dark_gray().italic(),
                )]
            } else if entry_lines.contains(&n) {
                let (m, rest) = l.split_once(' ').unwrap_or((l, ""));
                let mut v = vec![Span::styled(
                    format!("{m} "),
                    Style::new().fg(method_color(m)).bold(),
                )];
                v.extend(vars(rest, Style::new().bold()));
                v
            } else if t.starts_with('[') && t.ends_with(']') {
                vec![Span::styled(l.to_string(), Style::new().magenta())]
            } else if t.starts_with("HTTP") {
                vec![Span::styled(l.to_string(), Style::new().blue().bold())]
            } else if let Some((k, v)) = l.split_once(':') {
                let mut out = vec![
                    Span::styled(k.to_string(), Style::new().cyan()),
                    Span::raw(":"),
                ];
                out.extend(vars(v, Style::new()));
                out
            } else {
                vars(l, Style::new())
            };
            let mut spans = vec![gutter];
            spans.extend(content);
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scroll positions are in screen rows; a wrapped line spans several.
    #[test]
    fn locate_wrapped_rows() {
        let rows = [1, 3, 1];
        assert_eq!(locate(&rows, 0), (0, 0));
        assert_eq!(locate(&rows, 1), (1, 0));
        assert_eq!(locate(&rows, 3), (1, 2));
        assert_eq!(locate(&rows, 4), (2, 0));
        assert_eq!(locate(&rows, 5), (3, 0));
        let long = Line::from("x".repeat(25));
        assert_eq!(
            measure(&[long.clone(), Line::default()], 10, true),
            vec![3, 1]
        );
        assert_eq!(measure(&[long], 10, false), vec![1]);
    }
}
