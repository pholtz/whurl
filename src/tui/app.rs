//! TUI state and the actions commands call.

use crate::config::Config;
use crate::run::{Entry, Run, RunOpts};
use crate::store;
use crate::workspace::{search, RequestFile, Workspace};
use ratatui::text::Line;
use ratatui::widgets::ListState;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use super::jobs::{Job, Msg, Outcome};
use super::help::Help;
use super::cmdline::CmdLine;
use super::state::UiState;
use super::tree::{self, Row, Sel};
use crate::config::SidebarStyle;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

pub const HISTORY: usize = 30;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Tab {
    Request,
    Response,
    Meta,
    Source,
}
pub const TABS: [Tab; 4] = [Tab::Request, Tab::Response, Tab::Meta, Tab::Source];

impl Tab {
    pub fn title(self) -> &'static str {
        match self {
            Tab::Request => "Request",
            Tab::Response => "Response",
            Tab::Meta => "Meta",
            Tab::Source => "Source",
        }
    }
}

/// Overlays and text inputs that take keys before the command registry.
#[derive(PartialEq)]
pub enum Mode {
    Normal,
    Filter,
    Help,
    Picker,
    Command,
}

/// Which pane movement keys act on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Main,
}

/// (request name, env)
pub type Key = (String, String);

/// Everything the main pane's lines depend on; a change rebuilds them.
#[derive(PartialEq, Clone)]
pub struct ViewKey {
    pub key: Key,
    pub run: Option<String>,
    pub running: bool,
    pub entry: usize,
    pub tab: Tab,
    /// Only for views laid out to the pane width (Meta's bars); 0 otherwise.
    pub width: u16,
}

/// The main pane's rendered lines, plus how many screen rows each takes at `rows_for`.
pub struct View {
    pub key: ViewKey,
    pub lines: Vec<Line<'static>>,
    pub rows: Vec<usize>,
    /// (width, wrap) that `rows` was measured for.
    pub rows_for: (u16, bool),
}

/// The fuzzy request picker: `ctrl-p` anywhere, `/` when the sidebar is hidden.
#[derive(Default)]
pub struct Picker {
    pub query: String,
    /// Indices into `requests`, best first.
    pub results: Vec<usize>,
    pub sel: usize,
}

pub struct App {
    pub ws: Workspace,
    pub cfg: Config,
    pub requests: Vec<RequestFile>,
    pub filter: String,
    /// Requests matching the filter, best first.
    pub matched: Vec<usize>,
    /// Sidebar rows; `list` selects one of these.
    pub rows: Vec<Row>,
    pub list: ListState,
    pub tree: bool,
    pub show_sidebar: bool,
    pub picker: Picker,
    pub cmdline: CmdLine,
    /// Open folders in tree mode (ignored while filtering: every folder with a match is open).
    pub expanded: HashSet<String>,
    pub envs: Vec<String>,
    pub env: usize,
    pub asserts: bool,
    pub mode: Mode,
    pub help: Help,
    pub focus: Focus,
    pub tab: Tab,
    pub wrap: bool,
    pub entry: usize,
    /// (screen rows down, columns right) in the main pane.
    pub scroll: (usize, u16),
    /// Latest run per request/env; `None` means history was checked and is empty.
    pub runs: HashMap<Key, Option<Run>>,
    pub latencies: HashMap<Key, Vec<u64>>,
    /// In-flight runs; several may run at once, one per request/env.
    pub running: HashMap<Key, Job>,
    next_job: u64,
    tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
    pub flash: Option<(String, bool, Instant)>,
    pub tick: usize,
    pub view: Option<View>,
    pub content_height: usize,
    /// Total screen rows of the main pane's content, as last drawn.
    pub content_rows: usize,
    pub edit: Option<PathBuf>,
    pub quit: bool,
}

