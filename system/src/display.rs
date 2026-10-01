//! Outputs and night light. edex-comp reads the display settings from `config.toml`; the
//! settings panel saves them and asks it to reload.

use anyhow::{anyhow, Result};
use comp::{CompSocket, MonitorInfo};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayState {
    pub monitors: Vec<MonitorInfo>,
    pub night_light: bool,
    pub night_temp: u32,
}

pub fn query(socket: Option<&CompSocket>, night_light: bool, night_temp: u32) -> DisplayState {
    DisplayState {
        monitors: socket
            .and_then(|s| s.snapshot().ok())
            .map(|s| s.monitors)
            .unwrap_or_default(),
        night_light,
        night_temp,
    }
}

/// Make edex-comp apply the saved display, input and night-light settings now.
pub fn reload(socket: Option<&CompSocket>) -> Result<()> {
    socket
        .ok_or_else(|| anyhow!("edex-comp is not running"))?
        .reload()
}
