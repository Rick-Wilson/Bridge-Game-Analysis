use crate::error::{AnalysisError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub masterpoints_url: Option<String>,
}

impl Config {
    /// Load config from the default location (~/.config/bridge-analysis/config.toml)
    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            let contents = std::fs::read_to_string(&path)?;
            toml::from_str(&contents)
                .map_err(|e| AnalysisError::ConfigError(format!("Failed to parse config: {}", e)))
        } else {
            Ok(Self::default())
        }
    }

    /// Get the default config file path
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("bridge-analysis")
            .join("config.toml")
    }

    /// Save config to the default location
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self).map_err(|e| {
            AnalysisError::ConfigError(format!("Failed to serialize config: {}", e))
        })?;
        std::fs::write(&path, contents)?;
        Ok(())
    }

    /// Get the masterpoints URL, with optional override
    pub fn masterpoints_url<'a>(&'a self, override_url: Option<&'a str>) -> Option<&'a str> {
        override_url.or(self.masterpoints_url.as_deref())
    }
}