impl App {
    pub fn new(ws: Workspace, cfg: Config, query: Option<String>, env: String) -> anyhow::Result<Self> {
        let requests = ws.requests()?;
        let envs = ws.envs();
        let env = envs.iter().position(|e| *e == env).unwrap_or(0);
        let (tx, rx) = channel();
        let st = UiState::load(&ws);
        let cfg_hidden = cfg.ui.sidebar == SidebarStyle::Hidden;
        let mut app = App {
            matched: vec![],
            rows: vec![],
            tree: cfg.ui.sidebar != SidebarStyle::Flat,
            show_sidebar: cfg.ui.sidebar != SidebarStyle::Hidden,
            picker: Picker::default(),
            cmdline: CmdLine::default(),
            expanded: st.expanded.into_iter().collect(),
            ws,
            asserts: cfg.run.asserts,
            wrap: cfg.ui.wrap,
            cfg,
            requests,
            filter: query.unwrap_or_default(),
            list: ListState::default(),
            envs,
            env,
            mode: Mode::Normal,
            help: Help::default(),
            focus: if cfg_hidden { Focus::Main } else { Focus::Sidebar },
            tab: Tab::Request,
            entry: 0,
            scroll: (0, 0),
            runs: HashMap::new(),
            latencies: HashMap::new(),
            running: HashMap::new(),
            next_job: 0,
            tx,
            rx,
            flash: None,
            tick: 0,
            view: None,
            content_height: 20,
            content_rows: 0,
            edit: None,
            quit: false,
        };
        app.matched = search(&app.requests, &app.filter);
        let keep = if app.filter.is_empty() { st.selected.map(Sel::Req) } else { app.best() };
        app.rebuild(keep, true);
        app.on_select();
        app.leave_folder();
        Ok(app)
    }

    // ---- queries -------------------------------------------------------------------------

    pub fn env(&self) -> &str {
        self.envs.get(self.env).map(String::as_str).unwrap_or("local")
    }

    pub fn row(&self) -> Option<&Row> {
        self.list.selected().and_then(|i| self.rows.get(i))
    }

    /// The selected request; `None` when a folder is selected.
    pub fn current(&self) -> Option<&RequestFile> {
        match self.row()? {
            Row::Req { idx, .. } => Some(&self.requests[*idx]),
            Row::Dir { .. } => None,
        }
    }

    fn sel(&self) -> Option<Sel> {
        self.row().map(|r| r.sel(&self.requests))
    }

    fn best(&self) -> Option<Sel> {
        self.matched.first().map(|&i| Sel::Req(self.requests[i].rel.clone()))
    }

    pub fn filtering(&self) -> bool {
        !self.filter.is_empty()
    }

    pub fn key(&self) -> Option<Key> {
        self.current().map(|r| (r.rel.clone(), self.env().to_string()))
    }

    pub fn current_run(&self) -> Option<&Run> {
        self.key().and_then(|k| self.runs.get(&k)).and_then(Option::as_ref)
    }

    pub fn current_entry(&self) -> Option<&Entry> {
        self.current_run().and_then(|r| r.entries.get(self.entry))
    }

    pub fn say(&mut self, msg: impl Into<String>, err: bool) {
        self.flash = Some((msg.into(), err, Instant::now()));
    }

    // ---- selection -----------------------------------------------------------------------

    /// Rebuild the rows and select `keep` if it is still there. `reveal` opens the folders
    /// above a request that is hidden; otherwise the selection falls back to its nearest
    /// visible folder.
    fn rebuild(&mut self, keep: Option<Sel>, reveal: bool) {
        if reveal && self.tree && !self.filtering() && let Some(Sel::Req(rel)) = &keep {
            self.expanded.extend(tree::ancestors(rel).map(String::from));
        }
        self.rows = if self.tree {
            tree::build(&self.requests, &self.matched, &self.expanded, self.filtering())
        } else {
            tree::flat(&self.matched)
        };
        let find = |s: &Sel| self.rows.iter().position(|r| r.sel(&self.requests) == *s);
        let idx = keep.as_ref().and_then(|k| {
            let path = match k {
                Sel::Req(p) | Sel::Dir(p) => p.as_str(),
            };
            find(k).or_else(|| {
                let mut dirs: Vec<&str> = tree::ancestors(path).collect();
                dirs.reverse();
                dirs.into_iter().find_map(|d| find(&Sel::Dir(d.into())))
            })
        });
        self.list.select(if self.rows.is_empty() { None } else { Some(idx.unwrap_or(0)) });
    }

    /// Rebuild rows keeping the selection, and reset the view only if the selection changed.
    pub fn rebuild_keep(&mut self, reveal: bool) {
        let before = self.sel();
        self.rebuild(before.clone(), reveal);
        if self.sel() != before {
            self.on_select();
        }
    }

