//! The window manager's state as the shell sees it, whatever the backend.

/// An output as the window manager names it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WmOutput {
    pub name: String,
    pub focused: bool,
    /// Visible workspace (Hyprland); `None` where the backend has no workspaces.
    pub active_workspace: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WmWorkspace {
    pub id: i32,
    pub name: String,
    pub output: String,
    pub windows: u32,
}

/// An application window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WmWindow {
    /// Backend-specific handle (a Hyprland address, a foreign-toplevel id).
    pub id: String,
    pub class: String,
    pub title: String,
    /// Workspace it is on; `None` where the backend has no workspaces (it is on the visible
    /// one).
    pub workspace: Option<i32>,
    /// Outputs it is on; empty when unknown (treated as every output).
    pub outputs: Vec<String>,
    /// Not tiled into the shell's terminal slot.
    pub floating: bool,
    /// eDEX's maximize: full width, side panels hidden.
    pub maximized: bool,
    pub fullscreen: bool,
    pub minimized: bool,
    pub active: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WmState {
    pub connected: bool,
    /// Backend name and version, e.g. ("labwc", "0.20.2").
    pub name: String,
    pub version: String,
    pub outputs: Vec<WmOutput>,
    pub workspaces: Vec<WmWorkspace>,
    /// Windows, oldest first.
    pub windows: Vec<WmWindow>,
    /// (class, title) of the focused window.
    pub active_window: Option<(String, String)>,
    pub keyboard_layout: String,
}

impl WmState {
    /// Visible workspace of the named output, or of the focused one when `None`.
    pub fn active_workspace_of(&self, output: Option<&str>) -> Option<i32> {
        self.outputs
            .iter()
            .find(|o| output.map_or(o.focused, |n| o.name == n))
            .or(self.outputs.first())
            .and_then(|o| o.active_workspace)
    }

    fn visible_on(&self, w: &WmWindow, output: Option<&str>) -> bool {
        if w.minimized {
            return false;
        }
        match w.workspace {
            Some(ws) => Some(ws) == self.active_workspace_of(output),
            None => output.is_none_or(|o| w.outputs.is_empty() || w.outputs.iter().any(|n| n == o)),
        }
    }

    /// Workspaces for the top bar (on the given output, or all), with active flags. Backends
    /// without workspaces show none.
    pub fn workspace_strip(&self, output: Option<&str>) -> Vec<ui::state::WorkspaceInfo> {
        if self.workspaces.is_empty() && self.outputs.iter().all(|o| o.active_workspace.is_none()) {
            return Vec::new();
        }
        let active_ids: Vec<i32> = self
            .outputs
            .iter()
            .filter(|o| output.is_none_or(|n| o.name == n))
            .filter_map(|o| o.active_workspace)
            .collect();
        let mut strip: Vec<ui::state::WorkspaceInfo> = self
            .workspaces
            .iter()
            .filter(|w| w.id > 0)
            .filter(|w| output.is_none_or(|n| w.output == n))
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

    /// Tabs for the windows visible on the focused output, then the minimized windows.
    pub fn window_tabs(&self) -> Vec<ui::state::WindowTab> {
        let tab = |w: &WmWindow| ui::state::WindowTab {
            address: w.id.clone(),
            class: w.class.clone(),
            title: w.title.clone(),
            active: w.active && !w.minimized,
            minimized: w.minimized,
            maximized: w.maximized,
            floating: w.floating,
        };
        let focused = self
            .outputs
            .iter()
            .find(|o| o.focused)
            .or(self.outputs.first())
            .map(|o| o.name.clone());
        let mut tabs: Vec<_> = self
            .windows
            .iter()
            .filter(|w| self.visible_on(w, focused.as_deref()))
            .map(tab)
            .collect();
        // Focus is not always reported (a window opened while the shell held the keyboard): a
        // lone window is the one the controls act on.
        if !tabs.iter().any(|t| t.active) && tabs.len() == 1 {
            tabs[0].active = true;
        }
        tabs.extend(self.windows.iter().filter(|w| w.minimized).map(tab));
        tabs
    }

    /// A window visible on this output is maximized: the shell hides its side panels so it gets
    /// the full width (the top bar and the window controls stay).
    pub fn maximized_on(&self, output: &str) -> bool {
        self.windows
            .iter()
            .any(|w| w.maximized && self.visible_on(w, Some(output)))
    }

    /// Tiled windows cover the terminal on this output.
    pub fn tiled_on(&self, output: &str) -> bool {
        self.windows
            .iter()
            .any(|w| !w.floating && self.visible_on(w, Some(output)))
    }

    pub fn window(&self, id: &str) -> Option<&WmWindow> {
        self.windows.iter().find(|w| w.id == id)
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

    fn win(id: &str, ws: Option<i32>, outputs: &[&str]) -> WmWindow {
        WmWindow {
            id: id.into(),
            class: "app".into(),
            title: format!("title {id}"),
            workspace: ws,
            outputs: outputs.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn workspaceless_windows_follow_their_outputs() {
        let mut st = WmState {
            connected: true,
            outputs: vec![
                WmOutput {
                    name: "Virtual-1".into(),
                    focused: true,
                    active_workspace: None,
                },
                WmOutput {
                    name: "Virtual-2".into(),
                    focused: false,
                    active_workspace: None,
                },
            ],
            windows: vec![
                win("1", None, &["Virtual-1"]),
                win("2", None, &["Virtual-2"]),
                win("3", None, &[]),
            ],
            ..Default::default()
        };
        st.windows[1].minimized = true;
        assert!(st.workspace_strip(None).is_empty());
        let ids: Vec<_> = st.window_tabs().into_iter().map(|t| t.address).collect();
        assert_eq!(ids, ["1", "3", "2"]);
        assert!(st.tiled_on("Virtual-1"));
        assert!(!st.maximized_on("Virtual-1"));
        st.windows[2].maximized = true;
        assert!(st.maximized_on("Virtual-2"));
        st.windows[0].floating = true;
        st.windows[2].floating = true;
        assert!(!st.tiled_on("Virtual-1"));
    }

    #[test]
    fn workspaces_filter_windows() {
        let st = WmState {
            connected: true,
            outputs: vec![WmOutput {
                name: "DP-1".into(),
                focused: true,
                active_workspace: Some(2),
            }],
            workspaces: vec![WmWorkspace {
                id: 2,
                name: "2".into(),
                output: "DP-1".into(),
                windows: 1,
            }],
            windows: vec![win("a", Some(1), &[]), win("b", Some(2), &[])],
            ..Default::default()
        };
        let strip = st.workspace_strip(Some("DP-1"));
        assert_eq!(strip.len(), 5);
        assert!(strip.iter().find(|w| w.id == 2).unwrap().active);
        let tabs = st.window_tabs();
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].address, "b");
        assert!(tabs[0].active, "a lone window is the active one");
    }
}
