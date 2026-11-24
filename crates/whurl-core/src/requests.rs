//! Creating, copying, renaming and deleting request files under `requests/`, keeping run history
//! in step. Callers check that the request isn't running first.

use crate::store;
use crate::tree;
use crate::workspace::{RequestFile, Workspace};
use anyhow::{Context, Result, anyhow, bail};
use std::fs;
use std::path::{Path, PathBuf};

/// A request name from user input: `dir/name`, `.hurl` and a leading `/` optional.
pub fn name(input: &str) -> Result<String> {
    let s = input.trim().trim_start_matches('/');
    let s = s.strip_prefix("requests/").unwrap_or(s);
    let s = s.strip_suffix(".hurl").unwrap_or(s);
    let ok = |p: &str| !p.is_empty() && p != "." && p != ".." && !p.starts_with('.');
    if s.is_empty() || !s.split('/').all(ok) || s.contains(char::is_whitespace) {
        bail!("not a request name: '{input}' (want e.g. folder/name)");
    }
    Ok(s.to_string())
}

/// `requests/<rel>.hurl`, erroring if it already exists.
fn free_path(ws: &Workspace, rel: &str) -> Result<PathBuf> {
    let path = ws.requests_dir().join(format!("{rel}.hurl"));
    if path.exists() {
        bail!("{rel} already exists");
    }
    Ok(path)
}

fn parent_dirs(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path.parent().unwrap_or(path))
}

/// Create `requests/<rel>.hurl` from a stub that reuses `like`'s base URL variable (e.g.
/// `{{api_url}}`), else `https://example.com`.
pub fn create(ws: &Workspace, rel: &str, like: Option<&RequestFile>) -> Result<PathBuf> {
    let path = free_path(ws, rel)?;
    let base = like
        .and_then(|r| r.primary())
        .and_then(|e| {
            e.url
                .starts_with("{{")
                .then(|| e.url.find("}}").map(|i| e.url[..i + 2].to_string()))
                .flatten()
        })
        .unwrap_or_else(|| "https://example.com".into());
    let stub = format!("# {}\nGET {base}/\n", tree::basename(rel));
    parent_dirs(&path)
        .and_then(|_| fs::write(&path, stub))
        .with_context(|| format!("new {rel}"))?;
    Ok(path)
}

/// Copy `req` to `requests/<rel>.hurl`. History stays with the original.
pub fn duplicate(ws: &Workspace, req: &RequestFile, rel: &str) -> Result<PathBuf> {
    let path = free_path(ws, rel)?;
    parent_dirs(&path)
        .and_then(|_| fs::copy(&req.path, &path))
        .context("duplicate")?;
    Ok(path)
}

pub struct Renamed {
    pub path: PathBuf,
    /// Runs moved to the new name; an error leaves the file renamed but its history behind.
    pub history: Result<usize>,
}

/// Rename (or move) `req` to `rel`, taking its run history along and removing folders it left
/// empty. `None` when the name is unchanged.
pub fn rename(ws: &Workspace, req: &RequestFile, rel: &str) -> Result<Option<Renamed>> {
    if rel == req.rel {
        return Ok(None);
    }
    let path = free_path(ws, rel)?;
    parent_dirs(&path)
        .and_then(|_| fs::rename(&req.path, &path))
        .context("rename")?;
    prune_dirs(ws, &req.rel);
    let history = store::rename(ws, &req.rel, rel, &path);
    Ok(Some(Renamed { path, history }))
}

/// Delete `req`'s file, then its run history. The outer error means nothing was deleted; the
/// inner one that the file is gone but its history is not.
pub fn delete(ws: &Workspace, req: &RequestFile) -> Result<Result<usize>> {
    fs::remove_file(&req.path).map_err(|e| anyhow!("delete: {e}"))?;
    prune_dirs(ws, &req.rel);
    Ok(store::remove(ws, &req.rel))
}

/// Remove folders above `rel` that are now empty (remove_dir refuses non-empty ones).
fn prune_dirs(ws: &Workspace, rel: &str) {
    let dir = ws.requests_dir();
    for d in tree::ancestors(rel).collect::<Vec<_>>().into_iter().rev() {
        let _ = fs::remove_dir(dir.join(d));
    }
}
