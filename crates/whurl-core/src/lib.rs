//! whurl's frontend-independent core: workspace discovery, running hurl, run history, config,
//! exports, and the request and sidebar models the TUI (and any other frontend) build on.

pub mod body;
pub mod config;
pub mod export;
pub mod jobs;
pub mod requests;
pub mod run;
pub mod settings;
pub mod state;
pub mod store;
pub mod tree;
pub mod workspace;
