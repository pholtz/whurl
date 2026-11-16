//! Interactive view: pick a request, fire it, inspect the response.

use crate::export::{bytes, is_json, ms, pretty, reason};
use crate::print::PHASE_COLORS;
use crate::run::{self, Entry, Run, RunOpts};
use crate::store;
use crate::workspace::{search, RequestFile, Workspace};
use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph, Sparkline, Tabs};
use ratatui::{DefaultTerminal, Frame};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const HISTORY: usize = 30;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Tab {
    Body,
    Headers,
    Request,
    Timing,
    Source,
}
const TABS: [Tab; 5] = [Tab::Body, Tab::Headers, Tab::Request, Tab::Timing, Tab::Source];

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Tab::Body => "Body",
            Tab::Headers => "Headers",
            Tab::Request => "Request",
            Tab::Timing => "Timing",
            Tab::Source => "Source",
        }
    }
}

#[derive(PartialEq)]
enum Mode {
    Normal,
    Filter,
    Help,
}

/// (request name, env)
type Key = (String, String);

#[derive(PartialEq, Clone)]
struct ViewKey {
    key: Key,
    run: Option<String>,
    entry: usize,
    tab: Tab,
}

struct App {
    ws: Workspace,
    requests: Vec<RequestFile>,
    filter: String,
    visible: Vec<usize>,
    list: ListState,
    envs: Vec<String>,
    env: usize,
    asserts: bool,
    mode: Mode,
    tab: Tab,
    entry: usize,
    scroll: (usize, u16),
    /// Latest run per request/env; `None` means history was checked and is empty.
    runs: HashMap<Key, Option<Run>>,
    latencies: HashMap<Key, Vec<u64>>,
    running: HashMap<Key, Instant>,
    tx: Sender<(Key, Result<Run, String>)>,
    rx: Receiver<(Key, Result<Run, String>)>,
    flash: Option<(String, bool, Instant)>,
    tick: usize,
    view: Option<(ViewKey, Vec<Line<'static>>)>,
    content_height: usize,
    edit: Option<PathBuf>,
    quit: bool,
}

pub fn run(ws: Workspace, query: Option<String>, env: String) -> Result<()> {
    let requests = ws.requests()?;
    let envs = ws.envs();
    let env = envs.iter().position(|e| *e == env).unwrap_or(0);
    let (tx, rx) = channel();
    let mut app = App {
        ws,
        requests,
        filter: query.unwrap_or_default(),
        visible: vec![],
        list: ListState::default(),
        envs,
        env,
        asserts: false,
        mode: Mode::Normal,
        tab: Tab::Body,
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
        edit: None,
        quit: false,
    };
    app.refilter();

    let mut term = ratatui::init();
    let res = app.event_loop(&mut term);
    ratatui::restore();
    res
}

impl App {
    fn event_loop(&mut self, term: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            term.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(80))?
                && let Event::Key(k) = event::read()?
                    && k.kind == KeyEventKind::Press {
                        self.on_key(k);
                    }
            while let Ok((key, res)) = self.rx.try_recv() {
                self.on_result(key, res);
            }
            if let Some(path) = self.edit.take() {
                ratatui::restore();
                let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
                let status = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(format!("{editor} \"$1\""))
                    .arg("sh")
                    .arg(&path)
                    .status();
                *term = ratatui::init();
                if let Err(e) = status {
                    self.say(format!("editor: {e}"), true);
                }
                self.reload();
            }
            self.tick = self.tick.wrapping_add(1);
        }
        Ok(())
    }

    // ---- state helpers -------------------------------------------------------------------

    fn env(&self) -> &str {
        self.envs.get(self.env).map(String::as_str).unwrap_or("local")
    }

    fn current(&self) -> Option<&RequestFile> {
        self.list.selected().and_then(|i| self.visible.get(i)).map(|&i| &self.requests[i])
    }

    fn key(&self) -> Option<Key> {
        self.current().map(|r| (r.rel.clone(), self.env().to_string()))
    }

    fn current_run(&self) -> Option<&Run> {
        self.key().and_then(|k| self.runs.get(&k)).and_then(Option::as_ref)
    }

    fn current_entry(&self) -> Option<&Entry> {
        self.current_run().and_then(|r| r.entries.get(self.entry))
    }

    fn say(&mut self, msg: impl Into<String>, err: bool) {
        self.flash = Some((msg.into(), err, Instant::now()));
    }

    fn refilter(&mut self) {
        let keep = self.current().map(|r| r.rel.clone());
        self.visible = search(&self.requests, &self.filter);
        let idx = keep.and_then(|k| self.visible.iter().position(|&i| self.requests[i].rel == k));
        self.list.select(if self.visible.is_empty() { None } else { Some(idx.unwrap_or(0)) });
        self.on_select();
    }

    fn reload(&mut self) {
        match self.ws.requests() {
            Ok(r) => self.requests = r,
            Err(e) => self.say(format!("{e:#}"), true),
        }
        self.view = None;
        self.refilter();
    }

    /// Load the latest stored run for the selection (once) and reset the view.
    fn on_select(&mut self) {
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

    fn move_sel(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let n = self.visible.len() as isize;
        let cur = self.list.selected().unwrap_or(0) as isize;
        self.list.select(Some((cur + delta).clamp(0, n - 1) as usize));
        self.on_select();
    }

    fn start_run(&mut self) {
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

    fn on_result(&mut self, key: Key, res: Result<Run, String>) {
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

    fn export(&mut self, json: bool) {
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

    // ---- input ---------------------------------------------------------------------------

    fn on_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match self.mode {
            Mode::Help => self.mode = Mode::Normal,
            Mode::Filter => match k.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.mode = Mode::Normal;
                    self.refilter();
                }
                KeyCode::Enter => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.refilter();
                }
                KeyCode::Down => self.move_sel(1),
                KeyCode::Up => self.move_sel(-1),
                KeyCode::Char(c) if !ctrl => {
                    self.filter.push(c);
                    self.refilter();
                }
                _ => {}
            },
            Mode::Normal => self.on_normal_key(k, ctrl),
        }
    }

    fn on_normal_key(&mut self, k: KeyEvent, ctrl: bool) {
        let page = self.content_height.max(2) / 2;
        let lines = self.view.as_ref().map(|v| v.1.len()).unwrap_or(0);
        let max_scroll = lines.saturating_sub(self.content_height);
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.refilter();
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
            KeyCode::Char('g') | KeyCode::Home => self.move_sel(-(self.visible.len() as isize)),
            KeyCode::Char('G') | KeyCode::End => self.move_sel(self.visible.len() as isize),
            KeyCode::Enter | KeyCode::Char('r') => self.start_run(),
            KeyCode::Char('e') | KeyCode::Char('E') if !self.envs.is_empty() => {
                let n = self.envs.len();
                self.env = if k.code == KeyCode::Char('e') { (self.env + 1) % n } else { (self.env + n - 1) % n };
                self.on_select();
            }
            KeyCode::Char('t') => {
                self.asserts = !self.asserts;
                self.say(format!("asserts {}", if self.asserts { "on" } else { "off" }), false);
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let i = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
                let n = TABS.len();
                self.tab = TABS[if k.code == KeyCode::Tab { (i + 1) % n } else { (i + n - 1) % n }];
                self.scroll = (0, 0);
            }
            KeyCode::Char(c @ '1'..='5') => {
                self.tab = TABS[c as usize - '1' as usize];
                self.scroll = (0, 0);
            }
            KeyCode::Char(']') | KeyCode::Char('[') => {
                let n = self.current_run().map(|r| r.entries.len()).unwrap_or(0);
                if n > 0 {
                    self.entry = if k.code == KeyCode::Char(']') { (self.entry + 1) % n } else { (self.entry + n - 1) % n };
                    self.scroll = (0, 0);
                }
            }
            KeyCode::Char('d') if ctrl => self.scroll.0 = (self.scroll.0 + page).min(max_scroll),
            KeyCode::Char('u') if ctrl => self.scroll.0 = self.scroll.0.saturating_sub(page),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll.0 = (self.scroll.0 + page * 2).min(max_scroll),
            KeyCode::PageUp => self.scroll.0 = self.scroll.0.saturating_sub(page * 2),
            KeyCode::Char('J') => self.scroll.0 = (self.scroll.0 + 1).min(max_scroll),
            KeyCode::Char('K') => self.scroll.0 = self.scroll.0.saturating_sub(1),
            KeyCode::Char('l') | KeyCode::Right => self.scroll.1 = self.scroll.1.saturating_add(8),
            KeyCode::Char('h') | KeyCode::Left => self.scroll.1 = self.scroll.1.saturating_sub(8),
            KeyCode::Char('x') => self.export(false),
            KeyCode::Char('X') => self.export(true),
            KeyCode::Char('o') => self.edit = self.current().map(|r| r.path.clone()),
            KeyCode::Char('R') => {
                self.reload();
                self.say(format!("reloaded {} requests", self.requests.len()), false);
            }
            _ => {}
        }
    }

    // ---- drawing -------------------------------------------------------------------------

    fn draw(&mut self, f: &mut Frame) {
        let [top, main, foot] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
        self.draw_top(f, top);
        let list_w = self.requests.iter().map(|r| r.rel.len()).max().unwrap_or(20) as u16 + 9;
        let list_w = list_w.min(main.width * 2 / 5).max(24);
        let [left, right] = Layout::horizontal([Constraint::Length(list_w), Constraint::Min(0)]).areas(main);
        self.draw_list(f, left);
        let [summary, content] = Layout::vertical([Constraint::Length(6), Constraint::Min(0)]).areas(right);
        self.draw_summary(f, summary);
        self.draw_content(f, content);
        self.draw_footer(f, foot);
        if matches!(self.mode, Mode::Help) {
            draw_help(f);
        }
    }

    fn draw_top(&self, f: &mut Frame, area: Rect) {
        let env = self.env();
        let env_style = match env {
            "production" => Style::new().bg(Color::Red).fg(Color::White).bold(),
            "staging" => Style::new().bg(Color::Yellow).fg(Color::Black).bold(),
            "local" => Style::new().bg(Color::Green).fg(Color::Black).bold(),
            _ => Style::new().bg(Color::Blue).fg(Color::White).bold(),
        };
        let mut spans = vec![
            Span::styled(" whurl ", Style::new().bg(Color::Indexed(238)).bold()),
            Span::raw(" "),
            Span::styled(format!(" {env} "), env_style),
            Span::raw(" "),
            if self.asserts {
                Span::styled(" asserts on ", Style::new().fg(Color::Black).bg(Color::Cyan))
            } else {
                Span::styled("asserts off", Style::new().dark_gray())
            },
            Span::raw("  "),
            Span::styled(self.ws.root.display().to_string(), Style::new().dark_gray()),
        ];
        if !self.running.is_empty() {
            spans.push(Span::styled(
                format!("   {} {} running", SPINNER[self.tick % SPINNER.len()], self.running.len()),
                Style::new().yellow(),
            ));
        }
        f.render_widget(Line::from(spans), area);
    }

    fn draw_list(&mut self, f: &mut Frame, area: Rect) {
        let filtering = self.mode == Mode::Filter || !self.filter.is_empty();
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(format!(" Requests {}/{} ", self.visible.len(), self.requests.len()))
            .border_style(Style::new().fg(Color::Indexed(240)));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let (list_area, filter_area) = if filtering {
            let [a, b] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
            (a, Some(b))
        } else {
            (inner, None)
        };
        let env = self.env().to_string();
        let items: Vec<ListItem> = self
            .visible
            .iter()
            .map(|&i| {
                let r = &self.requests[i];
                let key = (r.rel.clone(), env.clone());
                let mark = if self.running.contains_key(&key) {
                    Span::styled(SPINNER[self.tick % SPINNER.len()], Style::new().yellow())
                } else {
                    match self.runs.get(&key).and_then(Option::as_ref) {
                        Some(run) => {
                            let st = run.primary().and_then(|e| e.response.as_ref()).map(|r| r.status);
                            Span::styled("●", Style::new().fg(st.map(status_color).unwrap_or(Color::Red)))
                        }
                        None => Span::raw(" "),
                    }
                };
                let m = r.primary().map(|e| e.method.as_str()).unwrap_or("?");
                ListItem::new(Line::from(vec![
                    mark,
                    Span::raw(" "),
                    Span::styled(format!("{:<4}", short_method(m)), Style::new().fg(method_color(m)).bold()),
                    Span::raw(" "),
                    Span::raw(r.rel.clone()),
                ]))
            })
            .collect();
        let list = List::new(items)
            .highlight_style(Style::new().bg(Color::Indexed(237)).add_modifier(Modifier::BOLD))
            .highlight_symbol("▌");
        f.render_stateful_widget(list, list_area, &mut self.list);
        if let Some(a) = filter_area {
            let cursor = if self.mode == Mode::Filter { "▏" } else { "" };
            f.render_widget(
                Line::from(vec![Span::styled("/", Style::new().yellow()), Span::raw(format!("{}{cursor}", self.filter))]),
                a,
            );
        }
    }

    fn draw_summary(&self, f: &mut Frame, area: Rect) {
        let Some(req) = self.current() else {
            f.render_widget(Block::bordered().border_type(BorderType::Rounded).title(" no requests "), area);
            return;
        };
        let entry = self.current_entry();
        let (method, url) = match (entry, req.primary()) {
            (Some(e), _) => (e.method.clone(), e.url.clone()),
            (None, Some(p)) => (p.method.clone(), self.ws.interpolate(&p.url, &self.ws.vars(self.env()))),
            _ => ("?".into(), String::new()),
        };
        let title = Line::from(vec![
            Span::raw(" "),
            Span::styled(req.rel.clone(), Style::new().bold()),
            Span::styled(req.title.as_ref().map(|t| format!("  {t} ")).unwrap_or(" ".into()), Style::new().dark_gray()),
        ]);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .padding(Padding::horizontal(1))
            .title(title)
            .border_style(Style::new().fg(Color::Indexed(240)));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let key = self.key().unwrap_or_default();
        let lat = self.latencies.get(&key).cloned().unwrap_or_default();
        let spark_w = if lat.len() > 1 && inner.width > 70 { 32 } else { 0 };
        let [left, spark] = Layout::horizontal([Constraint::Min(0), Constraint::Length(spark_w)]).areas(inner);

        let mut lines = vec![Line::from(vec![
            Span::styled(format!("{method} "), Style::new().fg(method_color(&method)).bold()),
            Span::raw(url),
        ])];
        let run = self.current_run();
        if let Some(t0) = self.running.get(&key) {
            lines.push(Line::from(Span::styled(
                format!("{} running… {:.1}s", SPINNER[self.tick % SPINNER.len()], t0.elapsed().as_secs_f32()),
                Style::new().yellow(),
            )));
        }
        match (run, entry) {
            (Some(run), Some(e)) => {
                let mut status = vec![];
                match &e.response {
                    Some(r) => {
                        status.push(Span::styled(
                            format!(" {} ", format!("{} {}", r.status, reason(r.status)).trim_end()),
                            Style::new().bg(status_color(r.status)).fg(Color::Black).bold(),
                        ));
                        status.push(Span::styled(
                            format!("  {}", e.total_us().map(ms).unwrap_or_default()),
                            Style::new().bold().fg(latency_color(e.total_us().unwrap_or(0))),
                        ));
                        status.push(Span::raw(format!("  {}", bytes(r.size))));
                        status.push(Span::styled(format!("  {}", r.http_version), Style::new().dark_gray()));
                    }
                    None => status.push(Span::styled(" no response ", Style::new().bg(Color::Red).fg(Color::White).bold())),
                }
                if run.entries.len() > 1 {
                    status.push(Span::styled(
                        format!("  entry {}/{} [ ]", self.entry + 1, run.entries.len()),
                        Style::new().cyan(),
                    ));
                }
                let failed = e.failed_asserts().count();
                if failed > 0 {
                    status.push(Span::styled(format!("  {failed} assert(s) failed"), Style::new().red().bold()));
                }
                status.push(Span::styled(format!("  {}", ago(&run.started_at)), Style::new().dark_gray()));
                lines.push(Line::from(status));
                if let Some(t) = &e.timings {
                    lines.push(waterfall_line(t, (left.width as usize).saturating_sub(2).min(60)));
                    lines.push(phase_legend(t));
                }
            }
            _ if !self.running.contains_key(&key) => lines.push(Line::from(Span::styled(
                format!("not run in {} yet — press enter", self.env()),
                Style::new().dark_gray(),
            ))),
            _ => {}
        }
        f.render_widget(Paragraph::new(lines), left);
        if spark_w > 0 {
            draw_spark(f, spark, &lat);
        }
    }

    fn draw_content(&mut self, f: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .padding(Padding::horizontal(1))
            .border_style(Style::new().fg(Color::Indexed(240)));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [tabs_area, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
        let titles: Vec<Line> = TABS.iter().enumerate().map(|(i, t)| Line::from(format!("{} {}", i + 1, t.title()))).collect();
        f.render_widget(
            Tabs::new(titles)
                .select(TABS.iter().position(|t| *t == self.tab).unwrap_or(0))
                .style(Style::new().dark_gray())
                .highlight_style(Style::new().fg(Color::White).bold().underlined())
                .divider(" "),
            tabs_area,
        );
        let [_, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
        self.content_height = body.height as usize;
        if self.tab == Tab::Timing {
            return self.draw_timing(f, body);
        }
        let Some(key) = self.key() else { return };
        let vk = ViewKey { key, run: self.current_run().map(|r| r.id.clone()), entry: self.entry, tab: self.tab };
        if self.view.as_ref().map(|v| &v.0) != Some(&vk) {
            let lines = self.build_lines();
            self.view = Some((vk, lines));
        }
        let lines = &self.view.as_ref().unwrap().1;
        let start = self.scroll.0.min(lines.len());
        let end = (start + body.height as usize).min(lines.len());
        f.render_widget(Paragraph::new(lines[start..end].to_vec()).scroll((0, self.scroll.1)), body);
        if lines.len() > body.height as usize {
            let pos = format!(" {}-{}/{} ", start + 1, end, lines.len());
            let w = pos.len() as u16;
            if area.width > w + 2 {
                let r = Rect { x: area.x + area.width - w - 2, y: area.y + area.height - 1, width: w, height: 1 };
                f.render_widget(Span::styled(pos, Style::new().dark_gray()), r);
            }
        }
    }

    fn build_lines(&self) -> Vec<Line<'static>> {
        let dim = Style::new().dark_gray();
        let hint = |s: &str| vec![Line::from(Span::styled(s.to_string(), dim))];
        if self.tab == Tab::Source {
            return self.current().map(|r| source_lines(r, self.current_entry().map(|e| e.line))).unwrap_or_default();
        }
        let Some(run) = self.current_run() else {
            return hint("press enter to run · 5 shows the hurl source");
        };
        let mut out = vec![];
        let Some(e) = self.current_entry() else { return hint("no entries") };
        match self.tab {
            Tab::Body => {
                if !run.stderr.is_empty() {
                    out.extend(run.stderr.lines().map(|l| Line::from(Span::styled(l.to_string(), Style::new().red()))));
                    out.push(Line::default());
                }
                match &e.response {
                    Some(r) if r.binary => out.extend(hint(&format!("<binary body, {}>", bytes(r.size)))),
                    Some(r) => match r.body.as_deref() {
                        Some(b) if !b.is_empty() => {
                            out.extend(body_lines(b));
                            if r.truncated {
                                out.extend(hint("<truncated>"));
                            }
                        }
                        _ => out.extend(hint("<empty body>")),
                    },
                    None => {}
                }
            }
            Tab::Headers => {
                if let Some(r) = &e.response {
                    out.push(section(&format!("Response headers ({})", r.headers.len())));
                    out.extend(header_lines(&r.headers));
                    out.push(Line::default());
                }
                out.push(section(&format!("Request headers ({})", e.request.headers.len())));
                out.extend(header_lines(&e.request.headers));
                if !e.captures.is_empty() {
                    out.push(Line::default());
                    out.push(section("Captures"));
                    out.extend(header_lines(&e.captures));
                }
                if !e.asserts.is_empty() {
                    out.push(Line::default());
                    let failed: Vec<_> = e.failed_asserts().collect();
                    out.push(section(&format!("Asserts {}/{} passed", e.asserts.len() - failed.len(), e.asserts.len())));
                    for a in failed {
                        out.extend(a.message.as_deref().unwrap_or("failed").lines().map(|l| Line::from(Span::styled(l.to_string(), Style::new().red()))));
                    }
                }
            }
            Tab::Request => {
                out.push(Line::from(vec![
                    Span::styled(format!("{} ", e.method), Style::new().fg(method_color(&e.method)).bold()),
                    Span::raw(e.url.clone()),
                ]));
                out.extend(header_lines(&e.request.headers));
                if let Some(b) = &e.request.body {
                    out.push(Line::default());
                    out.push(section(&format!("Body ({})", bytes(b.len() as u64))));
                    out.extend(body_lines(b));
                }
                out.push(Line::default());
                out.push(section("curl"));
                out.extend(e.curl.lines().map(|l| Line::from(Span::styled(l.to_string(), dim))));
            }
            Tab::Timing | Tab::Source => {}
        }
        out
    }

    fn draw_timing(&self, f: &mut Frame, area: Rect) {
        let Some(t) = self.current_entry().and_then(|e| e.timings.clone()) else {
            f.render_widget(Span::styled("no timings yet", Style::new().dark_gray()), area);
            return;
        };
        let phases = t.phases();
        let label_w = 10usize;
        let bar_w = (area.width as usize).saturating_sub(label_w + 12).min(70);
        let total = t.total.max(1) as f64;
        let mut lines = vec![];
        let mut offset = 0u64;
        for (i, p) in phases.iter().enumerate() {
            // Waterfall: each phase starts where the previous one ended.
            let start = ((offset as f64 / total) * bar_w as f64).round() as usize;
            let w = if p.us == 0 { 0 } else { (((p.us as f64 / total) * bar_w as f64).round() as usize).max(1) };
            let w = w.min(bar_w.saturating_sub(start));
            offset += p.us;
            lines.push(Line::from(vec![
                Span::styled(format!("{:<label_w$}", p.name), Style::new().fg(Color::Indexed(PHASE_COLORS[i]))),
                Span::styled("·".repeat(start), Style::new().fg(Color::Indexed(236))),
                Span::styled("█".repeat(w), Style::new().fg(Color::Indexed(PHASE_COLORS[i]))),
                Span::styled("·".repeat(bar_w - start - w), Style::new().fg(Color::Indexed(236))),
                Span::raw(format!(" {:>9}", ms(p.us))),
            ]));
        }
        lines.push(Line::from(vec![
            Span::styled(format!("{:<label_w$}", "total"), Style::new().bold()),
            Span::raw(" ".repeat(bar_w)),
            Span::styled(format!(" {:>9}", ms(t.total)), Style::new().bold().fg(latency_color(t.total))),
        ]));
        if let Some(run) = self.current_run() {
            lines.push(Line::from(Span::styled(
                format!("hurl process wall time {}ms (includes startup)", run.wall_ms),
                Style::new().dark_gray(),
            )));
        }
        let key = self.key().unwrap_or_default();
        let lat = self.latencies.get(&key).cloned().unwrap_or_default();
        let [top, bottom] = Layout::vertical([Constraint::Length(lines.len() as u16 + 1), Constraint::Min(0)]).areas(area);
        f.render_widget(Paragraph::new(lines), top);
        if lat.len() > 1 && bottom.height >= 4 {
            let mut sorted = lat.clone();
            sorted.sort_unstable();
            let pct = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
            let [stats, spark] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(bottom);
            f.render_widget(
                Line::from(vec![
                    Span::styled(format!("last {} runs in {}  ", lat.len(), self.env()), Style::new().bold()),
                    Span::styled(
                        format!("min {}  p50 {}  p90 {}  max {}", ms(sorted[0]), ms(pct(0.5)), ms(pct(0.9)), ms(*sorted.last().unwrap())),
                        Style::new().dark_gray(),
                    ),
                ]),
                stats,
            );
            let spark = Rect { height: spark.height.min(8), ..spark };
            draw_spark(f, spark, &lat);
        }
    }

    fn draw_footer(&self, f: &mut Frame, area: Rect) {
        if let Some((msg, err, at)) = &self.flash
            && at.elapsed() < Duration::from_secs(6) {
                let style = if *err { Style::new().red() } else { Style::new().green() };
                f.render_widget(Span::styled(format!(" {msg}"), style), area);
                return;
            }
        let keys = [
            ("enter", "run"),
            ("/", "filter"),
            ("e", "env"),
            ("t", "asserts"),
            ("tab", "view"),
            ("x", "export"),
            ("o", "edit"),
            ("?", "help"),
            ("q", "quit"),
        ];
        let mut spans = vec![Span::raw(" ")];
        for (k, v) in keys {
            spans.push(Span::styled(k, Style::new().cyan()));
            spans.push(Span::styled(format!(" {v}  "), Style::new().dark_gray()));
        }
        f.render_widget(Line::from(spans), area);
    }

}

