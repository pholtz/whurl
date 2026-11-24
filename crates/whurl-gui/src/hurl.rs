//! The primary request of a `.hurl` source as form fields (method, URL, headers, body), and the
//! source with those fields written back. Everything else in the file is kept as it was:
//! earlier entries, comments, sections like `[Options]`, and the `HTTP` response part.

use rune_kit::syntax::{self, Highlight, Paint, Syntax};
use std::ops::Range;
use whurl_core::workspace;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parts {
    pub method: String,
    pub url: String,
    /// `Name: value` lines.
    pub headers: String,
    /// The body as hurl writes it: JSON, XML, a ``` block, `file,…;` and so on.
    pub body: String,
}

/// Line ranges of the primary request.
struct Layout {
    req: usize,
    headers: Range<usize>,
    body: Range<usize>,
}

fn lines(src: &str) -> Vec<&str> {
    src.strip_suffix('\n').unwrap_or(src).split('\n').collect()
}

fn is_header(l: &str) -> bool {
    let Some((k, _)) = l.split_once(':') else {
        return false;
    };
    !k.is_empty()
        && !l.starts_with(char::is_whitespace)
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.{}$".contains(c))
}

fn is_section(l: &str) -> bool {
    let t = l.trim();
    t.len() > 2
        && t.starts_with('[')
        && t.ends_with(']')
        && t[1..t.len() - 1].chars().all(|c| c.is_ascii_alphabetic())
}

fn is_response(l: &str) -> bool {
    l.strip_prefix("HTTP")
        .is_some_and(|r| r.starts_with([' ', '/']))
}

fn layout(src: &str) -> Option<Layout> {
    let ls = lines(src);
    let req = workspace::entries(src).pop()?.line - 1;
    let mut i = req + 1;
    while i < ls.len() && is_header(ls[i]) {
        i += 1;
    }
    let headers = req + 1..i;
    // The request ends at its response (`HTTP 200`) or the end of the file; skip fenced bodies.
    let mut end = ls.len();
    let mut fence = false;
    for (j, l) in ls.iter().enumerate().skip(i) {
        if l.trim_start().starts_with("```") {
            fence = !fence;
        } else if !fence && is_response(l) {
            end = j;
            break;
        }
    }
    // The body is the first line that isn't blank, a comment, or a section and its fields.
    let mut in_section = false;
    let mut start = None;
    for (j, l) in ls.iter().enumerate().take(end).skip(i) {
        let t = l.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if is_section(t) {
            in_section = true;
            continue;
        }
        let opens_body = t.starts_with(['{', '[', '<', '"', '`']);
        if in_section && !opens_body && t.contains(':') {
            continue;
        }
        start = Some(j);
        break;
    }
    // Trailing blank lines stay between the body and the response.
    let mut stop = end;
    while stop > i && ls[stop - 1].trim().is_empty() {
        stop -= 1;
    }
    let body = match start {
        Some(s) => s..stop.max(s),
        None => stop..stop,
    };
    Some(Layout { req, headers, body })
}

/// The primary request's fields, or `None` when the source has no request line.
pub fn parts(src: &str) -> Option<Parts> {
    let l = layout(src)?;
    let ls = lines(src);
    let mut words = ls[l.req].split_whitespace();
    Some(Parts {
        method: words.next().unwrap_or_default().to_owned(),
        url: words.next().unwrap_or_default().to_owned(),
        headers: ls[l.headers].join("\n"),
        body: ls[l.body].join("\n"),
    })
}

/// `src` with the primary request's fields replaced by `p`.
pub fn with_parts(src: &str, p: &Parts) -> Option<String> {
    let l = layout(src)?;
    let ls = lines(src);
    let mut out: Vec<&str> = ls[..l.req].to_vec();
    let line = format!("{} {}", p.method.trim(), p.url.trim());
    out.push(&line);
    out.extend(p.headers.lines().filter(|h| !h.trim().is_empty()));
    out.extend(&ls[l.headers.end..l.body.start]);
    let body = p.body.trim_end_matches('\n');
    if !body.is_empty() {
        out.extend(body.split('\n'));
    }
    // A body added before a response keeps a line between them.
    let rest = &ls[l.body.end..];
    if l.body.is_empty() && !body.is_empty() && rest.first().is_some_and(|r| is_response(r)) {
        out.push("");
    }
    out.extend(rest);
    let mut s = out.join("\n");
    if src.ends_with('\n') {
        s.push('\n');
    }
    Some(s)
}

