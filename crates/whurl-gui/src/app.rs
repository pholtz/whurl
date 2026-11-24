//! The window's state and everything it can do. Views read it; commands, keys and controls call
//! the actions here, which go through whurl-core so the workspace stays the CLI's and TUI's.

use crate::hurl::{self, Parts};
use rune::{TextEdit, Tx, Waker};
use rune_kit::syntax::Highlight;
use rune_kit::{FileNode, FileTree, TreeModel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use whurl_core::body::{self, Side};
use whurl_core::config::Config;
use whurl_core::jobs::{Job, Key, Msg, Outcome};
use whurl_core::run::{Run, RunOpts};
use whurl_core::settings::{self, Value};
use whurl_core::state::UiState;
use whurl_core::workspace::{self, RequestFile, Workspace};
use whurl_core::{export, requests, store, tree};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tab {
    Request,
    Response,
    Meta,
    Source,
}

pub const TABS: [(Tab, &str); 4] = [
    (Tab::Request, "Request"),
    (Tab::Response, "Response"),
    (Tab::Meta, "Meta"),
    (Tab::Source, "Source"),
];

/// What the name dialog is for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Naming {
    New,
    Rename,
    Duplicate,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Confirm {
    Delete,
    ClearHistory,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PaletteMode {
    /// Commands and envs (⌘K).
    Commands,
    /// Requests (⌘P).
    Requests,
}

pub struct Palette {
    pub mode: PaletteMode,
    pub picked: usize,
}

/// A past run listed in the Meta tab.
pub struct Past {
    pub id: String,
    pub line: String,
    pub ok: bool,
}

/// The env files dialog: which file shows, each file as last read, and whether secrets show.
pub struct EnvEdit {
    pub file: usize,
    pub saved: [String; 3],
    pub reveal: bool,
}

/// Where a `{{variable}}` gets its value.
pub enum Var<'a> {
    Env(&'a str),
    Secret,
    /// A `[Captures]` line earlier in the file sets it at run time.
    Capture,
    /// hurl's own `newUuid` / `newDate`.
    Builtin,
    Missing,
}

impl Var<'_> {
    pub fn resolved(&self) -> bool {
        !matches!(self, Var::Missing)
    }
}

/// Write an env file, creating `env/` if needed; a new secrets file is readable only by its owner.
fn write_env_file(path: &Path, text: &str, secret: bool) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if secret {
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    }
    let _ = secret;
    let mut body = text.to_owned();
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    opts.open(path)?.write_all(body.as_bytes())
}

/// The names of `env`'s secrets, from its own and the shared secrets file.
fn secret_keys(ws: &Workspace, env: &str) -> Vec<String> {
    ws.secrets_files(env)
        .iter()
        .flat_map(|p| workspace::read_env(p))
        .map(|(k, _)| k)
        .collect()
}

/// (text, highlights)
pub type Cached = (Rc<str>, Rc<[Highlight]>);

