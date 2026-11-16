//! Workspace layout: `requests/**/*.hurl`, `env/<name>.env`, `env/<name>.secrets.env`,
//! `env/shared.secrets.env`. whurl keeps its own state (run history, exports) in `.whurl/`.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

const METHODS: &[&str] = &[
    "GET", "HEAD", "POST", "PUT", "DELETE", "CONNECT", "OPTIONS", "TRACE", "PATCH", "LINK", "UNLINK",
    "PURGE", "LOCK", "UNLOCK", "PROPFIND", "VIEW", "QUERY",
];

pub struct Workspace {
    pub root: PathBuf,
}

/// A `.hurl` file and the request lines in it (a file may chain several requests).
#[derive(Clone, Debug)]
pub struct RequestFile {
    pub path: PathBuf,
    /// Path under `requests/` without the `.hurl` suffix, e.g. `job/create_job`.
    pub rel: String,
    /// First comment line, e.g. `# createJob` → `createJob`.
    pub title: Option<String>,
    pub entries: Vec<EntryLine>,
}

#[derive(Clone, Debug)]
pub struct EntryLine {
    pub method: String,
    pub url: String,
    pub line: usize,
}

impl RequestFile {
    pub fn parse(path: &Path, requests_dir: &Path) -> Result<Self> {
        let src = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let rel = path
            .strip_prefix(requests_dir)
            .unwrap_or(path)
            .with_extension("")
            .to_string_lossy()
            .into_owned();
        let title = src
            .lines()
            .next()
            .and_then(|l| l.strip_prefix('#'))
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        Ok(Self { path: path.to_path_buf(), rel, title, entries: parse_entries(&src) })
    }

    /// The request that best represents the file: the last one (earlier ones are usually setup).
    pub fn primary(&self) -> Option<&EntryLine> {
        self.entries.last()
    }
}

fn parse_entries(src: &str) -> Vec<EntryLine> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for (i, line) in src.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            // A fence opens with ``` (+ optional lang) and closes with a bare ```.
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut parts = line.split_whitespace();
        if let (Some(m), Some(url)) = (parts.next(), parts.next())
            && METHODS.contains(&m) && !line.starts_with(char::is_whitespace) {
                out.push(EntryLine { method: m.to_string(), url: url.to_string(), line: i + 1 });
            }
    }
    out
}

impl Workspace {
    /// `--workspace`/`$WHURL_WORKSPACE`, else the nearest ancestor of the cwd holding `requests/`
    /// and `env/`.
    pub fn discover(explicit: Option<PathBuf>) -> Result<Self> {
        let is_ws = |p: &Path| p.join("requests").is_dir() && p.join("env").is_dir();
        if let Some(p) = explicit {
            let p = fs::canonicalize(&p).with_context(|| format!("workspace {}", p.display()))?;
            if !is_ws(&p) {
                bail!("{} has no requests/ and env/ directories", p.display());
            }
            return Ok(Self { root: p });
        }
        if let Ok(cwd) = std::env::current_dir()
            && let Some(p) = cwd.ancestors().find(|p| is_ws(p)) {
                return Ok(Self { root: p.to_path_buf() });
            }
        bail!("no workspace found; pass --workspace or set WHURL_WORKSPACE")
    }

    pub fn requests_dir(&self) -> PathBuf {
        self.root.join("requests")
    }

    pub fn state_dir(&self) -> PathBuf {
        self.root.join(".whurl")
    }

    pub fn requests(&self) -> Result<Vec<RequestFile>> {
        let dir = self.requests_dir();
        let mut paths = Vec::new();
        walk(&dir, &mut paths)?;
        paths.sort();
        paths.iter().map(|p| RequestFile::parse(p, &dir)).collect()
    }

    /// Resolve a path (absolute, cwd-relative, or workspace-relative, `.hurl` optional) or a
    /// fuzzy query against request names.
    pub fn resolve(&self, query: &str) -> Result<RequestFile> {
        let dir = self.requests_dir();
        let q = Path::new(query);
        for cand in [q.to_path_buf(), self.root.join(q), dir.join(q)] {
            for c in [cand.clone(), cand.with_extension("hurl")] {
                if c.is_file() {
                    let c = fs::canonicalize(&c)?;
                    return RequestFile::parse(&c, &fs::canonicalize(&dir)?);
                }
            }
        }
        let all = self.requests()?;
        let matches = search(&all, query);
        match matches.as_slice() {
            [] => bail!("no request matches '{query}' (try `whurl list`)"),
            [one] => Ok(all[*one].clone()),
            many => {
                // Prefer an exact name, then a unique exact file stem / path suffix.
                if let Some(&exact) = many.iter().find(|&&i| all[i].rel == query) {
                    return Ok(all[exact].clone());
                }
                let suffix = format!("/{query}");
                let tail: Vec<_> = many.iter().filter(|&&i| all[i].rel.ends_with(&suffix)).collect();
                if let [one] = tail[..] {
                    return Ok(all[*one].clone());
                }
                let names: Vec<_> = many.iter().take(15).map(|&i| format!("  {}", all[i].rel)).collect();
                bail!(
                    "'{query}' matches {} requests:\n{}{}",
                    many.len(),
                    names.join("\n"),
                    if many.len() > 15 { "\n  …" } else { "" }
                )
            }
        }
    }

