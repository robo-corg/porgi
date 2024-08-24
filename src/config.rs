use std::path::Path;
use std::path::PathBuf;

use color_eyre::config;
use eyre::anyhow;
use eyre::Result;
use serde::Deserialize;

use crate::project::ProjectOpener;
use crate::tui::ColorConfig;

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub project_dirs: Vec<String>,
    #[serde(default)]
    pub colors: ColorConfig,
    #[serde(default)]
    pub opener: ProjectOpener,
}

fn must_exist<'a>(p: &'a PathBuf) -> Option<&'a PathBuf> {
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

impl Config {
    fn get_paths() -> Vec<std::path::PathBuf> {
        let config_dir = dirs::config_dir().map(|config_dir| config_dir.join("porgi").join("porgi.toml"));
        let config_in_homedir = dirs::home_dir().map(|home_dir| home_dir.join(".config").join("porgi").join("porgi.toml"));

        if config_dir == config_in_homedir {
            config_dir.into_iter().collect()
        } else {
            config_dir.into_iter().chain(
                config_in_homedir.into_iter()
            ).collect()
        }
    }

    pub fn load() -> Result<Self> {
        let paths = Self::get_paths();

        if let Some(config_path) = paths.iter().find_map(must_exist) {
            let config = std::fs::read_to_string(config_path)?;
            Ok(toml::from_str(&config)?)
        } else {
            eprintln!("No config file found. Please create one at in:");
            for path in paths {
                eprintln!("  {}", path.display());
            }
            eprintln!();
            Ok(Self::default())
        }
    }
}
