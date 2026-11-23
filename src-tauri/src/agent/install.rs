//! Installs an agent's ACP adapter from npm into our own folder (pinned version), so it does
//! not depend on what the user has installed globally. Requires Node.js + npm on PATH.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tracing::{info, warn};

use super::spec::AgentSpec;
use crate::proxy::ProxyEnv;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn install_dir(agents_dir: &Path, spec: &AgentSpec) -> PathBuf {
    agents_dir.join(format!("{}-{}", spec.id, spec.npm_version))
}

pub fn entry_path(agents_dir: &Path, spec: &AgentSpec) -> PathBuf {
    let mut p = install_dir(agents_dir, spec).join("node_modules");
    for segment in spec.npm_package.split('/') {
        p.push(segment);
    }
    p.join(spec.entry)
}

pub fn is_installed(agents_dir: &Path, spec: &AgentSpec) -> bool {
    entry_path(agents_dir, spec).is_file()
}

fn command(program: &str) -> Command {
    let mut c = Command::new(program);
    c.creation_flags(CREATE_NO_WINDOW).stdin(Stdio::null());
    c
}

/// Blocking. Returns the script to run with `node`, or a message for the user.
pub fn ensure_installed(agents_dir: &Path, spec: &AgentSpec, proxy: &ProxyEnv) -> Result<PathBuf, String> {
    let entry = entry_path(agents_dir, spec);
    if entry.is_file() {
        return Ok(entry);
    }
    if command("node").arg("--version").output().map(|o| !o.status.success()).unwrap_or(true) {
        return Err("Не найден Node.js. Установите Node.js LTS (nodejs.org) и перезапустите приложение.".into());
    }
    if command("npm.cmd").arg("--version").output().map(|o| !o.status.success()).unwrap_or(true) {
        return Err("Не найден npm (идёт вместе с Node.js). Переустановите Node.js и перезапустите приложение.".into());
    }

    let dir = install_dir(agents_dir, spec);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Не удалось создать {}: {e}", dir.display()))?;
    let _ = std::fs::write(dir.join("package.json"), "{ \"private\": true }\n");
    let package = format!("{}@{}", spec.npm_package, spec.npm_version);
    info!("installing {package} into {}", dir.display());
    let out = command("npm.cmd")
        .args(["install", "--no-audit", "--no-fund", "--loglevel=error", "--save-exact", &package])
        .envs(proxy.vars())
        .current_dir(&dir)
        .output()
        .map_err(|e| format!("Не удалось запустить npm: {e}"))?;
    if !out.status.success() || !entry.is_file() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(4).collect();
        warn!("npm install failed: {stderr}");
        return Err(format!(
            "Не удалось установить адаптер {}: {}",
            spec.name,
            tail.into_iter().rev().collect::<Vec<_>>().join(" ")
        ));
    }
    info!("{package} installed");
    Ok(entry)
}
