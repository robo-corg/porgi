//! Tool to organize code projects
//!
//! Collects status for projects and their git status as well other metadata

mod config;
mod project;
mod tui;

use color_eyre::config::HookBuilder;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{prelude::*, style::palette::tailwind, widgets::*};

use eyre::Result;
use std::{io::{self, stdout}, sync::Arc};

use crate::{
    config::Config,
    project::{read_projects, Project}, tui::{init_error_hooks, init_terminal, restore_terminal, App},
};

fn main() -> Result<()> {
    let config = Config::load()?;

    let mut projects = read_projects(&config)?;

    projects.sort_by(|a, b| a.name.cmp(&b.name));
    projects.sort_by(|a, b| b.modified.cmp(&a.modified));

    // setup terminal
    init_error_hooks()?;
    let terminal = init_terminal()?;

    // create app and run it
    App::new(Arc::new(config), projects).run(terminal)?;

    restore_terminal()?;

    Ok(())
}