    /// Re-run the search. While typing, jump to the best match; otherwise keep the selection.
    pub fn refilter(&mut self, best: bool) {
        self.matched = search(&self.requests, &self.filter);
        if best && self.filtering() {
            let keep = self.best();
            self.rebuild(keep, false);
            self.on_select();
        } else {
            self.rebuild_keep(true);
        }
    }

    pub fn reload(&mut self) {
        match self.ws.requests() {
            Ok(r) => self.requests = r,
            Err(e) => self.say(format!("{e:#}"), true),
        }
        self.view = None;
        self.refilter(false);
    }

    /// Load the latest stored run for the selection (once) and reset the view.
    pub fn on_select(&mut self) {
        self.scroll = (0, 0);
        self.view = None;
        let Some(key) = self.key() else { return };
        if !self.runs.contains_key(&key) {
            let latest = store::ids_for(&self.ws, &key.0, Some(&key.1)).first().and_then(|id| store::load(&self.ws, id).ok());
            self.runs.insert(key.clone(), latest);
            self.latencies.insert(key.clone(), store::latencies(&self.ws, &key.0, &key.1, HISTORY));
        }
        self.entry = self.current_run().map(Run::primary_index).unwrap_or(0);
    }

    pub fn move_sel(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        let cur = self.list.selected().unwrap_or(0) as isize;
        self.list.select(Some((cur + delta).clamp(0, n - 1) as usize));
        self.on_select();
    }

    pub fn cycle_env(&mut self, delta: isize) {
        let n = self.envs.len() as isize;
        if n > 0 {
            self.env = (self.env as isize + delta).rem_euclid(n) as usize;
            self.on_select();
        }
    }

