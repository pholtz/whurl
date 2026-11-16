//! Run history in `<workspace>/.whurl/runs/<id>.json`. Ids sort chronologically and embed the
//! env and request name, so filtering doesn't require parsing every file.

use crate::run::Run;
use crate::workspace::Workspace;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::PathBuf;

fn runs_dir(ws: &Workspace) -> PathBuf {
    ws.state_dir().join("runs")
}

pub fn save(ws: &Workspace, run: &Run) -> Result<()> {
    let dir = runs_dir(ws);
    fs::create_dir_all(&dir)?;
    let gitignore = ws.state_dir().join(".gitignore");
    if !gitignore.exists() {
        fs::write(&gitignore, "*\n")?;
    }
    fs::write(dir.join(format!("{}.json", run.id)), serde_json::to_vec_pretty(run)?)?;
    Ok(())
}

/// Run ids, newest first.
pub fn ids(ws: &Workspace) -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(runs_dir(ws))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(String::from))
        .collect();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    ids
}

pub fn load(ws: &Workspace, id: &str) -> Result<Run> {
    let p = runs_dir(ws).join(format!("{id}.json"));
    let s = fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
    serde_json::from_str(&s).with_context(|| format!("parsing {}", p.display()))
}

fn id_suffix(env: &str, request: &str) -> String {
    format!("_{env}_{}", request.replace('/', "__"))
}

/// Ids for one request (optionally in one env), newest first.
pub fn ids_for(ws: &Workspace, request: &str, env: Option<&str>) -> Vec<String> {
    let tail = format!("_{}", request.replace('/', "__"));
    let exact = env.map(|e| id_suffix(e, request));
    ids(ws)
        .into_iter()
        .filter(|id| match &exact {
            Some(s) => id.ends_with(s.as_str()),
            None => id.ends_with(&tail),
        })
        .collect()
}

/// `last`, `last~N`, an exact id, a unique id prefix, or a request query (its latest run).
pub fn find(ws: &Workspace, sel: &str) -> Result<Run> {
    let all = ids(ws);
    if all.is_empty() {
        bail!("no runs yet");
    }
    if let Some(rest) = sel.strip_prefix("last") {
        let n: usize = rest.strip_prefix('~').map(str::parse).transpose()?.unwrap_or(0);
        return load(ws, all.get(n).context("not that many runs")?);
    }
    if all.iter().any(|i| i == sel) {
        return load(ws, sel);
    }
    let pre: Vec<_> = all.iter().filter(|i| i.starts_with(sel)).collect();
    if pre.len() == 1 {
        return load(ws, pre[0]);
    }
    let req = ws.resolve(sel)?;
    match ids_for(ws, &req.rel, None).first() {
        Some(id) => load(ws, id),
        None => bail!("no runs of {}", req.rel),
    }
}

/// Latencies (µs) of the primary entry for recent runs of a request in an env, oldest first.
pub fn latencies(ws: &Workspace, request: &str, env: &str, limit: usize) -> Vec<u64> {
    let mut out: Vec<u64> = ids_for(ws, request, Some(env))
        .iter()
        .take(limit)
        .filter_map(|id| load(ws, id).ok())
        .filter_map(|r| r.primary().and_then(|e| e.total_us()))
        .collect();
    out.reverse();
    out
}