    pub fn envs(&self) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(self.root.join("env"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let stem = name.strip_suffix(".env")?;
                (!stem.ends_with(".secrets")).then(|| stem.to_string())
            })
            .collect();
        // Conventional order first, then anything else alphabetically.
        let rank = |e: &str| ["local", "staging", "production"].iter().position(|x| *x == e).unwrap_or(9);
        out.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.cmp(b)));
        out
    }

    pub fn vars_file(&self, env: &str) -> Result<PathBuf> {
        let p = self.root.join("env").join(format!("{env}.env"));
        if !p.is_file() {
            bail!("no env file env/{env}.env (have: {})", self.envs().join(", "));
        }
        Ok(p)
    }

    pub fn secrets_files(&self, env: &str) -> Vec<PathBuf> {
        [format!("{env}.secrets.env"), "shared.secrets.env".into()]
            .iter()
            .map(|f| self.root.join("env").join(f))
            .filter(|p| p.is_file())
            .collect()
    }

    pub fn vars(&self, env: &str) -> Vec<(String, String)> {
        self.vars_file(env).map(|p| read_env(&p)).unwrap_or_default()
    }

    pub fn redactor(&self, env: &str) -> Redactor {
        Redactor::new(self.secrets_files(env).iter().flat_map(|p| read_env(p)).map(|(_, v)| v))
    }

    /// Replace `{{var}}` with non-secret env values for display; unknown vars stay as-is.
    pub fn interpolate(&self, s: &str, vars: &[(String, String)]) -> String {
        let mut out = s.to_string();
        for (k, v) in vars {
            out = out.replace(&format!("{{{{{k}}}}}"), v);
        }
        out
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let p = e?.path();
        if p.is_dir() {
            walk(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "hurl") {
            out.push(p);
        }
    }
    Ok(())
}

pub fn read_env(path: &Path) -> Vec<(String, String)> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

/// Indices into `all`, best match first. Every whitespace-separated term must match the
/// request's name/title as a substring or (weaker) a subsequence.
pub fn search(all: &[RequestFile], query: &str) -> Vec<usize> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return (0..all.len()).collect();
    }
    let mut scored: Vec<(i64, usize)> = all
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let hay = format!("{} {}", r.rel, r.title.as_deref().unwrap_or("")).to_lowercase();
            let mut score = 0i64;
            for t in &terms {
                score += if hay == *t || r.rel.to_lowercase() == *t {
                    1000
                } else if let Some(pos) = hay.find(t.as_str()) {
                    500 - pos.min(400) as i64
                } else if is_subsequence(t, &hay) {
                    10
                } else {
                    return None;
                };
            }
            Some((score, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// Masks secret values. Hurl redacts its own terminal output but not its JSON report, so every
/// string whurl prints, stores, or exports passes through here.
#[derive(Clone, Default)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub fn new(values: impl IntoIterator<Item = String>) -> Self {
        let mut secrets: Vec<String> = values.into_iter().filter(|v| v.len() >= 4).collect();
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        secrets.dedup();
        Self { secrets }
    }

    pub fn apply(&self, s: &str) -> String {
        let mut out = s.to_string();
        for v in &self.secrets {
            if out.contains(v.as_str()) {
                out = out.replace(v.as_str(), "***");
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_skip_bodies() {
        let src = "# t\nGET {{h}}/a\nX: 1\n```\nPOST not/a/request\n```\nHTTP *\n\nPOST {{h}}/b\n";
        let e = parse_entries(src);
        assert_eq!(e.len(), 2);
        assert_eq!((e[1].method.as_str(), e[1].line), ("POST", 9));
    }

    #[test]
    fn redacts_longest_first() {
        let r = Redactor::new(["abcd".into(), "abcdef".into()]);
        assert_eq!(r.apply("x abcdef abcd"), "x *** ***");
    }
}
