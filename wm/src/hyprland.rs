//! Hyprland through its IPC sockets: `.socket.sock` for requests, `.socket2.sock` for events.

use std::os::fd::{AsFd, BorrowedFd};

use anyhow::Result;
use hypr::{events::EventStream, HyprEvent, HyprSocket, HyprState};

use crate::{
    Capabilities, Update, WindowManager, WmKind, WmOutput, WmState, WmWindow, WmWorkspace,
};

pub struct Hyprland {
    socket: HyprSocket,
    events: Option<EventStream>,
    hypr: HyprState,
    state: WmState,
}

impl Hyprland {
    /// Connect to the instance named by `HYPRLAND_INSTANCE_SIGNATURE`.
    pub fn connect() -> Option<Self> {
        let socket = HyprSocket::from_env()?;
        let mut hypr = HyprState::default();
        hypr.resync(&socket);
        let events = hypr::instance_dir().and_then(|dir| match EventStream::connect(&dir) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!("Hyprland event socket: {e:#}");
                None
            }
        });
        let mut h = Self {
            socket,
            events,
            hypr,
            state: WmState::default(),
        };
        h.convert();
        Some(h)
    }

    /// The cached Hyprland model (monitor details for the display settings).
    pub fn hypr_state(&self) -> &HyprState {
        &self.hypr
    }

    fn convert(&mut self) {
        self.state = to_wm_state(&self.hypr);
    }
}

/// The generic view of a Hyprland state.
pub fn to_wm_state(h: &HyprState) -> WmState {
    WmState {
        connected: h.connected,
        name: "Hyprland".into(),
        version: h.version.clone(),
        outputs: h
            .monitors
            .iter()
            .map(|m| WmOutput {
                name: m.name.clone(),
                focused: m.focused,
                active_workspace: Some(m.active_workspace),
            })
            .collect(),
        workspaces: h
            .workspaces
            .iter()
            .map(|w| WmWorkspace {
                id: w.id,
                name: w.name.clone(),
                output: w.monitor.clone(),
                windows: w.windows,
            })
            .collect(),
        windows: h
            .clients
            .iter()
            .map(|c| WmWindow {
                id: c.address.clone(),
                class: c.class.clone(),
                title: c.title.clone(),
                workspace: Some(c.workspace_id),
                outputs: Vec::new(),
                floating: c.floating,
                maximized: c.fullscreen == 1,
                fullscreen: c.fullscreen == 2,
                minimized: c.minimized(),
                active: h.active_address.as_deref() == Some(c.address.as_str()),
            })
            .collect(),
        active_window: h.active_window.clone(),
        keyboard_layout: h.keyboard_layout.clone(),
    }
}

impl WindowManager for Hyprland {
    fn kind(&self) -> WmKind {
        WmKind::Hyprland
    }

    fn state(&self) -> &WmState {
        &self.state
    }

    fn event_fd(&self) -> Option<BorrowedFd<'_>> {
        self.events.as_ref().map(|e| e.stream().as_fd())
    }

    fn refresh(&mut self) -> Update {
        let Some(stream) = self.events.as_mut() else {
            return Update::default();
        };
        let mut update = Update::default();
        let mut events = Vec::new();
        match stream.drain(&mut events) {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!("Hyprland event socket closed");
                self.events = None;
                self.hypr.connected = false;
                update.lost = true;
                update.changed = true;
            }
            Err(e) => tracing::warn!("Hyprland events: {e:#}"),
        }
        let mut resync = false;
        for ev in &events {
            if self.hypr.apply(ev) {
                resync = true;
            }
            if let HyprEvent::Bell(_) = ev {
                update.bell = true;
            }
        }
        if resync {
            self.hypr.resync(&self.socket);
        }
        if !events.is_empty() {
            update.changed = true;
        }
        if update.changed {
            self.convert();
        }
        update
    }

    fn focus_window(&mut self, id: &str) -> Result<()> {
        self.socket.focus_window(id)
    }

    fn minimize(&mut self, id: &str) -> Result<()> {
        self.socket.minimize_window(id)
    }

    fn restore(&mut self, id: &str) -> Result<()> {
        match self.hypr.active_workspace_of(None) {
            Some(ws) => self.socket.restore_window(id, ws),
            None => Ok(()),
        }
    }

    fn toggle_maximize(&mut self, id: &str) -> Result<()> {
        self.socket
            .focus_window(id)
            .and_then(|_| self.socket.toggle_maximized(id))
    }

    fn close(&mut self, id: &str) -> Result<()> {
        self.socket.close_window(id)
    }

    fn supports_workspaces(&self) -> bool {
        true
    }

    fn focus_workspace(&mut self, id: i32) -> Result<()> {
        self.socket.focus_workspace(id)
    }

    fn reveal_shell(&mut self) -> Result<bool> {
        if self.socket.active_workspace_windows().unwrap_or(0) > 0 {
            self.socket.focus_empty_workspace()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn exec(&self, cmd: &str) -> Result<bool> {
        self.socket.exec(cmd).map(|_| true)
    }

    fn exit(&self) -> Result<()> {
        self.socket.exit()
    }

    fn export(&self, config: &settings::Config) -> Result<()> {
        settings::hypr_export::export(config, &settings::paths::hypr_config_dir(), "hyprlock")
    }

    fn reload(&self) -> Result<()> {
        self.socket.reload()
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            gaps_inner: true,
            gaps_outer: true,
            border: true,
            layout: true,
            workspaces: true,
            animations: true,
            blur: true,
            rounding: true,
            monitors: true,
            input: true,
        }
    }

    fn hypr_socket(&self) -> Option<&HyprSocket> {
        Some(&self.socket)
    }
}