/// Highlights for hurl source (and its header lines): comments, request and status lines,
/// headers, sections and `{{variables}}`; JSON bodies go to the built-in JSON highlighter.
pub fn highlight(src: &str) -> Vec<Highlight> {
    let mut out: Vec<Highlight> = vec![];
    let mut at = 0;
    let mut json: Option<usize> = None;
    let mut fence = false;
    for line in src.split_inclusive('\n') {
        let l = line.trim_end_matches(['\n', '\r']);
        let t = l.trim_start();
        let pad = l.len() - t.len();
        let span = |r: Range<usize>, k: Syntax| (at + r.start..at + r.end, Paint::Kind(k));
        // A JSON body runs from a line opening with { or [ to a line closing it at the margin.
        if json.is_none()
            && !fence
            && (t.starts_with('{') || (t.starts_with('[') && !is_section(t)))
        {
            json = Some(at);
        }
        if let Some(start) = json {
            if l.starts_with(['}', ']']) || (at == start && (t.ends_with('}') || t.ends_with(']')))
            {
                let end = at + l.len();
                let block = &src[start..end];
                // `{{vars}}` inside the JSON take over the JSON colours where they sit.
                let mut vs = vec![];
                vars(
                    block,
                    0,
                    &mut |r, k| {
                        if k == Syntax::Variable {
                            vs.push(r)
                        }
                    },
                    Syntax::String,
                );
                for (r, p) in syntax::highlight(Some("json"), block).iter() {
                    for piece in minus(r.clone(), &vs) {
                        out.push((start + piece.start..start + piece.end, *p));
                    }
                }
                out.extend(vs.into_iter().map(|r| {
                    (
                        start + r.start..start + r.end,
                        Paint::Kind(Syntax::Variable),
                    )
                }));
                json = None;
            }
            at += line.len();
            continue;
        }
        if t.starts_with("```") {
            fence = !fence;
            out.push(span(pad..l.len(), Syntax::Punctuation));
        } else if fence {
            vars(l, 0, &mut |r, k| out.push(span(r, k)), Syntax::String);
        } else if t.starts_with('#') {
            out.push(span(pad..l.len(), Syntax::Comment));
        } else if is_section(t) {
            out.push(span(pad..l.len(), Syntax::Type));
        } else if is_response(l) {
            let sp = l.find(' ').unwrap_or(l.len());
            out.push(span(0..sp, Syntax::Keyword));
            out.push(span(sp..l.len(), Syntax::Number));
        } else if let Some(m) = l
            .split_whitespace()
            .next()
            .filter(|m| workspace::METHODS.contains(m))
        {
            out.push(span(0..m.len(), Syntax::Keyword));
            vars(l, m.len(), &mut |r, k| out.push(span(r, k)), Syntax::Link);
        } else if is_header(l) {
            let colon = l.find(':').unwrap_or(0);
            out.push(span(0..colon, Syntax::Property));
            out.push(span(colon..colon + 1, Syntax::Punctuation));
            vars(
                l,
                colon + 1,
                &mut |r, k| out.push(span(r, k)),
                Syntax::String,
            );
        }
        at += line.len();
    }
    out
}

/// Highlights for a dotenv file: comments, then `NAME` `=` `value` lines.
pub fn highlight_env(src: &str) -> Vec<Highlight> {
    let mut out = vec![];
    let mut at = 0;
    for line in src.split_inclusive('\n') {
        let l = line.trim_end_matches(['\n', '\r']);
        let t = l.trim_start();
        let pad = l.len() - t.len();
        let span = |r: Range<usize>, k: Syntax| (at + r.start..at + r.end, Paint::Kind(k));
        if t.starts_with('#') {
            out.push(span(pad..l.len(), Syntax::Comment));
        } else if let Some(eq) = l.find('=') {
            out.push(span(0..eq, Syntax::Property));
            out.push(span(eq..eq + 1, Syntax::Punctuation));
            out.push(span(eq + 1..l.len(), Syntax::String));
        }
        at += line.len();
    }
    out
}

