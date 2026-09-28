//! `.socket.sock`: hyprctl-style requests. Each request opens a fresh connection and closes
//! it immediately, as the IPC documentation requires.

use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use crate::model::{MonitorInfo, WorkspaceInfo};

#[derive(Clone, Debug)]
pub struct HyprSocket {
    path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
struct RawWorkspaceRef {
    id: i32,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RawMonitor {
    id: i32,
    name: String,
    #[serde(default)]
    description: String,
    width: u32,
    height: u32,
    #[serde(rename = "refreshRate")]
    refresh_rate: f32,
    x: i32,
    y: i32,
    scale: f32,
    #[serde(default)]
    focused: bool,
    #[serde(rename = "activeWorkspace")]
    active_workspace: RawWorkspaceRef,
    #[serde(default)]
    transform: i32,
    #[serde(rename = "dpmsStatus", default = "default_true")]
    dpms: bool,
    #[serde(default)]
    disabled: bool,
    #[serde(rename = "availableModes", default)]
    available_modes: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
struct RawWorkspace {
    id: i32,
    name: String,
    monitor: String,
    windows: u32,
    #[serde(rename = "hasfullscreen", default)]
    has_fullscreen: bool,
    #[serde(rename = "lastwindowtitle", default)]
    last_window_title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActiveWindow {
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub floating: bool,
    #[serde(default)]
    pub fullscreen: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Bind {
    #[serde(default)]
    pub modmask: u32,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub dispatcher: String,
    #[serde(default)]
    pub arg: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Layer {
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub address: String,
}

impl HyprSocket {
    /// Connect to the running instance named by the environment.
    pub fn from_env() -> Option<Self> {
        let path = crate::instance_dir()?.join(".socket.sock");
        path.exists().then_some(Self { path })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Send a raw request (e.g. `j/monitors` or `dispatch workspace 2`).
    pub fn request(&self, command: &str) -> Result<String> {
        let mut stream = UnixStream::connect(&self.path)
            .with_context(|| format!("connecting to {}", self.path.display()))?;
        stream.set_read_timeout(Some(Duration::from_secs(4)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        stream.write_all(command.as_bytes())?;
        stream.flush()?;
        let mut out = String::new();
        stream.read_to_string(&mut out)?;
        Ok(out)
    }

    fn json<T: for<'de> Deserialize<'de>>(&self, command: &str) -> Result<T> {
        let raw = self.request(&format!("j/{command}"))?;
        serde_json::from_str(&raw).with_context(|| {
            format!(
                "parsing `{command}` reply: {}",
                raw.chars().take(200).collect::<String>()
            )
        })
    }

    pub fn dispatch(&self, dispatcher: &str) -> Result<()> {
        let reply = self.request(&format!("dispatch {dispatcher}"))?;
        if reply.trim() == "ok" {
            Ok(())
        } else {
            Err(anyhow!("dispatch `{dispatcher}` failed: {}", reply.trim()))
        }
    }

    /// Evaluate a Lua snippet in the compositor (`hyprctl eval`).
    pub fn eval(&self, lua: &str) -> Result<String> {
        self.request(&format!("eval {lua}"))
    }

    /// Apply a config option at runtime, e.g. `("general.gaps_in", "4")`.
    pub fn set_option(&self, key: &str, value: &str) -> Result<()> {
        let reply = self.eval(&format!("hl.config({{ [\"{key}\"] = {value} }})"))?;
        if reply.contains("error") || reply.contains("Error") {
            Err(anyhow!("setting {key}: {}", reply.trim()))
        } else {
            Ok(())
        }
    }

    pub fn reload(&self) -> Result<()> {
        self.request("reload").map(|_| ())
    }

    pub fn version(&self) -> Result<String> {
        let v: serde_json::Value = self.json("version")?;
        Ok(v.get("tag")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                v.get("commit")
                    .and_then(|c| c.as_str())
                    .unwrap_or("unknown")
                    .to_string()
            }))
    }

    pub fn monitors(&self) -> Result<Vec<MonitorInfo>> {
        let raw: Vec<RawMonitor> = self.json("monitors")?;
        Ok(raw
            .into_iter()
            .map(|m| MonitorInfo {
                id: m.id,
                name: m.name,
                description: m.description,
                width: m.width,
                height: m.height,
                refresh_rate: m.refresh_rate,
                x: m.x,
                y: m.y,
                scale: m.scale,
                focused: m.focused,
                active_workspace: m.active_workspace.id,
                active_workspace_name: m.active_workspace.name,
                transform: m.transform,
                dpms: m.dpms,
                disabled: m.disabled,
                available_modes: m.available_modes,
            })
            .collect())
    }

    pub fn workspaces(&self) -> Result<Vec<WorkspaceInfo>> {
        let raw: Vec<RawWorkspace> = self.json("workspaces")?;
        let mut ws: Vec<WorkspaceInfo> = raw
            .into_iter()
            .map(|w| WorkspaceInfo {
                id: w.id,
                name: w.name,
                monitor: w.monitor,
                windows: w.windows,
                has_fullscreen: w.has_fullscreen,
                last_window_title: w.last_window_title,
            })
            .collect();
        ws.sort_by_key(|w| w.id);
        Ok(ws)
    }

    pub fn active_window(&self) -> Result<Option<ActiveWindow>> {
        let raw = self.request("j/activewindow")?;
        if raw.trim() == "{}" || raw.trim().is_empty() {
            return Ok(None);
        }
        Ok(serde_json::from_str(&raw).ok())
    }

    pub fn binds(&self) -> Result<Vec<Bind>> {
        self.json("binds")
    }

    pub fn layers_namespaces(&self) -> Result<Vec<String>> {
        let v: serde_json::Value = self.json("layers")?;
        let mut out = Vec::new();
        fn walk(v: &serde_json::Value, out: &mut Vec<String>) {
            match v {
                serde_json::Value::Object(map) => {
                    if let Some(ns) = map.get("namespace").and_then(|n| n.as_str()) {
                        out.push(ns.to_string());
                    }
                    for child in map.values() {
                        walk(child, out);
                    }
                }
                serde_json::Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
                _ => {}
            }
        }
        walk(&v, &mut out);
        Ok(out)
    }

    /// Active keyboard layout name (from `devices`).
    pub fn active_layout(&self) -> Result<Option<String>> {
        let v: serde_json::Value = self.json("devices")?;
        Ok(v.get("keyboards")
            .and_then(|k| k.as_array())
            .and_then(|ks| {
                ks.iter()
                    .find(|k| k.get("main").and_then(|m| m.as_bool()).unwrap_or(false))
                    .or(ks.first())
            })
            .and_then(|k| {
                k.get("active_keymap")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string())
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_monitor_json() {
        let raw = r#"[{"id":0,"name":"DP-1","description":"AOC","width":2560,"height":1440,"refreshRate":165.0,"x":0,"y":0,"scale":1.0,"focused":true,"activeWorkspace":{"id":3,"name":"3"},"transform":0,"dpmsStatus":true,"disabled":false,"availableModes":["2560x1440@165.00Hz"]}]"#;
        let parsed: Vec<RawMonitor> = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed[0].active_workspace.id, 3);
        assert_eq!(parsed[0].available_modes.len(), 1);
    }
}
