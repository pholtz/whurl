//! The `?` modal: a Commands tab generated from the registry, and a Config tab that edits
//! `config.toml` live (each change applies at once and is written back with comments kept).

use super::app::{App, Mode};
use super::commands::COMMANDS;
use super::commands::Command;
use crate::config::{Config, REQUEST_SECTIONS, RESPONSE_SECTIONS, Sections, SidebarStyle};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpTab {
    #[default]
    Commands,
    Config,
}

#[derive(Default)]
pub struct Help {
    pub tab: HelpTab,
    /// Selection within `matches`.
    pub cmd: usize,
    pub cfg: usize,
    /// Narrows the Commands tab; `typing` while `/` is taking keys.
    pub query: String,
    pub typing: bool,
}

/// Commands matching the query: every word must appear in the keys, name, description, or pane.
/// A command bound to exactly what was typed (`y`, `ctrl-d`) comes first, then name or key
/// prefixes, then the rest in registry order.
pub fn matches(query: &str) -> Vec<&'static Command> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return COMMANDS.iter().collect();
    }
    let rank = |c: &Command| {
        let keys = c.key_labels().to_lowercase();
        let hay = format!(
            "{keys} {} {} {}",
            c.id,
            c.desc.to_lowercase(),
            c.scope.title().to_lowercase()
        );
        if !q.split_whitespace().all(|w| hay.contains(w)) {
            return None;
        }
        Some(
            if keys.split(' ').any(|k| k == q || k == format!(":{q}")) || c.id == q {
                0
            } else if c.id.starts_with(&q) || keys.split(' ').any(|k| k.starts_with(&q)) {
                1
            } else {
                2
            },
        )
    };
    let mut out: Vec<(u8, &Command)> = COMMANDS
        .iter()
        .filter_map(|c| rank(c).map(|r| (r, c)))
        .collect();
    out.sort_by_key(|(r, _)| *r);
    out.into_iter().map(|(_, c)| c).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Request,
    Response,
}