    pub fn cycle_entry(&mut self, delta: isize) {
        let n = self.current_run().map(|r| r.entries.len()).unwrap_or(0) as isize;
        if n > 0 {
            self.entry = (self.entry as isize + delta).rem_euclid(n) as usize;
            self.scroll = (0, 0);
        }
    }

    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.scroll = (0, 0);
    }

    // ---- main pane scrolling -------------------------------------------------------------

    fn max_scroll(&self) -> usize {
        self.content_rows.saturating_sub(self.content_height)
    }

    pub fn page(&self) -> isize {
        self.content_height.max(2) as isize
    }

    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll.0 = (self.scroll.0 as isize + delta).clamp(0, self.max_scroll() as isize) as usize;
    }

    /// Only meaningful with wrap off.
    pub fn scroll_h(&mut self, delta: i16) {
        if !self.wrap {
            self.scroll.1 = self.scroll.1.saturating_add_signed(delta);
        }
    }

    /// The pane keeps the same top line; it is re-measured on the next draw.
    pub fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
        self.scroll.1 = 0;
        self.say(format!("wrap {}", if self.wrap { "on" } else { "off" }), false);
    }

    // ---- actions -------------------------------------------------------------------------

    pub fn toggle_focus(&mut self) {
        if !self.show_sidebar {
            return;
        }
        self.focus = match self.focus {
            Focus::Sidebar => Focus::Main,
            Focus::Main => Focus::Sidebar,
        };
    }

    pub fn open_filter(&mut self) {
        if !self.show_sidebar {
            return self.open_picker();
        }
        self.focus = Focus::Sidebar;
        self.mode = Mode::Filter;
    }

    /// Esc with no overlay open: cancel the selected request's run, else clear the filter.
    pub fn escape(&mut self) {
        if let Some(job) = self.key().and_then(|k| self.running.remove(&k)) {
            job.cancel();
            self.view = None;
            self.say(format!("cancelled {}", self.current().map(|r| r.rel.as_str()).unwrap_or("")), false);
        } else if !self.filter.is_empty() {
            self.filter.clear();
            self.refilter(false);
        }
    }

    pub fn toggle_asserts(&mut self) {
        self.asserts = !self.asserts;
        self.say(format!("asserts {}", if self.asserts { "on" } else { "off" }), false);
    }

    pub fn start_run(&mut self) {
        if let Some(Row::Dir { .. }) = self.row() {
            return self.toggle_dir();
        }
        let Some(req) = self.current().cloned() else { return };
        let key = (req.rel.clone(), self.env().to_string());
        if self.running.contains_key(&key) {
            return self.say("already running · esc cancels", false);
        }
        let opts = RunOpts { env: key.1.clone(), asserts: self.asserts, extra_args: vec![] };
        self.next_job += 1;
        let job = Job::start(self.next_job, key.clone(), &self.ws, req, opts, self.tx.clone());
        self.running.insert(key, job);
        self.view = None;
    }

    pub fn on_msg(&mut self, msg: Msg) {
        let Msg { id, key, outcome } = msg;
        // A cancelled job's message, or one superseded by a newer run of the same key.
        if self.running.get(&key).map(|j| j.id) != Some(id) {
            return;
        }
        self.running.remove(&key);
        self.view = None;
        let selected = self.key().as_ref() == Some(&key);
        match outcome {
            Outcome::Done(r) => {
                let summary = summary_text(&r);
                if let Some(us) = r.primary().and_then(Entry::total_us) {
                    let l = self.latencies.entry(key.clone()).or_default();
                    l.push(us);
                    if l.len() > HISTORY {
                        l.remove(0);
                    }
                }
                let ok = r.success;
                if selected {
                    self.entry = r.primary_index();
                    self.scroll = (0, 0);
                    if self.cfg.ui.auto_switch_response {
                        self.tab = Tab::Response;
                    }
                }
                self.runs.insert(key.clone(), Some(r));
                if !selected || !ok {
                    self.say(format!("{} [{}]: {summary}", key.0, key.1), !ok);
                }
            }
            Outcome::Failed(e) => self.say(format!("{}: {e}", key.0), true),
            Outcome::Cancelled => {}
        }
    }

    // ---- tree ----------------------------------------------------------------------------

    fn set_open(&mut self, path: &str, open: bool) {
        if self.filtering() {
            return;
        }
        if open {
            self.expanded.insert(path.to_string());
        } else {
            self.expanded.remove(path);
        }
        self.rebuild_keep(false);
        self.save_state();
    }

    pub fn toggle_dir(&mut self) {
        if let Some(Row::Dir { path, open, .. }) = self.row().cloned() {
            self.set_open(&path, !open);
        }
    }

    /// ← / h: close an open folder, else jump to the parent folder.
    pub fn collapse(&mut self) {
        let Some(row) = self.row().cloned() else { return };
        let path = match &row {
            Row::Dir { path, open: true, .. } if !self.filtering() => return self.set_open(path, false),
            Row::Dir { path, .. } => path.clone(),
            Row::Req { idx, .. } => self.requests[*idx].rel.clone(),
        };
        if let Some(p) = tree::parent(&path) {
            let sel = Sel::Dir(p.to_string());
            if let Some(i) = self.rows.iter().position(|r| r.sel(&self.requests) == sel) {
                self.list.select(Some(i));
                self.on_select();
            }
        }
    }

    /// → / l: open a closed folder, else step into it.
    pub fn expand(&mut self) {
        match self.row().cloned() {
            Some(Row::Dir { path, open: false, .. }) if !self.filtering() => self.set_open(&path, true),
            Some(Row::Dir { .. }) => self.move_sel(1),
            _ => {}
        }
    }

    pub fn expand_all(&mut self) {
        self.expanded = self.requests.iter().flat_map(|r| tree::ancestors(&r.rel)).map(String::from).collect();
        self.rebuild_keep(false);
        self.save_state();
    }

    pub fn collapse_all(&mut self) {
        self.expanded.clear();
        self.rebuild_keep(false);
        self.save_state();
    }

    pub fn toggle_sidebar(&mut self) {
        self.tree = !self.tree;
        self.rebuild_keep(true);
        self.say(format!("sidebar: {}", if self.tree { "tree" } else { "flat" }), false);
    }

    /// `\`: hide or show the sidebar for this session (config sets the startup default).
    pub fn toggle_sidebar_visible(&mut self) {
        self.show_sidebar = !self.show_sidebar;
        self.focus = if self.show_sidebar { Focus::Sidebar } else { Focus::Main };
        self.rebuild_keep(true);
        self.leave_folder();
    }

    /// Match `cfg.ui.sidebar` after it changed.
    pub fn apply_sidebar_style(&mut self) {
        let style = self.cfg.ui.sidebar;
        self.show_sidebar = style != SidebarStyle::Hidden;
        if style != SidebarStyle::Hidden {
            self.tree = style == SidebarStyle::Tree;
        }
        if !self.show_sidebar {
            self.focus = Focus::Main;
        }
        self.rebuild_keep(true);
        self.leave_folder();
    }

    /// With no sidebar a selected folder is a dead end: move to its first request.
    fn leave_folder(&mut self) {
        if self.show_sidebar {
            return;
        }
        let Some(Row::Dir { path, .. }) = self.row().cloned() else { return };
        let prefix = format!("{path}/");
        let mut inside: Vec<&String> =
            self.matched.iter().map(|&i| &self.requests[i].rel).filter(|r| r.starts_with(&prefix)).collect();
        inside.sort();
        if let Some(rel) = inside.first().map(|r| r.to_string()) {
            self.rebuild(Some(Sel::Req(rel)), true);
            self.on_select();
        }
    }

    // ---- picker --------------------------------------------------------------------------

    pub fn open_picker(&mut self) {
        let cur = self.current().map(|r| r.rel.clone());
        self.picker = Picker::default();
        self.picker.results = search(&self.requests, "");
        self.picker.sel = cur.and_then(|c| self.picker.results.iter().position(|&i| self.requests[i].rel == c)).unwrap_or(0);
        self.mode = Mode::Picker;
    }

    pub fn picker_refilter(&mut self) {
        self.picker.results = search(&self.requests, &self.picker.query);
        self.picker.sel = 0;
    }

    pub fn picker_move(&mut self, delta: isize) {
        let n = self.picker.results.len() as isize;
        if n > 0 {
            self.picker.sel = (self.picker.sel as isize + delta).clamp(0, n - 1) as usize;
        }
    }

    pub fn picker_choose(&mut self) {
        self.mode = Mode::Normal;
        let Some(&idx) = self.picker.results.get(self.picker.sel) else { return };
        // The sidebar filter could hide the pick; drop it rather than select nothing.
        if !self.matched.contains(&idx) {
            self.filter.clear();
            self.matched = search(&self.requests, "");
        }
        let rel = self.requests[idx].rel.clone();
        self.rebuild(Some(Sel::Req(rel)), true);
        self.on_select();
    }

    fn save_state(&self) {
        let mut expanded: Vec<String> = self.expanded.iter().cloned().collect();
        expanded.sort();
        UiState { expanded, selected: self.current().map(|r| r.rel.clone()) }.save(&self.ws);
    }

    /// Kill every in-flight hurl before exiting.
    pub fn shutdown(&mut self) {
        self.save_state();
        for (_, job) in self.running.drain() {
            job.shutdown();
        }
    }

    pub fn export(&mut self, json: bool) {
        let Some(r) = self.current_run().cloned() else {
            return self.say("nothing to export: run the request first", true);
        };
        let dir = self.ws.state_dir().join("exports");
        let path = dir.join(format!("{}.{}", r.id, if json { "json" } else { "md" }));
        let text = if json {
            serde_json::to_string_pretty(&r).unwrap_or_default()
        } else {
            crate::export::markdown(&r, std::fs::read_to_string(&r.file).ok().as_deref())
        };
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, text)) {
            Ok(()) => self.say(format!("exported {}", path.display()), false),
            Err(e) => self.say(format!("export failed: {e}"), true),
        }
    }

    pub fn edit_current(&mut self) {
        self.edit = self.current().map(|r| r.path.clone());
    }

    /// After the config file was edited by hand.
    pub fn reload_config(&mut self) {
        match Config::load() {
            Ok(cfg) => {
                self.wrap = cfg.ui.wrap;
                self.asserts = cfg.run.asserts;
                self.cfg = cfg;
                self.view = None;
                self.apply_sidebar_style();
                self.say("config reloaded", false);
            }
            Err(e) => self.say(format!("{e:#}"), true),
        }
    }

    pub fn reload_requests(&mut self) {
        self.reload();
        self.say(format!("reloaded {} requests", self.requests.len()), false);
    }
}

fn summary_text(r: &Run) -> String {
    match r.primary() {
        Some(e) => match &e.response {
            Some(resp) => format!("{} in {}", resp.status, e.total_us().map(crate::export::ms).unwrap_or_default()),
            None => "no response".into(),
        },
        None => "no entries".into(),
    }
}
