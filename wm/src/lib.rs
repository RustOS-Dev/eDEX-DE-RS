//! Window-manager backends. The shell draws the panels on any wlr-layer-shell compositor; the
//! window manager behind it provides the centre tab strip (windows, minimize, maximize, close,
//! activate), workspaces, launching on the current workspace, logging out and the exported
//! configuration.
//!
//! * [`hyprland::Hyprland`] — Hyprland's IPC sockets (eDEX-OS).
//! * [`labwc::Labwc`] — `zwlr_foreign_toplevel_management_v1` (labwc, also sway/wayfire) with
//!   labwc's `rc.xml` export and reconfigure/exit through `LABWC_PID`.
//! * [`NoWm`] — panels only.

pub mod hyprland;
pub mod labwc;
pub mod model;

use std::os::fd::BorrowedFd;

use anyhow::Result;

pub use model::{WmOutput, WmState, WmWindow, WmWorkspace};

/// Which backend drives the windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmKind {
    Hyprland,
    Labwc,
    None,
}

impl WmKind {
    pub fn name(self) -> &'static str {
        match self {
            WmKind::Hyprland => "hyprland",
            WmKind::Labwc => "labwc",
            WmKind::None => "none",
        }
    }
}

/// `--wm` on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WmChoice {
    /// Hyprland when its instance signature is set, else foreign-toplevel when the compositor
    /// offers it, else none.
    #[default]
    Auto,
    Hyprland,
    Labwc,
    None,
}

impl std::str::FromStr for WmChoice {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(WmChoice::Auto),
            "hyprland" | "hypr" => Ok(WmChoice::Hyprland),
            "labwc" | "wlr" | "foreign-toplevel" => Ok(WmChoice::Labwc),
            "none" => Ok(WmChoice::None),
            other => Err(format!(
                "unknown window manager `{other}` (auto|hyprland|labwc|none)"
            )),
        }
    }
}

/// What a backend reports after processing events.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Update {
    /// The state changed: rebuild the tab strip / workspaces.
    pub changed: bool,
    /// An urgent-window bell (Hyprland).
    pub bell: bool,
    /// The connection to the window manager is gone.
    pub lost: bool,
}

/// Window actions behind the tab strip controls, key bindings and IPC.
pub trait WindowManager {
    fn kind(&self) -> WmKind;

    /// The current view of outputs, workspaces and windows.
    fn state(&self) -> &WmState;

    /// A file descriptor that becomes readable when events are waiting (Hyprland's event
    /// socket); the shell calls [`WindowManager::refresh`] then. Backends on the Wayland
    /// connection are refreshed on `PlatformEvent::ToplevelsChanged` instead.
    fn event_fd(&self) -> Option<BorrowedFd<'_>> {
        None
    }

    /// Process pending events.
    fn refresh(&mut self) -> Update;

    /// The shell's outputs (names, primary first), for backends that cannot tell which output
    /// is focused.
    fn set_outputs(&mut self, _outputs: &[String]) {}

    fn focus_window(&mut self, id: &str) -> Result<()>;
    fn minimize(&mut self, id: &str) -> Result<()>;
    /// Bring a minimized window back (onto the current workspace) and focus it.
    fn restore(&mut self, id: &str) -> Result<()>;
    /// eDEX's maximize: the window gets the full width while the side panels step aside.
    fn toggle_maximize(&mut self, id: &str) -> Result<()>;
    fn close(&mut self, id: &str) -> Result<()>;

    fn supports_workspaces(&self) -> bool {
        false
    }
    fn focus_workspace(&mut self, _id: i32) -> Result<()> {
        Ok(())
    }

    /// Make the shell's own terminal visible before it takes the keyboard (Hyprland switches to
    /// an empty workspace when windows cover it). Returns whether it did anything.
    fn reveal_shell(&mut self) -> Result<bool> {
        Ok(false)
    }

    /// Start a shell command through the window manager (so it opens on the current workspace).
    /// `Ok(false)`: not supported, the caller spawns it itself.
    fn exec(&self, _cmd: &str) -> Result<bool> {
        Ok(false)
    }

    /// End the session (log out).
    fn exit(&self) -> Result<()>;

    /// Write the window manager's configuration generated from the eDEX settings.
    fn export(&self, _config: &settings::Config) -> Result<()> {
        Ok(())
    }

    /// Make the window manager re-read its configuration.
    fn reload(&self) -> Result<()> {
        Ok(())
    }

    /// Settings → Window manager rows this backend applies.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    /// The Hyprland command socket, for the system backends that configure monitors and input
    /// through it.
    fn hypr_socket(&self) -> Option<&hypr::HyprSocket> {
        None
    }
}

