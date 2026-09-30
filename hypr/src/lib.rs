//! Hyprland integration: command socket, event socket and a cached state model.

pub mod events;
pub mod model;
pub mod socket;

pub use events::HyprEvent;
pub use model::{HyprState, MonitorInfo, WorkspaceInfo};
pub use socket::{lua_string, HyprSocket};

use std::path::PathBuf;

/// Directory holding the sockets of the running Hyprland instance, if any.
pub fn instance_dir() -> Option<PathBuf> {
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let dir = PathBuf::from(runtime).join("hypr").join(signature);
    dir.is_dir().then_some(dir)
}
