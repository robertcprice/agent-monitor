//! Configuration management for the daemon.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Main configuration for the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Directory for storing data
    pub data_dir: PathBuf,

    /// Path to SQLite database
    pub db_path: PathBuf,

    /// Path to Unix socket
    pub socket_path: PathBuf,

    /// Path to config directory
    pub config_dir: PathBuf,

    /// Claude Code home directory
    pub claude_home: PathBuf,

    /// Log level
    pub log_level: String,

    /// Poll interval in seconds
    pub poll_interval: u64,

    /// HTTP port for web server
    pub http_port: u16,
}

impl Default for Config {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| home.join(".local/share"))
            .join("agent-monitor");
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| home.join(".config"))
            .join("agent-monitor");

        Self {
            db_path: data_dir.join("sessions.db"),
            socket_path: PathBuf::from("/tmp/agent-monitor.sock"),
            config_dir,
            data_dir,
            claude_home: home.join(".claude"),
            log_level: "info".to_string(),
            poll_interval: 30,
            http_port: 8765,
        }
    }
}

impl Config {
    /// Load configuration from a file.
    pub fn load(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = serde_json::from_str(&content)?;
        Ok(config)
    }

    /// Save configuration to a file.
    pub fn save(&self, path: &str) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Ensure all directories exist.
    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.config_dir)?;
        Ok(())
    }
}