// ---- free helpers --------------------------------------------------------------------------

fn draw_help(f: &mut Frame) {
    let rows = [
        ("enter / r", "run selected request"),
        ("j k ↑ ↓ g G", "move selection"),
        ("/", "filter (esc clears)"),
        ("e / E", "next / previous env"),
        ("t", "toggle asserts"),
        ("tab / 1-5", "switch view"),
        ("[ ]", "previous / next entry in a multi-request file"),
        ("space ctrl-d ctrl-u J K", "scroll"),
        ("h l ← →", "scroll horizontally"),
        ("x / X", "export run as Markdown / JSON to .whurl/exports"),
        ("o", "open the .hurl file in $EDITOR"),
        ("R", "reload request files"),
        ("q", "quit"),
    ];
    let area = centered(f.area(), 84, rows.len() as u16 + 2);
    f.render_widget(Clear, area);
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| Line::from(vec![Span::styled(format!(" {k:<24}"), Style::new().cyan()), Span::raw(*v)]))
        .collect();
    f.render_widget(Paragraph::new(lines).block(Block::bordered().border_type(BorderType::Rounded).title(" keys ")), area);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn draw_spark(f: &mut Frame, area: Rect, lat: &[u64]) {
    let w = area.width.saturating_sub(2) as usize;
    let data: Vec<u64> = lat.iter().rev().take(w).rev().copied().collect();
    let max = data.iter().copied().max().unwrap_or(0);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Indexed(238)))
        .title(Span::styled(format!(" latency · max {} ", ms(max)), Style::new().dark_gray()));
    f.render_widget(Sparkline::default().block(block).data(data).style(Style::new().fg(Color::Indexed(214))), area);
}

