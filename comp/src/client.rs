//! Requests to edex-comp over its control socket.

use std::path::PathBuf;

use anyhow::{Context, Result};
use comp_proto::{EventReader, Rect, Request, Snapshot, WindowId, WorkspaceId, WorkspaceTarget};

/// The control socket of the running compositor.
#[derive(Clone, Debug)]
pub struct CompSocket {
    path: PathBuf,
}

impl CompSocket {
    /// The compositor named by `$EDEX_COMP_SOCKET` (or the default path), if it answers.
    pub fn from_env() -> Option<Self> {
        let path = comp_proto::socket_path()?;
        path.exists().then_some(Self { path })
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn call(&self, request: &Request) -> Result<serde_json::Value> {
        comp_proto::call_at(&self.path, request)
    }

    fn act(&self, request: Request) -> Result<()> {
        self.call(&request).map(|_| ())
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let v = self.call(&Request::State)?;
        serde_json::from_value(v).context("malformed state from edex-comp")
    }

    pub fn events(&self) -> Result<EventReader> {
        EventReader::connect(&self.path)
    }

    pub fn focus_workspace(&self, id: WorkspaceId) -> Result<()> {
        self.act(Request::FocusWorkspace {
            workspace: WorkspaceTarget::Id(id),
        })
    }

    pub fn focus_empty_workspace(&self) -> Result<()> {
        self.act(Request::FocusWorkspace {
            workspace: WorkspaceTarget::Empty,
        })
    }

    /// Start a program in the session.
    pub fn exec(&self, command: &str) -> Result<()> {
        self.act(Request::Exec {
            command: command.to_string(),
            env: Vec::new(),
        })
    }

    pub fn exit(&self) -> Result<()> {
        self.act(Request::Exit)
    }

    pub fn lock(&self) -> Result<()> {
        self.act(Request::Lock)
    }

    pub fn reload(&self) -> Result<()> {
        self.act(Request::Reload)
    }

    pub fn close_window(&self, id: WindowId) -> Result<()> {
        self.act(Request::Close { id: Some(id) })
    }

    pub fn toggle_maximized(&self, id: WindowId) -> Result<()> {
        self.act(Request::ToggleMaximize { id: Some(id) })
    }

    pub fn minimize_window(&self, id: WindowId) -> Result<()> {
        self.act(Request::Minimize { id: Some(id) })
    }

    pub fn restore_window(&self, id: WindowId, workspace: Option<WorkspaceId>) -> Result<()> {
        self.act(Request::Restore { id, workspace })
    }

    pub fn focus_window(&self, id: WindowId) -> Result<()> {
        self.act(Request::FocusWindow { id })
    }

    /// Tell the compositor where application windows go on `output`.
    pub fn set_app_area(&self, output: &str, tiled: Rect, maximized: Rect) -> Result<()> {
        self.act(Request::SetAppArea {
            output: output.to_string(),
            tiled,
            maximized,
        })
    }

    pub fn dpms(&self, on: bool) -> Result<()> {
        self.act(Request::Dpms { on })
    }

    pub fn screenshot(&self, path: &str, output: Option<&str>) -> Result<()> {
        self.act(Request::Screenshot {
            path: path.to_string(),
            output: output.map(str::to_string),
            region: None,
        })
    }

    /// The compositor's key bindings, one `KEYS → action` line each.
    pub fn binds(&self) -> Result<Vec<String>> {
        let v = self.call(&Request::Binds)?;
        Ok(v["binds"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn version(&self) -> Result<String> {
        let v = self.call(&Request::Version)?;
        Ok(v["version"].as_str().unwrap_or_default().to_string())
    }
}