impl Group {
    fn table(self) -> &'static str {
        match self {
            Group::Request => "request",
            Group::Response => "response",
        }
    }

    fn allowed(self) -> &'static [&'static str] {
        match self {
            Group::Request => REQUEST_SECTIONS,
            Group::Response => RESPONSE_SECTIONS,
        }
    }

    fn sections(self, cfg: &Config) -> &Sections {
        match self {
            Group::Request => &cfg.request,
            Group::Response => &cfg.response,
        }
    }

    /// Shown sections in display order.
    fn enabled(self, cfg: &Config) -> Vec<&'static str> {
        let allowed = self.allowed();
        self.sections(cfg)
            .or(allowed)
            .iter()
            .filter_map(|s| allowed.iter().find(|a| *a == s).copied())
            .collect()
    }

    /// Shown sections in order, then hidden ones.
    fn order(self, cfg: &Config) -> Vec<&'static str> {
        let mut out = self.enabled(cfg);
        let hidden: Vec<_> = self
            .allowed()
            .iter()
            .filter(|a| !out.contains(a))
            .copied()
            .collect();
        out.extend(hidden);
        out
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Setting {
    Sidebar,
    Wrap,
    AutoSwitch,
    Mouse,
    Asserts,
    DefaultEnv,
    Section(Group, &'static str),
    OpenFile,
}

pub fn settings(cfg: &Config) -> Vec<Setting> {
    let mut out = vec![
        Setting::Sidebar,
        Setting::Wrap,
        Setting::AutoSwitch,
        Setting::Mouse,
        Setting::Asserts,
        Setting::DefaultEnv,
    ];
    for g in [Group::Request, Group::Response] {
        out.extend(g.order(cfg).into_iter().map(|s| Setting::Section(g, s)));
    }
    out.push(Setting::OpenFile);
    out
}

/// (label, value, description) for the Config table.
pub fn describe(app: &App, s: &Setting) -> (String, String, String) {
    let toggle = |on: bool| if on { "[✓] on" } else { "[ ] off" }.to_string();
    let cfg = &app.cfg;
    match s {
        Setting::Sidebar => (
            "Sidebar".into(),
            format!("‹ {} ›", cfg.ui.sidebar.name()),
            "Tree folds requests into folders; flat is one ranked list of full paths; hidden gives the main pane the full width and / or ctrl-p open the picker. `\\` hides/shows it for this session only. ui.sidebar".into(),
        ),
        Setting::Wrap => (
            "Wrap lines".into(),
            toggle(cfg.ui.wrap),
            "Wrap long lines in the main pane. Off scrolls horizontally with h/l. `w` flips it for this session only. ui.wrap"
                .into(),
        ),
        Setting::AutoSwitch => (
            "Show response after run".into(),
            toggle(cfg.ui.auto_switch_response),
            "Switch to the Response tab when a run you started finishes and its request is still selected. ui.auto_switch_response"
                .into(),
        ),
        Setting::Mouse => (
            "Mouse".into(),
            toggle(cfg.ui.mouse),
            "Wheel scrolls the pane under the pointer; click focuses a pane, selects a row, opens a folder, or switches tab. While on, the terminal's own text selection needs a modifier: ⌥-drag in iTerm2/Ghostty, shift-drag in most others. ui.mouse".into(),
        ),
        Setting::Asserts => (
            "Asserts by default".into(),
            toggle(cfg.run.asserts),
            "Start with asserts on (hurl --test semantics). `t` flips it for this session only. run.asserts".into(),
        ),
        Setting::DefaultEnv => (
            "Default env".into(),
            format!("‹ {} ›", cfg.run.default_env.as_deref().unwrap_or("local (unset)")),
            "Env used at startup unless -e or $HURL_ENV is given. run.default_env".into(),
        ),
        Setting::Section(g, name) => {
            let on = g.enabled(cfg).contains(name);
            let title = if *g == Group::Request { "Request" } else { "Response" };
            (
                format!("{title} · {name}"),
                toggle(on),
                format!("Show {name} in the {title} tab. J/K move it up/down; the order is the display order. {}.sections", g.table()),
            )
        }
        Setting::OpenFile => (
            "Edit config file".into(),
            "enter".into(),
            format!("Open {} in $EDITOR. It is reloaded when you return.", Config::display_path()),
        ),
    }
}

#[derive(Clone, Copy)]
enum Change {
    Off,
    On,
    Flip,
}

pub fn open(app: &mut App) {
    app.help.query.clear();
    app.help.typing = false;
    app.mode = Mode::Help;
}

/// Keys while `/` is typing a query: text goes in, arrows still move, enter runs the pick.
fn on_search_key(app: &mut App, k: &KeyEvent) -> bool {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let h = &mut app.help;
    match k.code {
        KeyCode::Char('u') if ctrl => h.query.clear(),
        KeyCode::Char(c) if !ctrl => h.query.push(c),
        KeyCode::Backspace if h.query.is_empty() => h.typing = false,
        KeyCode::Backspace => {
            h.query.pop();
        }
        _ => return false,
    }
    h.cmd = 0;
    true
}

/// Keys while the modal is open.
pub fn on_key(app: &mut App, k: &KeyEvent) {
    let searching = app.help.tab == HelpTab::Commands;
    if searching && app.help.typing && on_search_key(app, k) {
        return;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match k.code {
        // Esc first drops the search, then closes.
        KeyCode::Esc if searching && (app.help.typing || !app.help.query.is_empty()) => {
            app.help.query.clear();
            app.help.typing = false;
            return;
        }
        KeyCode::Char('/') if searching => {
            app.help.typing = true;
            return;
        }
        KeyCode::Char('n' | 'j') if ctrl && searching => {
            return app.help.cmd =
                (app.help.cmd + 1).min(matches(&app.help.query).len().saturating_sub(1));
        }
        KeyCode::Char('p' | 'k') if ctrl && searching => {
            return app.help.cmd = app.help.cmd.saturating_sub(1);
        }
        KeyCode::Esc | KeyCode::Char('?' | 'q') => return app.mode = Mode::Normal,
        KeyCode::Tab | KeyCode::BackTab => {
            app.help.tab = match app.help.tab {
                HelpTab::Commands => HelpTab::Config,
                HelpTab::Config => HelpTab::Commands,
            };
            return;
        }
        _ => {}
    }
    let len = match app.help.tab {
        HelpTab::Commands => matches(&app.help.query).len(),
        HelpTab::Config => settings(&app.cfg).len(),
    };
    let sel = match app.help.tab {
        HelpTab::Commands => &mut app.help.cmd,
        HelpTab::Config => &mut app.help.cfg,
    };
    let last = len.saturating_sub(1);
    match k.code {
        KeyCode::Down | KeyCode::Char('j') => return *sel = (*sel + 1).min(last),
        KeyCode::Up | KeyCode::Char('k') => return *sel = sel.saturating_sub(1),
        KeyCode::PageDown => return *sel = (*sel + 8).min(last),
        KeyCode::PageUp => return *sel = sel.saturating_sub(8),
        KeyCode::Home | KeyCode::Char('g') => return *sel = 0,
        KeyCode::End | KeyCode::Char('G') => return *sel = last,
        _ => {}
    }
    match app.help.tab {
        HelpTab::Commands => match k.code {
            KeyCode::Left | KeyCode::Right => app.help.tab = HelpTab::Config,
            KeyCode::Enter => {
                if let Some(c) = matches(&app.help.query).get(app.help.cmd) {
                    app.mode = Mode::Normal;
                    (c.run)(app);
                }
            }
            _ => {}
        },
        HelpTab::Config => {
            let Some(s) = settings(&app.cfg).get(app.help.cfg).cloned() else {
                return;
            };
            match k.code {
                KeyCode::Left => change(app, &s, Change::Off),
                KeyCode::Right => change(app, &s, Change::On),
                KeyCode::Enter | KeyCode::Char(' ') => change(app, &s, Change::Flip),
                KeyCode::Char('J') => reorder(app, &s, 1),
                KeyCode::Char('K') => reorder(app, &s, -1),
                _ => {}
            }
        }
    }
}

fn set(old: bool, c: Change) -> bool {
    match c {
        Change::Off => false,
        Change::On => true,
        Change::Flip => !old,
    }
}

/// Cycle `cur` through `opts`: left goes back, right/enter forward.
fn cycle<T: PartialEq + Clone>(opts: &[T], cur: &T, c: Change) -> T {
    let i = opts.iter().position(|o| o == cur).unwrap_or(0) as isize;
    let delta = if matches!(c, Change::Off) { -1 } else { 1 };
    opts[(i + delta).rem_euclid(opts.len() as isize) as usize].clone()
}

fn change(app: &mut App, s: &Setting, c: Change) {
    let on_off = |v: bool| if v { "on" } else { "off" }.to_string();
    let cfg = &app.cfg;
    let (key, value) = match s {
        Setting::Sidebar => (
            "ui.sidebar",
            cycle(&SidebarStyle::ALL, &cfg.ui.sidebar, c)
                .name()
                .to_string(),
        ),
        Setting::Wrap => ("ui.wrap", on_off(set(cfg.ui.wrap, c))),
        Setting::AutoSwitch => (
            "ui.auto_switch_response",
            on_off(set(cfg.ui.auto_switch_response, c)),
        ),
        Setting::Mouse => ("ui.mouse", on_off(set(cfg.ui.mouse, c))),
        Setting::Asserts => ("run.asserts", on_off(set(cfg.run.asserts, c))),
        Setting::DefaultEnv => {
            let mut opts: Vec<Option<String>> = vec![None];
            opts.extend(app.envs.iter().cloned().map(Some));
            (
                "run.default_env",
                cycle(&opts, &cfg.run.default_env, c).unwrap_or_else(|| "unset".into()),
            )
        }
        Setting::Section(g, name) => {
            let on = g.enabled(cfg).contains(name);
            let want = set(on, c);
            if want == on {
                return;
            }
            // Keep the displayed order, flipping just this one.
            let list: Vec<&str> = g
                .order(cfg)
                .into_iter()
                .filter(|s| {
                    if s == name {
                        want
                    } else {
                        g.enabled(cfg).contains(s)
                    }
                })
                .collect();
            write_sections(app, *g, list);
            // Rows are enabled-then-hidden, so the selection follows the moved row.
            let rows = settings(&app.cfg);
            app.help.cfg = rows.iter().position(|r| r == s).unwrap_or(app.help.cfg);
            return;
        }
        Setting::OpenFile => {
            if matches!(c, Change::Flip) {
                app.mode = Mode::Normal;
                app.edit = Config::path();
            }
            return;
        }
    };
    if let Err(e) = super::settings::set(app, key, Some(&value), true) {
        app.say(e, true);
    }
}

fn reorder(app: &mut App, s: &Setting, delta: isize) {
    let Setting::Section(g, name) = s else { return };
    let mut list = g.enabled(&app.cfg);
    let Some(i) = list.iter().position(|x| x == name) else {
        return app.say("show the section before moving it", true);
    };
    let j = i as isize + delta;
    if j < 0 || j as usize >= list.len() {
        return;
    }
    list.swap(i, j as usize);
    write_sections(app, *g, list);
    app.help.cfg = (app.help.cfg as isize + delta) as usize;
}

fn write_sections(app: &mut App, g: Group, list: Vec<&str>) {
    let key = format!("{}.sections", g.table());
    if let Err(e) = super::settings::set(app, &key, Some(&list.join(",")), true) {
        app.say(e, true);
    }
}
