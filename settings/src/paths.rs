//! XDG paths used by the shell.

use std::path::PathBuf;

fn home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

pub fn config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join(".config"))
        .join("edex-de")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn state_dir() -> PathBuf {
    std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join(".local/state"))
        .join("edex-de")
}

pub fn user_theme_dir() -> PathBuf {
    config_dir().join("themes")
}

/// System data directory (`/usr/share/edex-de`), overridable for development.
pub fn system_share_dir() -> PathBuf {
    std::env::var("EDEX_SHARE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/share/edex-de"))
}

pub fn hypr_config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join(".config"))
        .join("hypr")
}
