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

    pub fn workspace(&self) -> Option<PathBuf> {
        self.workspace.as_deref().map(expand_tilde)
    }

    /// `-e` / `$HURL_ENV` (already folded into `cli`), then config, then `local`.
    pub fn env(&self, cli: Option<String>) -> String {
        cli.or_else(|| self.run.default_env.clone()).unwrap_or_else(|| "local".into())
    }
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