pub struct State {
    /// Set when startup failed; the window shows it and nothing else.
    pub fatal: Option<String>,
    pub ws: Workspace,
    pub cfg: Config,
    pub requests: Vec<RequestFile>,
    /// Ids are request names (`dir/name`) and folder paths, as `.whurl/state.toml` keeps them.
    pub files: TreeModel<FileNode<usize>>,
    dirs: Vec<String>,
    saved: UiState,
    pub filter: TextEdit,
    pub sidebar: bool,
    /// The sidebar's width in points; the grip beside it drags it. `.whurl/gui.toml` keeps it.
    pub sidebar_w: f32,
    saved_sidebar_w: f32,
    /// The grip is being dragged: the width is saved when it's let go.
    pub resizing: bool,
    pub envs: Vec<String>,
    pub env: String,
    /// The env's non-secret variables, to show URLs as they will be sent.
    pub vars: Vec<(String, String)>,
    /// The names (never the values) of the env's secrets.
    pub secret_keys: Vec<String>,
    /// The `{{var}}` chip under the pointer, for its tooltip.
    pub hover_var: Option<String>,
    pub env_open: bool,
    pub tab: Tab,
    /// Session settings (the config's until changed here).
    pub asserts: bool,
    pub wrap: bool,
    /// The run shown per (request, env): the latest, or one picked from history.
    pub runs: HashMap<Key, Option<Run>>,
    pub history: HashMap<Key, Vec<Past>>,
    pub latencies: HashMap<Key, Vec<u64>>,
    /// The entry shown in a chained file; `None` is the run's primary.
    pub entry: Option<usize>,
    pub running: HashMap<Key, Job>,
    next_job: u64,
    inbox: Arc<Mutex<Vec<Msg>>>,
    pub waker: Option<Waker>,
    /// The selected file as it is on disk.
    pub source: String,
    /// The file as edited here (Source tab and the form); `draft_for` is where it saves.
    pub draft: TextEdit,
    pub draft_for: Option<PathBuf>,
    /// The draft's primary request as form fields; `form` is false when it has no request line.
    pub form: bool,
    pub method: String,
    pub method_open: bool,
    pub url: TextEdit,
    pub headers: TextEdit,
    pub body: TextEdit,
    /// Run bodies shown in editors, by what they show: selectable and editable as scratch.
    pub scratch: RefCell<HashMap<String, TextEdit>>,
    pub flash: Option<(String, bool)>,
    pub palette: Option<Palette>,
    /// The palette's query; outside `palette` so its field can always reach it.
    pub query: TextEdit,
    pub naming: Option<Naming>,
    pub name: TextEdit,
    pub confirm: Option<Confirm>,
    pub settings_open: bool,
    /// The env files dialog, when open.
    pub env_edit: Option<EnvEdit>,
    /// Its editors: the env's variables, its secrets, and the shared secrets.
    pub env_texts: [TextEdit; 3],
    /// Settings changed this session and not yet saved to the config file.
    pub unsaved: Vec<&'static str>,
    pub menu_open: bool,
    /// When `requests/` was last looked at, and how it looked.
    scanned: (f64, u64),
    /// Highlighted code for the view, keyed by what it shows.
    pub cache: RefCell<HashMap<String, Cached>>,
}

impl State {
    pub fn new(ws: Workspace, cfg: Config, env: String, fatal: Option<String>) -> Self {
        let mut s = State {
            fatal,
            envs: ws.envs(),
            vars: ws.vars(&env),
            secret_keys: secret_keys(&ws, &env),
            hover_var: None,
            ws,
            asserts: cfg.run.asserts,
            wrap: cfg.ui.wrap,
            cfg,
            requests: vec![],
            files: FileTree::new().build(std::iter::empty::<(String, usize)>()),
            dirs: vec![],
            saved: UiState::default(),
            filter: TextEdit::new(""),
            sidebar: true,
            sidebar_w: SIDEBAR_W,
            saved_sidebar_w: SIDEBAR_W,
            resizing: false,
            env,
            env_open: false,
            tab: Tab::Response,
            runs: HashMap::new(),
            history: HashMap::new(),
            latencies: HashMap::new(),
            entry: None,
            running: HashMap::new(),
            next_job: 0,
            inbox: Arc::default(),
            waker: None,
            source: String::new(),
            draft: TextEdit::new(""),
            draft_for: None,
            form: false,
            method: String::new(),
            method_open: false,
            url: TextEdit::new(""),
            headers: TextEdit::new(""),
            body: TextEdit::new(""),
            scratch: RefCell::default(),
            flash: None,
            palette: None,
            query: TextEdit::new(""),
            naming: None,
            name: TextEdit::new(""),
            confirm: None,
            settings_open: false,
            env_edit: None,
            env_texts: Default::default(),
            unsaved: vec![],
            menu_open: false,
            scanned: (0.0, 0),
            cache: RefCell::default(),
        };
        if s.fatal.is_none() {
            s.saved = UiState::load(&s.ws);
            s.sidebar_w = load_sidebar_w(&s.ws).unwrap_or(SIDEBAR_W);
            s.saved_sidebar_w = s.sidebar_w;
            let open = s.saved.expanded.clone();
            let sel = s.saved.selected.clone();
            s.load(&open, sel.as_deref());
            s.scanned.1 = signature(&s.ws.requests_dir());
        }
        s
    }

    // --- Requests and the tree -------------------------------------------------------------

