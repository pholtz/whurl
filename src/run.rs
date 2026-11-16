//! Running hurl and turning its JSON report into a redacted, self-contained `Run` record.

use crate::workspace::{Redactor, RequestFile, Workspace};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// Response bodies larger than this are truncated in the stored record.
const MAX_BODY: usize = 2 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Run {
    pub id: String,
    /// Request name, e.g. `job/create_job`.
    pub request: String,
    pub file: PathBuf,
    pub env: String,
    pub asserts: bool,
    pub started_at: String,
    /// Wall time of the hurl process, including startup.
    pub wall_ms: u64,
    pub success: bool,
    pub exit_code: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stderr: String,
    pub entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    pub index: usize,
    pub line: usize,
    pub method: String,
    pub url: String,
    pub curl: String,
    pub request: Request,
    pub response: Option<Response>,
    pub timings: Option<Timings>,
    #[serde(default)]
    pub captures: Vec<Header>,
    #[serde(default)]
    pub asserts: Vec<AssertResult>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Request {
    pub headers: Vec<Header>,
    pub body: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub http_version: String,
    pub headers: Vec<Header>,
    pub body: Option<String>,
    pub size: u64,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub binary: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AssertResult {
    pub line: usize,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// curl's cumulative timestamps, in microseconds from the start of the call.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Timings {
    pub name_lookup: u64,
    pub connect: u64,
    pub app_connect: u64,
    pub pre_transfer: u64,
    pub start_transfer: u64,
    pub total: u64,
}

/// One segment of a request's latency.
pub struct Phase {
    pub name: &'static str,
    pub us: u64,
}

impl Timings {
    /// Split the cumulative timestamps into consecutive phases that sum to `total`.
    pub fn phases(&self) -> Vec<Phase> {
        let dns = self.name_lookup;
        let tcp = self.connect.saturating_sub(dns);
        let tls = if self.app_connect > 0 { self.app_connect.saturating_sub(self.connect) } else { 0 };
        let setup_end = self.connect.max(self.app_connect);
        let send = self.pre_transfer.saturating_sub(setup_end);
        let wait = self.start_transfer.saturating_sub(self.pre_transfer.max(setup_end));
        let recv = self.total.saturating_sub(self.start_transfer);
        vec![
            Phase { name: "dns", us: dns },
            Phase { name: "connect", us: tcp },
            Phase { name: "tls", us: tls },
            Phase { name: "send", us: send },
            Phase { name: "wait", us: wait },
            Phase { name: "download", us: recv },
        ]
    }
}

impl Entry {
    pub fn total_us(&self) -> Option<u64> {
        self.timings.as_ref().map(|t| t.total)
    }
    pub fn failed_asserts(&self) -> impl Iterator<Item = &AssertResult> {
        self.asserts.iter().filter(|a| !a.success)
    }
}

impl Run {
    /// The entry to show by default: the last one that produced a response.
    pub fn primary(&self) -> Option<&Entry> {
        self.entries.iter().rev().find(|e| e.response.is_some()).or(self.entries.last())
    }
    pub fn primary_index(&self) -> usize {
        self.entries
            .iter()
            .rposition(|e| e.response.is_some())
            .unwrap_or(self.entries.len().saturating_sub(1))
    }
}

pub struct RunOpts {
    pub env: String,
    pub asserts: bool,
    pub extra_args: Vec<String>,
}

pub fn run(ws: &Workspace, req: &RequestFile, opts: &RunOpts) -> Result<Run> {
    let tmp = tempfile::tempdir()?;
    let report_dir = tmp.path().join("report");
    let mut cmd = Command::new("hurl");
    let (vars, secrets) = env_files(ws, &opts.env, tmp.path())?;
    cmd.arg("--variables-file").arg(vars);
    if let Some(s) = secrets {
        cmd.arg("--secrets-file").arg(s);
    }
    cmd.arg("--report-json").arg(&report_dir).arg("--no-output");
    if !opts.asserts {
        cmd.arg("--no-assert");
    }
    cmd.args(&opts.extra_args).arg(&req.path);
    cmd.env("NO_COLOR", "1");

    let started_at = chrono::Local::now();
    let t0 = Instant::now();
    let out = cmd.output().context("running hurl (is it installed and on PATH?)")?;
    let wall_ms = t0.elapsed().as_millis() as u64;
    let redact = ws.redactor(&opts.env);
    let stderr = redact.apply(strip_ansi(&String::from_utf8_lossy(&out.stderr)).trim_end());

    let report_path = report_dir.join("report.json");
    let report: Value = match fs::read_to_string(&report_path) {
        Ok(s) => serde_json::from_str(&s).context("parsing hurl report")?,
        Err(_) if !stderr.is_empty() => Value::Array(vec![]),
        Err(e) => bail!("hurl wrote no report ({e})"),
    };
    let file_report = report.as_array().and_then(|a| a.first());
    let entries = file_report
        .and_then(|f| f["entries"].as_array())
        .map(|es| es.iter().map(|e| parse_entry(e, &report_dir, &redact)).collect())
        .unwrap_or_default();

    let run = Run {
        id: format!("{}_{}_{}", started_at.format("%Y%m%d-%H%M%S-%3f"), opts.env, req.rel.replace('/', "__")),
        request: req.rel.clone(),
        file: req.path.clone(),
        env: opts.env.clone(),
        asserts: opts.asserts,
        started_at: started_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        wall_ms,
        success: out.status.success(),
        exit_code: out.status.code().unwrap_or(-1),
        stderr,
        entries,
    };
    crate::store::save(ws, &run)?;
    Ok(run)
}

/// Write the variables and secrets files hurl actually gets into `dir`. hurl treats an empty
/// secret as the string to redact and puts `***` between every character of its output, so
/// empty-valued secrets move to the variables file (an empty value has nothing to hide).
fn env_files(ws: &Workspace, env: &str, dir: &Path) -> Result<(PathBuf, Option<PathBuf>)> {
    let mut vars = fs::read_to_string(ws.vars_file(env)?)?;
    let mut secrets = String::new();
    for f in ws.secrets_files(env) {
        let (kept, empty) = split_empty_secrets(&fs::read_to_string(&f)?);
        secrets += &kept;
        if !vars.is_empty() && !vars.ends_with('\n') {
            vars.push('\n');
        }
        vars += &empty;
    }
    let vars_path = dir.join("vars.env");
    fs::write(&vars_path, vars)?;
    if secrets.is_empty() {
        return Ok((vars_path, None));
    }
    let secrets_path = dir.join("secrets.env");
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    std::io::Write::write_all(&mut opts.open(&secrets_path)?, secrets.as_bytes())?;
    Ok((vars_path, Some(secrets_path)))
}

/// Split a secrets file into (lines to keep as secrets, `name=` lines with empty values).
/// Comments and blank lines are dropped; kept lines are passed through verbatim.
fn split_empty_secrets(src: &str) -> (String, String) {
    let (mut kept, mut empty) = (String::new(), String::new());
    for line in src.lines() {
        let t = line.trim();
        let Some((name, value)) = t.split_once('=') else { continue };
        if t.starts_with('#') || name.trim().is_empty() {
            continue;
        }
        if value.trim().is_empty() {
            empty += &format!("{}=\n", name.trim());
        } else {
            kept += line;
            kept.push('\n');
        }
    }
    (kept, empty)
}

fn parse_entry(e: &Value, report_dir: &Path, redact: &Redactor) -> Entry {
    let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let headers = |v: &Value| -> Vec<Header> {
        v.as_array()
            .into_iter()
            .flatten()
            .map(|h| Header { name: s(&h["name"]), value: redact.apply(&s(&h["value"])) })
            .collect()
    };
    let curl = s(&e["curl_cmd"]);
    let call = e["calls"].as_array().and_then(|c| c.last());
    let (method, url) = match call {
        Some(c) => (s(&c["request"]["method"]), s(&c["request"]["url"])),
        None => curl_method_url(&curl),
    };
    let request = Request {
        headers: call.map(|c| headers(&c["request"]["headers"])).unwrap_or_default(),
        body: curl_body(&curl).map(|b| redact.apply(&b)),
    };
    let response = call.map(|c| {
        let r = &c["response"];
        let (body, size, truncated, binary) = read_body(r["body"].as_str().map(|p| report_dir.join(p)));
        Response {
            status: r["status"].as_u64().unwrap_or(0) as u16,
            http_version: s(&r["http_version"]),
            headers: headers(&r["headers"]),
            body: body.map(|b| redact.apply(&b)),
            size,
            truncated,
            binary,
        }
    });
    let timings = call.map(|c| {
        let t = &c["timings"];
        let n = |k: &str| t[k].as_u64().unwrap_or(0);
        Timings {
            name_lookup: n("name_lookup"),
            connect: n("connect"),
            app_connect: n("app_connect"),
            pre_transfer: n("pre_transfer"),
            start_transfer: n("start_transfer"),
            total: n("total"),
        }
    });
    Entry {
        index: e["index"].as_u64().unwrap_or(0) as usize,
        line: e["line"].as_u64().unwrap_or(0) as usize,
        method,
        url: redact.apply(&url),
        curl: redact.apply(&curl),
        request,
        response,
        timings,
        captures: headers(&e["captures"]),
        asserts: e["asserts"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| AssertResult {
                line: a["line"].as_u64().unwrap_or(0) as usize,
                success: a["success"].as_bool().unwrap_or(false),
                message: a["message"].as_str().map(|m| redact.apply(m)),
            })
            .collect(),
    }
}

fn read_body(path: Option<PathBuf>) -> (Option<String>, u64, bool, bool) {
    let Some(bytes) = path.and_then(|p| fs::read(p).ok()) else {
        return (None, 0, false, false);
    };
    let size = bytes.len() as u64;
    match String::from_utf8(bytes) {
        Ok(mut text) => {
            let truncated = text.len() > MAX_BODY;
            if truncated {
                let mut cut = MAX_BODY;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
            }
            (Some(text), size, truncated, false)
        }
        Err(_) => (None, size, false, true),
    }
}

fn curl_method_url(curl: &str) -> (String, String) {
    let args = shell_words(curl);
    let method = args
        .windows(2)
        .find(|w| w[0] == "--request" || w[0] == "-X")
        .map(|w| w[1].clone())
        .unwrap_or_else(|| if curl_body(curl).is_some() { "POST" } else { "GET" }.into());
    let url = args.iter().rev().find(|a| a.contains("://")).cloned().unwrap_or_default();
    (method, url)
}

/// Hurl's report doesn't include the request body, but its curl command does.
fn curl_body(curl: &str) -> Option<String> {
    let args = shell_words(curl);
    let parts: Vec<&str> = args
        .windows(2)
        .filter(|w| matches!(w[0].as_str(), "--data" | "--data-binary" | "--data-raw" | "--data-urlencode" | "--form"))
        .map(|w| w[1].as_str())
        .collect();
    match parts.as_slice() {
        [] => None,
        [one] => Some(one.to_string()),
        many => Some(many.join("&")),
    }
}

/// Split a POSIX-shell command line as hurl writes it: '…', $'…' (ANSI-C), "…", and \x.
fn shell_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    cur.push(c);
                }
            }
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '\'' => break,
                        '\\' => match chars.next() {
                            Some('n') => cur.push('\n'),
                            Some('t') => cur.push('\t'),
                            Some('r') => cur.push('\r'),
                            Some('0') => cur.push('\0'),
                            Some('x') => {
                                let hex: String = (0..2).filter_map(|_| chars.next()).collect();
                                if let Ok(b) = u8::from_str_radix(&hex, 16) {
                                    cur.push(b as char);
                                }
                            }
                            Some('u') => {
                                let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                    cur.push(ch);
                                }
                            }
                            Some(o) => cur.push(o),
                            None => {}
                        },
                        o => cur.push(o),
                    }
                }
            }
            '"' => {
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => {
                            if let Some(n) = chars.next() {
                                cur.push(n);
                            }
                        }
                        o => cur.push(o),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            o => {
                in_word = true;
                cur.push(o);
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    out
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty secret makes hurl interleave `***` through its whole error output.
    #[test]
    fn empty_secrets_become_variables() {
        let (kept, empty) = split_empty_secrets("# c\na=xyz1\nb=\n c = \n\nd=k=v\n");
        assert_eq!(kept, "a=xyz1\nd=k=v\n");
        assert_eq!(empty, "b=\nc=\n");
    }

    #[test]
    fn curl_body_ansi_c() {
        let c = r#"curl --header 'Authorization: Bearer x' --data $'{"a": "it\'s", "b": 1}\n' 'http://h/x'"#;
        assert_eq!(curl_body(c).as_deref(), Some("{\"a\": \"it's\", \"b\": 1}\n"));
        assert_eq!(curl_method_url(c), ("POST".into(), "http://h/x".into()));
    }

    #[test]
    fn curl_concatenated_quotes() {
        assert_eq!(shell_words(r"--data 'a'\''b' x"), vec!["--data", "a'b", "x"]);
    }

    #[test]
    fn phases_sum_to_total() {
        let t = Timings { name_lookup: 10, connect: 30, app_connect: 70, pre_transfer: 75, start_transfer: 900, total: 950 };
        let p = t.phases();
        assert_eq!(p.iter().map(|p| p.us).sum::<u64>(), 950);
        assert_eq!(p[4].us, 825);
    }
}