/// `r` without the (sorted, disjoint) ranges in `holes`.
fn minus(r: Range<usize>, holes: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut out = vec![];
    let mut at = r.start;
    for h in holes.iter().filter(|h| h.start < r.end && h.end > r.start) {
        if h.start > at {
            out.push(at..h.start);
        }
        at = at.max(h.end);
    }
    if at < r.end {
        out.push(at..r.end);
    }
    out
}

/// Each `{{variable}}` in `src`: its range, braces included, and its name.
pub fn var_ranges(src: &str) -> Vec<(Range<usize>, &str)> {
    let mut out = vec![];
    let mut at = 0;
    while let Some(o) = src[at..].find("{{").map(|o| at + o) {
        let Some(c) = src[o..].find("}}").map(|c| o + c) else {
            break;
        };
        out.push((o..c + 2, src[o + 2..c].trim()));
        at = c + 2;
    }
    out
}

/// The names of the `{{variables}}` in `src`, each once, in order.
pub fn var_names(src: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for (_, name) in var_ranges(src) {
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_owned());
        }
    }
    out
}

/// Variables `[Captures]` sections set for the entries after them.
pub fn captures(src: &str) -> Vec<String> {
    let mut out = vec![];
    let mut on = false;
    for l in src.lines() {
        let t = l.trim();
        if is_section(t) {
            on = t == "[Captures]";
        } else if on && is_header(t) {
            out.push(t.split(':').next().unwrap_or_default().trim().to_owned());
        } else if on && !t.is_empty() && !t.starts_with('#') {
            on = false;
        }
    }
    out
}

/// `l[from..]` as `rest` with its `{{variables}}` picked out.
fn vars(l: &str, from: usize, push: &mut dyn FnMut(Range<usize>, Syntax), rest: Syntax) {
    let mut i = from;
    while let Some(o) = l[i..].find("{{").map(|o| i + o) {
        let Some(c) = l[o..].find("}}").map(|c| o + c + 2) else {
            break;
        };
        if o > i {
            push(i..o, rest);
        }
        push(o..c, Syntax::Variable);
        i = c;
    }
    if i < l.len() {
        push(i..l.len(), rest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "# login\nPOST {{host}}/login\nContent-Type: application/json\n[Options]\nverbose: true\n{\n  \"user\": \"a\"\n}\n\nHTTP 200\n[Captures]\ntoken: jsonpath \"$.t\"\n";

    #[test]
    fn reads_the_primary_request() {
        let p = parts(SRC).unwrap();
        assert_eq!(
            (p.method.as_str(), p.url.as_str()),
            ("POST", "{{host}}/login")
        );
        assert_eq!(p.headers, "Content-Type: application/json");
        assert_eq!(p.body, "{\n  \"user\": \"a\"\n}");
    }

    #[test]
    fn writes_fields_back_keeping_the_rest() {
        let same = with_parts(SRC, &parts(SRC).unwrap()).unwrap();
        assert_eq!(same, SRC);
        let p = Parts {
            method: "PUT".into(),
            url: "{{host}}/login2".into(),
            headers: "Accept: */*\nX-A: 1".into(),
            body: String::new(),
        };
        let out = with_parts(SRC, &p).unwrap();
        assert_eq!(
            out,
            "# login\nPUT {{host}}/login2\nAccept: */*\nX-A: 1\n[Options]\nverbose: true\n\nHTTP 200\n[Captures]\ntoken: jsonpath \"$.t\"\n"
        );
        // Adding the body back lands it before the response again.
        let back = with_parts(&out, &parts(SRC).unwrap()).unwrap();
        assert!(back.contains("verbose: true\n{\n  \"user\": \"a\"\n}\n\nHTTP 200"));
    }

    #[test]
    fn only_the_last_entry_is_the_form() {
        let src = "GET a\nHTTP 200\n\nGET b\n```\nGET c\n```\n";
        let p = parts(src).unwrap();
        assert_eq!((p.url.as_str(), p.body.as_str()), ("b", "```\nGET c\n```"));
    }
}
