//! TUI state and the actions commands call.

use crate::config::Config;
use crate::run::{self, Entry, Run, RunOpts};
use crate::store;
use crate::workspace::{search, RequestFile, Workspace};
use ratatui::text::Line;
use ratatui::widgets::ListState;
use std::collections::HashMap;
use std::path::PathBuf;
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

pub struct App {
    pub ws: Workspace,
    pub cfg: Config,
    pub requests: Vec<RequestFile>,
    pub filter: String,
    pub visible: Vec<usize>,
    pub list: ListState,
    pub envs: Vec<String>,
    pub env: usize,
    pub asserts: bool,
    pub mode: Mode,
    pub focus: Focus,
    pub tab: Tab,
    pub wrap: bool,
    pub entry: usize,
    /// (screen rows down, columns right) in the main pane.
    pub scroll: (usize, u16),
    /// Latest run per request/env; `None` means history was checked and is empty.
    pub runs: HashMap<Key, Option<Run>>,
    pub latencies: HashMap<Key, Vec<u64>>,
    pub running: HashMap<Key, Instant>,
    tx: Sender<(Key, Result<Run, String>)>,
    pub rx: Receiver<(Key, Result<Run, String>)>,
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
        let mut app = App {
            ws,
            asserts: cfg.run.asserts,
            wrap: cfg.ui.wrap,
            cfg,
            requests,
            filter: query.unwrap_or_default(),
            visible: vec![],
            list: ListState::default(),
            envs,
            env,
            mode: Mode::Normal,
            focus: Focus::Sidebar,
            tab: Tab::Request,
            entry: 0,
            scroll: (0, 0),
            runs: HashMap::new(),
            latencies: HashMap::new(),
            running: HashMap::new(),
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
        app.refilter();
        Ok(app)
    }

    // ---- queries -------------------------------------------------------------------------

    pub fn env(&self) -> &str {
        self.envs.get(self.env).map(String::as_str).unwrap_or("local")
    }

    pub fn current(&self) -> Option<&RequestFile> {
        self.list.selected().and_then(|i| self.visible.get(i)).map(|&i| &self.requests[i])
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

    pub fn refilter(&mut self) {
        let keep = self.current().map(|r| r.rel.clone());
        self.visible = search(&self.requests, &self.filter);
        let idx = keep.and_then(|k| self.visible.iter().position(|&i| self.requests[i].rel == k));
        self.list.select(if self.visible.is_empty() { None } else { Some(idx.unwrap_or(0)) });
        self.on_select();
    }

    pub fn reload(&mut self) {
        match self.ws.requests() {
            Ok(r) => self.requests = r,
            Err(e) => self.say(format!("{e:#}"), true),
        }
        self.view = None;
        self.refilter();
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
        if self.visible.is_empty() {
            return;
        }
        let n = self.visible.len() as isize;
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
        self.focus = match self.focus {
            Focus::Sidebar => Focus::Main,
            Focus::Main => Focus::Sidebar,
        };
    }

    pub fn open_filter(&mut self) {
        self.focus = Focus::Sidebar;
        self.mode = Mode::Filter;
    }

    /// Esc with no overlay open. (Phase 4: cancels the selected request's in-flight run.)
    pub fn escape(&mut self) {
        if !self.filter.is_empty() {
            self.filter.clear();
            self.refilter();
        }
    }

    pub fn toggle_asserts(&mut self) {
        self.asserts = !self.asserts;
        self.say(format!("asserts {}", if self.asserts { "on" } else { "off" }), false);
    }

    pub fn start_run(&mut self) {
        let Some(req) = self.current().cloned() else { return };
        let key = (req.rel.clone(), self.env().to_string());
        if self.running.contains_key(&key) {
            return;
        }
        let opts = RunOpts { env: key.1.clone(), asserts: self.asserts, extra_args: vec![] };
        let root = self.ws.root.clone();
        let tx = self.tx.clone();
        self.running.insert(key.clone(), Instant::now());
        std::thread::spawn(move || {
            let ws = Workspace { root };
            let res = run::run(&ws, &req, &opts).map_err(|e| format!("{e:#}"));
            let _ = tx.send((key, res));
        });
    }

    pub fn on_result(&mut self, key: Key, res: Result<Run, String>) {
        self.running.remove(&key);
        let selected = self.key().as_ref() == Some(&key);
        match res {
            Ok(r) => {
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
                self.view = None;
                if !selected || !ok {
                    self.say(format!("{} [{}]: {summary}", key.0, key.1), !ok);
                }
            }
            Err(e) => self.say(format!("{}: {e}", key.0), true),
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
