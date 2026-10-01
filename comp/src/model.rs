//! Cached view of edex-comp's outputs, workspaces, windows and focus, kept current by events.

use comp_proto::{Event, Monitor, Snapshot, Window, WindowId, WindowMode, Workspace};

use crate::client::CompSocket;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompState {
    pub connected: bool,
    pub version: String,
    pub monitors: Vec<Monitor>,
    pub workspaces: Vec<Workspace>,
    pub windows: Vec<Window>,
    pub focused: Option<WindowId>,
    pub keyboard_layout: String,
    pub locked: bool,
}

impl CompState {
    pub fn unavailable() -> Self {
        Self::default()
    }

    pub fn from_snapshot(s: Snapshot) -> Self {
        Self {
            connected: true,
            version: s.version,
            monitors: s.monitors,
            workspaces: s.workspaces,
            windows: s.windows,
            focused: s.focused,
            keyboard_layout: s.keyboard_layout,
            locked: s.locked,
        }
    }

    /// Full resync from the control socket.
    pub fn resync(&mut self, socket: &CompSocket) {
        match socket.snapshot() {
            Ok(s) => *self = Self::from_snapshot(s),
            Err(e) => {
                tracing::warn!("edex-comp state query failed: {e:#}");
                self.connected = false;
            }
        }
    }

    /// Apply an event. Events carry complete records, so no resync is ever needed; the return
    /// value says whether anything the shell shows changed.
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::WindowOpened { window } | Event::WindowChanged { window } => {
                match self.windows.iter_mut().find(|w| w.id == window.id) {
                    Some(w) => *w = window.clone(),
                    None => self.windows.push(window.clone()),
                }
            }
            Event::WindowClosed { id } => {
                self.windows.retain(|w| w.id != *id);
                if self.focused == Some(*id) {
                    self.focused = None;
                }
            }
            Event::Focus { id } => self.focused = *id,
            Event::Workspace { monitor, workspace } => {
                if let Some(m) = self.monitors.iter_mut().find(|m| &m.name == monitor) {
                    m.active_workspace = *workspace;
                }
            }
            Event::Monitors { monitors } => self.monitors = monitors.clone(),
            Event::Workspaces { workspaces } => self.workspaces = workspaces.clone(),
            Event::KeyboardLayout { name } => self.keyboard_layout = name.clone(),
            Event::Locked { locked } => self.locked = *locked,
            Event::Bell | Event::ConfigReloaded => return false,
        }
        true
    }

    /// The focused window's app id and title.
    pub fn active_window(&self) -> Option<(String, String)> {
        let id = self.focused?;
        self.windows
            .iter()
            .find(|w| w.id == id)
            .map(|w| (w.app_id.clone(), w.title.clone()))
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
        let tab = |w: &Window| ui::state::WindowTab {
            address: w.id.to_string(),
            class: w.app_id.clone(),
            title: w.title.clone(),
            active: self.focused == Some(w.id) && !w.minimized,
            minimized: w.minimized,
            maximized: w.mode == WindowMode::Maximized,
            floating: w.mode == WindowMode::Floating,
        };
        let mut tabs: Vec<_> = self
            .windows
            .iter()
            .filter(|w| Some(w.workspace) == ws && !w.minimized)
            .map(tab)
            .collect();
        // A lone window is the one the controls act on, even while the shell has the keyboard.
        if !tabs.iter().any(|t| t.active) && tabs.len() == 1 {
            tabs[0].active = true;
        }
        tabs.extend(self.windows.iter().filter(|w| w.minimized).map(tab));
        tabs
    }

    /// A window on this monitor's visible workspace is maximized: the shell hides its side
    /// panels so it gets the full width (the top bar and the window controls stay).
    pub fn maximized_on(&self, monitor: &str) -> bool {
        let ws = self.active_workspace_of(Some(monitor));
        self.windows
            .iter()
            .any(|w| Some(w.workspace) == ws && !w.minimized && w.mode == WindowMode::Maximized)
    }

    /// Tiled windows cover the terminal on this monitor's visible workspace.
    pub fn tiled_on(&self, monitor: &str) -> bool {
        let ws = self.active_workspace_of(Some(monitor));
        self.windows
            .iter()
            .any(|w| Some(w.workspace) == ws && !w.minimized && w.mode != WindowMode::Floating)
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

/// Parse a tab's address back into a window id.
pub fn window_id(address: &str) -> Option<WindowId> {
    address.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor() -> Monitor {
        Monitor {
            name: "DP-1".into(),
            width: 1920,
            height: 1080,
            scale: 1.0,
            focused: true,
            active_workspace: 1,
            ..Default::default()
        }
    }

    fn win(id: WindowId, ws: i32, minimized: bool) -> Window {
        Window {
            id,
            app_id: "app".into(),
            title: format!("title {id}"),
            workspace: ws,
            minimized,
            ..Default::default()
        }
    }

    #[test]
    fn events_update_state() {
        let mut st = CompState {
            connected: true,
            monitors: vec![monitor()],
            ..Default::default()
        };
        assert!(st.apply(&Event::Workspace {
            monitor: "DP-1".into(),
            workspace: 2
        }));
        assert_eq!(st.monitors[0].active_workspace, 2);
        st.apply(&Event::Workspaces {
            workspaces: vec![Workspace {
                id: 2,
                name: "2".into(),
                monitor: "DP-1".into(),
                windows: 1,
                has_fullscreen: false,
            }],
        });
        let strip = st.workspace_strip(Some("DP-1"));
        assert_eq!(strip.len(), 5);
        assert!(strip.iter().find(|w| w.id == 2).unwrap().active);
        st.apply(&Event::KeyboardLayout {
            name: "English (US)".into(),
        });
        assert_eq!(st.short_layout(), "US");
        assert!(!st.apply(&Event::Bell));
    }

    #[test]
    fn window_tabs_list_the_workspace_then_minimized() {
        let mut st = CompState {
            connected: true,
            monitors: vec![monitor()],
            ..Default::default()
        };
        for w in [
            win(1, 1, false),
            win(2, 1, true),
            win(3, 2, false),
            win(4, 1, false),
        ] {
            st.apply(&Event::WindowOpened { window: w });
        }
        st.apply(&Event::Focus { id: Some(4) });
        let tabs = st.window_tabs();
        let ids: Vec<_> = tabs.iter().map(|t| t.address.as_str()).collect();
        assert_eq!(ids, ["1", "4", "2"]);
        assert!(tabs[1].active && !tabs[0].active);
        assert!(tabs[2].minimized);
        assert!(st.tiled_on("DP-1"));
        assert!(!st.maximized_on("DP-1"));
        let mut w4 = win(4, 1, false);
        w4.mode = WindowMode::Maximized;
        st.apply(&Event::WindowChanged { window: w4 });
        assert!(st.maximized_on("DP-1"));
        assert_eq!(st.active_window().unwrap().1, "title 4");
        st.apply(&Event::WindowClosed { id: 4 });
        assert_eq!(st.focused, None);
        assert_eq!(window_id("17"), Some(17));
    }
}
