//! Human-readable terminal output for the CLI.

use crate::export::{bytes, ms, pretty, reason};
use crate::run::{Entry, Header, Run, Timings};
use std::io::IsTerminal;

#[derive(Clone, Copy)]
pub struct Paint(bool);

impl Paint {
    pub fn stdout() -> Self {
        Paint(std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none())
    }
    pub fn stderr() -> Self {
        Paint(std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none())
    }
    fn wrap(self, code: &str, s: &str) -> String {
        if self.0 {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    pub fn bold(self, s: &str) -> String {
        self.wrap("1", s)
    }
    pub fn dim(self, s: &str) -> String {
        self.wrap("2", s)
    }
    pub fn red(self, s: &str) -> String {
        self.wrap("31", s)
    }
    pub fn green(self, s: &str) -> String {
        self.wrap("32", s)
    }
    pub fn cyan(self, s: &str) -> String {
        self.wrap("36", s)
    }
    pub fn fg(self, code: u8, s: &str) -> String {
        self.wrap(&format!("38;5;{code}"), s)
    }
    pub fn method(self, m: &str) -> String {
        match m {
            "GET" => self.wrap("1;32", m),
            "POST" => self.wrap("1;33", m),
            "PUT" | "PATCH" => self.wrap("1;34", m),
            "DELETE" => self.wrap("1;31", m),
            _ => self.bold(m),
        }
    }
    pub fn status(self, s: u16) -> String {
        let txt = format!("{s} {}", reason(s)).trim_end().to_string();
        match s {
            200..=299 => self.wrap("1;32", &txt),
            300..=399 => self.wrap("1;36", &txt),
            400..=499 => self.wrap("1;33", &txt),
            _ => self.wrap("1;31", &txt),
        }
    }
}

/// 256-color codes per latency phase; the TUI uses the same palette.
pub const PHASE_COLORS: [u8; 6] = [44, 33, 170, 245, 214, 77];

pub fn waterfall(p: Paint, t: &Timings, width: usize) -> String {
    let phases = t.phases();
    let total = t.total.max(1) as f64;
    let mut bar = String::new();
    let mut used = 0;
    for (i, ph) in phases.iter().enumerate() {
        if ph.us == 0 {
            continue;
        }
        let w = ((ph.us as f64 / total) * width as f64).round().max(1.0) as usize;
        let w = w.min(width.saturating_sub(used));
        used += w;
        bar += &p.fg(PHASE_COLORS[i], &"█".repeat(w));
    }
    let legend: Vec<String> = phases
        .iter()
        .enumerate()
        .filter(|(_, ph)| ph.us > 0)
        .map(|(i, ph)| format!("{} {}", p.fg(PHASE_COLORS[i], ph.name), ms(ph.us)))
        .collect();
    format!(
        "{bar}{}  {}",
        " ".repeat(width.saturating_sub(used)),
        legend.join(&p.dim(" · "))
    )
}

fn headers(p: Paint, hs: &[Header], prefix: &str) {
    for h in hs {
        println!("{}{}: {}", p.dim(prefix), p.cyan(&h.name), h.value);
    }
}

pub struct Show {
    pub headers: bool,
    pub body: bool,
}

pub fn run(run: &Run, show: &Show) {
    let p = Paint::stdout();
    println!(
        "{} {}",
        p.dim(&format!("[{}]", run.env)),
        p.bold(&run.request)
    );
    let primary = run.primary_index();
    for (i, e) in run.entries.iter().enumerate() {
        entry(p, e, show.headers, show.body && i == primary);
    }
    if !run.stderr.is_empty() {
        eprintln!("\n{}", Paint::stderr().red(&run.stderr));
    }
    println!("{}", p.dim(&format!("run {}", run.id)));
}

fn entry(p: Paint, e: &Entry, show_headers: bool, show_body: bool) {
    println!("\n{} {}", p.method(&e.method), e.url);
    if show_headers {
        headers(p, &e.request.headers, "> ");
        if let Some(b) = &e.request.body {
            println!("{}", p.dim(&format!("> body {}", bytes(b.len() as u64))));
            for l in pretty(b).trim_end().lines() {
                println!("{} {}", p.dim(">"), l);
            }
        }
    }
    let Some(r) = &e.response else {
        println!("{}", p.red("no response"));
        return;
    };
    let total = e.total_us().map(ms).unwrap_or_default();
    println!(
        "{}  {}  {}  {}",
        p.status(r.status),
        p.bold(&total),
        bytes(r.size),
        p.dim(&r.http_version)
    );
    if let Some(t) = &e.timings {
        println!("{}", waterfall(p, t, 40));
    }
    if show_headers {
        headers(p, &r.headers, "< ");
    }
    for c in &e.captures {
        println!("{} {} = {}", p.dim("capture"), p.cyan(&c.name), c.value);
    }
    for a in e.failed_asserts() {
        println!("{}", p.red(a.message.as_deref().unwrap_or("assert failed")));
    }
    if show_body {
        match (&r.body, r.binary) {
            (_, true) => println!("\n{}", p.dim(&format!("<binary body, {}>", bytes(r.size)))),
            (Some(b), _) if !b.is_empty() => {
                println!();
                println!("{}", pretty(b).trim_end());
                if r.truncated {
                    println!("{}", p.dim("<truncated>"));
                }
            }
            _ => {}
        }
    }
}

pub fn history_line(p: Paint, run: &Run) -> String {
    let e = run.primary();
    let status = e
        .and_then(|e| e.response.as_ref())
        .map(|r| p.status(r.status))
        .unwrap_or_else(|| p.red("ERR"));
    let lat = e
        .and_then(|e| e.total_us())
        .map(ms)
        .unwrap_or_else(|| "-".into());
    let size = e
        .and_then(|e| e.response.as_ref())
        .map(|r| bytes(r.size))
        .unwrap_or_default();
    format!(
        "{}  {:<10} {:<34} {:>8} {:>9}  {}",
        p.dim(&run.started_at[..19].replace('T', " ")),
        run.env,
        run.request,
        lat,
        size,
        status
    )
}

pub fn green(msg: &str) -> String {
    Paint::stderr().green(msg)
}
