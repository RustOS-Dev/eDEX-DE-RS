//! eDEX-DE configuration: `~/.config/edex-de/config.toml` and live reload. The shell and
//! edex-comp both read it.

pub mod config;
pub mod io;
pub mod paths;

pub use config::*;
pub use io::{load, save, watch, ConfigWatcher};
pub use paths::{
    config_dir, config_path, first_existing, state_dir, system_share_dir, user_theme_dir,
};
