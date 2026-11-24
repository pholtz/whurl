//! Config settings by `table.key`: listing, validating, applying to a `Config`, and saving to
//! `config.toml` with its comments kept. Frontends layer session-only values on top.

use crate::config::{self, Config, REQUEST_SECTIONS, RESPONSE_SECTIONS, SidebarStyle};
use anyhow::Result;

/// `ui.theme` values; whurl-gui draws them, `default` following the system's light or dark.
pub const THEMES: &[&str] = &["default", "light", "dark", "nord"];

/// (key, accepted values) in the order frontends list them.
pub const KEYS: &[(&str, &str)] = &[
    ("ui.sidebar", "tree | flat | hidden"),
    ("ui.wrap", "on | off"),
    ("ui.auto_switch_response", "on | off"),
    ("ui.mouse", "on | off"),
    ("ui.theme", "default | light | dark | nord (whurl-gui)"),
    ("run.asserts", "on | off"),
    ("run.default_env", "an env name, or unset"),
    ("request.sections", "comma list of headers, body, curl"),
    ("response.sections", "comma list of headers, body"),
];

pub fn is_bool(key: &str) -> bool {
    KEYS.iter().any(|(k, v)| *k == key && *v == "on | off")
}

fn sections_of(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "request.sections" => Some(REQUEST_SECTIONS),
        "response.sections" => Some(RESPONSE_SECTIONS),
        _ => None,
    }
}

pub fn on_off(v: bool) -> String {
    if v { "on" } else { "off" }.into()
}

fn parse_bool(s: &str) -> Result<bool, String> {
    match s.to_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => Err(format!("expected on or off, got {s}")),
    }
}

/// The value in `cfg`, as shown and typed; `None` for an unknown key.
pub fn get(cfg: &Config, key: &str) -> Option<String> {
    Some(match key {
        "ui.sidebar" => cfg.ui.sidebar.name().into(),
        "ui.wrap" => on_off(cfg.ui.wrap),
        "ui.auto_switch_response" => on_off(cfg.ui.auto_switch_response),
        "ui.mouse" => on_off(cfg.ui.mouse),
        "ui.theme" => cfg.ui.theme.clone(),
        "run.asserts" => on_off(cfg.run.asserts),
        "run.default_env" => cfg
            .run
            .default_env
            .clone()
            .unwrap_or_else(|| "unset".into()),
        "request.sections" => cfg.request.or(REQUEST_SECTIONS).join(","),
        "response.sections" => cfg.response.or(RESPONSE_SECTIONS).join(","),
        _ => return None,
    })
}

/// Values to offer for `key`; for section lists, `typed` is the list so far.
pub fn values(key: &str, typed: &str, envs: &[String]) -> Vec<String> {
    if is_bool(key) {
        return vec!["on".into(), "off".into()];
    }
    if let Some(allowed) = sections_of(key) {
        // Complete the next name in the list, keeping what is already typed.
        let (done, _) = typed.rsplit_once(',').unwrap_or(("", typed));
        let have: Vec<&str> = done.split(',').filter(|s| !s.is_empty()).collect();
        let prefix = if done.is_empty() {
            String::new()
        } else {
            format!("{done},")
        };
        return allowed
            .iter()
            .filter(|a| !have.contains(a))
            .map(|a| format!("{prefix}{a}"))
            .collect();
    }
    match key {
        "ui.sidebar" => SidebarStyle::ALL
            .iter()
            .map(|s| s.name().to_string())
            .collect(),
        "ui.theme" => THEMES.iter().map(|t| t.to_string()).collect(),
        "run.default_env" => std::iter::once("unset".to_string())
            .chain(envs.iter().cloned())
            .collect(),
        _ => vec![],
    }
}

/// A validated setting value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Sidebar(SidebarStyle),
    Bool(bool),
    /// `None` is `unset`.
    Env(Option<String>),
    Sections(Vec<String>),
    Theme(String),
}