fn waterfall_line(t: &run::Timings, width: usize) -> Line<'static> {
    let total = t.total.max(1) as f64;
    let mut spans = vec![];
    let mut used = 0;
    for (i, p) in t.phases().iter().enumerate() {
        if p.us == 0 {
            continue;
        }
        let w = (((p.us as f64 / total) * width as f64).round() as usize).max(1).min(width.saturating_sub(used));
        used += w;
        spans.push(Span::styled("█".repeat(w), Style::new().fg(Color::Indexed(PHASE_COLORS[i]))));
    }
    Line::from(spans)
}

fn phase_legend(t: &run::Timings) -> Line<'static> {
    let mut spans = vec![];
    for (i, p) in t.phases().iter().enumerate().filter(|(_, p)| p.us > 0) {
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", Style::new().dark_gray()));
        }
        spans.push(Span::styled(p.name, Style::new().fg(Color::Indexed(PHASE_COLORS[i]))));
        spans.push(Span::raw(format!(" {}", ms(p.us))));
    }
    Line::from(spans)
}

fn summary_text(r: &Run) -> String {
    match r.primary() {
        Some(e) => match &e.response {
            Some(resp) => format!("{} in {}", resp.status, e.total_us().map(ms).unwrap_or_default()),
            None => "no response".into(),
        },
        None => "no entries".into(),
    }
}

