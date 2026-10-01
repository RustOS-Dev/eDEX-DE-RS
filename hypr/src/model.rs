//! Cached view of Hyprland's monitors, workspaces and focus, kept up to date by events.

use crate::{events::HyprEvent, socket::HyprSocket};

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    pub id: i32,
    pub name: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub refresh_rate: f32,
    pub x: i32,
    pub y: i32,
    pub scale: f32,
    pub focused: bool,
    pub active_workspace: i32,
    pub active_workspace_name: String,
    pub transform: i32,
    pub dpms: bool,
    pub disabled: bool,
    pub available_modes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceInfo {
    pub id: i32,
    pub name: String,
    pub monitor: String,
    pub windows: u32,
    pub has_fullscreen: bool,
    pub last_window_title: String,
}

/// A mapped application window.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientInfo {
    pub address: String,
    pub class: String,
    pub title: String,
    pub workspace_id: i32,
    pub workspace: String,
    pub floating: bool,
    /// 0 none, 1 maximized, 2 fullscreen (Hyprland's internal state).
    pub fullscreen: i32,
}

impl ClientInfo {
    pub fn minimized(&self) -> bool {
        self.workspace == crate::socket::MINIMIZED_WORKSPACE
    }
}

/// Addresses in events lack the `0x` prefix that `clients` prints.
fn same_address(a: &str, b: &str) -> bool {
    a.trim_start_matches("0x") == b.trim_start_matches("0x")
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HyprState {
    pub connected: bool,
    pub version: String,
    pub monitors: Vec<MonitorInfo>,
    pub workspaces: Vec<WorkspaceInfo>,
    pub active_window: Option<(String, String)>,
    pub keyboard_layout: String,
    pub submap: String,
    pub screencast_active: bool,
    pub urgent_windows: Vec<String>,
    pub clients: Vec<ClientInfo>,
    /// Address of the focused window.
    pub active_address: Option<String>,
}

impl HyprState {
    pub fn unavailable() -> Self {
        Self::default()
    }

    /// Full resync from the command socket.
    pub fn resync(&mut self, socket: &HyprSocket) {
        match socket.monitors() {
            Ok(m) => {
                self.monitors = m;
                self.connected = true;
            }
            Err(e) => {
                tracing::warn!("hyprland monitors query failed: {e:#}");
                self.connected = false;
                return;
            }
        }
        match socket.workspaces() {
            Ok(w) => self.workspaces = w,
            Err(e) => tracing::warn!("hyprland workspaces query failed: {e:#}"),
        }
        match socket.active_window() {
            Ok(Some(w)) => {
                self.active_address = (!w.address.is_empty()).then_some(w.address);
                self.active_window = Some((w.class, w.title));
            }
            Ok(None) => {
                self.active_window = None;
                self.active_address = None;
            }
            Err(e) => tracing::debug!("activewindow query failed: {e:#}"),
        }
        match socket.clients() {
            Ok(c) => self.clients = c,
            Err(e) => tracing::warn!("hyprland clients query failed: {e:#}"),
        }
        if let Ok(Some(layout)) = socket.active_layout() {
            self.keyboard_layout = layout;
        }
        if self.version.is_empty() {
            self.version = socket.version().unwrap_or_default();
        }
        self.normalize();
    }

    /// Apply an event; returns true when a full resync is advisable.
    pub fn apply(&mut self, event: &HyprEvent) -> bool {
        match event {
            HyprEvent::Workspace { id, name } => {
                let id = id.or_else(|| {
                    self.workspaces
                        .iter()
                        .find(|w| &w.name == name)
                        .map(|w| w.id)
                });
                if let Some(id) = id {
                    if let Some(mon) = self.monitors.iter_mut().find(|m| m.focused) {
                        mon.active_workspace = id;
                        mon.active_workspace_name = name.clone();
                    }
                }
                false
            }
            HyprEvent::FocusedMonitor { monitor, workspace } => {
                for m in &mut self.monitors {
                    m.focused = &m.name == monitor;
                    if m.focused {
                        m.active_workspace_name = workspace.clone();
                        if let Some(ws) = self.workspaces.iter().find(|w| &w.name == workspace) {
                            m.active_workspace = ws.id;
                        }
                    }
                }
                false
            }
            HyprEvent::ActiveWindow { class, title } => {
                self.active_window = if class.is_empty() && title.is_empty() {
                    None
                } else {
                    Some((class.clone(), title.clone()))
                };
                false
            }
            HyprEvent::ActiveWindowAddress(addr) => {
                self.active_address = if addr.is_empty() {
                    None
                } else if let Some(c) = self.clients.iter().find(|c| same_address(&c.address, addr))
                {
                    Some(c.address.clone())
                } else {
                    // A window we have not seen yet: resync picks it up.
                    return true;
                };
                false
            }
            HyprEvent::CreateWorkspace { id, name } => {
                if !self.workspaces.iter().any(|w| &w.name == name) {
                    let monitor = self
                        .monitors
                        .iter()
                        .find(|m| m.focused)
                        .map(|m| m.name.clone())
                        .unwrap_or_default();
                    self.workspaces.push(WorkspaceInfo {
                        id: id.unwrap_or_else(|| name.parse().unwrap_or(0)),
                        name: name.clone(),
                        monitor,
                        windows: 0,
                        has_fullscreen: false,
                        last_window_title: String::new(),
                    });
                    self.normalize();
                }
                false
            }
            HyprEvent::DestroyWorkspace { name, .. } => {
                self.workspaces.retain(|w| &w.name != name);
                false
            }
            HyprEvent::RenameWorkspace { id, name } => {
                if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == *id) {
                    w.name = name.clone();
                }
                false
            }
            HyprEvent::MoveWorkspace { name, monitor } => {
                if let Some(w) = self.workspaces.iter_mut().find(|w| &w.name == name) {
                    w.monitor = monitor.clone();
                }
                false
            }
            HyprEvent::OpenWindow { workspace, .. } => {
                if let Some(w) = self.workspaces.iter_mut().find(|w| &w.name == workspace) {
                    w.windows += 1;
                }
                // The window list needs the full client record.
                true
            }
            HyprEvent::CloseWindow(_)
            | HyprEvent::MoveWindow { .. }
            | HyprEvent::FloatingMode { .. } => true,
            HyprEvent::ActiveLayout { layout, .. } => {
                self.keyboard_layout = layout.clone();
                false
            }
            HyprEvent::Submap(s) => {
                self.submap = s.clone();
                false
            }
            HyprEvent::Urgent(addr) => {
                if !self.urgent_windows.contains(addr) {
                    self.urgent_windows.push(addr.clone());
                }
                false
            }
            HyprEvent::Screencast { active, .. } => {
                self.screencast_active = *active;
                false
            }
            HyprEvent::MonitorAdded(_)
            | HyprEvent::MonitorRemoved(_)
            | HyprEvent::ConfigReloaded
            | HyprEvent::Fullscreen(_) => true,
            HyprEvent::WindowTitle { address, title } => match title {
                Some(t) => {
                    if let Some(c) = self
                        .clients
                        .iter_mut()
                        .find(|c| same_address(&c.address, address))
                    {
                        c.title = t.clone();
                    }
                    false
                }
                None => true,
            },
            HyprEvent::OpenLayer(_)
            | HyprEvent::CloseLayer(_)
            | HyprEvent::Bell(_)
            | HyprEvent::Other { .. } => false,
        }
    }

    fn normalize(&mut self) {
        self.workspaces.sort_by_key(|w| w.id);
    }

    /// Workspaces shown on the given monitor (or all when `None`), with active flags.
    pub fn workspace_strip(&self, monitor: Option<&str>) -> Vec<ui::state::WorkspaceInfo> {
        let active_ids: Vec<i32> = self
            .monitors
            .iter()
            .filter(|m| monitor.is_none_or(|n| m.name == n))
            .map(|m| m.active_workspace)
            .collect();
        let mut strip: Vec<ui::state::WorkspaceInfo> = self
            .workspaces
            .iter()
            .filter(|w| w.id > 0)
            .filter(|w| monitor.is_none_or(|n| w.monitor == n))
            .map(|w| ui::state::WorkspaceInfo {
                id: w.id,
                name: w.name.clone(),
                active: active_ids.contains(&w.id),
                windows: w.windows,
            })
            .collect();
        // Always show 1..=5 so the strip is stable.
        for id in 1..=5 {
            if !strip.iter().any(|w| w.id == id) {
                strip.push(ui::state::WorkspaceInfo {
                    id,
                    name: id.to_string(),
                    active: active_ids.contains(&id),
                    windows: 0,
                });
            }
        }
        strip.sort_by_key(|w| w.id);
        strip
    }

    /// Active workspace of the named monitor, or of the focused one when `None`.
    pub fn active_workspace_of(&self, monitor: Option<&str>) -> Option<i32> {
        self.monitors
            .iter()
            .find(|m| monitor.map_or(m.focused, |n| m.name == n))
            .or(self.monitors.first())
            .map(|m| m.active_workspace)
    }

    /// Tabs for the windows on the focused monitor's workspace, then minimized windows.
    pub fn window_tabs(&self) -> Vec<ui::state::WindowTab> {
        let ws = self.active_workspace_of(None);
        let tab = |c: &ClientInfo| ui::state::WindowTab {
            address: c.address.clone(),
            class: c.class.clone(),
            title: c.title.clone(),
            active: self.active_address.as_deref() == Some(c.address.as_str()) && !c.minimized(),
            minimized: c.minimized(),
            maximized: c.fullscreen == 1,
            floating: c.floating,
        };
        let mut tabs: Vec<_> = self
            .clients
            .iter()
            .filter(|c| Some(c.workspace_id) == ws && !c.minimized())
            .map(tab)
            .collect();
        // Hyprland does not always report focus (e.g. a window opened while the shell held the
        // keyboard): a lone window is the one the controls act on.
        if !tabs.iter().any(|t| t.active) && tabs.len() == 1 {
            tabs[0].active = true;
        }
        tabs.extend(self.clients.iter().filter(|c| c.minimized()).map(tab));
        tabs
    }

    /// A window on this monitor's visible workspace is maximized: the shell hides its side
    /// panels so it gets the full width (the top bar and the window controls stay).
    pub fn maximized_on(&self, monitor: &str) -> bool {
        let ws = self.active_workspace_of(Some(monitor));
        self.clients
            .iter()
            .any(|c| Some(c.workspace_id) == ws && c.fullscreen == 1)
    }

    /// Tiled windows cover the terminal on this monitor's visible workspace.
    pub fn tiled_on(&self, monitor: &str) -> bool {
        let ws = self.active_workspace_of(Some(monitor));
        self.clients
            .iter()
            .any(|c| Some(c.workspace_id) == ws && !c.floating)
    }

    /// Short summary of the active keyboard layout (e.g. "English (US)" → "US").
    pub fn short_layout(&self) -> String {
        let l = self.keyboard_layout.trim();
        if l.is_empty() {
            return "us".into();
        }
        if let Some(start) = l.find('(') {
            if let Some(end) = l[start..].find(')') {
                return l[start + 1..start + end].to_string();
            }
        }
        l.split_whitespace()
            .next()
            .unwrap_or(l)
            .chars()
            .take(3)
            .collect::<String>()
            .to_ascii_uppercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_update_state() {
        let mut st = HyprState {
            connected: true,
            ..Default::default()
        };
        st.monitors.push(MonitorInfo {
            id: 0,
            name: "DP-1".into(),
            description: String::new(),
            width: 1920,
            height: 1080,
            refresh_rate: 60.0,
            x: 0,
            y: 0,
            scale: 1.0,
            focused: true,
            active_workspace: 1,
            active_workspace_name: "1".into(),
            transform: 0,
            dpms: true,
            disabled: false,
            available_modes: vec![],
        });
        assert!(!st.apply(&HyprEvent::CreateWorkspace {
            id: Some(2),
            name: "2".into()
        }));
        assert!(!st.apply(&HyprEvent::Workspace {
            id: Some(2),
            name: "2".into()
        }));
        assert_eq!(st.monitors[0].active_workspace, 2);
        // New windows need the full client record: resync.
        assert!(st.apply(&HyprEvent::OpenWindow {
            address: "a".into(),
            workspace: "2".into(),
            class: "kitty".into(),
            title: "t".into()
        }));
        assert_eq!(st.workspaces[0].windows, 1);
        let strip = st.workspace_strip(Some("DP-1"));
        assert_eq!(strip.len(), 5);
        assert!(strip.iter().find(|w| w.id == 2).unwrap().active);
        st.apply(&HyprEvent::ActiveLayout {
            keyboard: "k".into(),
            layout: "English (US)".into(),
        });
        assert_eq!(st.short_layout(), "US");
        assert!(st.apply(&HyprEvent::ConfigReloaded));
    }

    fn client(addr: &str, ws: i32, name: &str, fullscreen: i32) -> ClientInfo {
        ClientInfo {
            address: addr.into(),
            class: "app".into(),
            title: format!("title {addr}"),
            workspace_id: ws,
            workspace: name.into(),
            floating: false,
            fullscreen,
        }
    }

    #[test]
    fn window_tabs_list_the_workspace_then_minimized() {
        let mut st = HyprState {
            connected: true,
            ..Default::default()
        };
        st.monitors.push(MonitorInfo {
            id: 0,
            name: "DP-1".into(),
            description: String::new(),
            width: 1920,
            height: 1080,
            refresh_rate: 60.0,
            x: 0,
            y: 0,
            scale: 1.0,
            focused: true,
            active_workspace: 1,
            active_workspace_name: "1".into(),
            transform: 0,
            dpms: true,
            disabled: false,
            available_modes: vec![],
        });
        st.clients = vec![
            client("0xa", 1, "1", 0),
            client("0xb", -98, crate::socket::MINIMIZED_WORKSPACE, 0),
            client("0xc", 2, "2", 0),
            client("0xd", 1, "1", 0),
        ];
        assert!(!st.apply(&HyprEvent::ActiveWindowAddress("d".into())));
        let tabs = st.window_tabs();
        let addrs: Vec<_> = tabs.iter().map(|t| t.address.as_str()).collect();
        assert_eq!(addrs, ["0xa", "0xd", "0xb"]);
        assert!(tabs[1].active && !tabs[0].active);
        assert!(tabs[2].minimized);
        assert!(st.tiled_on("DP-1"));
        assert!(!st.maximized_on("DP-1"));
        st.clients[3].fullscreen = 1;
        assert!(st.maximized_on("DP-1"));
        // Title updates arrive with addresses lacking 0x.
        assert!(!st.apply(&HyprEvent::WindowTitle {
            address: "a".into(),
            title: Some("new".into())
        }));
        assert_eq!(st.clients[0].title, "new");
        // An unknown focused window asks for a resync.
        assert!(st.apply(&HyprEvent::ActiveWindowAddress("ff".into())));
    }
}
