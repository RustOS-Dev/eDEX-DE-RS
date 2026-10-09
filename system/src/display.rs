//! Monitors via the Hyprland socket; night light via hyprsunset.

use anyhow::Result;
use hypr::{HyprSocket, MonitorInfo};

use crate::runner::CommandRunner;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayState {
    pub monitors: Vec<MonitorInfo>,
    pub night_light: bool,
    pub night_temp: u32,
}

pub fn query(socket: Option<&HyprSocket>, night_light: bool, night_temp: u32) -> DisplayState {
    DisplayState {
        monitors: socket.and_then(|s| s.monitors().ok()).unwrap_or_default(),
        night_light,
        night_temp,
    }
}

/// Apply a monitor rule immediately (persisted separately through generated.lua).
pub fn apply_monitor(
    socket: &HyprSocket,
    name: &str,
    mode: &str,
    position: &str,
    scale: f32,
    transform: u32,
    disabled: bool,
) -> Result<()> {
    let lua = if disabled {
        format!("hl.monitor({{ output = \"{name}\", disabled = true }})")
    } else {
        format!("hl.monitor({{ output = \"{name}\", mode = \"{mode}\", position = \"{position}\", scale = {scale}, transform = {transform} }})")
    };
    socket.eval(&lua).map(|_| ())
}

pub fn set_night_light(r: &dyn CommandRunner, on: bool, temp: u32) -> Result<()> {
    if on {
        // hyprsunset runs as a user service; if absent, launch it detached.
        if r.run("hyprctl", &["hyprsunset", "temperature", &temp.to_string()])
            .map(|o| o.ok())
            .unwrap_or(false)
        {
            return Ok(());
        }
        launcher::runner::spawn_detached(&format!("hyprsunset -t {temp}"))?;
        Ok(())
    } else {
        if r.run("hyprctl", &["hyprsunset", "identity"])
            .map(|o| o.ok())
            .unwrap_or(false)
        {
            return Ok(());
        }
        let _ = r.run("pkill", &["-x", "hyprsunset"]);
        Ok(())
    }
}
