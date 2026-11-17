//! The `:` command bar: every registry command by id, plus commands that take arguments.
//! Completion is prefix-then-substring over the word under the cursor (always the last one).

// `Ok(app.action())` keeps the command table compact.
#![allow(clippy::unit_arg)]

use super::app::{App, Mode, TABS};
use super::commands::COMMANDS;
use super::settings;

#[derive(Default)]
pub struct CmdLine {
    pub input: String,
    /// Highlighted completion.
    pub sel: usize,
}

/// A completion: the word to insert, plus what the list shows beside it.
pub struct Cand {
    pub text: String,
    pub usage: &'static str,
    pub desc: String,
    pub keys: String,
    /// Another word follows, so accepting it adds a space.
    more: bool,
}

type Complete = fn(&App, &[&str], &str) -> Vec<(String, String, bool)>;

struct ArgCmd {
    name: &'static str,
    usage: &'static str,
    desc: &'static str,
    /// Candidates for the next argument, given the ones before it and the partial word.
    complete: Complete,
    run: fn(&mut App, &[&str]) -> Result<(), String>,
}

fn vals(list: &[&str]) -> Vec<(String, String, bool)> {
    list.iter().map(|v| (v.to_string(), String::new(), false)).collect()
}

/// Commands with arguments. A name shared with the registry (e.g. `sidebar`) replaces it here;
/// with no arguments these fall back to the registry's behaviour.
static ARG_CMDS: &[ArgCmd] = &[
    ArgCmd {
        name: "env",
        usage: "<name>",
        desc: "switch env",
        complete: |a, args, _| {
            if !args.is_empty() {
                return vec![];
            }
            a.envs.iter().map(|e| (e.clone(), if e == a.env() { "current".into() } else { String::new() }, false)).collect()
        },
        run: |a, args| {
            let [name] = args else { return usage("env") };
            let i = a.envs.iter().position(|e| e == name).ok_or_else(|| format!("no env {name}; one of {}", a.envs.join(", ")))?;
            a.env = i;
            a.on_select();
            Ok(a.say(format!("env {name}"), false))
        },
    },
    ArgCmd {
        name: "set",
        usage: "<key> [value]",
        desc: "change a setting for this session; with no value a toggle flips, others show",
        complete: complete_set,
        run: |a, args| set(a, args, false),
    },
    ArgCmd {
        name: "set!",
        usage: "<key> [value]",
        desc: "change a setting and save it to config.toml; with no value saves the current one",
        complete: complete_set,
        run: |a, args| set(a, args, true),
    },
    ArgCmd {
        name: "export",
        usage: "md|json",
        desc: "export the run to .whurl/exports",
        complete: |_, args, _| if args.is_empty() { vals(&["md", "json"]) } else { vec![] },
        run: |a, args| match args {
            ["md"] => Ok(a.export(false)),
            ["json"] => Ok(a.export(true)),
            _ => usage("export"),
        },
    },
    ArgCmd {
        name: "sidebar",
        usage: "[tree|flat|hidden]",
        desc: "sidebar style for this session; no argument flips tree ↔ flat",
        complete: |_, args, _| if args.is_empty() { vals(&["tree", "flat", "hidden"]) } else { vec![] },
        run: |a, args| match args {
            [] => Ok(a.toggle_sidebar()),
            [s] => settings::set(a, "ui.sidebar", Some(s), false),
            _ => usage("sidebar"),
        },
    },
    ArgCmd {
        name: "tab",
        usage: "request|response|meta|source",
        desc: "switch tab",
        complete: |_, args, _| if args.is_empty() { vals(&["request", "response", "meta", "source"]) } else { vec![] },
        run: |a, args| {
            let [name] = args else { return usage("tab") };
            let tab = TABS.into_iter().find(|t| t.title().eq_ignore_ascii_case(name)).ok_or("no such tab")?;
            Ok(a.set_tab(tab))
        },
    },
    ArgCmd {
        name: "asserts",
        usage: "[on|off]",
        desc: "asserts for this session; no argument toggles",
        complete: |_, args, _| if args.is_empty() { vals(&["on", "off"]) } else { vec![] },
        run: |a, args| match args {
            [] => Ok(a.toggle_asserts()),
            [v] => settings::set(a, "run.asserts", Some(v), false),
            _ => usage("asserts"),
        },
    },
    ArgCmd {
        name: "wrap",
        usage: "[on|off]",
        desc: "line wrap for this session; no argument toggles",
        complete: |_, args, _| if args.is_empty() { vals(&["on", "off"]) } else { vec![] },
        run: |a, args| match args {
            [] => Ok(a.toggle_wrap()),
            [v] => settings::set(a, "ui.wrap", Some(v), false),
            _ => usage("wrap"),
        },
    },
    ArgCmd {
        name: "cancel",
        usage: "[all]",
        desc: "cancel the selected request's run, or every run",
        complete: |_, args, _| if args.is_empty() { vals(&["all"]) } else { vec![] },
        run: |a, args| match args {
            [] => {
                let job = a.key().and_then(|k| a.running.remove(&k)).ok_or("nothing running here")?;
                job.cancel();
                a.view = None;
                Ok(a.say("cancelled", false))
            }
            ["all"] => {
                let n = a.running.len();
                a.running.drain().for_each(|(_, j)| j.cancel());
                a.view = None;
                Ok(a.say(format!("cancelled {n}"), false))
            }
            _ => usage("cancel"),
        },
    },
];

/// (alias, registry id)
const ALIASES: &[(&str, &str)] = &[("q", "quit"), ("open", "edit"), ("history", "tab-meta")];

