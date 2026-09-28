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
            Ok(Some(w)) => self.active_window = Some((w.class, w.title)),
            Ok(None) => self.active_window = None,
            Err(e) => tracing::debug!("activewindow query failed: {e:#}"),
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
                    false
                } else {
                    true
                }
            }
            HyprEvent::CloseWindow(_) | HyprEvent::MoveWindow { .. } => true,
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
            HyprEvent::WindowTitle { .. }
            | HyprEvent::OpenLayer(_)
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
        assert!(!st.apply(&HyprEvent::OpenWindow {
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
}
