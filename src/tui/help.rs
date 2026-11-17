//! The `?` modal: a Commands tab generated from the registry, and a Config tab that edits
//! `config.toml` live (each change applies at once and is written back with comments kept).

use super::app::{App, Mode};
use super::commands::COMMANDS;
use crate::config::{self, Config, Sections, SidebarStyle, REQUEST_SECTIONS, RESPONSE_SECTIONS};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpTab {
    #[default]
    Commands,
    Config,
}

#[derive(Default)]
pub struct Help {
    pub tab: HelpTab,
    pub cmd: usize,
    pub cfg: usize,
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

    fn sections_mut(self, cfg: &mut Config) -> &mut Sections {
        match self {
            Group::Request => &mut cfg.request,
            Group::Response => &mut cfg.response,
        }
    }

    /// Shown sections in display order.
    fn enabled(self, cfg: &Config) -> Vec<&'static str> {
        let allowed = self.allowed();
        self.sections(cfg).or(allowed).iter().filter_map(|s| allowed.iter().find(|a| *a == s).copied()).collect()
    }

    /// Shown sections in order, then hidden ones.
    fn order(self, cfg: &Config) -> Vec<&'static str> {
        let mut out = self.enabled(cfg);
        let hidden: Vec<_> = self.allowed().iter().filter(|a| !out.contains(a)).copied().collect();
        out.extend(hidden);
        out
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Setting {
    Sidebar,
    Wrap,
    AutoSwitch,
    Asserts,
    DefaultEnv,
    Section(Group, &'static str),
    OpenFile,
}

pub fn settings(cfg: &Config) -> Vec<Setting> {
    let mut out = vec![Setting::Sidebar, Setting::Wrap, Setting::AutoSwitch, Setting::Asserts, Setting::DefaultEnv];
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
            format!("‹ {} ›", if cfg.ui.sidebar == SidebarStyle::Tree { "tree" } else { "flat" }),
            "Tree folds requests into folders; flat is one ranked list of full paths. ui.sidebar".into(),
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
    app.mode = Mode::Help;
}

/// Keys while the modal is open.
pub fn on_key(app: &mut App, k: &KeyEvent) {
    match k.code {
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
        HelpTab::Commands => COMMANDS.len(),
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
                if let Some(c) = COMMANDS.get(app.help.cmd) {
                    app.mode = Mode::Normal;
                    (c.run)(app);
                }
            }
            _ => {}
        },
        HelpTab::Config => {
            let Some(s) = settings(&app.cfg).get(app.help.cfg).cloned() else { return };
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

fn change(app: &mut App, s: &Setting, c: Change) {
    match s {
        Setting::Sidebar => {
            // Two values, so ←, → and enter all flip it.
            let tree = app.cfg.ui.sidebar != SidebarStyle::Tree;
            app.cfg.ui.sidebar = if tree { SidebarStyle::Tree } else { SidebarStyle::Flat };
            if app.tree != tree {
                app.tree = tree;
                app.rebuild_keep(true);
            }
            save(app, "ui", "sidebar", Some(if tree { "tree" } else { "flat" }.into()));
        }
        Setting::Wrap => {
            let v = set(app.cfg.ui.wrap, c);
            app.cfg.ui.wrap = v;
            app.wrap = v;
            app.scroll.1 = 0;
            save(app, "ui", "wrap", Some(v.into()));
        }
        Setting::AutoSwitch => {
            let v = set(app.cfg.ui.auto_switch_response, c);
            app.cfg.ui.auto_switch_response = v;
            save(app, "ui", "auto_switch_response", Some(v.into()));
        }
        Setting::Asserts => {
            let v = set(app.cfg.run.asserts, c);
            app.cfg.run.asserts = v;
            app.asserts = v;
            save(app, "run", "asserts", Some(v.into()));
        }
        Setting::DefaultEnv => {
            // Cycle through "unset" and every env in the workspace.
            let mut opts: Vec<Option<String>> = vec![None];
            opts.extend(app.envs.iter().cloned().map(Some));
            let cur = opts.iter().position(|o| *o == app.cfg.run.default_env).unwrap_or(0) as isize;
            let delta = if matches!(c, Change::Off) { -1 } else { 1 };
            let next = opts[(cur + delta).rem_euclid(opts.len() as isize) as usize].clone();
            save(app, "run", "default_env", next.as_deref().map(Into::into));
            app.cfg.run.default_env = next;
        }
        Setting::Section(g, name) => {
            let on = g.enabled(&app.cfg).contains(name);
            let want = set(on, c);
            if want == on {
                return;
            }
            // Keep the displayed order, flipping just this one.
            let list: Vec<&str> =
                g.order(&app.cfg).into_iter().filter(|s| if s == name { want } else { g.enabled(&app.cfg).contains(s) }).collect();
            write_sections(app, *g, list);
            // Rows are enabled-then-hidden, so the selection follows the moved row.
            let rows = settings(&app.cfg);
            app.help.cfg = rows.iter().position(|r| r == s).unwrap_or(app.help.cfg);
        }
        Setting::OpenFile => {
            if matches!(c, Change::Flip) {
                app.mode = Mode::Normal;
                app.edit = Config::path();
            }
        }
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
    g.sections_mut(&mut app.cfg).sections = Some(list.iter().map(|s| s.to_string()).collect());
    app.view = None;
    save(app, g.table(), "sections", Some(toml_edit::Array::from_iter(list).into()));
}

fn save(app: &mut App, table: &str, key: &str, value: Option<toml_edit::Value>) {
    let shown = value.as_ref().map(|v| v.to_string().trim().to_string()).unwrap_or_else(|| "unset".into());
    match config::save_key(table, key, value) {
        Ok(_) => app.say(format!("saved {table}.{key} = {shown}"), false),
        Err(e) => app.say(format!("config not saved: {e:#}"), true),
    }
}