fn ago(ts: &str) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(ts) else { return String::new() };
    let s = (chrono::Local::now().fixed_offset() - t).num_seconds().max(0);
    match s {
        0..=4 => "just now".into(),
        5..=59 => format!("{s}s ago"),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86400),
    }
}

fn section(s: &str) -> Line<'static> {
    Line::from(Span::styled(s.to_string(), Style::new().bold().underlined()))
}

fn header_lines(hs: &[run::Header]) -> Vec<Line<'static>> {
    let w = hs.iter().map(|h| h.name.len()).max().unwrap_or(0).min(32);
    hs.iter()
        .map(|h| {
            Line::from(vec![
                Span::styled(format!("{:<w$}", h.name), Style::new().cyan()),
                Span::styled("  ", Style::new()),
                Span::raw(h.value.clone()),
            ])
        })
        .collect()
}

fn status_color(s: u16) -> Color {
    match s {
        200..=299 => Color::Green,
        300..=399 => Color::Cyan,
        400..=499 => Color::Yellow,
        _ => Color::Red,
    }
}

fn latency_color(us: u64) -> Color {
    match us {
        0..=300_000 => Color::Green,
        300_001..=1_000_000 => Color::Yellow,
        _ => Color::Red,
    }
}

fn method_color(m: &str) -> Color {
    match m {
        "GET" => Color::Green,
        "POST" => Color::Yellow,
        "PUT" | "PATCH" => Color::Blue,
        "DELETE" => Color::Red,
        _ => Color::Magenta,
    }
}