    /// Read `requests/` and rebuild the tree with `open` folders and `select`ed.
    fn load(&mut self, open: &[String], select: Option<&str>) {
        self.requests = match self.ws.requests() {
            Ok(r) => r,
            Err(e) => return self.say(format!("{e:#}"), true),
        };
        self.files = FileTree::new().compact_folders(false).build(
            self.requests
                .iter()
                .enumerate()
                .map(|(i, r)| (r.rel.clone(), i)),
        );
        let mut dirs: Vec<String> = self
            .requests
            .iter()
            .flat_map(|r| tree::ancestors(&r.rel).map(str::to_owned))
            .collect();
        dirs.sort();
        dirs.dedup();
        self.dirs = dirs;
        for d in open {
            self.files.set_expanded(d, true);
        }
        self.apply_filter();
        if let Some(sel) = select
            && self.request(sel).is_some()
        {
            self.select(sel);
        }
    }

    /// Reload request files, envs and the selected source, keeping what's open and selected.
    pub fn reload(&mut self) {
        let open = self.open_dirs();
        let sel = self.files.cursor().map(str::to_owned);
        self.load(&open, sel.as_deref());
        self.envs = self.ws.envs();
        self.load_vars();
        let same = self.current().map(|r| &r.path) == self.draft_for.as_ref();
        if !same || !self.dirty() {
            self.load_source();
        } else if let Some(p) = &self.draft_for {
            let disk = std::fs::read_to_string(p).unwrap_or_default();
            if disk != self.source {
                self.source = disk;
                self.say(
                    "the file changed on disk; your unsaved edits are kept",
                    true,
                );
            }
        }
        self.scanned.1 = signature(&self.ws.requests_dir());
    }

    /// Reload when files under `requests/` changed on disk (an editor saved, git switched).
    pub fn watch(&mut self, t: f64) {
        if t - self.scanned.0 < 1.0 {
            return;
        }
        self.scanned.0 = t;
        if signature(&self.ws.requests_dir()) != self.scanned.1 {
            self.reload();
        }
    }

    fn open_dirs(&self) -> Vec<String> {
        self.dirs
            .iter()
            .filter(|d| self.files.is_expanded(d))
            .cloned()
            .collect()
    }

    pub fn request(&self, rel: &str) -> Option<&RequestFile> {
        self.files
            .get(rel)
            .and_then(|n| n.data)
            .map(|i| &self.requests[i])
    }

    /// The selected request, if a request (not a folder) is selected.
    pub fn current(&self) -> Option<&RequestFile> {
        self.files.cursor().and_then(|id| self.request(id))
    }

    pub fn key(&self) -> Option<Key> {
        self.current().map(|r| (r.rel.clone(), self.env.clone()))
    }

    pub fn run(&self) -> Option<&Run> {
        self.key()
            .and_then(|k| self.runs.get(&k))
            .and_then(Option::as_ref)
    }

    /// The index of the entry shown in the current run.
    pub fn entry_index(&self) -> Option<usize> {
        let r = self.run()?;
        Some(
            self.entry
                .filter(|&i| i < r.entries.len())
                .unwrap_or_else(|| r.primary_index()),
        )
    }

    pub fn select(&mut self, rel: &str) {
        for d in tree::ancestors(rel) {
            self.files.set_expanded(d, true);
        }
        self.files.select(rel);
        self.selected_changed();
    }

    /// Load the source, latest run and history of the selection, and remember it.
    pub fn selected_changed(&mut self) {
        self.save();
        if self.current().is_none() {
            return;
        }
        self.load_source();
        self.entry = None;
        self.menu_open = false;
        self.trim();
        self.load_runs(false);
        self.persist();
    }

    /// Drop what only other requests showed: their loaded runs (read again from the store when
    /// they're back), response editors and highlights. Running requests keep theirs.
    fn trim(&mut self) {
        let key = self.key();
        let running = &self.running;
        self.runs
            .retain(|k, _| Some(k) == key.as_ref() || running.contains_key(k));
        self.scratch.get_mut().clear();
        self.cache.get_mut().clear();
    }