impl Value {
    /// What `config.toml` stores; `None` removes the key.
    pub fn to_toml(&self) -> Option<toml_edit::Value> {
        match self {
            Value::Sidebar(s) => Some(s.name().into()),
            Value::Bool(b) => Some((*b).into()),
            Value::Env(e) => e.as_deref().map(Into::into),
            Value::Sections(l) => Some(toml_edit::Array::from_iter(l).into()),
            Value::Theme(t) => Some(t.as_str().into()),
        }
    }
}

/// Validate `value` for `key`. `envs` are the workspace's, for `run.default_env`.
pub fn parse(key: &str, value: &str, envs: &[String]) -> Result<Value, String> {
    if !KEYS.iter().any(|(k, _)| *k == key) {
        let keys: Vec<_> = KEYS.iter().map(|k| k.0).collect();
        return Err(format!("unknown setting {key}; one of {}", keys.join(", ")));
    }
    if is_bool(key) {
        return parse_bool(value).map(Value::Bool);
    }
    if let Some(allowed) = sections_of(key) {
        let mut list: Vec<String> = vec![];
        for s in value.split([',', ' ']).filter(|s| !s.is_empty()) {
            if !allowed.contains(&s) {
                return Err(format!("no section {s}; one of {}", allowed.join(", ")));
            }
            if !list.iter().any(|l| l == s) {
                list.push(s.to_string());
            }
        }
        return Ok(Value::Sections(list));
    }
    match key {
        "ui.sidebar" => SidebarStyle::ALL
            .into_iter()
            .find(|s| s.name() == value)
            .map(Value::Sidebar)
            .ok_or_else(|| format!("expected tree, flat or hidden, got {value}")),
        "ui.theme" => THEMES
            .contains(&value)
            .then(|| Value::Theme(value.to_string()))
            .ok_or_else(|| format!("expected one of {}, got {value}", THEMES.join(", "))),
        _ => {
            let v = (value != "unset").then(|| value.to_string());
            if let Some(e) = &v
                && !envs.contains(e)
            {
                return Err(format!("no env {e}; one of {}", envs.join(", ")));
            }
            Ok(Value::Env(v))
        }
    }
}

/// Set `key` in `cfg`. `v` must come from `parse` for the same key.
pub fn apply(cfg: &mut Config, key: &str, v: Value) {
    match (key, v) {
        ("ui.sidebar", Value::Sidebar(s)) => cfg.ui.sidebar = s,
        ("ui.wrap", Value::Bool(b)) => cfg.ui.wrap = b,
        ("ui.auto_switch_response", Value::Bool(b)) => cfg.ui.auto_switch_response = b,
        ("ui.mouse", Value::Bool(b)) => cfg.ui.mouse = b,
        ("ui.theme", Value::Theme(t)) => cfg.ui.theme = t,
        ("run.asserts", Value::Bool(b)) => cfg.run.asserts = b,
        ("run.default_env", Value::Env(e)) => cfg.run.default_env = e,
        ("request.sections", Value::Sections(l)) => cfg.request.sections = Some(l),
        ("response.sections", Value::Sections(l)) => cfg.response.sections = Some(l),
        (key, v) => debug_assert!(false, "{key} can't take {v:?}"),
    }
}

/// Write `key = v` to `config.toml`, keeping its comments. Returns the value as saved.
pub fn save(key: &str, v: &Value) -> Result<String> {
    let (table, k) = key.split_once('.').unwrap_or_default();
    let value = v.to_toml();
    let shown = value
        .as_ref()
        .map(|v| v.to_string().trim().to_string())
        .unwrap_or_else(|| "unset".into());
    config::save_key(table, k, value)?;
    Ok(shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_validates_and_normalises() {
        let envs = vec!["local".to_string(), "prod".to_string()];
        assert_eq!(parse("ui.wrap", "yes", &envs), Ok(Value::Bool(true)));
        assert_eq!(
            parse("request.sections", "body, curl,body", &envs),
            Ok(Value::Sections(vec!["body".into(), "curl".into()]))
        );
        assert!(parse("response.sections", "curl", &envs).is_err());
        assert_eq!(
            parse("run.default_env", "unset", &envs),
            Ok(Value::Env(None))
        );
        assert!(parse("run.default_env", "staging", &envs).is_err());
        assert!(parse("ui.nope", "on", &envs).is_err());
    }
}