fn short_method(m: &str) -> &str {
    match m {
        "DELETE" => "DEL",
        "PATCH" => "PTCH",
        "OPTIONS" => "OPT",
        m => m,
    }
}

fn body_lines(body: &str) -> Vec<Line<'static>> {
    if is_json(body) {
        pretty(body).lines().map(json_line).collect()
    } else {
        body.lines().map(|l| Line::from(l.replace('\t', "    "))).collect()
    }
}

/// Highlight one line of pretty-printed JSON.
fn json_line(line: &str) -> Line<'static> {
    let key = Style::new().fg(Color::Cyan);
    let string = Style::new().fg(Color::Green);
    let number = Style::new().fg(Color::Yellow);
    let lit = Style::new().fg(Color::Magenta);
    let punct = Style::new().fg(Color::Indexed(244));
    let chars: Vec<char> = line.chars().collect();
    let mut spans = vec![];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i = (i + 1).min(chars.len());
            let s: String = chars[start..i].iter().collect();
            let is_key = chars[i..].iter().find(|c| !c.is_whitespace()) == Some(&':');
            spans.push(Span::styled(s, if is_key { key } else { string }));
        } else if c == '-' || c.is_ascii_digit() {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '.' | '-' | '+')) {
                i += 1;
            }
            spans.push(Span::styled(chars[start..i].iter().collect::<String>(), number));
        } else if c.is_ascii_alphabetic() {
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            spans.push(Span::styled(chars[start..i].iter().collect::<String>(), lit));
        } else {
            while i < chars.len() && !(chars[i] == '"' || chars[i] == '-' || chars[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            spans.push(Span::styled(chars[start..i].iter().collect::<String>(), punct));
        }
    }
    Line::from(spans)
}