    /// The selection's shown run, history and latencies in this env; `fresh` reloads them.
    fn load_runs(&mut self, fresh: bool) {
        let Some(key) = self.key() else { return };
        if fresh || !self.runs.contains_key(&key) {
            let run = store::ids_for(&self.ws, &key.0, Some(&key.1))
                .first()
                .and_then(|id| store::load(&self.ws, id).ok());
            self.runs.insert(key.clone(), run);
        }
        if fresh || !self.history.contains_key(&key) {
            let past = store::ids_for(&self.ws, &key.0, Some(&key.1))
                .iter()
                .take(15)
                .filter_map(|id| store::load(&self.ws, id).ok())
                .map(|r| Past {
                    line: format!(
                        "{}  {}",
                        r.started_at
                            .chars()
                            .take(19)
                            .collect::<String>()
                            .replace('T', " "),
                        export::summary(&r)
                    ),
                    ok: r.success,
                    id: r.id,
                })
                .collect();
            self.history.insert(key.clone(), past);
            let lat = store::latencies(&self.ws, &key.0, &key.1, 50);
            self.latencies.insert(key, lat);
        }
    }

    /// Show a past run of the selection.
    pub fn show_run(&mut self, id: &str) {
        let Some(key) = self.key() else { return };
        match store::load(&self.ws, id) {
            Ok(r) => {
                self.runs.insert(key, Some(r));
                self.entry = None;
                self.tab = Tab::Response;
            }
            Err(e) => self.say(format!("{e:#}"), true),
        }
    }

    /// Write open folders and the selection to `.whurl/state.toml` when they changed.
    pub fn persist(&mut self) {
        if self.fatal.is_some() {
            return;
        }
        let now = UiState {
            expanded: self.open_dirs(),
            selected: self
                .current()
                .map(|r| r.rel.clone())
                .or(self.saved.selected.clone()),
        };
        if now.expanded != self.saved.expanded || now.selected != self.saved.selected {
            now.save(&self.ws);
            self.saved = now;
        }
        // The TUI rewrites state.toml without fields it doesn't know, so the GUI's own go apart.
        if !self.resizing && self.sidebar_w != self.saved_sidebar_w {
            let _ = std::fs::create_dir_all(self.ws.state_dir());
            let _ = std::fs::write(
                gui_state(&self.ws),
                format!("sidebar_width = {}\n", self.sidebar_w),
            );
            self.saved_sidebar_w = self.sidebar_w;
        }
    }

    pub fn apply_filter(&mut self) {
        let q = self.filter.text.trim().to_owned();
        if q.is_empty() {
            return self.files.clear_filter();
        }
        let hits: HashSet<String> = workspace::search(&self.requests, &q)
            .into_iter()
            .map(|i| self.requests[i].rel.clone())
            .collect();
        self.files.filter(|n| !n.dir && hits.contains(&n.path));
    }

    pub fn say(&mut self, msg: impl Into<String>, error: bool) {
        self.flash = Some((msg.into(), error));
    }

    // --- Env, tabs, entries ------------------------------------------------------------------

    pub fn set_env(&mut self, env: &str) {
        if env == self.env {
            return;
        }
        self.env = env.to_owned();
        self.load_vars();
        self.env_open = false;
        self.entry = None;
        self.trim();
        self.load_runs(false);
        self.say(format!("env {env}"), false);
    }

    pub fn load_vars(&mut self) {
        self.vars = self.ws.vars(&self.env);
        self.secret_keys = secret_keys(&self.ws, &self.env);
    }