/// Which window-manager settings mean something on this backend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub gaps_inner: bool,
    pub gaps_outer: bool,
    pub border: bool,
    pub layout: bool,
    pub workspaces: bool,
    pub animations: bool,
    pub blur: bool,
    pub rounding: bool,
    /// Monitor modes and positions can be applied at run time.
    pub monitors: bool,
    /// Keyboard layout, repeat and pointer settings apply at run time.
    pub input: bool,
}

/// Panels only: no window list, logging out quits the shell's parent session by signal.
pub struct NoWm {
    state: WmState,
}

impl NoWm {
    pub fn new() -> Self {
        Self {
            state: WmState::default(),
        }
    }
}

impl Default for NoWm {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowManager for NoWm {
    fn kind(&self) -> WmKind {
        WmKind::None
    }
    fn state(&self) -> &WmState {
        &self.state
    }
    fn refresh(&mut self) -> Update {
        Update::default()
    }
    fn focus_window(&mut self, _: &str) -> Result<()> {
        Ok(())
    }
    fn minimize(&mut self, _: &str) -> Result<()> {
        Ok(())
    }
    fn restore(&mut self, _: &str) -> Result<()> {
        Ok(())
    }
    fn toggle_maximize(&mut self, _: &str) -> Result<()> {
        Ok(())
    }
    fn close(&mut self, _: &str) -> Result<()> {
        Ok(())
    }
    fn exit(&self) -> Result<()> {
        Err(anyhow::anyhow!("no window manager to log out of"))
    }
}

/// Pick and connect the backend.
pub fn connect<E: 'static>(
    choice: WmChoice,
    platform: &platform::Platform<E>,
) -> Box<dyn WindowManager> {
    let hypr_env = hypr::HyprSocket::from_env().is_some();
    let toplevels = platform.toplevels();
    let kind = match choice {
        WmChoice::Hyprland => WmKind::Hyprland,
        WmChoice::Labwc => WmKind::Labwc,
        WmChoice::None => WmKind::None,
        WmChoice::Auto if hypr_env => WmKind::Hyprland,
        WmChoice::Auto if toplevels.is_some() => WmKind::Labwc,
        WmChoice::Auto => WmKind::None,
    };
    match kind {
        WmKind::Hyprland => match hyprland::Hyprland::connect() {
            Some(h) => {
                tracing::info!(version = %h.state().version, "window manager: Hyprland");
                Box::new(h)
            }
            None => {
                tracing::warn!("Hyprland requested but its sockets are not available");
                Box::new(NoWm::new())
            }
        },
        WmKind::Labwc => match toplevels {
            Some(t) => {
                let l = labwc::Labwc::new(t);
                tracing::info!(labwc = l.is_labwc(), "window manager: foreign-toplevel");
                Box::new(l)
            }
            None => {
                tracing::warn!(
                    "the compositor offers no zwlr_foreign_toplevel_manager_v1: no window tabs"
                );
                Box::new(NoWm::new())
            }
        },
        WmKind::None => {
            tracing::info!("window manager: none");
            Box::new(NoWm::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_parse() {
        assert_eq!("labwc".parse::<WmChoice>(), Ok(WmChoice::Labwc));
        assert_eq!("Hyprland".parse::<WmChoice>(), Ok(WmChoice::Hyprland));
        assert_eq!("none".parse::<WmChoice>(), Ok(WmChoice::None));
        assert!("kwin".parse::<WmChoice>().is_err());
    }
}