/// The .hurl file with light highlighting; the selected entry's request line is marked.
fn source_lines(r: &RequestFile, mark: Option<usize>) -> Vec<Line<'static>> {
    let src = std::fs::read_to_string(&r.path).unwrap_or_default();
    let entry_lines: HashSet<usize> = r.entries.iter().map(|e| e.line).collect();
    let mut in_fence = false;
    src.lines()
        .enumerate()
        .map(|(i, l)| {
            let n = i + 1;
            let gutter = Span::styled(
                format!("{n:>4} {} ", if Some(n) == mark { "▶" } else { " " }),
                Style::new().fg(Color::Indexed(if Some(n) == mark { 214 } else { 239 })),
            );
            let t = l.trim_start();
            let content = if t.starts_with("```") {
                in_fence = !in_fence;
                vec![Span::styled(l.to_string(), Style::new().dark_gray())]
            } else if in_fence {
                vars(l, Style::new())
            } else if t.starts_with('#') {
                vec![Span::styled(l.to_string(), Style::new().dark_gray().italic())]
            } else if entry_lines.contains(&n) {
                let (m, rest) = l.split_once(' ').unwrap_or((l, ""));
                let mut v = vec![Span::styled(format!("{m} "), Style::new().fg(method_color(m)).bold())];
                v.extend(vars(rest, Style::new().bold()));
                v
            } else if t.starts_with('[') && t.ends_with(']') {
                vec![Span::styled(l.to_string(), Style::new().magenta())]
            } else if t.starts_with("HTTP") {
                vec![Span::styled(l.to_string(), Style::new().blue().bold())]
            } else if let Some((k, v)) = l.split_once(':') {
                let mut out = vec![Span::styled(k.to_string(), Style::new().cyan()), Span::raw(":")];
                out.extend(vars(v, Style::new()));
                out
            } else {
                vars(l, Style::new())
            };
            let mut spans = vec![gutter];
            spans.extend(content);
            Line::from(spans)
        })
        .collect()
}

/// Split out `{{var}}` references so they stand out.
fn vars(s: &str, base: Style) -> Vec<Span<'static>> {
    let mut out = vec![];
    let mut rest = s;
    while let Some(a) = rest.find("{{") {
        let Some(b) = rest[a..].find("}}") else { break };
        out.push(Span::styled(rest[..a].to_string(), base));
        out.push(Span::styled(rest[a..a + b + 2].to_string(), Style::new().yellow()));
        rest = &rest[a + b + 2..];
    }
    out.push(Span::styled(rest.to_string(), base));
    out
}
