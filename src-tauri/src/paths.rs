//! Where the app keeps its files.
//!
//!   %APPDATA%\game-overlai\config.toml          settings (roams with the user profile)
//!   %LOCALAPPDATA%\game-overlai\logs\           daily log files (last 7 kept)
//!   %LOCALAPPDATA%\game-overlai\agents\         installed agent adapters (npm packages)
//!   %LOCALAPPDATA%\game-overlai\profiles\       agent working directories (per game later)

use std::path::PathBuf;

pub const APP_DIR: &str = "game-overlai";

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_file: PathBuf,
    pub log_dir: PathBuf,
    pub agents_dir: PathBuf,
    pub profiles_dir: PathBuf,
    pub history_dir: PathBuf,
}

impl Paths {
    pub fn resolve() -> Self {
        let roaming = env_dir("APPDATA").join(APP_DIR);
        let local = env_dir("LOCALAPPDATA").join(APP_DIR);
        Self {
            config_file: roaming.join("config.toml"),
            log_dir: local.join("logs"),
            agents_dir: local.join("agents"),
            profiles_dir: local.join("profiles"),
            history_dir: local,
        }
    }
}

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}