    /// Where `{{name}}` gets its value from when the draft runs in this env.
    pub fn var(&self, name: &str) -> Var<'_> {
        if let Some((_, v)) = self.vars.iter().find(|(k, _)| k == name) {
            Var::Env(v)
        } else if self.secret_keys.iter().any(|k| k == name) {
            Var::Secret
        } else if hurl::captures(&self.draft.text).iter().any(|c| c == name) {
            Var::Capture
        } else if ["newUuid", "newDate"].contains(&name) {
            Var::Builtin
        } else {
            Var::Missing
        }
    }

    pub fn cycle_env(&mut self, by: isize) {
        if self.envs.is_empty() {
            return;
        }
        let n = self.envs.len() as isize;
        let at = self.envs.iter().position(|e| *e == self.env).unwrap_or(0) as isize;
        let next = self.envs[(at + by).rem_euclid(n) as usize].clone();
        self.set_env(&next);
    }

    pub fn cycle_entry(&mut self, by: isize) {
        let (Some(r), Some(at)) = (self.run(), self.entry_index()) else {
            return;
        };
        let n = r.entries.len() as isize;
        if n > 1 {
            self.entry = Some((at as isize + by).rem_euclid(n) as usize);
        }
    }

    // --- Running -----------------------------------------------------------------------------

    pub fn run_current(&mut self) {
        let (Some(req), Some(key)) = (self.current().cloned(), self.key()) else {
            return self.say("select a request to run", false);
        };
        if self.running.contains_key(&key) {
            return self.say(format!("{} is already running", key.0), false);
        }
        if !self.save() {
            return;
        }
        if let Err(e) = self.ws.vars_file(&self.env) {
            return self.say(format!("{e:#}"), true);
        }
        self.next_job += 1;
        let (inbox, waker) = (self.inbox.clone(), self.waker.clone());
        let opts = RunOpts {
            env: self.env.clone(),
            asserts: self.asserts,
            extra_args: vec![],
        };
        let job = Job::start(self.next_job, key.clone(), &self.ws, req, opts, move |m| {
            inbox.lock().unwrap().push(m);
            if let Some(w) = waker {
                w();
            }
        });
        self.running.insert(key, job);
        self.flash = None;
    }

    pub fn cancel_current(&mut self) {
        match self.key().and_then(|k| self.running.get(&k)) {
            Some(job) => job.cancel(),
            None => self.say("nothing running", false),
        }
    }

    /// Take finished runs from the job threads; true when any arrived.
    pub fn drain(&mut self) -> bool {
        let msgs = std::mem::take(&mut *self.inbox.lock().unwrap());
        let any = !msgs.is_empty();
        for m in msgs {
            if self.running.get(&m.key).is_none_or(|j| j.id != m.id) {
                continue; // superseded
            }
            if let Some(job) = self.running.remove(&m.key) {
                job.shutdown();
            }
            let here = self.key().as_ref() == Some(&m.key);
            match m.outcome {
                Outcome::Done(run) => {
                    let msg = format!("{} [{}]: {}", m.key.0, m.key.1, export::summary(&run));
                    self.say(msg, !run.success);
                    if here {
                        self.entry = None;
                        if self.cfg.ui.auto_switch_response && self.tab != Tab::Meta {
                            self.tab = Tab::Response;
                        }
                    }
                    self.runs.insert(m.key.clone(), Some(run));
                    self.history.remove(&m.key);
                    if here {
                        self.load_runs(false);
                    }
                }
                Outcome::Failed(e) => self.say(format!("{}: {e}", m.key.0), true),
                Outcome::Cancelled => self.say(format!("{} cancelled", m.key.0), false),
            }
        }
        any
    }

    // --- Exports and clipboard ---------------------------------------------------------------

    pub fn export(&mut self, json: bool) {
        let Some(r) = self.run() else {
            return self.say("no run to export", false);
        };
        match export::save(&self.ws, r, json) {
            Ok(p) => {
                let shown = p
                    .strip_prefix(&self.ws.root)
                    .unwrap_or(&p)
                    .display()
                    .to_string();
                self.say(format!("exported {shown}"), false)
            }
            Err(e) => self.say(format!("export: {e}"), true),
        }
    }

    /// The body on the Request tab, else the response body.
    pub fn copy_body(&mut self, tx: &mut Tx) {
        let side = if self.tab == Tab::Request {
            Side::Request
        } else {
            Side::Response
        };
        let (Some(r), Some(i)) = (self.run(), self.entry_index()) else {
            return self.say("no run", false);
        };
        match body::of(&r.entries[i], side) {
            Ok(b) => {
                tx.copy(b.text);
                self.say(format!("copied the {} body", b.what), false);
            }
            Err(e) => self.say(e, false),
        }
    }

    pub fn copy_curl(&mut self, tx: &mut Tx) {
        let (Some(r), Some(i)) = (self.run(), self.entry_index()) else {
            return self.say("no run", false);
        };
        tx.copy(r.entries[i].curl.clone());
        self.say("copied as curl", false);
    }

    pub fn copy_export(&mut self, tx: &mut Tx) {
        let Some(r) = self.run() else {
            return self.say("no run", false);
        };
        tx.copy(export::render(r, false));
        self.say("copied the run as Markdown", false);
    }

    // --- Files -------------------------------------------------------------------------------

    /// Open the selected `.hurl` file in the default app for it.
    pub fn edit(&mut self) {
        let Some(path) = self.current().map(|r| r.path.clone()) else {
            return;
        };
        if let Err(e) = open(&path, false) {
            self.say(format!("open: {e}"), true);
        }
    }

    pub fn reveal(&mut self) {
        let Some(path) = self.current().map(|r| r.path.clone()) else {
            return;
        };
        if let Err(e) = open(&path, true) {
            self.say(format!("reveal: {e}"), true);
        }
    }

    pub fn start_naming(&mut self, what: Naming) {
        self.save();
        let cur = self.current().map(|r| r.rel.clone());
        if what != Naming::New && cur.is_none() {
            return self.say("select a request first", false);
        }
        if what == Naming::Rename && self.key().is_some_and(|k| self.running.contains_key(&k)) {
            return self.say("wait for the run to finish", false);
        }
        // New goes beside the selection; the others start from its name.
        let start = match (what, &cur) {
            (Naming::New, Some(c)) => tree::parent(c).map(|p| format!("{p}/")).unwrap_or_default(),
            (Naming::New, None) => match self.files.cursor() {
                Some(dir) => format!("{dir}/"),
                None => String::new(),
            },
            (Naming::Duplicate, Some(c)) => format!("{c}_copy"),
            (_, c) => c.clone().unwrap_or_default(),
        };
        self.name = TextEdit::new(start);
        self.name.focused = true;
        self.naming = Some(what);
        self.menu_open = false;
    }

    pub fn finish_naming(&mut self) {
        let Some(what) = self.naming else { return };
        let rel = match requests::name(&self.name.text) {
            Ok(r) => r,
            Err(e) => return self.say(format!("{e:#}"), true),
        };
        let cur = self.current().cloned();
        let done = match (what, cur) {
            (Naming::New, like) => {
                requests::create(&self.ws, &rel, like.as_ref()).map(|_| format!("created {rel}"))
            }
            (Naming::Duplicate, Some(req)) => {
                requests::duplicate(&self.ws, &req, &rel).map(|_| format!("copied to {rel}"))
            }
            (Naming::Rename, Some(req)) => {
                requests::rename(&self.ws, &req, &rel).map(|r| match r {
                    None => "name unchanged".into(),
                    Some(r) => {
                        self.forget(&req.rel);
                        match r.history {
                            Ok(n) => format!("renamed to {rel} ({n} runs moved)"),
                            Err(e) => format!("renamed to {rel}; history not moved: {e:#}"),
                        }
                    }
                })
            }
            (_, None) => return,
        };
        match done {
            Ok(msg) => {
                self.naming = None;
                self.reload();
                self.select(&rel);
                self.say(msg, false);
                if what != Naming::Rename {
                    self.tab = Tab::Source;
                }
            }
            Err(e) => self.say(format!("{e:#}"), true),
        }
    }

    pub fn ask(&mut self, what: Confirm) {
        self.save();
        if what == Confirm::Delete && self.current().is_none() {
            return self.say("select a request first", false);
        }
        self.menu_open = false;
        self.confirm = Some(what);
    }

    pub fn confirmed(&mut self) {
        match self.confirm.take() {
            Some(Confirm::Delete) => {
                let Some(req) = self.current().cloned() else {
                    return;
                };
                if self.running.keys().any(|k| k.0 == req.rel) {
                    return self.say("wait for the run to finish", false);
                }
                match requests::delete(&self.ws, &req) {
                    Ok(history) => {
                        self.forget(&req.rel);
                        self.reload();
                        let msg = match history {
                            Ok(n) => format!("deleted {} and {n} runs", req.rel),
                            Err(e) => format!("deleted {}; history not removed: {e:#}", req.rel),
                        };
                        self.say(msg, false);
                    }
                    Err(e) => self.say(format!("{e:#}"), true),
                }
            }
            Some(Confirm::ClearHistory) => match store::clear(&self.ws) {
                Ok(n) => {
                    self.runs.clear();
                    self.history.clear();
                    self.latencies.clear();
                    self.say(format!("deleted {n} runs"), false);
                }
                Err(e) => self.say(format!("{e:#}"), true),
            },
            None => {}
        }
    }

    /// Drop what's cached for a request that moved or went away.
    fn forget(&mut self, rel: &str) {
        self.runs.retain(|k, _| k.0 != rel);
        self.history.retain(|k, _| k.0 != rel);
        self.latencies.retain(|k, _| k.0 != rel);
    }

    // --- Editing -----------------------------------------------------------------------------

    /// Read the selected file into the draft and the form.
    fn load_source(&mut self) {
        let Some(path) = self.current().map(|r| r.path.clone()) else {
            return;
        };
        self.source = std::fs::read_to_string(&path).unwrap_or_default();
        self.draft = TextEdit::new(self.source.clone());
        self.draft_for = Some(path);
        self.sync_form();
    }

    /// Unsaved edits to the draft.
    pub fn dirty(&self) -> bool {
        self.draft_for.is_some() && self.draft.text != self.source
    }

    /// The form from the draft, after the Source tab changed it.
    pub fn sync_form(&mut self) {
        let p = hurl::parts(&self.draft.text);
        self.form = p.is_some();
        let p = p.unwrap_or_default();
        self.method = p.method;
        for (e, v) in [
            (&mut self.url, p.url),
            (&mut self.headers, p.headers),
            (&mut self.body, p.body),
        ] {
            if e.text != v {
                e.replace_text(v);
            }
        }
    }

    /// The draft from the form, after a field changed.
    pub fn form_changed(&mut self) {
        let p = Parts {
            method: self.method.clone(),
            url: self.url.text.clone(),
            headers: self.headers.text.clone(),
            body: self.body.text.clone(),
        };
        if let Some(src) = hurl::with_parts(&self.draft.text, &p)
            && src != self.draft.text
        {
            self.draft.replace_text(src);
        }
    }

    pub fn set_method(&mut self, m: &str) {
        self.method = m.to_owned();
        self.method_open = false;
        self.form_changed();
    }

    /// Write unsaved edits to their file; false when that failed.
    pub fn save(&mut self) -> bool {
        if !self.dirty() {
            return true;
        }
        let Some(path) = self.draft_for.clone() else {
            return true;
        };
        if let Err(e) = std::fs::write(&path, &self.draft.text) {
            self.say(format!("not saved: {e}"), true);
            return false;
        }
        self.source = self.draft.text.clone();
        // The request lines may have changed; the tree's names haven't.
        if let Some(i) = self.requests.iter().position(|r| r.path == path)
            && let Ok(r) = RequestFile::parse(&path, &self.ws.requests_dir())
        {
            self.requests[i] = r;
        }
        self.scanned.1 = signature(&self.ws.requests_dir());
        true
    }

    /// Drop unsaved edits.
    pub fn revert(&mut self) {
        let path = self.draft_for.clone();
        self.draft = TextEdit::new(self.source.clone());
        self.draft_for = path;
        self.sync_form();
    }

    // --- Palette ----------------------------------------------------------------------------

    pub fn open_palette(&mut self, mode: PaletteMode) {
        self.query = TextEdit::new("");
        self.query.focused = true;
        self.menu_open = false;
        self.palette = Some(Palette { mode, picked: 0 });
    }

    // --- Settings ---------------------------------------------------------------------------

    /// The live value of a config key, session changes included.
    pub fn setting(&self, key: &str) -> String {
        match key {
            "ui.wrap" => settings::on_off(self.wrap),
            "run.asserts" => settings::on_off(self.asserts),
            _ => settings::get(&self.cfg, key).unwrap_or_default(),
        }
    }

    /// Set a config key for this session; `save_settings` writes the changed ones.
    pub fn set(&mut self, key: &'static str, value: &str) {
        match settings::parse(key, value, &self.envs) {
            Ok(v) => {
                match (key, &v) {
                    ("ui.wrap", Value::Bool(b)) => self.wrap = *b,
                    ("run.asserts", Value::Bool(b)) => self.asserts = *b,
                    _ => {}
                }
                settings::apply(&mut self.cfg, key, v);
                if !self.unsaved.contains(&key) {
                    self.unsaved.push(key);
                }
            }
            Err(e) => self.say(e, true),
        }
    }

    pub fn set_theme(&mut self, theme: &str) {
        self.set("ui.theme", theme);
        self.say(format!("theme {theme} (this session)"), false);
    }

    pub fn cycle_theme(&mut self) {
        let at = settings::THEMES
            .iter()
            .position(|t| *t == self.cfg.ui.theme)
            .unwrap_or(0);
        self.set_theme(settings::THEMES[(at + 1) % settings::THEMES.len()]);
    }

    pub fn toggle(&mut self, key: &'static str) {
        let on = self.setting(key) == "on";
        self.set(key, &settings::on_off(!on));
        self.say(format!("{key} {} (this session)", self.setting(key)), false);
    }

    // --- Env files -----------------------------------------------------------------------------

    /// The env's variables, its secrets, and the shared secrets, under `env/`.
    pub fn env_paths(&self) -> [PathBuf; 3] {
        let dir = self.ws.root.join("env");
        [
            dir.join(format!("{}.env", self.env)),
            dir.join(format!("{}.secrets.env", self.env)),
            dir.join("shared.secrets.env"),
        ]
    }

    pub fn open_env_edit(&mut self) {
        if self.env.is_empty() {
            self.say("no env selected: add env/<name>.env first", true);
            return;
        }
        let saved = self
            .env_paths()
            .map(|p| std::fs::read_to_string(p).unwrap_or_default());
        self.env_texts = saved.clone().map(|t| TextEdit::new(&t));
        self.env_edit = Some(EnvEdit {
            file: 0,
            saved,
            reveal: false,
        });
        self.settings_open = false;
        self.palette = None;
    }

    pub fn env_dirty(&self) -> bool {
        self.env_edit.as_ref().is_some_and(|e| {
            e.saved
                .iter()
                .zip(&self.env_texts)
                .any(|(a, b)| *a != b.text)
        })
    }

    /// Write the changed env files (secrets files are created owner-only) and reload vars.
    pub fn save_env_files(&mut self) {
        let Some(e) = &self.env_edit else { return };
        let paths = self.env_paths();
        let mut wrote = 0;
        for (i, (path, edit)) in paths.iter().zip(&self.env_texts).enumerate() {
            if edit.text == e.saved[i] {
                continue;
            }
            if let Err(err) = write_env_file(path, &edit.text, i > 0) {
                self.say(format!("couldn't save {}: {err}", path.display()), true);
                return;
            }
            wrote += 1;
        }
        self.env_edit = None;
        self.env_texts = Default::default();
        self.envs = self.ws.envs();
        self.load_vars();
        self.say(
            format!(
                "saved {wrote} env file{}",
                if wrote == 1 { "" } else { "s" }
            ),
            false,
        );
    }

    pub fn close_env_edit(&mut self) {
        self.env_edit = None;
        self.env_texts = Default::default();
    }

    pub fn save_settings(&mut self) {
        let mut saved = vec![];
        for key in std::mem::take(&mut self.unsaved) {
            let v = match settings::parse(key, &self.setting(key), &self.envs) {
                Ok(v) => v,
                Err(e) => return self.say(e, true),
            };
            if let Err(e) = settings::save(key, &v) {
                return self.say(format!("config not saved: {e:#}"), true);
            }
            saved.push(key);
        }
        self.say(
            format!("saved {} to {}", saved.join(", "), Config::display_path()),
            false,
        );
    }
}

