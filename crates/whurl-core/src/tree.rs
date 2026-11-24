//! Sidebar rows: request paths folded into a folder tree, or a flat ranked list.

use crate::workspace::RequestFile;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A folder, by its path under `requests/` (e.g. `job/admin`).
    Dir {
        path: String,
        depth: usize,
        open: bool,
        count: usize,
    },
    /// Index into the request list.
    Req { idx: usize, depth: usize },
}

/// What is selected, by identity, so it survives rebuilding the rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sel {
    Dir(String),
    Req(String),
}

impl Row {
    pub fn sel(&self, reqs: &[RequestFile]) -> Sel {
        match self {
            Row::Dir { path, .. } => Sel::Dir(path.clone()),
            Row::Req { idx, .. } => Sel::Req(reqs[*idx].rel.clone()),
        }
    }

    pub fn depth(&self) -> usize {
        match self {
            Row::Dir { depth, .. } | Row::Req { depth, .. } => *depth,
        }
    }
}

#[derive(Default)]
struct Node {
    dirs: BTreeMap<String, Node>,
    reqs: Vec<(String, usize)>,
    count: usize,
}

/// Folders first, then files, each alphabetical. `matched` limits which requests appear; when
/// `filtering`, every folder holding a match is shown open regardless of `expanded`.
pub fn build(
    reqs: &[RequestFile],
    matched: &[usize],
    expanded: &HashSet<String>,
    filtering: bool,
) -> Vec<Row> {
    let mut root = Node::default();
    for &i in matched {
        let mut node = &mut root;
        node.count += 1;
        let mut parts: Vec<&str> = reqs[i].rel.split('/').collect();
        let name = parts.pop().unwrap_or_default();
        for p in parts {
            node = node.dirs.entry(p.to_string()).or_default();
            node.count += 1;
        }
        node.reqs.push((name.to_string(), i));
    }
    let mut out = vec![];
    walk(&root, "", 0, expanded, filtering, &mut out);
    out
}

fn walk(
    node: &Node,
    prefix: &str,
    depth: usize,
    expanded: &HashSet<String>,
    filtering: bool,
    out: &mut Vec<Row>,
) {
    for (name, child) in &node.dirs {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let open = filtering || expanded.contains(&path);
        out.push(Row::Dir {
            path: path.clone(),
            depth,
            open,
            count: child.count,
        });
        if open {
            walk(child, &path, depth + 1, expanded, filtering, out);
        }
    }
    let mut reqs = node.reqs.clone();
    reqs.sort();
    out.extend(reqs.into_iter().map(|(_, idx)| Row::Req { idx, depth }));
}

pub fn flat(matched: &[usize]) -> Vec<Row> {
    matched
        .iter()
        .map(|&idx| Row::Req { idx, depth: 0 })
        .collect()
}

/// `a/b/c` → [`a`, `a/b`]: the folders that must be open for `rel` to show.
pub fn ancestors(rel: &str) -> impl Iterator<Item = &str> {
    rel.match_indices('/').map(move |(i, _)| &rel[..i])
}

pub fn parent(path: &str) -> Option<&str> {
    path.rfind('/').map(|i| &path[..i])
}

pub fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn reqs(rels: &[&str]) -> Vec<RequestFile> {
        rels.iter()
            .map(|r| RequestFile {
                path: PathBuf::new(),
                rel: r.to_string(),
                title: None,
                entries: vec![],
            })
            .collect()
    }

    fn render(reqs: &[RequestFile], rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                Row::Dir {
                    path,
                    depth,
                    open,
                    count,
                } => {
                    format!(
                        "{}{} {}/ {count}",
                        "  ".repeat(*depth),
                        if *open { "v" } else { ">" },
                        basename(path)
                    )
                }
                Row::Req { idx, depth } => {
                    format!("{}{}", "  ".repeat(*depth), basename(&reqs[*idx].rel))
                }
            })
            .collect()
    }

    #[test]
    fn folders_first_collapsed_and_filtered() {
        let r = reqs(&[
            "zeta",
            "job/create",
            "job/admin/purge",
            "auth/login",
            "alpha",
        ]);
        let all: Vec<usize> = (0..r.len()).collect();
        assert_eq!(
            render(&r, &build(&r, &all, &HashSet::new(), false)),
            ["> auth/ 1", "> job/ 2", "alpha", "zeta"]
        );
        let open: HashSet<String> = ["job".into(), "job/admin".into()].into();
        assert_eq!(
            render(&r, &build(&r, &all, &open, false)),
            [
                "> auth/ 1",
                "v job/ 2",
                "  v admin/ 1",
                "    purge",
                "  create",
                "alpha",
                "zeta"
            ]
        );
        // A filter opens every folder on the way to a match and hides the rest.
        assert_eq!(
            render(&r, &build(&r, &[2], &HashSet::new(), true)),
            ["v job/ 1", "  v admin/ 1", "    purge"]
        );
        assert_eq!(
            ancestors("job/admin/purge").collect::<Vec<_>>(),
            ["job", "job/admin"]
        );
    }
}
