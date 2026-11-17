//! Settings by `table.key`, shared by `:set` / `:set!` and the help modal's Config tab.
//! A change applies to the running app; `persist` also writes it to `config.toml`.

use super::app::App;
use crate::config::{self, SidebarStyle, REQUEST_SECTIONS, RESPONSE_SECTIONS};

/// (key, accepted values) in the order `:set` completes them.
pub const KEYS: &[(&str, &str)] = &[
    ("ui.sidebar", "tree | flat | hidden"),
    ("ui.wrap", "on | off"),
    ("ui.auto_switch_response", "on | off"),
    ("ui.mouse", "on | off"),
    ("run.asserts", "on | off"),
    ("run.default_env", "an env name, or unset"),
    ("request.sections", "comma list of headers, body, curl"),
    ("response.sections", "comma list of headers, body"),
];

fn is_bool(key: &str) -> bool {
    KEYS.iter().any(|(k, v)| *k == key && *v == "on | off")
}

fn sections_of(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "request.sections" => Some(REQUEST_SECTIONS),
        "response.sections" => Some(RESPONSE_SECTIONS),
        _ => None,
    }
}

fn on_off(v: bool) -> String {
    if v { "on" } else { "off" }.into()
}

/// The live value: session toggles (`w`, `t`, `\`) included.
pub fn get(app: &App, key: &str) -> Option<String> {
    let c = &app.cfg;
    Some(match key {
        "ui.sidebar" => match (app.show_sidebar, app.tree) {
            (false, _) => "hidden".into(),
            (true, true) => "tree".into(),
            (true, false) => "flat".into(),
        },
        "ui.wrap" => on_off(app.wrap),
        "ui.auto_switch_response" => on_off(c.ui.auto_switch_response),
        "ui.mouse" => on_off(c.ui.mouse),
        "run.asserts" => on_off(app.asserts),
        "run.default_env" => c.run.default_env.clone().unwrap_or_else(|| "unset".into()),
        "request.sections" => c.request.or(REQUEST_SECTIONS).join(","),
        "response.sections" => c.response.or(RESPONSE_SECTIONS).join(","),
        _ => return None,
    })
}

/// Values to offer for `key`; for section lists, `typed` is the list so far.
pub fn values(app: &App, key: &str, typed: &str) -> Vec<String> {
    if is_bool(key) {
        return vec!["on".into(), "off".into()];
    }
    if let Some(allowed) = sections_of(key) {
        // Complete the next name in the list, keeping what is already typed.
        let (done, _) = typed.rsplit_once(',').unwrap_or(("", typed));
        let have: Vec<&str> = done.split(',').filter(|s| !s.is_empty()).collect();
        let prefix = if done.is_empty() { String::new() } else { format!("{done},") };
        return allowed.iter().filter(|a| !have.contains(a)).map(|a| format!("{prefix}{a}")).collect();
    }
    match key {
        "ui.sidebar" => SidebarStyle::ALL.iter().map(|s| s.name().to_string()).collect(),
        "run.default_env" => std::iter::once("unset".to_string()).chain(app.envs.iter().cloned()).collect(),
        _ => vec![],
    }
}

fn parse_bool(s: &str) -> Result<bool, String> {
    match s.to_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => Err(format!("expected on or off, got {s}")),
    }
}

/// Apply `key = value`. With no value a toggle flips; anything else is shown as is (and with
/// `persist`, saved as is, which keeps a session change such as `\`).
pub fn set(app: &mut App, key: &str, value: Option<&str>, persist: bool) -> Result<(), String> {
    let Some(cur) = get(app, key) else {
        let keys: Vec<_> = KEYS.iter().map(|k| k.0).collect();
        return Err(format!("unknown setting {key}; one of {}", keys.join(", ")));
    };
    let value = match value {
        Some(v) => v.to_string(),
        None if is_bool(key) => on_off(cur != "on"),
        None if !persist => {
            app.say(format!("{key} = {cur}"), false);
            return Ok(());
        }
        None => cur,
    };
    let saved: Option<toml_edit::Value> = match key {
        "ui.sidebar" => {
            let s = SidebarStyle::ALL
                .into_iter()
                .find(|s| s.name() == value)
                .ok_or_else(|| format!("expected tree, flat or hidden, got {value}"))?;
            app.cfg.ui.sidebar = s;
            app.apply_sidebar_style();
            Some(s.name().into())
        }
        "ui.wrap" => {
            let v = parse_bool(&value)?;
            app.cfg.ui.wrap = v;
            app.wrap = v;
            app.scroll.1 = 0;
            Some(v.into())
        }
        "ui.auto_switch_response" => {
            let v = parse_bool(&value)?;
            app.cfg.ui.auto_switch_response = v;
            Some(v.into())
        }
        "ui.mouse" => {
            let v = parse_bool(&value)?;
            app.cfg.ui.mouse = v;
            Some(v.into())
        }
        "run.asserts" => {
            let v = parse_bool(&value)?;
            app.cfg.run.asserts = v;
            app.asserts = v;
            Some(v.into())
        }
        "run.default_env" => {
            if !persist {
                return Err("run.default_env only applies at startup: :set! saves it, :env switches now".into());
            }
            let v = (value != "unset").then_some(value);
            if let Some(e) = &v
                && !app.envs.contains(e)
            {
                return Err(format!("no env {e}; one of {}", app.envs.join(", ")));
            }
            app.cfg.run.default_env = v.clone();
            v.map(Into::into)
        }
        _ => {
            let allowed = sections_of(key).unwrap_or_default();
            let mut list: Vec<&str> = vec![];
            for s in value.split([',', ' ']).filter(|s| !s.is_empty()) {
                let s = allowed.iter().find(|a| **a == s).ok_or_else(|| format!("no section {s}; one of {}", allowed.join(", ")))?;
                if !list.contains(s) {
                    list.push(s);
                }
            }
            let sections = if key.starts_with("request") { &mut app.cfg.request } else { &mut app.cfg.response };
            sections.sections = Some(list.iter().map(|s| s.to_string()).collect());
            app.view = None;
            Some(toml_edit::Array::from_iter(list).into())
        }
    };
    if persist {
        let (table, k) = key.split_once('.').unwrap_or_default();
        save(app, table, k, saved);
    } else {
        app.say(format!("{key} = {} (this session)", get(app, key).unwrap_or_default()), false);
    }
    Ok(())
}

fn save(app: &mut App, table: &str, key: &str, value: Option<toml_edit::Value>) {
    let shown = value.as_ref().map(|v| v.to_string().trim().to_string()).unwrap_or_else(|| "unset".into());
    match config::save_key(table, key, value) {
        Ok(_) => app.say(format!("saved {table}.{key} = {shown}"), false),
        Err(e) => app.say(format!("config not saved: {e:#}"), true),
    }
}
