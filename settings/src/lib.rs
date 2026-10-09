//! eDEX-DE configuration: `~/.config/edex-de/config.toml`, live reload and the window
//! manager export (Hyprland Lua, labwc rc.xml).

pub mod config;
pub mod hypr_export;
pub mod io;
pub mod labwc_export;
pub mod paths;

pub use config::*;
pub use io::{load, save, watch, ConfigWatcher};
pub use paths::{config_dir, config_path, state_dir, system_share_dir, user_theme_dir};
