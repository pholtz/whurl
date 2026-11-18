//! Colors and small styled-text helpers shared by the views.

use crate::export::{is_json, ms, pretty};
use crate::print::PHASE_COLORS;
use crate::run::{Header, Timings};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Sparkline};

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const BORDER: Color = Color::Indexed(240);
pub const ACCENT: Color = Color::Cyan;

/// Rounded panel; the focused one gets the accent border.
pub fn panel(focused: bool) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if focused { ACCENT } else { BORDER }))
}

pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

pub fn status_color(s: u16) -> Color {
    match s {
        200..=299 => Color::Green,
        300..=399 => Color::Cyan,
        400..=499 => Color::Yellow,
        _ => Color::Red,
    }
}

pub fn latency_color(us: u64) -> Color {
    match us {
        0..=300_000 => Color::Green,
        300_001..=1_000_000 => Color::Yellow,
        _ => Color::Red,
    }
}

pub fn method_color(m: &str) -> Color {
    match m {
        "GET" => Color::Green,
        "POST" => Color::Yellow,
        "PUT" | "PATCH" => Color::Blue,
        "DELETE" => Color::Red,
        _ => Color::Magenta,
    }
}

pub fn short_method(m: &str) -> &str {
    match m {
        "DELETE" => "DEL",
        "PATCH" => "PTCH",
        "OPTIONS" => "OPT",
        m => m,
    }
}

pub fn ago(ts: &str) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(ts) else {
        return String::new();
    };
    let s = (chrono::Local::now().fixed_offset() - t)
        .num_seconds()
        .max(0);
    match s {
        0..=4 => "just now".into(),
        5..=59 => format!("{s}s ago"),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86400),
    }
}

pub fn draw_spark(f: &mut Frame, area: Rect, lat: &[u64]) {
    let w = area.width.saturating_sub(2) as usize;
    let data: Vec<u64> = lat.iter().rev().take(w).rev().copied().collect();
    let max = data.iter().copied().max().unwrap_or(0);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Indexed(238)))
        .title(Span::styled(
            format!(" latency · max {} ", ms(max)),
            Style::new().dark_gray(),
        ));
    f.render_widget(
        Sparkline::default()
            .block(block)
            .data(data)
            .style(Style::new().fg(Color::Indexed(214))),
        area,
    );
}

pub fn waterfall_line(t: &Timings, width: usize) -> Line<'static> {
    let total = t.total.max(1) as f64;
    let mut spans = vec![];
    let mut used = 0;
    for (i, p) in t.phases().iter().enumerate() {
        if p.us == 0 {
            continue;
        }
        let w = (((p.us as f64 / total) * width as f64).round() as usize)
            .max(1)
            .min(width.saturating_sub(used));
        used += w;
        spans.push(Span::styled(
            "█".repeat(w),
            Style::new().fg(Color::Indexed(PHASE_COLORS[i])),
        ));
    }
    Line::from(spans)
}

pub fn phase_legend(t: &Timings) -> Line<'static> {
    let mut spans = vec![];
    for (i, p) in t.phases().iter().enumerate().filter(|(_, p)| p.us > 0) {
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", Style::new().dark_gray()));
        }
        spans.push(Span::styled(
            p.name,
            Style::new().fg(Color::Indexed(PHASE_COLORS[i])),
        ));
        spans.push(Span::raw(format!(" {}", ms(p.us))));
    }
    Line::from(spans)
}

pub fn section(s: &str) -> Line<'static> {
    Line::from(Span::styled(
        s.to_string(),
        Style::new().bold().underlined(),
    ))
}

pub fn header_lines(hs: &[Header]) -> Vec<Line<'static>> {
    let w = hs.iter().map(|h| h.name.len()).max().unwrap_or(0).min(32);
    hs.iter()
        .map(|h| {
            Line::from(vec![
                Span::styled(format!("{:<w$}", h.name), Style::new().cyan()),
                Span::raw("  "),
                Span::raw(h.value.clone()),
            ])
        })
        .collect()
}

pub fn body_lines(body: &str) -> Vec<Line<'static>> {
    if is_json(body) {
        pretty(body).lines().map(json_line).collect()
    } else {
        body.lines()
            .map(|l| Line::from(l.replace('\t', "    ")))
            .collect()
    }
}

/// Highlight one line of pretty-printed JSON.
fn json_line(line: &str) -> Line<'static> {
    let key = Style::new().fg(Color::Cyan);
    let string = Style::new().fg(Color::Green);
    let number = Style::new().fg(Color::Yellow);
    let lit = Style::new().fg(Color::Magenta);
    let punct = Style::new().fg(Color::Indexed(244));
    let chars: Vec<char> = line.chars().collect();
    let mut spans = vec![];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i = (i + 1).min(chars.len());
            let s: String = chars[start..i].iter().collect();
            let is_key = chars[i..].iter().find(|c| !c.is_whitespace()) == Some(&':');
            spans.push(Span::styled(s, if is_key { key } else { string }));
        } else if c == '-' || c.is_ascii_digit() {
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '.' | '-' | '+'))
            {
                i += 1;
            }
            spans.push(Span::styled(
                chars[start..i].iter().collect::<String>(),
                number,
            ));
        } else if c.is_ascii_alphabetic() {
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            spans.push(Span::styled(
                chars[start..i].iter().collect::<String>(),
                lit,
            ));
        } else {
            while i < chars.len()
                && !(chars[i] == '"' || chars[i] == '-' || chars[i].is_ascii_alphanumeric())
            {
                i += 1;
            }
            spans.push(Span::styled(
                chars[start..i].iter().collect::<String>(),
                punct,
            ));
        }
    }
    Line::from(spans)
}

/// Split out `{{var}}` references so they stand out.
pub fn vars(s: &str, base: Style) -> Vec<Span<'static>> {
    let mut out = vec![];
    let mut rest = s;
    while let Some(a) = rest.find("{{") {
        let Some(b) = rest[a..].find("}}") else { break };
        out.push(Span::styled(rest[..a].to_string(), base));
        out.push(Span::styled(
            rest[a..a + b + 2].to_string(),
            Style::new().yellow(),
        ));
        rest = &rest[a + b + 2..];
    }
    out.push(Span::styled(rest.to_string(), base));
    out
}
