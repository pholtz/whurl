//! The command table: one list drives keyboard shortcuts and the ⌘K palette, so a command
//! added here is both bound and findable.

use crate::app::{Confirm, Naming, PaletteMode, State, Tab};
use rune::{Chord, KeyEvent, Tx};

pub struct Command {
    pub id: &'static str,
    pub title: &'static str,
    /// Chords as `Chord::parse` reads them; the first is shown.
    pub keys: &'static [&'static str],
    pub run: fn(&mut State, &mut Tx),
}

macro_rules! cmd {
    ($id:literal, $title:literal, [$($k:literal),*], $run:expr) => {
        Command { id: $id, title: $title, keys: &[$($k),*], run: $run }
    };
}

pub static COMMANDS: &[Command] = &[
    cmd!("run", "Run request", ["mod+enter", "mod+r"], |s, _| s
        .run_current()),
    cmd!("cancel", "Cancel run", ["mod+."], |s, _| s.cancel_current()),
    cmd!("find", "Go to request…", ["mod+p"], |s, _| s
        .open_palette(PaletteMode::Requests)),
    cmd!(
        "command",
        "Command palette…",
        ["mod+k", "mod+shift+p"],
        |s, _| s.open_palette(PaletteMode::Commands)
    ),
    cmd!("filter", "Filter requests", ["mod+f"], |s, _| {
        s.sidebar = true;
        s.filter.focused = true;
    }),
    cmd!(
        "sidebar-toggle",
        "Show/hide sidebar",
        ["mod+backslash"],
        |s, _| s.sidebar = !s.sidebar
    ),
    cmd!("env-next", "Next env", ["mod+e"], |s, _| s.cycle_env(1)),
    cmd!("env-prev", "Previous env", ["mod+shift+e"], |s, _| s
        .cycle_env(-1)),
    cmd!("asserts", "Toggle asserts", ["mod+t"], |s, _| s
        .toggle("run.asserts")),
    cmd!("wrap", "Toggle line wrap", ["alt+z"], |s, _| s
        .toggle("ui.wrap")),
    cmd!("tab-request", "Request tab", ["mod+1"], |s, _| s.tab =
        Tab::Request),
    cmd!("tab-response", "Response tab", ["mod+2"], |s, _| s.tab =
        Tab::Response),
    cmd!(
        "tab-meta",
        "Meta tab: timing, asserts, history",
        ["mod+3"],
        |s, _| s.tab = Tab::Meta
    ),
    cmd!("tab-source", "Source tab", ["mod+4"], |s, _| s.tab =
        Tab::Source),
    cmd!(
        "entry-next",
        "Next entry in a chained file",
        ["mod+bracketright"],
        |s, _| s.cycle_entry(1)
    ),
    cmd!(
        "entry-prev",
        "Previous entry in a chained file",
        ["mod+bracketleft"],
        |s, _| s.cycle_entry(-1)
    ),
    cmd!(
        "export-md",
        "Export run as Markdown",
        ["mod+shift+m"],
        |s, _| s.export(false)
    ),
    cmd!(
        "export-json",
        "Export run as JSON",
        ["mod+shift+j"],
        |s, _| s.export(true)
    ),
    cmd!("copy", "Copy body", ["mod+shift+c"], |s, tx| s
        .copy_body(tx)),
    cmd!("copy-curl", "Copy as curl", [], |s, tx| s.copy_curl(tx)),
    cmd!("copy-run", "Copy run as Markdown", [], |s, tx| s
        .copy_export(tx)),
    cmd!("edit", "Open .hurl file in editor", ["mod+o"], |s, _| s
        .edit()),
    cmd!("reveal", "Reveal .hurl file", ["mod+shift+o"], |s, _| s
        .reveal()),
    cmd!("new", "New request…", ["mod+n"], |s, _| s
        .start_naming(Naming::New)),
    cmd!("rename", "Rename or move request…", ["f2"], |s, _| s
        .start_naming(Naming::Rename)),
    cmd!("duplicate", "Duplicate request…", ["mod+d"], |s, _| s
        .start_naming(Naming::Duplicate)),
    cmd!("delete", "Delete request…", ["mod+backspace"], |s, _| s
        .ask(Confirm::Delete)),
    cmd!("clear-history", "Delete all stored runs…", [], |s, _| s
        .ask(Confirm::ClearHistory)),
    cmd!("save", "Save request", ["mod+s"], |s, _| {
        if s.save() {
            s.say("saved", false);
        }
    }),
    cmd!("revert", "Revert unsaved edits", [], |s, _| s.revert()),
    cmd!("theme", "Next theme", [], |s, _| s.cycle_theme()),
    cmd!("reload", "Reload request files", ["mod+shift+r"], |s, _| {
        s.reload();
        s.say("reloaded", false);
    }),
    cmd!("env-edit", "Edit env files…", [], |s, _| s
        .open_env_edit()),
    cmd!("settings", "Settings…", ["mod+comma"], |s, _| s
        .settings_open =
        true),
];

/// The first key of a command as the platform writes it (⌘↩).
pub fn label(c: &Command) -> Option<String> {
    c.keys
        .first()
        .and_then(|k| Chord::parse(k).ok())
        .map(|c| c.label())
}

/// Run the command bound to `ev`, if any.
pub fn dispatch(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let hit = COMMANDS.iter().find(|c| {
        c.keys
            .iter()
            .any(|k| Chord::parse(k).is_ok_and(|ch| ch.matches_tx(ev, tx)))
    });
    match hit {
        Some(c) => {
            (c.run)(s, tx);
            true
        }
        None => false,
    }
}

/// Fuzzy score of `query` against `hay`: substring beats subsequence; `None` is no match.
pub fn score(query: &str, hay: &str) -> Option<i64> {
    let (q, h) = (query.trim().to_lowercase(), hay.to_lowercase());
    if q.is_empty() {
        return Some(0);
    }
    let mut total = 0;
    for t in q.split_whitespace() {
        total += if let Some(pos) = h.find(t) {
            500 - pos.min(400) as i64
        } else {
            let mut it = h.chars();
            if !t.chars().all(|c| it.any(|x| x == c)) {
                return None;
            }
            10
        };
    }
    Some(total)
}
