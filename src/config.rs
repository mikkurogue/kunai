use std::{
    fs,
    path::PathBuf,
};

use anyhow::Result;
use serde::{
    Deserialize,
    Serialize,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub keyboards: Vec<KeyboardConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyboardConfig {
    pub name:         String,
    pub vendor_id:    String,
    pub product_id:   String,
    pub layout_index: u32,
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = get_config_path()?;
        if !config_path.exists() {
            return Ok(Config { keyboards: vec![] });
        }
        let data = fs::read_to_string(&config_path)?;
        Ok(toml::from_str(&data)?)
    }

    pub fn save(&self) -> Result<()> {
        let config_path = get_config_path()?;
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&config_path, toml::to_string(self)?)?;
        Ok(())
    }
}

fn get_config_path() -> Result<PathBuf> {
    let config_dir =
        dirs::config_dir().ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;
    Ok(config_dir.join("kunai").join("config.toml"))
}
