//! The command registry: one table drives key dispatch and the help modal (later: the `:` bar
//! and statusbar hints). Add a command here and it is bound, documented, and runnable.

use super::app::{App, Focus, Tab};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Where a binding applies. Focus-scoped bindings win over global ones.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Global,
    Sidebar,
    Main,
}

impl Scope {
    pub fn title(self) -> &'static str {
        match self {
            Scope::Global => "Global",
            Scope::Sidebar => "Sidebar",
            Scope::Main => "Main pane",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bind(KeyCode, KeyModifiers);

const fn k(c: char) -> Bind {
    Bind(KeyCode::Char(c), KeyModifiers::NONE)
}
const fn ctrl(c: char) -> Bind {
    Bind(KeyCode::Char(c), KeyModifiers::CONTROL)
}
const fn code(c: KeyCode) -> Bind {
    Bind(c, KeyModifiers::NONE)
}

impl Bind {
    /// Shift is ignored: it is already reflected in the char (`E`) or code (`BackTab`).
    fn matches(&self, ev: &KeyEvent) -> bool {
        self.0 == ev.code && self.1 == ev.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT)
    }

    pub fn label(&self) -> String {
        let base = match self.0 {
            KeyCode::Enter => "enter".into(),
            KeyCode::Esc => "esc".into(),
            KeyCode::Tab => "tab".into(),
            KeyCode::BackTab => "shift-tab".into(),
            KeyCode::Up => "↑".into(),
            KeyCode::Down => "↓".into(),
            KeyCode::Left => "←".into(),
            KeyCode::Right => "→".into(),
            KeyCode::Home => "home".into(),
            KeyCode::End => "end".into(),
            KeyCode::PageUp => "pgup".into(),
            KeyCode::PageDown => "pgdn".into(),
            KeyCode::Char(' ') => "space".into(),
            KeyCode::Char(c) => c.to_string(),
            other => format!("{other:?}").to_lowercase(),
        };
        if self.1.contains(KeyModifiers::CONTROL) { format!("ctrl-{base}") } else { base }
    }
}

pub struct Command {
    #[allow(dead_code)] // the `:` bar (phase 8) looks commands up by id
    pub id: &'static str,
    pub scope: Scope,
    pub keys: &'static [Bind],
    pub desc: &'static str,
    pub run: fn(&mut App),
}

impl Command {
    pub fn key_labels(&self) -> String {
        self.keys.iter().map(Bind::label).collect::<Vec<_>>().join(" ")
    }
}

use KeyCode as K;
use Scope::*;

macro_rules! cmd {
    ($id:literal, $scope:expr, [$($key:expr),*], $desc:literal, $run:expr) => {
        Command { id: $id, scope: $scope, keys: &[$($key),*], desc: $desc, run: $run }
    };
}

pub static COMMANDS: &[Command] = &[
    cmd!("run", Global, [code(K::Enter), k('r')], "run the selected request", |a| a.start_run()),
    cmd!("escape", Global, [code(K::Esc)], "cancel the selected run, else clear the filter", |a| a.escape()),
    cmd!("focus", Global, [code(K::Tab), code(K::BackTab)], "switch focus: sidebar ↔ main", |a| a.toggle_focus()),
    cmd!("filter", Global, [k('/')], "filter requests", |a| a.open_filter()),
    cmd!("env-next", Global, [k('e')], "next env", |a| a.cycle_env(1)),
    cmd!("env-prev", Global, [k('E')], "previous env", |a| a.cycle_env(-1)),
    cmd!("asserts", Global, [k('t')], "toggle asserts", |a| a.toggle_asserts()),
    cmd!("tab-request", Global, [k('1')], "Request tab", |a| a.set_tab(Tab::Request)),
    cmd!("tab-response", Global, [k('2')], "Response tab", |a| a.set_tab(Tab::Response)),
    cmd!("tab-meta", Global, [k('3')], "Meta tab: timing, history, asserts, captures", |a| a.set_tab(Tab::Meta)),
    cmd!("tab-source", Global, [k('4')], "Source tab", |a| a.set_tab(Tab::Source)),
    cmd!("wrap", Global, [k('w')], "toggle line wrap", |a| a.toggle_wrap()),
    cmd!("entry-next", Global, [k(']')], "next entry in a chained file", |a| a.cycle_entry(1)),
    cmd!("entry-prev", Global, [k('[')], "previous entry in a chained file", |a| a.cycle_entry(-1)),
    cmd!("export-md", Global, [k('x')], "export run as Markdown to .whurl/exports", |a| a.export(false)),
    cmd!("export-json", Global, [k('X')], "export run as JSON to .whurl/exports", |a| a.export(true)),
    cmd!("edit", Global, [k('o')], "open the .hurl file in $EDITOR", |a| a.edit_current()),
    cmd!("reload", Global, [k('R')], "reload request files", |a| a.reload_requests()),
    cmd!("help", Global, [k('?')], "help", |a| a.mode = super::app::Mode::Help),
    cmd!("quit", Global, [k('q'), ctrl('c')], "quit", |a| a.quit = true),
    cmd!("down", Sidebar, [k('j'), code(K::Down)], "next request", |a| a.move_sel(1)),
    cmd!("up", Sidebar, [k('k'), code(K::Up)], "previous request", |a| a.move_sel(-1)),
    cmd!("top", Sidebar, [k('g'), code(K::Home)], "first request", |a| a.move_sel(isize::MIN / 2)),
    cmd!("bottom", Sidebar, [k('G'), code(K::End)], "last request", |a| a.move_sel(isize::MAX / 2)),
    cmd!("scroll-down", Main, [k('j'), code(K::Down)], "scroll down", |a| a.scroll_by(1)),
    cmd!("scroll-up", Main, [k('k'), code(K::Up)], "scroll up", |a| a.scroll_by(-1)),
    cmd!("page-down", Main, [k(' '), code(K::PageDown)], "page down", |a| a.scroll_by(a.page())),
    cmd!("page-up", Main, [code(K::PageUp)], "page up", |a| a.scroll_by(-a.page())),
    cmd!("half-down", Main, [ctrl('d')], "half page down", |a| a.scroll_by(a.page() / 2)),
    cmd!("half-up", Main, [ctrl('u')], "half page up", |a| a.scroll_by(-a.page() / 2)),
    cmd!("scroll-top", Main, [k('g'), code(K::Home)], "top", |a| a.scroll_by(isize::MIN / 2)),
    cmd!("scroll-bottom", Main, [k('G'), code(K::End)], "bottom", |a| a.scroll_by(isize::MAX / 2)),
    cmd!("scroll-right", Main, [k('l'), code(K::Right)], "scroll right (wrap off)", |a| a.scroll_h(8)),
    cmd!("scroll-left", Main, [k('h'), code(K::Left)], "scroll left (wrap off)", |a| a.scroll_h(-8)),
];

fn find(scope: Scope, ev: &KeyEvent) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.scope == scope && c.keys.iter().any(|b| b.matches(ev)))
}

/// Run the command bound to `ev` in the focused pane, else globally. Returns whether one ran.
pub fn dispatch(app: &mut App, ev: &KeyEvent) -> bool {
    let scope = match app.focus {
        Focus::Sidebar => Sidebar,
        Focus::Main => Main,
    };
    match find(scope, ev).or_else(|| find(Global, ev)) {
        Some(c) => {
            (c.run)(app);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key bound twice in one scope, or in a pane and globally, silently shadows a command.
    #[test]
    fn no_conflicting_bindings() {
        let mut seen: Vec<(Scope, Bind, &str)> = vec![];
        for c in COMMANDS {
            for b in c.keys {
                for (s, b2, id) in &seen {
                    let clash = *b2 == *b && (*s == c.scope || *s == Global || c.scope == Global);
                    assert!(!clash, "{} and {id} both bind {}", c.id, b.label());
                }
                seen.push((c.scope, *b, c.id));
            }
        }
        let mut ids: Vec<_> = COMMANDS.iter().map(|c| c.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), COMMANDS.len(), "duplicate command id");
    }
}