pub const SIDEBAR_W: f32 = 280.0;

/// A cheap fingerprint of the `.hurl` files under `dir`: names, sizes and modified times.
fn gui_state(ws: &Workspace) -> PathBuf {
    ws.state_dir().join("gui.toml")
}

fn load_sidebar_w(ws: &Workspace) -> Option<f32> {
    let s = std::fs::read_to_string(gui_state(ws)).ok()?;
    s.lines()
        .find_map(|l| l.strip_prefix("sidebar_width"))
        .and_then(|v| v.trim_start().strip_prefix('=')?.trim().parse::<f32>().ok())
        .map(|w| w.clamp(180.0, 640.0))
}

fn signature(dir: &Path) -> u64 {
    fn walk(dir: &Path, h: &mut std::hash::DefaultHasher) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            if p.is_dir() {
                walk(&p, h);
            } else if p.extension().is_some_and(|x| x == "hurl") {
                p.hash(h);
                if let Ok(m) = e.metadata() {
                    m.len().hash(h);
                    m.modified().ok().hash(h);
                }
            }
        }
    }
    let mut h = std::hash::DefaultHasher::new();
    walk(dir, &mut h);
    h.finish()
}

/// Open `path` in its default app, or show it in the file manager.
fn open(path: &Path, reveal: bool) -> std::io::Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(if reveal { "-R" } else { "-t" });
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("explorer");
        if reveal {
            c.arg("/select,");
        }
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    let target = if reveal && !cfg!(any(target_os = "macos", windows)) {
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    };
    cmd.arg(target).spawn().map(|_| ())
}
