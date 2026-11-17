//! User config: `$XDG_CONFIG_HOME/whurl/config.toml`, else `~/.config/whurl/config.toml`.
//! A missing file means defaults; unknown keys are errors so typos surface.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Workspace used when neither `--workspace`/`$WHURL_WORKSPACE` nor the cwd provides one.
    pub workspace: Option<String>,
    pub ui: Ui,
    pub request: Sections,
    pub response: Sections,
    pub run: RunCfg,
    pub viewer: Viewer,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    #[default]
    Sidebar,
    Focus,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum SidebarStyle {
    #[default]
    Tree,
    Flat,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ui {
    pub layout: Layout,
    pub sidebar: SidebarStyle,
    pub wrap: bool,
    pub auto_switch_response: bool,
    pub theme: String,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            layout: Layout::default(),
            sidebar: SidebarStyle::default(),
            wrap: true,
            auto_switch_response: true,
            theme: "default".into(),
        }
    }
}

pub const REQUEST_SECTIONS: &[&str] = &["headers", "body", "curl"];
pub const RESPONSE_SECTIONS: &[&str] = &["headers", "body"];

/// Which sections a tab shows, in display order. `None` means the tab's default.
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Sections {
    pub sections: Option<Vec<String>>,
}

impl Sections {
    pub fn or<'a>(&'a self, default: &'a [&'a str]) -> Vec<&'a str> {
        match &self.sections {
            Some(v) => v.iter().map(String::as_str).collect(),
            None => default.to_vec(),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct RunCfg {
    /// Overridden by `-e` / `$HURL_ENV`.
    pub default_env: Option<String>,
    pub asserts: bool,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Viewer {
    /// Empty → `$EDITOR -R`, then `less`.
    pub command: String,
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".config")))?;
        Some(base.join("whurl").join("config.toml"))
    }

    pub fn load() -> Result<Self> {
        let Some(path) = Self::path() else { return Ok(Self::default()) };
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let cfg: Self = toml::from_str(&src).with_context(|| format!("parsing {}", path.display()))?;
        for (name, s, allowed) in
            [("request", &cfg.request, REQUEST_SECTIONS), ("response", &cfg.response, RESPONSE_SECTIONS)]
        {
            if let Some(bad) = s.sections.iter().flatten().find(|x| !allowed.contains(&x.as_str())) {
                anyhow::bail!("{}: unknown {name} section '{bad}' (have: {})", path.display(), allowed.join(", "));
            }
        }
        Ok(cfg)
    }

    /// The config path for display, with `$HOME` shown as `~`.
    pub fn display_path() -> String {
        let Some(p) = Self::path() else { return "(no config location)".into() };
        match home().and_then(|h| p.strip_prefix(h).ok().map(|r| format!("~/{}", r.display()))) {
            Some(s) => s,
            None => p.display().to_string(),
        }
    }

    pub fn workspace(&self) -> Option<PathBuf> {
        self.workspace.as_deref().map(expand_tilde)
    }

    /// `-e` / `$HURL_ENV` (already folded into `cli`), then config, then `local`.
    pub fn env(&self, cli: Option<String>) -> String {
        cli.or_else(|| self.run.default_env.clone()).unwrap_or_else(|| "local".into())
    }
}

/// Write one key back to the config file, keeping its comments and layout. `None` removes it.
pub fn save_key(table: &str, key: &str, value: Option<toml_edit::Value>) -> Result<PathBuf> {
    let path = Config::path().context("no config location: set $HOME or $XDG_CONFIG_HOME")?;
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let out = set_key(&src, table, key, value).with_context(|| format!("parsing {}", path.display()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, out).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

fn set_key(src: &str, table: &str, key: &str, value: Option<toml_edit::Value>) -> Result<String> {
    let mut doc: toml_edit::DocumentMut = src.parse()?;
    match value {
        Some(mut v) => {
            if !doc.contains_key(table) {
                doc[table] = toml_edit::table();
            }
            // Keep a trailing `# comment` on the line being replaced.
            if let Some(old) = doc[table].get(key).and_then(toml_edit::Item::as_value) {
                *v.decor_mut() = old.decor().clone();
            }
            doc[table][key] = toml_edit::Item::Value(v);
        }
        None => {
            if let Some(t) = doc.get_mut(table).and_then(toml_edit::Item::as_table_like_mut) {
                t.remove(key);
            }
        }
    }
    Ok(doc.to_string())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn expand_tilde(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), home()) {
        (Some(rest), Some(h)) => h.join(rest),
        _ if p == "~" => home().unwrap_or_else(|| p.into()),
        _ => p.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settings changed from the TUI must not wipe the user's comments or reorder their file.
    #[test]
    fn write_back_keeps_comments() {
        let src = "# mine\n[ui]\nwrap = true # keep\nsidebar = \"tree\"\n\n[run]\n# env\ndefault_env = \"local\"\n";
        let out = set_key(src, "ui", "wrap", Some(false.into())).unwrap();
        assert_eq!(out, src.replace("wrap = true", "wrap = false"));
        let out = set_key(&out, "run", "default_env", None).unwrap();
        assert!(out.contains("# mine") && !out.contains("default_env"), "{out}");
        let out = set_key("", "response", "sections", Some(toml_edit::Array::from_iter(["body"]).into())).unwrap();
        assert_eq!(out, "[response]\nsections = [\"body\"]\n");
        let cfg: Config = toml::from_str(&out).unwrap();
        assert_eq!(cfg.response.sections, Some(vec!["body".into()]));
    }
}
