//! Dynamic shell completion: registration scripts and value completers for requests, envs, and
//! stored runs.

use crate::workspace::Workspace;
use crate::{config, store};
use anyhow::Result;
use clap::{CommandFactory, ValueEnum};
use clap_complete::env::{Bash, Elvish, EnvCompleter, Fish, Powershell, Zsh};
use clap_complete::{CompleteEnv, CompletionCandidate};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

const ENV_VAR: &str = "WHURL_COMPLETE";
/// How many `last~N` selectors to offer; beyond this, run ids are more useful.
const MAX_LAST_N: usize = 10;

#[derive(Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Powershell,
    Elvish,
}

/// Answers a completion request and exits if the shell invoked us for one; otherwise returns.
pub fn handle<C: CommandFactory>() {
    CompleteEnv::with_factory(C::command)
        .var(ENV_VAR)
        .complete();
}

pub fn print_script(shell: Shell) -> Result<()> {
    let sh: &dyn EnvCompleter = match shell {
        Shell::Bash => &Bash,
        Shell::Zsh => &Zsh,
        Shell::Fish => &Fish,
        Shell::Powershell => &Powershell,
        Shell::Elvish => &Elvish,
    };
    let completer = executable();
    sh.write_registration(
        ENV_VAR,
        "whurl",
        "whurl",
        &completer,
        &mut std::io::stdout().lock(),
    )?;
    Ok(())
}

pub fn requests(current: &OsStr) -> Vec<CompletionCandidate> {
    workspace()
        .map(|ws| request_candidates(&ws, current))
        .unwrap_or_default()
}

pub fn envs(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    workspace()
        .map(|ws| {
            ws.envs()
                .into_iter()
                .filter(|env| env.starts_with(current))
                .map(CompletionCandidate::new)
                .collect()
        })
        .unwrap_or_default()
}

pub fn runs(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    workspace()
        .map(|ws| run_candidates(&ws, current))
        .unwrap_or_default()
}

/// Run selectors without reading any run file: request and env are recovered from each id's
/// `_{env}_{request}` suffix by matching it against the workspace's requests.
fn run_candidates(ws: &Workspace, current: &str) -> Vec<CompletionCandidate> {
    let ids = store::ids(ws);
    let requests: Vec<String> = ws
        .requests()
        .unwrap_or_default()
        .into_iter()
        .map(|r| r.rel)
        .collect();
    let tails: Vec<(String, &str)> = requests
        .iter()
        .map(|r| (format!("_{}", r.replace('/', "__")), r.as_str()))
        .collect();

    let mut help: HashMap<&str, String> = HashMap::new();
    let mut with_runs: Vec<&str> = Vec::new();
    for id in &ids {
        // Longest match wins, so a run of `a/b` isn't read as request `b` in env `..._a`.
        let Some((tail, request)) = tails
            .iter()
            .filter(|(t, _)| id.ends_with(t.as_str()))
            .max_by_key(|(t, _)| t.len())
        else {
            continue;
        };
        let rest = &id[..id.len() - tail.len()];
        if let Some((_, env)) = rest.rsplit_once('_') {
            help.insert(id, format!("{request} [{env}]"));
        }
        if !with_runs.contains(request) {
            with_runs.push(*request);
        }
    }
    with_runs.sort_unstable();

    let last = std::iter::once("last".to_string())
        .chain((1..ids.len().min(MAX_LAST_N + 1)).map(|n| format!("last~{n}")));
    last.chain(ids.iter().cloned())
        .chain(with_runs.into_iter().map(String::from))
        .filter(|value| value.starts_with(current))
        .map(|value| {
            let h = help.get(value.as_str()).cloned();
            CompletionCandidate::new(value).help(h.map(Into::into))
        })
        .collect()
}

fn request_candidates(ws: &Workspace, current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    let prefix = if current.starts_with("requests/") {
        "requests/"
    } else {
        ""
    };
    ws.requests()
        .unwrap_or_default()
        .into_iter()
        .map(|request| (format!("{prefix}{}", request.rel), request.title))
        .filter(|(value, _)| value.starts_with(current))
        .map(|(value, title)| CompletionCandidate::new(value).help(title.map(Into::into)))
        .collect()
}

