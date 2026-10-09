//! Load, save and watch the configuration file.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

use anyhow::{Context, Result};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use tracing::{info, warn};

use crate::config::Config;

/// Load the config, creating it with defaults on first launch. Invalid files are kept
/// (renamed with `.broken`) and defaults are used so the shell always starts.
pub fn load(path: &Path) -> Config {
    match fs::read_to_string(path) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(mut c) => {
                c.sanitize();
                c
            }
            Err(e) => {
                warn!("config {} is invalid ({e}); using defaults", path.display());
                let broken = path.with_extension("toml.broken");
                let _ = fs::copy(path, &broken);
                Config::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let c = system_default();
            match save(path, &c) {
                Ok(()) => info!("wrote default config to {}", path.display()),
                Err(e) => warn!("could not write default config: {e:#}"),
            }
            c
        }
        Err(e) => {
            warn!("cannot read {}: {e}; using defaults", path.display());
            Config::default()
        }
    }
}

/// Defaults for a first start: the distribution's `config.toml` in the system share directory
/// (e.g. RustOS sets foot as the terminal), or the built-in defaults.
pub fn system_default() -> Config {
    let path = crate::paths::system_share_dir().join("config.toml");
    match fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(mut c) => {
                c.sanitize();
                info!("first start: defaults from {}", path.display());
                c
            }
            Err(e) => {
                warn!(
                    "{} is invalid ({e}); using built-in defaults",
                    path.display()
                );
                Config::default()
            }
        },
        Err(_) => Config::default(),
    }
}

/// Atomically write the config (temp file + rename).
pub fn save(path: &Path, config: &Config) -> Result<()> {
    let dir = path.parent().context("config path has no parent")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let text = format!(
        "# eDEX-DE configuration — edited by the Settings panel; hand edits are fine too.\n{}",
        toml::to_string_pretty(config)?
    );
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

/// Watches the config file's directory and reports changes.
pub struct ConfigWatcher {
    _watcher: notify::RecommendedWatcher,
    rx: mpsc::Receiver<()>,
    path: PathBuf,
}

impl ConfigWatcher {
    /// Non-blocking check for a change since the last call.
    pub fn changed(&self) -> bool {
        let mut changed = false;
        while self.rx.try_recv().is_ok() {
            changed = true;
        }
        changed
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Start watching `path` (its parent directory, so editors that replace the file are seen).
pub fn watch(path: &Path) -> Result<ConfigWatcher> {
    let (tx, rx) = mpsc::channel();
    let target = path.file_name().map(|n| n.to_os_string());
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            if matches!(
                event.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
            ) {
                let relevant = event.paths.iter().any(|p| {
                    p.file_name()
                        .map(|n| Some(n.to_os_string()) == target)
                        .unwrap_or(false)
                });
                if relevant {
                    let _ = tx.send(());
                }
            }
        }
    })?;
    let dir = path.parent().context("config path has no parent")?;
    fs::create_dir_all(dir)?;
    watcher.watch(dir, RecursiveMode::NonRecursive)?;
    Ok(ConfigWatcher {
        _watcher: watcher,
        rx,
        path: path.to_path_buf(),
    })
}

/// Debounce helper for callers polling `ConfigWatcher::changed`.
pub const RELOAD_DEBOUNCE: Duration = Duration::from_millis(250);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_defaults_and_reloads_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let c = load(&path);
        assert!(path.exists());
        assert_eq!(c, Config::default());
        fs::write(&path, "[appearance]\ntheme = \"amber\"\n").unwrap();
        assert_eq!(load(&path).appearance.theme, "amber");
        fs::write(&path, "this is not toml [[[").unwrap();
        assert_eq!(load(&path), Config::default());
        assert!(path.with_extension("toml.broken").exists());
    }
}