fn usage(name: &str) -> Result<(), String> {
    let c = ARG_CMDS.iter().find(|c| c.name == name).expect("known command");
    Err(format!("usage: :{} {}", c.name, c.usage))
}

fn complete_set(app: &App, args: &[&str], partial: &str) -> Vec<(String, String, bool)> {
    match args {
        [] => settings::KEYS
            .iter()
            .map(|(k, v)| (k.to_string(), format!("{v} · now {}", settings::get(app, k).unwrap_or_default()), true))
            .collect(),
        [key] => settings::values(app, key, partial).into_iter().map(|v| (v, String::new(), false)).collect(),
        _ => vec![],
    }
}

fn set(app: &mut App, args: &[&str], persist: bool) -> Result<(), String> {
    let name = if persist { "set!" } else { "set" };
    let Some((key, rest)) = args.split_first() else { return usage(name) };
    let value = (!rest.is_empty()).then(|| rest.join(" "));
    settings::set(app, key, value.as_deref(), persist)
}

pub fn open(app: &mut App) {
    app.cmdline = CmdLine::default();
    app.mode = Mode::Command;
}

/// Finished words, and the partial one being typed (empty after a space).
fn words(input: &str) -> (Vec<&str>, &str) {
    let mut w: Vec<&str> = input.split_whitespace().collect();
    let partial = if input.ends_with(char::is_whitespace) { "" } else { w.pop().unwrap_or("") };
    (w, partial)
}

fn keys_of(id: &str) -> String {
    COMMANDS.iter().find(|c| c.id == id && !c.keys.is_empty()).map(|c| c.key_labels()).unwrap_or_default()
}

pub fn completions(app: &App) -> Vec<Cand> {
    let (done, partial) = words(&app.cmdline.input);
    let all: Vec<Cand> = match done.split_first() {
        None => {
            let args = ARG_CMDS.iter().map(|c| Cand {
                text: c.name.into(),
                usage: c.usage,
                desc: c.desc.into(),
                keys: keys_of(c.name),
                more: true,
            });
            let reg = COMMANDS.iter().filter(|c| !ARG_CMDS.iter().any(|a| a.name == c.id)).map(|c| Cand {
                text: c.id.into(),
                usage: "",
                desc: c.desc.into(),
                keys: keys_of(c.id),
                more: false,
            });
            let aliases = ALIASES.iter().map(|(a, id)| Cand {
                text: a.to_string(),
                usage: "",
                desc: format!("= :{id}"),
                keys: keys_of(id),
                more: false,
            });
            args.chain(reg).chain(aliases).collect()
        }
        Some((name, args)) => match ARG_CMDS.iter().find(|c| c.name == *name) {
            Some(c) => (c.complete)(app, args, partial)
                .into_iter()
                .map(|(text, desc, more)| Cand { text, usage: "", desc, keys: String::new(), more })
                .collect(),
            None => vec![],
        },
    };
    let p = partial.to_lowercase();
    let (mut out, rest): (Vec<_>, Vec<_>) = all
        .into_iter()
        .filter(|c| c.text.to_lowercase().contains(&p))
        .partition(|c| c.text.to_lowercase().starts_with(&p));
    out.extend(rest);
    out
}

/// The command whose arguments are being completed, for the list's title.
pub fn completing(app: &App) -> Option<(&'static str, &'static str)> {
    let (done, _) = words(&app.cmdline.input);
    let c = ARG_CMDS.iter().find(|c| Some(&c.name) == done.first())?;
    Some((c.name, c.usage))
}

pub fn move_sel(app: &mut App, delta: isize) {
    let n = completions(app).len() as isize;
    if n > 0 {
        app.cmdline.sel = (app.cmdline.sel as isize + delta).rem_euclid(n) as usize;
    }
}

/// Tab: replace the partial word with the highlighted completion.
pub fn accept(app: &mut App) {
    let cands = completions(app);
    let Some(c) = cands.get(app.cmdline.sel) else { return };
    let (_, partial) = words(&app.cmdline.input);
    let keep = app.cmdline.input.len() - partial.len();
    app.cmdline.input.truncate(keep);
    app.cmdline.input.push_str(&c.text);
    if c.more {
        app.cmdline.input.push(' ');
    }
    app.cmdline.sel = 0;
}

pub fn edited(app: &mut App) {
    app.cmdline.sel = 0;
}

/// Enter: a partial word that matches nothing exactly takes the highlighted completion first,
/// so `:exp<enter>` runs what the list shows.
pub fn submit(app: &mut App) {
    let (_, partial) = words(&app.cmdline.input);
    if !partial.is_empty() {
        let cands = completions(app);
        if !cands.iter().any(|c| c.text == partial) && cands.get(app.cmdline.sel).is_some() {
            accept(app);
        }
    }
    app.mode = Mode::Normal;
    let line = std::mem::take(&mut app.cmdline.input);
    execute(app, &line);
}

pub fn execute(app: &mut App, line: &str) {
    let mut it = line.split_whitespace();
    let Some(name) = it.next() else { return };
    let args: Vec<&str> = it.collect();
    let res = match ARG_CMDS.iter().find(|c| c.name == name) {
        Some(c) => (c.run)(app, &args),
        None => {
            let id = ALIASES.iter().find(|a| a.0 == name).map_or(name, |a| a.1);
            match COMMANDS.iter().find(|c| c.id == id) {
                Some(c) if args.is_empty() => Ok((c.run)(app)),
                Some(_) => Err(format!(":{name} takes no arguments")),
                None => Err(format!("unknown command :{name}")),
            }
        }
    };
    if let Err(e) = res {
        app.say(e, true);
    }
}
