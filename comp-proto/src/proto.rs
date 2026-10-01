//! Request, reply and event schema, and the state records they carry.

use serde::{Deserialize, Serialize};

/// Window identifier, stable for the window's lifetime and never reused within a session.
pub type WindowId = u64;

/// Workspace id. Regular workspaces are `1..=wm.workspaces`; the special ones are negative.
pub type WorkspaceId = i32;

/// Holds minimized windows; they show up as tabs in the shell's strip.
pub const MINIMIZED_WORKSPACE: WorkspaceId = -1;
/// The scratchpad (SUPER+S).
pub const SCRATCH_WORKSPACE: WorkspaceId = -2;

/// Rectangle in logical (scaled) output coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Monitor {
    pub name: String,
    /// Make, model and serial from the EDID.
    pub description: String,
    /// Current mode in pixels.
    pub width: u32,
    pub height: u32,
    pub refresh_rate: f32,
    /// Position in the global logical space.
    pub x: i32,
    pub y: i32,
    pub scale: f32,
    /// wl_output transform: 0 normal, 1 90°, 2 180°, 3 270°, 4–7 flipped.
    pub transform: u32,
    pub focused: bool,
    pub active_workspace: WorkspaceId,
    pub dpms: bool,
    pub disabled: bool,
    /// Modes as `WIDTHxHEIGHT@HZ`.
    pub modes: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    /// Output the workspace lives on.
    pub monitor: String,
    pub windows: u32,
    pub has_fullscreen: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowMode {
    #[default]
    Tiled,
    Floating,
    /// Fills the maximized area the shell reported (side panels hidden, tab strip visible).
    Maximized,
    /// Covers the whole output, above the shell.
    Fullscreen,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub id: WindowId,
    /// xdg app_id, or the X11 WM_CLASS for Xwayland windows.
    pub app_id: String,
    pub title: String,
    pub workspace: WorkspaceId,
    pub mode: WindowMode,
    /// Set while the window sits in [`MINIMIZED_WORKSPACE`]; `workspace` is the one it returns to.
    pub minimized: bool,
    pub urgent: bool,
    pub xwayland: bool,
    pub pid: Option<i32>,
}

/// Everything a client needs to draw the workspace strip and window tabs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: String,
    pub monitors: Vec<Monitor>,
    pub workspaces: Vec<Workspace>,
    pub windows: Vec<Window>,
    pub focused: Option<WindowId>,
    /// Long xkb name of the active layout, e.g. "English (US)".
    pub keyboard_layout: String,
    pub locked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Workspace selector for focus and move requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind", content = "id")]
pub enum WorkspaceTarget {
    Id(WorkspaceId),
    Next,
    Prev,
    /// The first workspace without windows.
    Empty,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    /// Replies `{"version": …}`.
    Version,
    /// Replies with a [`Snapshot`].
    State,
    /// Turn this connection into an event stream.
    Subscribe,

    // Windows. `id: None` acts on the focused window.
    FocusWindow {
        id: WindowId,
    },
    Close {
        id: Option<WindowId>,
    },
    /// SIGKILL the client (or disconnect it when the pid is unknown).
    Kill {
        id: Option<WindowId>,
    },
    Minimize {
        id: Option<WindowId>,
    },
    /// Bring a minimized window back; to `workspace`, else the one it was minimized from.
    Restore {
        id: WindowId,
        workspace: Option<WorkspaceId>,
    },
    ToggleMaximize {
        id: Option<WindowId>,
    },
    ToggleFullscreen {
        id: Option<WindowId>,
    },
    ToggleFloat {
        id: Option<WindowId>,
    },
    MoveToWorkspace {
        id: Option<WindowId>,
        workspace: WorkspaceTarget,
        follow: bool,
    },
    FocusDirection {
        direction: Direction,
    },
    MoveDirection {
        direction: Direction,
    },
    CycleFocus {
        reverse: bool,
    },
    FocusWorkspace {
        workspace: WorkspaceTarget,
    },
    ToggleScratch,

    /// The shell's layout for one output: tiled windows fill `tiled` (the terminal slot);
    /// maximized windows fill `maximized` (the slot with the side panels hidden). Empty rects
    /// mean "the whole output".
    SetAppArea {
        output: String,
        tiled: Rect,
        maximized: Rect,
    },

