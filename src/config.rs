use eyre::anyhow;
use eyre::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub project_dirs: Vec<String>,
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
