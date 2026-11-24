//! Settings for `:set` / `:set!` and the help modal's Config tab: whurl-core's settings, plus the
//! session toggles (`w`, `t`, `\`) that override the config while the TUI runs.

use super::app::App;
use whurl_core::settings::{self as core, Value};

pub use whurl_core::settings::KEYS;

/// The live value: session toggles included.
pub fn get(app: &App, key: &str) -> Option<String> {
    Some(match key {
        "ui.sidebar" => match (app.show_sidebar, app.tree) {
            (false, _) => "hidden".into(),
            (true, true) => "tree".into(),
            (true, false) => "flat".into(),
        },
        "ui.wrap" => core::on_off(app.wrap),
        "run.asserts" => core::on_off(app.asserts),
        _ => return core::get(&app.cfg, key),
    })
}

/// Values to offer for `key`; for section lists, `typed` is the list so far.
pub fn values(app: &App, key: &str, typed: &str) -> Vec<String> {
    core::values(key, typed, &app.envs)
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
        None if core::is_bool(key) => core::on_off(cur != "on"),
        None if !persist => {
            app.say(format!("{key} = {cur}"), false);
            return Ok(());
        }
        None => cur,
    };
    if key == "run.default_env" && !persist {
        return Err(
            "run.default_env only applies at startup: :set! saves it, :env switches now".into(),
        );
    }
    let v = core::parse(key, &value, &app.envs)?;
    core::apply(&mut app.cfg, key, v.clone());
    match (key, &v) {
        ("ui.sidebar", _) => app.apply_sidebar_style(),
        ("ui.wrap", Value::Bool(b)) => {
            app.wrap = *b;
            app.scroll.1 = 0;
        }
        ("run.asserts", Value::Bool(b)) => app.asserts = *b,
        (_, Value::Sections(_)) => app.view = None,
        _ => {}
    }
    if persist {
        match core::save(key, &v) {
            Ok(shown) => app.say(format!("saved {key} = {shown}"), false),
            Err(e) => app.say(format!("config not saved: {e:#}"), true),
        }
    } else {
        app.say(
            format!(
                "{key} = {} (this session)",
                get(app, key).unwrap_or_default()
            ),
            false,
        );
    }
    Ok(())
}
