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

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = dirs::config_dir()
            .ok_or_else(|| anyhow!("Could not find config directory"))?
            .join("porgi")
            .join("porgi.toml");

        if config_path.exists() {
            let config = std::fs::read_to_string(config_path)?;
            Ok(toml::from_str(&config)?)
        } else {
            Ok(Self::default())
        }
    }
}
