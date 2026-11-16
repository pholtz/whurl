//! UI state kept per workspace in `.whurl/state.toml`: open folders and the last selection.
//! Best effort: a missing or unreadable file means a fresh start, and save errors are ignored.

use crate::workspace::Workspace;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct UiState {
    pub expanded: Vec<String>,
    pub selected: Option<String>,
}

fn path(ws: &Workspace) -> PathBuf {
    ws.state_dir().join("state.toml")
}

impl UiState {
    pub fn load(ws: &Workspace) -> Self {
        std::fs::read_to_string(path(ws)).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, ws: &Workspace) {
        if let Ok(s) = toml::to_string(self) {
            let _ = std::fs::create_dir_all(ws.state_dir());
            let _ = std::fs::write(path(ws), s);
        }
    }
}
