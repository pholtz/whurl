//! Shared formatting plus the Markdown export meant for handing a run to an agent.

use crate::run::{Entry, Header, Run};
use crate::workspace::Workspace;
use std::fmt::Write;
use std::path::PathBuf;

/// A run as pretty JSON or as Markdown with its hurl source.
pub fn render(run: &Run, json: bool) -> String {
    if json {
        serde_json::to_string_pretty(run).unwrap_or_default() + "\n"
    } else {
        markdown(run, std::fs::read_to_string(&run.file).ok().as_deref())
    }
}

/// Write `render(run, json)` to `.whurl/exports/<run id>.{json,md}`.
pub fn save(ws: &Workspace, run: &Run, json: bool) -> std::io::Result<PathBuf> {
    let dir = ws.state_dir().join("exports");
    let path = dir.join(format!("{}.{}", run.id, if json { "json" } else { "md" }));
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&path, render(run, json))?;
    Ok(path)
}

/// One line for a finished run: `200 in 41ms`.
pub fn summary(run: &Run) -> String {
    match run.primary() {
        Some(e) => match &e.response {
            Some(resp) => format!(
                "{} in {}",
                resp.status,
                e.total_us().map(ms).unwrap_or_default()
            ),
            None => "no response".into(),
        },
        None => "no entries".into(),
    }
}

pub fn ms(us: u64) -> String {
    if us >= 10_000_000 {
        format!("{:.1}s", us as f64 / 1e6)
    } else if us >= 1_000_000 {
        format!("{:.2}s", us as f64 / 1e6)
    } else if us >= 10_000 {
        format!("{}ms", us / 1000)
    } else if us >= 1000 {
        format!("{:.1}ms", us as f64 / 1000.0)
    } else {
        format!("{us}µs")
    }
}

pub fn bytes(n: u64) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

pub fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

/// Pretty-print JSON bodies; leave anything else untouched.
pub fn pretty(body: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v) if body.trim_start().starts_with(['{', '[']) => {
            serde_json::to_string_pretty(&v).unwrap_or_else(|_| body.to_string())
        }
        _ => body.to_string(),
    }
}

pub fn is_json(body: &str) -> bool {
    let t = body.trim_start();
    t.starts_with(['{', '[']) && serde_json::from_str::<serde_json::Value>(body).is_ok()
}

fn fence(body: &str) -> String {
    let lang = if is_json(body) { "json" } else { "" };
    // Use a fence longer than any backtick run in the body.
    let longest = body.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let ticks = "`".repeat(longest.max(2) + 1);
    format!("{ticks}{lang}\n{}\n{ticks}\n", pretty(body).trim_end())
}

fn headers_md(out: &mut String, hs: &[Header]) {
    out.push_str("```http\n");
    for h in hs {
        let _ = writeln!(out, "{}: {}", h.name, h.value);
    }
    out.push_str("```\n");
}

pub fn markdown(run: &Run, source: Option<&str>) -> String {
    let mut o = String::new();
    let status = run
        .primary()
        .and_then(|e| e.response.as_ref())
        .map(|r| r.status);
    let _ = writeln!(o, "# whurl run: {}\n", run.request);
    let _ = writeln!(o, "- **Run id:** `{}`", run.id);
    let _ = writeln!(o, "- **Environment:** {}", run.env);
    let _ = writeln!(o, "- **Started:** {}", run.started_at);
    let _ = writeln!(o, "- **Hurl file:** `{}`", run.file.display());
    let _ = writeln!(
        o,
        "- **Asserts:** {}",
        if run.asserts {
            "evaluated"
        } else {
            "skipped (--no-assert)"
        }
    );
    let _ = writeln!(
        o,
        "- **Result:** {} (hurl exit {}){}",
        if run.success { "success" } else { "FAILED" },
        run.exit_code,
        status
            .map(|s| format!(", final status {s}"))
            .unwrap_or_default()
    );
    let _ = writeln!(o, "- **Requests in file:** {}", run.entries.len());
    o.push_str("\nSecret values from the env's secrets files are replaced with `***`.\n");

    if !run.stderr.is_empty() {
        let _ = write!(o, "\n## Hurl errors\n\n```\n{}\n```\n", run.stderr);
    }
    for e in &run.entries {
        entry_md(&mut o, e, run.entries.len() > 1);
    }
    if let Some(src) = source {
        let _ = write!(o, "\n## Hurl source\n\n```hurl\n{}\n```\n", src.trim_end());
    }
    o
}

fn entry_md(o: &mut String, e: &Entry, numbered: bool) {
    let _ = writeln!(
        o,
        "\n## {}{} {}\n",
        if numbered {
            format!("Entry {} (line {}): ", e.index, e.line)
        } else {
            String::new()
        },
        e.method,
        e.url
    );
    let _ = writeln!(o, "### Request\n");
    headers_md(o, &e.request.headers);
    if let Some(b) = &e.request.body {
        let _ = writeln!(o, "\nBody ({}):\n", bytes(b.len() as u64));
        o.push_str(&fence(b));
    }
    let _ = writeln!(o, "\nEquivalent curl:\n\n```sh\n{}\n```", e.curl);

    let Some(r) = &e.response else {
        o.push_str(
            "\n### Response\n\nNo response (connection or earlier failure; see hurl errors).\n",
        );
        return;
    };
    let _ = writeln!(
        o,
        "\n### Response: {} {} ({})\n",
        r.status,
        reason(r.status),
        r.http_version
    );
    if let Some(t) = &e.timings {
        let _ = writeln!(
            o,
            "Total latency {}, body {}.\n",
            ms(t.total),
            bytes(r.size)
        );
        o.push_str("| phase | time |\n|---|---|\n");
        for p in t.phases().iter().filter(|p| p.us > 0) {
            let _ = writeln!(o, "| {} | {} |", p.name, ms(p.us));
        }
        o.push('\n');
    }
    headers_md(o, &r.headers);
    match (&r.body, r.binary) {
        (_, true) => {
            let _ = writeln!(o, "\nBody: binary, {}.", bytes(r.size));
        }
        (Some(b), _) => {
            let _ = writeln!(
                o,
                "\nBody{}:\n",
                if r.truncated { " (truncated)" } else { "" }
            );
            o.push_str(&fence(b));
        }
        (None, _) => o.push_str("\nBody: empty.\n"),
    }
    if !e.captures.is_empty() {
        o.push_str("\nCaptures:\n\n");
        for c in &e.captures {
            let _ = writeln!(o, "- `{}` = `{}`", c.name, c.value);
        }
    }
    let failed: Vec<_> = e.failed_asserts().collect();
    if !e.asserts.is_empty() {
        let _ = writeln!(
            o,
            "\nAsserts: {}/{} passed.",
            e.asserts.len() - failed.len(),
            e.asserts.len()
        );
        for a in failed {
            let _ = write!(
                o,
                "\n```\n{}\n```\n",
                a.message.as_deref().unwrap_or("failed")
            );
        }
    }
}
