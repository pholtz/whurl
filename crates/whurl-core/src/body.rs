//! The request or response body of one entry, ready to view or copy.

use crate::export::{is_json, pretty};
use crate::run::Entry;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Request,
    Response,
}

pub struct Body {
    /// Pretty-printed when JSON; already redacted when stored.
    pub text: String,
    /// File extension for its content type, so a viewer picks the right syntax.
    pub ext: &'static str,
    /// `"request"` or `"response"`, for messages.
    pub what: &'static str,
    /// The stored response body was cut short.
    pub truncated: bool,
}

/// The entry's request or response body; an error says why there is none to show.
pub fn of(e: &Entry, side: Side) -> Result<Body, String> {
    let (body, headers, what) = match (side, &e.response) {
        (Side::Request, _) => (e.request.body.as_deref(), &e.request.headers, "request"),
        (_, Some(r)) if r.binary => {
            return Err("binary response body: export it instead".into());
        }
        (_, Some(r)) => (r.body.as_deref(), &r.headers, "response"),
        (_, None) => return Err("no response".into()),
    };
    let body = body
        .filter(|b| !b.is_empty())
        .ok_or_else(|| format!("empty {what} body"))?;
    let ctype = headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.as_str());
    Ok(Body {
        text: pretty(body),
        ext: extension(ctype, body),
        what,
        truncated: side == Side::Response && e.response.as_ref().is_some_and(|r| r.truncated),
    })
}

/// File extension for a body by its content type, else by sniffing for JSON.
pub fn extension(ctype: Option<&str>, body: &str) -> &'static str {
    let c = ctype.unwrap_or_default().to_lowercase();
    let by_type = [
        ("json", "json"),
        ("html", "html"),
        ("xml", "xml"),
        ("javascript", "js"),
        ("css", "css"),
        ("csv", "csv"),
        ("yaml", "yaml"),
        ("graphql", "graphql"),
        ("x-www-form-urlencoded", "txt"),
        ("text/plain", "txt"),
    ];
    match by_type.iter().find(|(k, _)| c.contains(k)) {
        Some((_, ext)) => ext,
        None if is_json(body) => "json",
        None => "txt",
    }
}