    /// Start a program in the session (`sh -c command`) with the session environment.
    Exec {
        command: String,
        #[serde(default)]
        env: Vec<(String, String)>,
    },
    /// Lock the session (starts the lock screen client).
    Lock,
    /// End the session.
    Exit,
    /// Re-read `config.toml` (input, outputs, look, binds, idle, night light).
    Reload,
    /// Turn every output on or off.
    Dpms {
        on: bool,
    },
    /// Write a PNG of one output (`None` = the focused one) or of a region of the global space.
    Screenshot {
        path: String,
        output: Option<String>,
        region: Option<Rect>,
    },

    /// Greeter mode only: authenticate and, on success, start `session` (a name from
    /// `/usr/share/wayland-sessions`, or `None` for the eDEX session) as `user`. Lock mode: unlock.
    Login {
        user: String,
        password: String,
        session: Option<String>,
    },
    /// Greeter mode only.
    PowerOff,
    /// Greeter mode only.
    Reboot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Reply {
    pub fn ok() -> Self {
        Self {
            ok: true,
            data: None,
            error: None,
        }
    }

    pub fn with_data(data: serde_json::Value) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    WindowOpened {
        window: Window,
    },
    WindowClosed {
        id: WindowId,
    },
    /// Title, mode, workspace, minimized or urgent changed.
    WindowChanged {
        window: Window,
    },
    Focus {
        id: Option<WindowId>,
    },
    /// `monitor` now shows `workspace`.
    Workspace {
        monitor: String,
        workspace: WorkspaceId,
    },
    /// Monitors were added, removed or reconfigured, or the focused one changed.
    Monitors {
        monitors: Vec<Monitor>,
    },
    Workspaces {
        workspaces: Vec<Workspace>,
    },
    KeyboardLayout {
        name: String,
    },
    /// A client rang the bell (xdg-system-bell).
    Bell,
    ConfigReloaded,
    Locked {
        locked: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        let reqs = [
            Request::State,
            Request::Close { id: Some(7) },
            Request::MoveToWorkspace {
                id: None,
                workspace: WorkspaceTarget::Id(3),
                follow: true,
            },
            Request::FocusWorkspace {
                workspace: WorkspaceTarget::Next,
            },
            Request::SetAppArea {
                output: "DP-1".into(),
                tiled: Rect::new(400, 40, 1120, 800),
                maximized: Rect::new(0, 40, 1920, 800),
            },
            Request::Exec {
                command: "foot".into(),
                env: vec![("A".into(), "b".into())],
            },
            Request::Login {
                user: "root".into(),
                password: "x".into(),
                session: None,
            },
        ];
        for r in reqs {
            let line = serde_json::to_string(&r).unwrap();
            assert!(!line.contains('\n'));
            assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), r);
        }
    }

    #[test]
    fn wire_format_is_stable() {
        assert_eq!(
            serde_json::to_string(&Request::Minimize { id: Some(3) }).unwrap(),
            r#"{"cmd":"minimize","id":3}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::FocusWorkspace {
                workspace: WorkspaceTarget::Id(2)
            })
            .unwrap(),
            r#"{"cmd":"focus-workspace","workspace":{"kind":"id","id":2}}"#
        );
        assert_eq!(
            serde_json::to_string(&Event::Focus { id: None }).unwrap(),
            r#"{"event":"focus","id":null}"#
        );
        // Optional fields may be left out by scripts.
        let r: Request = serde_json::from_str(r#"{"cmd":"exec","command":"foot"}"#).unwrap();
        assert_eq!(
            r,
            Request::Exec {
                command: "foot".into(),
                env: vec![]
            }
        );
    }

    #[test]
    fn events_round_trip() {
        let w = Window {
            id: 1,
            app_id: "foot".into(),
            title: "~".into(),
            workspace: 1,
            mode: WindowMode::Maximized,
            ..Default::default()
        };
        for e in [
            Event::WindowOpened { window: w.clone() },
            Event::WindowChanged { window: w },
            Event::Workspace {
                monitor: "DP-1".into(),
                workspace: 2,
            },
            Event::Locked { locked: true },
        ] {
            let line = serde_json::to_string(&e).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), e);
        }
    }
}