/// Completion must be silent when the cwd is not a workspace or configuration is invalid.
fn workspace() -> Option<Workspace> {
    let explicit = workspace_arg(&args()).or_else(|| {
        std::env::var_os("WHURL_WORKSPACE")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
    });
    let cfg = config::Config::load().ok()?;
    Workspace::discover(explicit, cfg.workspace()).ok()
}

/// Shell completion runs in a fresh process. The arguments following its `--` delimiter are the
/// command line the user is completing, not the arguments passed to the completion process.
fn args() -> Vec<OsString> {
    let args: Vec<_> = std::env::args_os().collect();
    args.iter()
        .position(|arg| arg == OsStr::new("--"))
        .map(|i| args[i + 1..].to_vec())
        .unwrap_or(args)
}

fn workspace_arg(args: &[OsString]) -> Option<PathBuf> {
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        if arg == OsStr::new("--") {
            break;
        }
        if arg == OsStr::new("--workspace") {
            return args.next().map(PathBuf::from);
        }
        if let Some(path) = arg
            .to_str()
            .and_then(|arg| arg.strip_prefix("--workspace="))
        {
            return Some(PathBuf::from(path));
        }
    }
    None
}

fn executable() -> String {
    let path = std::env::args_os()
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("whurl"));
    if path.components().count() > 1 {
        std::fs::canonicalize(&path)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    } else {
        path.to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn values(cs: Vec<CompletionCandidate>) -> Vec<String> {
        cs.iter()
            .map(|c| c.get_value().to_string_lossy().into_owned())
            .collect()
    }

    fn workspace_with(files: &[&str]) -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        for f in files {
            let p = dir.path().join("requests").join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "GET https://example.test/\n").unwrap();
        }
        let ws = Workspace {
            root: dir.path().to_path_buf(),
        };
        (dir, ws)
    }

    #[test]
    fn workspace_arg_stops_at_hurl_args() {
        let args = os(&["whurl", "run", "x", "--", "--workspace", "nope"]);
        assert_eq!(workspace_arg(&args), None);
        let args = os(&["whurl", "run", "--workspace=/tmp/ws", "x"]);
        assert_eq!(workspace_arg(&args), Some(PathBuf::from("/tmp/ws")));
    }

    #[test]
    fn request_completion_uses_canonical_or_requests_prefixed_names() {
        let (_dir, ws) =
            workspace_with(&["billing/list_invoices.hurl", "billing/list_payments.hurl"]);
        assert_eq!(
            values(request_candidates(&ws, OsStr::new("billing/list_"))),
            ["billing/list_invoices", "billing/list_payments"]
        );
        assert_eq!(
            values(request_candidates(
                &ws,
                OsStr::new("requests/billing/list_")
            )),
            [
                "requests/billing/list_invoices",
                "requests/billing/list_payments"
            ]
        );
    }

    #[test]
    fn run_completion_derives_request_and_env_from_ids() {
        let (dir, ws) = workspace_with(&["b.hurl", "a/b.hurl"]);
        let runs = dir.path().join(".whurl/runs");
        std::fs::create_dir_all(&runs).unwrap();
        for id in [
            "20260101-000000-000_local_a__b",
            "20260102-000000-000_staging_b",
        ] {
            std::fs::write(runs.join(format!("{id}.json")), "{}").unwrap();
        }
        let cs = run_candidates(&ws, "");
        let help: Vec<_> = cs
            .iter()
            .map(|c| c.get_help().map(|h| h.to_string()))
            .collect();
        assert_eq!(
            values(cs),
            [
                "last",
                "last~1",
                "20260102-000000-000_staging_b",
                "20260101-000000-000_local_a__b",
                "a/b",
                "b"
            ]
        );
        assert_eq!(
            help,
            [
                None,
                None,
                Some("b [staging]".into()),
                Some("a/b [local]".into()),
                None,
                None
            ]
        );
    }
}
