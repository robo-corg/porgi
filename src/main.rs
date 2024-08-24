//! Tool to organize code projects
//!
//! Collects status for projects and their git status as well other metadata

mod config;
mod project;
mod tui;

use eyre::{anyhow, Result};
use std::sync::Arc;

use crate::{
    config::Config,
    project::ProjectLoader,
    tui::{init_error_hooks, init_terminal, restore_terminal, App},
};

#[tokio::main]
async fn main() -> Result<()> {
    let config = Arc::new(Config::load()?);

    if config.project_dirs.is_empty() {
        return Err(anyhow!("No project directories configured"));
    }

    let mut project_events = ProjectLoader::new(config.clone())?;

    // projects.sort_by(|a, b| a.name.cmp(&b.name));
    // projects.sort_by(|a, b| b.modified.cmp(&a.modified));

    // setup terminal
    init_error_hooks()?;
    let terminal = init_terminal()?;

    // create app and run it
    App::new(config, project_events).run(terminal).await?;

    restore_terminal()?;

    Ok(())
}
